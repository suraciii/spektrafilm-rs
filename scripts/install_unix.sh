#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "$(uname -s)" == "Darwin" ]]; then
    default_root="$HOME/Library/Application Support/Spektrafilm"
else
    default_root="${XDG_DATA_HOME:-$HOME/.local/share}/spektrafilm"
fi
install_root="${SPEKTRAFILM_INSTALL_ROOT:-$default_root}"
bin_dir="${SPEKTRAFILM_BIN_DIR:-$HOME/.local/bin}"

mkdir -p "$install_root" "$bin_dir"

for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    source="$repo_root/target/release/$binary"
    test -x "$source" || { echo "missing release binary: $source" >&2; exit 1; }
    cp "$source" "$install_root/$binary"
    chmod 755 "$install_root/$binary"
done

rm -rf "$install_root/data"
cp -R "$repo_root/data" "$install_root/data"

for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui; do
    ln -sfn "$install_root/$binary" "$bin_dir/$binary"
done

cat <<EOF
Installed Spektrafilm to:
  $install_root

Command links:
  $bin_dir/spektrafilm-gui
  $bin_dir/spektrafilm
  $bin_dir/spektrafilm-f64
  $bin_dir/decode_raw_gui

Add $bin_dir to PATH if it is not already there.
EOF
