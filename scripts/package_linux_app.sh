#!/usr/bin/env bash
# Package a relocated, portable Linux x64 directory from already-built binaries.
# This script never builds; callers produce the release binaries first.
set -euo pipefail

BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target/linux-app}"
RELEASE_SUBDIR="${RELEASE_SUBDIR:-release}"
DIST_DIR="${DIST_DIR:-dist/spektrafilm-linux-x64}"
APP_NAME="$(basename "$DIST_DIR")"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail() { echo "linux packaging: $*" >&2; exit 1; }
package_path="$(realpath -m -- "$DIST_DIR")"
case "$package_path" in
    /|"$repo_root"|"$repo_root"/scripts|"$repo_root"/data|"$repo_root"/crates)
        fail "DIST_DIR must name a generated package directory: $DIST_DIR" ;;
esac
[[ "$package_path" != "$(realpath -m -- "$BUILD_TARGET_DIR")" ]] ||
    fail "DIST_DIR must differ from BUILD_TARGET_DIR"
DIST_DIR="$package_path"
canonical_file() {
    local resolved
    resolved="$(readlink -f -- "$1")" || return 1
    [[ -f "$resolved" ]] || return 1
    printf '%s\n' "$resolved"
}

for tool in patchelf readelf ldd tar; do
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
done

pkg-config --atleast-version=0.22.0 libraw ||
    fail "LibRaw >= 0.22.0 is required (found $(pkg-config --modversion libraw 2>/dev/null || echo none))"
for module in lensfun exiv2 glib-2.0 OpenImageIO; do
    pkg-config --exists "$module" || fail "native development dependency missing: $module"
done

release_dir="$BUILD_TARGET_DIR/$RELEASE_SUBDIR"
[[ -d "$release_dir" ]] || fail "release directory missing: $release_dir"
for executable in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    [[ -x "$release_dir/$executable" ]] || fail "built executable missing: $release_dir/$executable"
done

library_dirs=()
for module in libraw lensfun exiv2 glib-2.0 OpenImageIO; do
    directory="$(pkg-config --variable=libdir "$module" 2>/dev/null || true)"
    [[ -z "$directory" ]] || library_dirs+=("$directory")
done

loader_cache="${TMPDIR:-/tmp}/spektrafilm-ldconfig.$$"
trap 'rm -f "$loader_cache"' EXIT
ldconfig -p >"$loader_cache" 2>/dev/null || fail "cannot read the system loader cache"

elf_needed() {
    readelf -d "$1" | awk '$2 == "(NEEDED)" { sub(/^.*\[/, ""); sub(/\]$/, ""); print }'
}

elf_rpaths() {
    readelf -d "$1" | awk '$2 == "(RPATH)" || $2 == "(RUNPATH)" { sub(/^.*\[/, ""); sub(/\]$/, ""); print }'
}

expand_loader_token() {
    local path="$1" loader="$2"
    case "$path" in
        '$ORIGIN'*) printf '%s%s\n' "$(dirname "$loader")" "${path#'$ORIGIN'}" ;;
        '${ORIGIN}'*) printf '%s%s\n' "$(dirname "$loader")" "${path#'${ORIGIN}'}" ;;
        *) printf '%s\n' "$path" ;;
    esac
}

resolve_dependency() {
    local dependency="$1" loader="$2" candidate directory expanded rpath
    while IFS= read -r rpath; do
        [[ -n "$rpath" ]] || continue
        expanded="$(expand_loader_token "$rpath" "$loader")"
        candidate="$expanded/$dependency"
        [[ -f "$candidate" ]] && { canonical_file "$candidate"; return; }
    done < <(elf_rpaths "$loader" | tr ':' '\n')
    for directory in ${library_dirs[@]+"${library_dirs[@]}"}; do
        candidate="$directory/$dependency"
        [[ -f "$candidate" ]] && { canonical_file "$candidate"; return; }
    done
    candidate="$(awk -v library="$dependency" '$1 == library { print $NF; exit }' "$loader_cache")"
    [[ -n "$candidate" ]] && { canonical_file "$candidate"; return; }
    return 1
}

system_library() {
    case "$1" in
        libc.so.*|libm.so.*|ld-linux*.so.*|libpthread.so.*|libdl.so.*|librt.so.*|libresolv.so.*|libutil.so.*|libnsl.so.*|libnss_*) return 0 ;;
        *) return 1 ;;
    esac
}

internal_library() {
    case "$1" in
        libspektrafilm*) return 0 ;;
        *) return 1 ;;
    esac
}

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/bin" "$DIST_DIR/lib/spektrafilm" "$DIST_DIR/share/licenses"

for executable in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    canonical_file "$release_dir/$executable" >"$DIST_DIR/.source.$executable"
    cp "$(cat "$DIST_DIR/.source.$executable")" "$DIST_DIR/lib/spektrafilm/$executable"
    chmod 0755 "$DIST_DIR/lib/spektrafilm/$executable"
    rm "$DIST_DIR/.source.$executable"
done

declare -A dependency_sources
queue=()
for executable in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    queue+=("$(canonical_file "$release_dir/$executable")")
done
# winit/xkbcommon-dl loads these through dlopen, so they have no ELF NEEDED edge.
for dependency in libxkbcommon.so.0 libxkbcommon-x11.so.0; do
    source_path="$(resolve_dependency "$dependency" "$release_dir/spektrafilm-gui" || true)"
    [[ -n "$source_path" ]] || fail "GUI runtime provider missing: $dependency"
    dependency_sources["$dependency"]="$source_path"
    cp "$source_path" "$DIST_DIR/lib/$dependency"
    queue+=("$source_path")
done
ln -s libxkbcommon.so.0 "$DIST_DIR/lib/libxkbcommon.so"
ln -s libxkbcommon-x11.so.0 "$DIST_DIR/lib/libxkbcommon-x11.so"
while ((${#queue[@]})); do
    next_queue=()
    for loader in "${queue[@]}"; do
        readelf -h "$loader" >/dev/null 2>&1 || fail "not an ELF file: $loader"
        while IFS= read -r dependency; do
            system_library "$dependency" && continue
            source_path="$(resolve_dependency "$dependency" "$loader" || true)"
            [[ -n "${source_path:-}" ]] || fail "unresolved native dependency $dependency needed by $loader"
            if [[ -n "${dependency_sources[$dependency]:-}" ]]; then
                [[ "${dependency_sources[$dependency]}" == "$source_path" ]] ||
                    fail "conflicting providers for $dependency: ${dependency_sources[$dependency]} and $source_path"
                continue
            fi
            dependency_sources["$dependency"]="$source_path"
            cp "$source_path" "$DIST_DIR/lib/$dependency"
            chmod 0755 "$DIST_DIR/lib/$dependency"
            next_queue+=("$source_path")
        done < <(elf_needed "$loader")
    done
    queue=("${next_queue[@]+"${next_queue[@]}"}")
done

for elf in "$DIST_DIR"/lib/spektrafilm/* "$DIST_DIR"/lib/*; do
    [[ -f "$elf" ]] || continue
    patchelf --remove-rpath "$elf" ||
        fail "cannot remove the original RPATH from $elf"
    case "$elf" in
        "$DIST_DIR"/lib/spektrafilm/*) patchelf --set-rpath '$ORIGIN/..' "$elf" ;;
        *) patchelf --set-rpath '$ORIGIN' "$elf" ;;
    esac || fail "cannot set the origin RPATH on $elf"
done

for elf in "$DIST_DIR"/lib/*; do
    [[ -f "$elf" ]] || continue
    actual="$(elf_rpaths "$elf")"
    [[ "$actual" == '$ORIGIN' ]] ||
        fail "unexpected RPATH on $elf: expected \$ORIGIN, got ${actual:-none}"
done

for executable in "$DIST_DIR"/lib/spektrafilm/*; do
    unresolved="$(LD_LIBRARY_PATH= ldd "$executable" 2>&1 || true)"
    if grep -q 'not found' <<<"$unresolved"; then
        fail "loader verification failed for $executable: $unresolved"
    fi
done

cp -a data "$DIST_DIR/share/data"

lensfun_database="${SPEKTRAFILM_LENSFUN_DATABASE:-}"
if [[ -z "$lensfun_database" ]]; then
    data_root="$(pkg-config --variable=datadir lensfun)"
    for candidate in "$data_root/lensfun/version_1" /var/lib/lensfun-updates/version_1 /usr/share/lensfun/version_1; do
        if [[ -d "$candidate" ]]; then lensfun_database="$candidate"; break; fi
    done
fi
[[ -n "$lensfun_database" && -d "$lensfun_database" ]] || fail "Lensfun database directory missing"
mkdir -p "$DIST_DIR/share/lensfun"
database_files=0
for file in "$lensfun_database"/*.xml; do
    [[ -f "$file" ]] || continue
    cp "$file" "$DIST_DIR/share/lensfun/"
    database_files=$((database_files + 1))
done
((database_files > 0)) || fail "no Lensfun XML files in $lensfun_database"

cp LICENSE "$DIST_DIR/share/licenses/spektrafilm-GPL-3.0.txt"
[[ -s licenses/lensfun-cc-by-sa-3.0.txt ]] || fail "full Lensfun database license missing"
mkdir -p "$DIST_DIR/share/licenses/lensfun-database"
cp licenses/lensfun-cc-by-sa-3.0.txt "$DIST_DIR/share/licenses/lensfun-database/LICENSE-CC-BY-SA-3.0.txt"

libraw_prefix="$(pkg-config --variable=prefix libraw)"
declare -A licensed_packages=()
for dependency in "${!dependency_sources[@]}"; do
    source_path="${dependency_sources[$dependency]}"
    if internal_library "$dependency"; then continue; fi
    package=""
    if command -v dpkg-query >/dev/null 2>&1; then
        package="$(dpkg-query -S "$source_path" 2>/dev/null | awk 'NR == 1 { print $1; exit }' || true)"
    fi
    if [[ -n "$package" ]]; then
        package_name="${package%%:*}"
        [[ -n "${licensed_packages[$package_name]:-}" ]] && continue
        copyright="/usr/share/doc/$package_name/copyright"
        [[ -s "$copyright" ]] || fail "license missing for native package $package_name"
        mkdir -p "$DIST_DIR/share/licenses/native/$package_name"
        cp "$copyright" "$DIST_DIR/share/licenses/native/$package_name/copyright"
        licensed_packages["$package_name"]=1
        continue
    fi
    case "$dependency" in
        libraw*)
            documentation="${SPEKTRAFILM_LIBRAW_DOCS:-$libraw_prefix/share/doc/libraw}"
            for license in COPYRIGHT LICENSE.CDDL LICENSE.LGPL; do
                [[ -s "$documentation/$license" ]] || fail "LibRaw license text missing: $documentation/$license"
                mkdir -p "$DIST_DIR/share/licenses/native/libraw"
                cp "$documentation/$license" "$DIST_DIR/share/licenses/native/libraw/$license"
            done
            ;;
        *) fail "no license provenance for native dependency $dependency ($source_path)" ;;
    esac
done

for required in libraw lensfun exiv2 glib openimageio; do
    found=0
    for dependency in "${!dependency_sources[@]}"; do
        [[ "${dependency,,}" == *"$required"* ]] && { found=1; break; }
    done
    ((found == 1)) || fail "required native dependency was not bundled: $required"
done

for executable in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    launcher="$DIST_DIR/bin/$executable"
    cat >"$launcher" <<LAUNCHER
#!/bin/sh
set -eu
root=\$(CDPATH= cd -- "\$(dirname -- "\$0")/.." && pwd)
export LD_LIBRARY_PATH="\$root/lib"
export SPEKTRAFILM_DATA_DIR="\$root/share/data"
export SPEKTRAFILM_LENSFUN_DATABASE="\$root/share/lensfun"
export SPEKTRAFILM_F64_CLI="\$root/lib/spektrafilm/spektrafilm-f64"
exec "\$root/lib/spektrafilm/$executable" "\$@"
LAUNCHER
    chmod 0755 "$launcher"
done

mkdir -p "$(dirname "$DIST_DIR")"
tar --sort=name --owner=0 --group=0 --numeric-owner \
    -czf "$DIST_DIR.tar.gz" -C "$(dirname "$DIST_DIR")" "$APP_NAME"
echo "Packaged: $DIST_DIR.tar.gz"
