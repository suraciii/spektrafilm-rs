#!/usr/bin/env bash
# Rebuild the pinned source-authored preparation reactor with Swift's official WASI SDK.
set -euo pipefail
REPO_ROOT=$(git rev-parse --show-toplevel)
PREPARATION_ROOT="$REPO_ROOT/crates/spektrafilm-fotufilm/vendor/fotufilm/preparation"
SWIFT_COMPILER=${FOTUFILM_SWIFT_COMPILER:-swift}
SWIFT_SDK=${FOTUFILM_SWIFT_WASM_SDK:-swift-6.4.0-RELEASE_wasm}
BUILD_ROOT=${FOTUFILM_PREPARATION_BUILD_DIR:-"$REPO_ROOT/target/fotufilm-preparation"}
OUTPUT_PATH=${FOTUFILM_PREPARATION_OUTPUT:-"$REPO_ROOT/data/fotufilm/preparation.wasm"}
TOOLCHAIN_BIN=$(dirname "$(command -v "$SWIFT_COMPILER")")
STRIP_TOOL=${FOTUFILM_WASM_OBJCOPY:-"$TOOLCHAIN_BIN/llvm-objcopy"}
mkdir -p "$BUILD_ROOT" "$(dirname "$OUTPUT_PATH")"
# Swift's driver removes LD_LIBRARY_PATH before the linker; restore only the caller's value.
LINKER_WRAPPER="$BUILD_ROOT/wasm-linker.sh"
{
  printf '#!/usr/bin/env bash\nset -euo pipefail\n'
  printf 'export LD_LIBRARY_PATH=%q\n' "${LD_LIBRARY_PATH:-}"
  printf 'exec %q "$@"\n' "${FOTUFILM_WASM_LD:-"$TOOLCHAIN_BIN/wasm-ld"}"
} > "$LINKER_WRAPPER"
chmod +x "$LINKER_WRAPPER"
export FOTUFILM_LINKER_LIBRARY_PATH=${LD_LIBRARY_PATH:-}
export FOTUFILM_WASM_LD=${FOTUFILM_WASM_LD:-"$TOOLCHAIN_BIN/wasm-ld"}
"$SWIFT_COMPILER" build --package-path "$PREPARATION_ROOT" --scratch-path "$BUILD_ROOT/swift" \
  -c release --swift-sdk "$SWIFT_SDK" --product fotufilm-web-profile \
  -Xswiftc -Xclang-linker -Xswiftc -mexec-model=reactor -Xswiftc "-use-ld=$LINKER_WRAPPER"
BIN_PATH=$("$SWIFT_COMPILER" build --package-path "$PREPARATION_ROOT" --scratch-path "$BUILD_ROOT/swift" \
  -c release --swift-sdk "$SWIFT_SDK" --show-bin-path)
"$STRIP_TOOL" --strip-all "$BIN_PATH/fotufilm-web-profile.wasm" "$OUTPUT_PATH"
sha256sum "$OUTPUT_PATH"
