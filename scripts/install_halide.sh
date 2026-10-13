#!/usr/bin/env bash
# Install checksum-pinned official Halide 21 headers, shared libraries and notices.
set -euo pipefail
[[ $# == 1 ]] || { echo 'Usage: bash scripts/install_halide.sh <prefix>' >&2; exit 1; }
prefix="$1"
revision=b629c80de18f1534ec71fddd8b567aa7027a0876
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) platform=x86-64-linux; digest=b56139ddc5d863486b9b339e1c9b7cc3f6aadd4dd8a2eff2202e79ca68706091 ;;
    Linux-aarch64) platform=arm-64-linux; digest=6fa8be9a556ddf3e899a0db59af84d69d0bebbf91ead90c03f90f731ab314d95 ;;
    Darwin-arm64) platform=arm-64-osx; digest=040a6fbde5ba264870df4975138417ce2ff2c8e9de550302c8b17f36c36e5afa ;;
    Darwin-x86_64) platform=x86-64-osx; digest=d7d26c91adcfe62528e20e248ba673aa635de519669b19c47e5a367f856a8ab0 ;;
    *) echo 'No pinned official Halide distribution for this platform' >&2; exit 1 ;;
esac
archive="Halide-21.0.0-$platform-$revision.tar.gz"
url="https://github.com/halide/Halide/releases/download/v21.0.0/$archive"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 -o "$work/$archive" "$url"
if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$digest" "$work/$archive" | sha256sum -c -
else
    printf '%s  %s\n' "$digest" "$work/$archive" | shasum -a 256 -c -
fi
mkdir -p "$prefix"
tar -xzf "$work/$archive" --strip-components=1 -C "$prefix"
[[ -s "$prefix/include/Halide.h" && -s "$prefix/share/doc/Halide/LICENSE.txt" ]] || {
    echo 'Official Halide distribution headers/notices are incomplete' >&2; exit 1;
}
printf 'version=21.0.0\nrevision=%s\nurl=%s\nsha256=%s\n' "$revision" "$url" "$digest" > "$prefix/Halide-provenance.txt"
printf 'Installed Halide 21.0.0 in %s; set HALIDE_ROOT to this prefix.\n' "$prefix"
