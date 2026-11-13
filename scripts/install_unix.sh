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
mkdir -p "$bin_dir"

if [[ "$(uname -s)" == Linux ]]; then
    package_dir="$repo_root/dist/spektrafilm-linux-$(uname -m)"
    BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target}" RELEASE_SUBDIR="${RELEASE_SUBDIR:-release}" DIST_DIR="$package_dir" \
        bash "$repo_root/scripts/package_linux_app.sh"
    mkdir -p "$install_root"
    cp -a "$package_dir/." "$install_root/"
    for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui spektrafilm-lab; do
        ln -sfn "$install_root/bin/$binary" "$bin_dir/$binary"
    done
elif [[ "$(uname -s)" == Darwin ]]; then
    app_dist_dir="$repo_root/dist/spektrafilm-macos"
    BUILD_TARGET_DIR="${BUILD_TARGET_DIR:-target/macos-app}" DIST_DIR="$app_dist_dir" \
        bash "$repo_root/scripts/package_macos_app.sh"
    mkdir -p "$install_root"
    cp -a "$app_dist_dir/Spektrafilm.app" "$install_root/"
    for binary in spektrafilm spektrafilm-f64 decode_raw_gui spektrafilm-lab; do
        ln -sfn "$install_root/Spektrafilm.app/Contents/MacOS/$binary" "$bin_dir/$binary"
    done
    ln -sfn "$install_root/Spektrafilm.app/Contents/MacOS/spektrafilm-gui" "$bin_dir/spektrafilm-gui"
else
    mkdir -p "$install_root" "$bin_dir"
    for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui spektrafilm-lab; do
        source="$repo_root/target/release/$binary"
        test -x "$source" || { echo "missing release binary: $source" >&2; exit 1; }
        cp "$source" "$install_root/$binary"
        chmod 755 "$install_root/$binary"
    done
    rm -rf "$install_root/data"
    cp -R "$repo_root/data" "$install_root/data"
    for binary in spektrafilm-gui spektrafilm spektrafilm-f64 decode_raw_gui spektrafilm-lab; do
        ln -sfn "$install_root/$binary" "$bin_dir/$binary"
    done
fi

cat <<EOF
Installed Spektrafilm to:
  $install_root

  $bin_dir/spektrafilm-gui
  $bin_dir/spektrafilm
  $bin_dir/spektrafilm-f64
  $bin_dir/decode_raw_gui
  $bin_dir/spektrafilm-lab

Add $bin_dir to PATH if it is not already there.
EOF
