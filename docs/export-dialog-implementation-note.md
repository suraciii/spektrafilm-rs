# Export contract implementation note

This note records behavior present in the export worktree and the verification run listed below. Product design and prototype claims remain separate.

## Present in this worktree

- Shared image I/O detects JPEG, PNG, TIFF, and OpenEXR from the output extension and rejects invalid combinations before writing. JPEG and PNG require 8-bit encoded output; OpenEXR requires linear 16-bit half or 32-bit float; JPEG quality is restricted to 1–100; JPEG-only fields are rejected for other formats; and compression is rejected for JPEG/PNG.
- OIIO native writing receives explicit JPEG quality, JPEG subsampling (`4:4:4` or `4:2:0`), TIFF/EXR compression, and EXR chromaticities. Unspecified values use shared writer defaults (JPEG `4:4:4`, compression `zip`).
- CLI `process` exposes `--format`, format-dependent `--bit-depth`, `--jpeg-quality`, `--jpeg-subsampling`, and `--compression`; it validates extension/format and option combinations before rendering. Structured `render` retains its fixed quality-85 JPEG contract.
- GUI state persists format, JPEG quality/subsampling, compression, and bit depth. Native controls expose the settings, preview Save uses the selected format, and full Export forwards explicit settings through the staged f64 worker while retaining cancellation and atomic publication.

## Verification

- `PKG_CONFIG_PATH=/tmp/sf-pkgconfig cargo test -p spektrafilm-core --test image_io`: 3 tests passed.
- `PKG_CONFIG_PATH=/tmp/sf-pkgconfig cargo test -p spektrafilm-gui`: 10 tests passed, including legacy Rust-state migration and export-setting round-trip.
- `PKG_CONFIG_PATH=/tmp/sf-pkgconfig cargo check -p spektrafilm-cli -p spektrafilm-gui`: passed; existing warnings only.
- An 8×8 PNG was processed through the f64 CPU CLI to JPEG quality 95 with both 4:4:4 and 4:2:0; JPEG SOF sampling factors were `(1,1)/(1,1)/(1,1)` and `(2,2)/(1,1)/(1,1)` respectively. The same input was exported to TIFF16 ZIP and EXR16 ZIP; `file` confirmed 8×8 TIFF/deflate and 8×8 OpenEXR/ZIP headers.
- The native GUI launched under Xvfb and remained alive for the 8-second smoke window; the timeout was intentional. This does not replace full interaction acceptance or prove high-resolution visual quality.

The design and prototype remain in [`export-dialog-design.md`](export-dialog-design.md) and [`export-dialog-prototype.html`](export-dialog-prototype.html); their proposed options and interaction states are not implementation evidence.
