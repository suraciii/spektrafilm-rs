# Shared manifest admission for shell package/install producers.
# BUILD_MANIFEST points to an existing attested build; otherwise build once.
if [[ -z "${BUILD_MANIFEST:-}" ]]; then
    evidence_parent="$(mktemp -d "${TMPDIR:-/tmp}/spektrafilm-build.XXXXXX")"
    python3 scripts/build_evidence.py --out-dir "$evidence_parent/build" --profile release
    BUILD_MANIFEST="$evidence_parent/build/manifest.json"
fi
python3 scripts/build_evidence.py --manifest "$BUILD_MANIFEST" >/dev/null
binary_source() {
    local label
    case "$1" in
        spektrafilm-gui) label=gui ;;
        spektrafilm) label=cli ;;
        spektrafilm-f64) label=exporter ;;
        decode_raw_gui) label=raw_helper ;;
        *) return 1 ;;
    esac
    python3 scripts/build_evidence.py --manifest "$BUILD_MANIFEST" --binary "$label"
}
