#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

platform="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
dist_root="${SPEKTRAFILM_DIST_DIR:-$repo_root/dist/spektrafilm-${platform}-${arch}}"
archive="${dist_root}.tar.gz"
if [[ "$platform" == linux ]]; then
    BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target}"
    BUILD_TARGET_DIR="$BUILD_TARGET_DIR" RELEASE_SUBDIR="${RELEASE_SUBDIR:-release}" DIST_DIR="$dist_root" \
        bash "$repo_root/scripts/package_linux_app.sh"
    exit 0
fi
if [[ "$platform" == darwin ]]; then
    app_dist_dir="${SPEKTRAFILM_MACOS_DIST_DIR:-$repo_root/dist/spektrafilm-macos}"
    BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target/macos-app}" DIST_DIR="$app_dist_dir" \
        bash "$repo_root/scripts/package_macos_app.sh"
    rm -rf "$dist_root"
    mkdir -p "$dist_root"
    cp -a "$app_dist_dir/." "$dist_root/"
    tar -C "$(dirname "$dist_root")" -czf "$archive" "$(basename "$dist_root")"
    printf 'Package: %s\nArchive: %s\n' "$dist_root" "$archive"
    exit 0
fi

rm -rf "$dist_root" "$archive"
mkdir -p "$dist_root"

for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui spektrafilm-lab; do
    source="$repo_root/target/release/$binary"
    test -x "$source" || { echo "missing release binary: $source" >&2; exit 1; }
    cp "$source" "$dist_root/$binary"
    chmod 755 "$dist_root/$binary"
done

mkdir -p "$dist_root/licenses"
for license in nvlabs-flip-bsd-3-clause.txt littlecms-mit.txt; do
    test -s "$repo_root/licenses/$license" || { echo "missing benchmark license: $repo_root/licenses/$license" >&2; exit 1; }
    cp "$repo_root/licenses/$license" "$dist_root/licenses/$license"
done
 
cp -R "$repo_root/data" "$dist_root/data"
cat > "$dist_root/README.txt" <<EOF
Spektrafilm ${platform}-${arch}

Run ./spektrafilm-gui for the desktop application.
The CLI, f64 exporter, and lab instrument are in this directory, together with data/.
The package uses the default wgpu/WGSL backend.
EOF

# GNU tar and BSD tar both support this portable form.
tar -C "$(dirname "$dist_root")" -czf "$archive" "$(basename "$dist_root")"
printf 'Package: %s\nArchive: %s\n' "$dist_root" "$archive"
