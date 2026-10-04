#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

platform="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
dist_root="${SPEKTRAFILM_DIST_DIR:-$repo_root/dist/spektrafilm-${platform}-${arch}}"
archive="${dist_root}.tar.gz"

rm -rf "$dist_root" "$archive"
mkdir -p "$dist_root"

for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    source="$repo_root/target/release/$binary"
    test -x "$source" || { echo "missing release binary: $source" >&2; exit 1; }
    cp "$source" "$dist_root/$binary"
    chmod 755 "$dist_root/$binary"
done

cp -R "$repo_root/data" "$dist_root/data"

cat > "$dist_root/README.txt" <<EOF
Spektrafilm ${platform}-${arch}

Run ./spektrafilm-gui for the desktop application.
The CLI and the f64 exporter are in this directory, together with data/.
The package uses the default wgpu/WGSL backend.
EOF

# GNU tar and BSD tar both support this portable form.
tar -C "$(dirname "$dist_root")" -czf "$archive" "$(basename "$dist_root")"
printf 'Package: %s\nArchive: %s\n' "$dist_root" "$archive"
