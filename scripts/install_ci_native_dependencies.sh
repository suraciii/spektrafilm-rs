#!/usr/bin/env bash
# Run as root on the Ubuntu CI runner. Cache archives, never installed state.
set -euo pipefail
archive_dir="${1:?Pass an absolute apt archive directory}"
log_dir="${2:?Pass an absolute log directory}"
[[ "$archive_dir" = /* && "$log_dir" = /* ]]
mkdir -p "$archive_dir/partial" "$log_dir"
exec > >(tee "$log_dir/install.log") 2>&1
phase=setup
started=$SECONDS
trap 'status=$?; printf "phase=%s exit=%s elapsed_seconds=%s\n" "$phase" "$status" "$((SECONDS - started))"; exit "$status"' EXIT

packages=(
    build-essential pkg-config curl ca-certificates
    libgtk-3-dev libudev-dev libxkbcommon-dev
    liblensfun-dev libexiv2-dev libglib2.0-dev libopenimageio-dev
    libjpeg-dev zlib1g-dev libopenblas-dev
    libvulkan1 mesa-vulkan-drivers
)
options=(
    -o "Dir::Cache::archives=$archive_dir"
    -o APT::Keep-Downloaded-Packages=true
    -o Acquire::Retries=2
    -o Acquire::http::Timeout=20
    -o Acquire::https::Timeout=20
    -o DPkg::Lock::Timeout=30
)
export DEBIAN_FRONTEND=noninteractive
run_phase() {
    phase="$1"
    local limit="$2"
    shift 2
    local phase_start=$SECONDS
    printf '\nphase=%s started_at=%s\n' "$phase" "$(date -u +%FT%TZ)"
    timeout --signal=TERM --kill-after=15s "$limit" "$@"
    printf 'phase=%s elapsed_seconds=%s\n' "$phase" "$((SECONDS - phase_start))"
}

# A failed index refresh must not silently continue with stale package metadata.
run_phase update 120s apt-get "${options[@]}" -o APT::Update::Error-Mode=any update
run_phase download 300s apt-get "${options[@]}" install -y --no-install-recommends --download-only "${packages[@]}"
# Separate download from unpack/configure and prove the cached archives suffice.
run_phase install 180s apt-get "${options[@]}" install -y --no-install-recommends --no-download "${packages[@]}"
dpkg-query -W -f='${Package}\t${Version}\n' "${packages[@]}" > "$log_dir/packages.tsv"
