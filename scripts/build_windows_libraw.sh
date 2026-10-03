#!/usr/bin/env bash
set -euo pipefail

# Build the release-pinned decoder without changing the UCRT64 system packages.
if [[ "${MSYSTEM:-}" != UCRT64 ]]; then
    echo "LibRaw must be built in the MSYS2 UCRT64 shell." >&2
    exit 1
fi
if [[ $# -ne 1 ]]; then
    echo "Usage: bash scripts/build_windows_libraw.sh <native-prefix>" >&2
    exit 1
fi

prefix="$(cygpath -u "$1")"
mkdir -p "$prefix"
prefix="$(cd "$prefix" && pwd)"
if [[ "$prefix" == /ucrt64 || "$prefix" == "$(cygpath -u "$MINGW_PREFIX")" ]]; then
    echo "Choose a task-specific prefix outside the UCRT64 system packages." >&2
    exit 1
fi
work="$(mktemp -d "$(cygpath -u "${RUNNER_TEMP:-${TMPDIR:-/tmp}}")/spektrafilm-libraw-XXXXXX")"
trap 'rm -rf "$work"' EXIT

archive="$work/LibRaw-0.22.0.tar.gz"
curl -fsSL --retry 3 -o "$archive" https://www.libraw.org/data/LibRaw-0.22.0.tar.gz
printf '%s  %s\n' 1071e6e8011593c366ffdadc3d3513f57c90202d526e133174945ec1dd53f2a1 "$archive" | sha256sum -c -
tar -xzf "$archive" -C "$work"
cd "$work/LibRaw-0.22.0"

# Native Windows paths in the installed .pc files also work with pkg-config in PowerShell.
./configure --prefix="$(cygpath -m "$prefix")" --enable-shared --disable-static --disable-examples
make -j"$(nproc)"
make install
mkdir -p "$prefix/share/licenses/libraw"
cp COPYRIGHT LICENSE.CDDL LICENSE.LGPL "$prefix/share/licenses/libraw/"

PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
    pkg-config --exact-version=0.22.0 libraw
printf 'Installed LibRaw 0.22.0 into %s\n' "$(cygpath -m "$prefix")"
