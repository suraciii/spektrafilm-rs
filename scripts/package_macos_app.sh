#!/usr/bin/env bash
set -euo pipefail

ICON_SOURCE="${ICON_SOURCE:-assets/spektrafilm-icon.jpg}"
BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target/macos-app}"
DIST_DIR="${DIST_DIR:-dist/spektrafilm-macos}"
APP_NAME="${APP_NAME:-Spektrafilm}"
APP_BUNDLE="$DIST_DIR/$APP_NAME.app"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail() { echo "macOS packaging: $*" >&2; exit 1; }

# Resolve symlinks so Cellar ownership and duplicate library names are unambiguous.
canonical_file() {
    local path="$1" target
    while [[ -L "$path" ]]; do
        target="$(readlink "$path")"
        case "$target" in
            /*) path="$target" ;;
            *) path="$(dirname "$path")/$target" ;;
        esac
    done
    [[ -f "$path" ]] || return 1
    printf '%s/%s\n' "$(cd "$(dirname "$path")" && pwd -P)" "$(basename "$path")"
}

binary_rpaths() {
    local metadata
    metadata="$(otool -l "$1")" || fail "cannot inspect load commands in $1"
    printf '%s\n' "$metadata" | awk '
        $1 == "cmd" { rpath = ($2 == "LC_RPATH") }
        rpath && $1 == "path" { sub(/^[[:space:]]*path /, ""); sub(/ \(offset.*$/, ""); print; rpath = 0 }
    '
}

expand_loader_path() {
    local path="$1" loader="$2" executable="$3"
    case "$path" in
        @loader_path*) printf '%s%s\n' "$(dirname "$loader")" "${path#@loader_path}" ;;
        @executable_path*) printf '%s%s\n' "$(dirname "$executable")" "${path#@executable_path}" ;;
        /*) printf '%s\n' "$path" ;;
        *) return 1 ;;
    esac
}

resolve_dependency() {
    local dependency="$1" loader="$2" executable="$3" inherited="$4"
    local candidate rpath expanded
    case "$dependency" in
        @rpath/*)
            while IFS= read -r rpath; do
                [[ -n "$rpath" ]] || continue
                expanded="$(expand_loader_path "$rpath" "$loader" "$executable")" || continue
                candidate="$expanded/${dependency#@rpath/}"
                if [[ -f "$candidate" ]]; then canonical_file "$candidate"; return; fi
            done < <(binary_rpaths "$loader"; printf '%s\n' "$inherited")
            for rpath in ${library_dirs[@]+"${library_dirs[@]}"}; do
                candidate="$rpath/${dependency#@rpath/}"
                if [[ -f "$candidate" ]]; then canonical_file "$candidate"; return; fi
            done
            ;;
        @loader_path/*|@executable_path/*|/*)
            candidate="$(expand_loader_path "$dependency" "$loader" "$executable")" || return 1
            if [[ -f "$candidate" ]]; then canonical_file "$candidate"; return; fi
            ;;
    esac
    return 1
}

collect_library_dirs() {
    library_dirs=()
    local module directory formula prefix
    for module in libraw lensfun exiv2 glib-2.0 OpenImageIO; do
        directory="$(pkg-config --variable=libdir "$module" 2>/dev/null || true)"
        [[ -z "$directory" ]] || library_dirs+=("$directory")
    done
    if command -v brew >/dev/null 2>&1; then
        prefix="$(brew --prefix)"
        library_dirs+=("$prefix/lib")
        for formula in libraw lensfun exiv2 glib openimageio; do
            prefix="$(brew --prefix "$formula" 2>/dev/null || true)"
            [[ -z "$prefix" ]] || library_dirs+=("$prefix/lib")
        done
    fi
}

package_native_dependencies() {
    local frameworks="$APP_BUNDLE/Contents/Frameworks"
    local source destination executable inherited dependency resolved name replacement rpath expanded
    local index=0 existing found id dependencies rpaths
    local sources=() destinations=() executables=() inherited_paths=()
    bundled_sources=()
    mkdir -p "$frameworks"
    for name in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
        source="$(canonical_file "$BUILD_TARGET_DIR/release/$name")" || fail "missing executable $name"
        sources+=("$source")
        destinations+=("$APP_BUNDLE/Contents/MacOS/$name")
        executables+=("$source")
        inherited_paths+=("")
    done
    while (( index < ${#sources[@]} )); do
        source="${sources[$index]}"
        destination="${destinations[$index]}"
        executable="${executables[$index]}"
        inherited="${inherited_paths[$index]}"
        # Inherited runpaths are expanded at their defining loader, not at its child.
        rpaths="$(binary_rpaths "$source")" || fail "cannot inspect runpaths in $source"
        dependencies="$(otool -L "$source")" || fail "cannot inspect dependencies in $source"
        while IFS= read -r rpath; do
            [[ -n "$rpath" ]] || continue
            expanded="$(expand_loader_path "$rpath" "$source" "$executable")" || continue
            inherited="$inherited
$expanded"
        done <<< "$rpaths"
        id=""
        if [[ "$source" == *.dylib ]]; then
            id="$(otool -D "$source" | awk 'NR == 2 {print}')" || fail "cannot inspect dylib ID in $source"
        fi
        while IFS= read -r dependency; do
            [[ -n "$dependency" && "$dependency" != "$id" ]] || continue
            case "$dependency" in /usr/lib/*|/System/*) continue ;; esac
            resolved="$(resolve_dependency "$dependency" "$source" "$executable" "$inherited")" ||
                fail "unresolved dependency $dependency in $source"
            case "$resolved" in /usr/lib/*|/System/*) continue ;; esac
            name="$(basename "$resolved")"
            found=0
            for existing in ${bundled_sources[@]+"${bundled_sources[@]}"}; do
                if [[ "$(basename "$existing")" == "$name" ]]; then
                    [[ "$existing" == "$resolved" ]] || fail "library name collision: $existing and $resolved"
                    found=1
                    break
                fi
            done
            if (( found == 0 )); then
                cp "$resolved" "$frameworks/$name"
                chmod u+w "$frameworks/$name"
                bundled_sources+=("$resolved")
                sources+=("$resolved")
                destinations+=("$frameworks/$name")
                executables+=("$executable")
                inherited_paths+=("$inherited")
            fi
            if [[ "$destination" == "$APP_BUNDLE/Contents/MacOS/"* ]]; then
                replacement="@executable_path/../Frameworks/$name"
            else
                replacement="@loader_path/$name"
            fi
            install_name_tool -change "$dependency" "$replacement" "$destination"
        done < <(printf '%s\n' "$dependencies" | awk 'NR > 1 { sub(/^[ \t]+/, ""); sub(/ \(compatibility version.*$/, ""); print }')
        if [[ "$destination" == "$frameworks/"* ]]; then
            install_name_tool -id "@loader_path/$(basename "$destination")" "$destination"
        fi
        # Original LC_RPATH values can expose developer paths and are unnecessary after rewriting.
        while IFS= read -r rpath; do
            [[ -n "$rpath" ]] || continue
            install_name_tool -delete_rpath "$rpath" "$destination"
        done <<< "$rpaths"
        index=$((index + 1))
    done
}

bundle_lensfun_database() {
    local database="${SPEKTRAFILM_LENSFUN_DATABASE:-}" datadir prefix file found=0
    if [[ -z "$database" ]]; then
        datadir="$(pkg-config --variable=datadir lensfun 2>/dev/null || true)"
        for database in "$datadir/lensfun/version_1" "$datadir/version_1" "$datadir/lensfun"; do
            [[ -n "$datadir" && -d "$database" ]] && break
            database=""
        done
        if [[ -z "$database" ]] && command -v brew >/dev/null 2>&1; then
            prefix="$(brew --prefix lensfun)"
            database="$prefix/share/lensfun/version_1"
        fi
    fi
    [[ -d "$database" ]] || fail "Lensfun XML database missing; set SPEKTRAFILM_LENSFUN_DATABASE"
    lensfun_database_directory="$database"
    mkdir -p "$APP_BUNDLE/Contents/Resources/lensfun"
    for file in "$database"/*.xml; do
        [[ -f "$file" ]] || continue
        cp "$file" "$APP_BUNDLE/Contents/Resources/lensfun/"
        found=1
    done
    (( found == 1 )) || fail "no Lensfun XML files in $database"
}

bundle_native_licenses() {
    local output="$APP_BUNDLE/Contents/Resources/licenses"
    local source cellar formula version root file relative found required directory
    local roots=() seen
    mkdir -p "$output"
    cp LICENSE "$output/Spektrafilm-LICENSE"
    for source in ${bundled_sources[@]+"${bundled_sources[@]}"}; do
        case "$source" in
            */Cellar/*)
                cellar="${source%%/Cellar/*}/Cellar"
                relative="${source#"$cellar/"}"
                formula="${relative%%/*}"
                version="${relative#*/}"; version="${version%%/*}"
                root="$cellar/$formula/$version"
                ;;
            *) fail "cannot locate Homebrew license provenance for $source" ;;
        esac
        seen=0
        for directory in ${roots[@]+"${roots[@]}"}; do [[ "$directory" != "$root" ]] || seen=1; done
        (( seen == 0 )) || continue
        roots+=("$root")
        found=0
        # Homebrew installs resolved metafiles under their target basename:
        # https://github.com/Homebrew/brew/blob/2170a64c0ff9549d78a9b48b26217d9fd17f6a2d/Library/Homebrew/extend/pathname.rb#L375
        # GLib 2.88.3 COPYING -> LICENSES/LGPL-2.1-or-later.txt therefore
        # becomes LGPL-2.1-or-later.txt at the keg root. Include SPDX filenames
        # and LICENSES directories, copying readable symlinks as full text.
        while IFS= read -r -d '' file; do
            [[ -f "$file" && -s "$file" ]] || continue
            case "$(basename "$file")" in
                [Rr][Ee][Aa][Dd][Mm][Ee]*)
                    awk 'tolower($0) ~ /license|copyright/ { found = 1 } END { exit !found }' "$file" || continue
                    ;;
            esac
            relative="${file#"$root/"}"
            mkdir -p "$output/$formula/$(dirname "$relative")"
            cp "$file" "$output/$formula/$relative"
            found=1
        done < <(find "$root" \( -type f -o -type l \) \( -iname '*LICENSE*' -o -iname '*LICENCE*' -o -iname '*COPYING*' -o -iname '*COPYRIGHT*' -o -iname '*GPL*.txt' -o -iname 'Apache-*.txt' -o -iname 'MIT.txt' -o -iname 'BSD-*.txt' -o -iname 'MPL-*.txt' -o -iname 'ISC.txt' -o -iname 'CC0-*.txt' -o -iname 'CC-BY-*.txt' -o -ipath '*/LICENSES/*' -o -iname 'README*' \) -print0)
        # Keep the installed formula's SPDX declaration and source provenance as well.
        if [[ -d "$root/.brew" ]]; then cp -R "$root/.brew" "$output/$formula/"; fi
        (( found == 1 )) || fail "missing native license files for $formula ($root)"
    done
    for required in libraw lensfun exiv2 glib openimageio; do
        [[ -d "$output/$required" ]] || fail "missing required core license: $required"
    done
    # Database licensing is separate from the Lensfun library's LGPL license.
    # Exact upstream text from lensfun v0.3.4 docs/cc-by-sa-3.0.txt.
    [[ -s licenses/lensfun-cc-by-sa-3.0.txt ]] || fail "missing full Lensfun database license text"
    mkdir -p "$output/lensfun-database"
    cp licenses/lensfun-cc-by-sa-3.0.txt "$output/lensfun-database/LICENSE-CC-BY-SA-3.0.txt"
    for directory in "$lensfun_database_directory" "$(dirname "$lensfun_database_directory")" \
        "$(dirname "$(dirname "$lensfun_database_directory")")"; do
        while IFS= read -r -d '' file; do
            if awk 'tolower($0) ~ /cc[ -]by[ -]sa|creativecommons.org\/licenses\/by-sa|share[ -]?alike/ { found = 1 } END { exit !found }' "$file"; then
                cp "$file" "$output/lensfun-database/$(basename "$file")"
            fi
        done < <(find "$directory" -maxdepth 1 -type f \( -iname '*LICENSE*' -o -iname '*COPYING*' -o -iname '*COPYRIGHT*' -o -iname 'README*' \) -print0)
    done
}

cargo build --release -p spektrafilm-gui --target-dir "$BUILD_TARGET_DIR"
cargo build --release -p spektrafilm-cli --bin spektrafilm --bin decode_raw_gui --target-dir "$BUILD_TARGET_DIR"
cargo build --release -p spektrafilm-cli --bin spektrafilm-f64 --features precision-f64 --target-dir "$BUILD_TARGET_DIR"

rm -rf "$DIST_DIR"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"
for executable in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    cp "$BUILD_TARGET_DIR/release/$executable" "$APP_BUNDLE/Contents/MacOS/$executable"
    chmod +x "$APP_BUNDLE/Contents/MacOS/$executable"
done
cp -R data "$APP_BUNDLE/Contents/Resources/data"
collect_library_dirs
package_native_dependencies
bundle_lensfun_database
bundle_native_licenses

cat > "$APP_BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Spektrafilm</string>
  <key>CFBundleExecutable</key>
  <string>spektrafilm-gui</string>
  <key>CFBundleIconFile</key>
  <string>spektrafilm.icns</string>
  <key>CFBundleIdentifier</key>
  <string>dev.spektrafilm.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Spektrafilm</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.2</string>
  <key>CFBundleVersion</key>
  <string>0.1.2</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

if [[ -f "$ICON_SOURCE" ]]; then
    iconset="$BUILD_TARGET_DIR/spektrafilm.iconset"
    rm -rf "$iconset"
    mkdir -p "$iconset"
    sips -s format png -z 16 16 "$ICON_SOURCE" --out "$iconset/icon_16x16.png" >/dev/null
    sips -s format png -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_16x16@2x.png" >/dev/null
    sips -s format png -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_32x32.png" >/dev/null
    sips -s format png -z 64 64 "$ICON_SOURCE" --out "$iconset/icon_32x32@2x.png" >/dev/null
    sips -s format png -z 128 128 "$ICON_SOURCE" --out "$iconset/icon_128x128.png" >/dev/null
    sips -s format png -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_128x128@2x.png" >/dev/null
    sips -s format png -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_256x256.png" >/dev/null
    sips -s format png -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_256x256@2x.png" >/dev/null
    sips -s format png -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_512x512.png" >/dev/null
    sips -s format png -z 1024 1024 "$ICON_SOURCE" --out "$iconset/icon_512x512@2x.png" >/dev/null
    iconutil -c icns "$iconset" -o "$APP_BUNDLE/Contents/Resources/spektrafilm.icns"
fi

cat > "$DIST_DIR/README.txt" <<'README'
spektrafilm-rs macOS

Run Spektrafilm.app for the GUI.

Backends:
- macOS uses the WGPU/WGSL backend through Metal by default, with CPU fallback when no usable adapter is available.

Native image support:
- decode_raw_gui, LibRaw, Lensfun, Exiv2, GLib and OpenImageIO are bundled.
- Lensfun XML data is included in Contents/Resources/lensfun.
- App and native dependency licenses are included in Contents/Resources/licenses.

f64 export:
- spektrafilm-f64 is bundled inside Spektrafilm.app/Contents/MacOS.
- The GUI uses it for CPU f64 Export.

Gatekeeper:
- This package is ad-hoc signed for local testing. It is not Apple-notarized.
- If macOS blocks it after download, right-click Spektrafilm.app and choose Open.
README

if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null
fi

zip_path="$DIST_DIR.zip"
rm -f "$zip_path"
(
    cd "$DIST_DIR/.."
    ditto -c -k --sequesterRsrc --keepParent "$(basename "$DIST_DIR")" "$(basename "$zip_path")"
)

echo "Packaged: $repo_root/$DIST_DIR"
echo "Zip:      $repo_root/$zip_path"
