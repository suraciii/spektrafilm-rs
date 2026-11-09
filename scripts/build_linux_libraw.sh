#!/usr/bin/env bash
set -euo pipefail

# Build the release-pinned decoder in a task-specific prefix.
if [[ $# -ne 1 ]]; then
    echo "Usage: bash scripts/build_linux_libraw.sh <native-prefix>" >&2
    exit 2
fi

prefix="$1"
mkdir -p "$prefix"
work="$(mktemp -d "$(dirname "$prefix")/spektrafilm-libraw-XXXXXX")"
trap 'rm -rf "$work"' EXIT

archive="$work/LibRaw-0.22.0.tar.gz"
curl -fsSL --retry 3 -o "$archive" https://www.libraw.org/data/LibRaw-0.22.0.tar.gz
printf '%s  %s\n' 1071e6e8011593c366ffdadc3d3513f57c90202d526e133174945ec1dd53f2a1 "$archive" | sha256sum -c -
tar -xzf "$archive" -C "$work"
cd "$work/LibRaw-0.22.0"

./configure --prefix="$prefix" --disable-static
make -j"$(nproc)"
make install
mkdir -p "$prefix/share/doc/libraw"
cp COPYRIGHT LICENSE.CDDL LICENSE.LGPL "$prefix/share/doc/libraw/"
