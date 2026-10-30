# Photo export

## Contract

- Export re-renders the image at the requested output resolution and writes PNG, JPEG, TIFF, or OpenEXR.
- Export backend selection, computation scope, and numerical alignment follow [GPU execution](../gpu-execution/spec.md). The production default MUST prefer GPU; the explicit CPU f64 reference remains available.
- An explicit GPU request MUST NOT use CPU image stages.
- Save writes retained floating output without rerunning the film simulation. It MAY convert to a separately selected saving color space and transfer function before writing.
- Existing material controls, [common finishing](../finishing/spec.md#persistence-and-replay), and [output mix](../output-mix/spec.md#output-mix) MUST be captured in the immutable Export snapshot.
- Export MUST evaluate preflash and chemistry at their existing eligible exposure and development stages and preserve scanner calibration and inverse semantics.
- The `input` workflow MUST NOT run material processing. Save writes the retained completed result without repeating material processing, common finishing, CMYK ink style, output gamut processing, grain, or output mix.
- Saving color conversion follows the completed output mix in linear output RGB.
- PNG and JPEG require 8-bit output. TIFF supports 8-bit, 16-bit, and float output; OpenEXR supports half and float output.
- Integer output clips to [0,1]; float output preserves negative and super-white values.
- JPEG quality is 1–100 with 4:4:4 or 4:2:0 sampling. TIFF and OpenEXR support ZIP or no compression where the format allows it.
- Invalid format, bit depth, compression, color, or geometry combinations fail before final publication.
- Image publication MUST be atomic for CLI processing, GUI Save, and Export. A failed write or publication MUST leave an existing destination unchanged.
- Failure MUST leave no completed destination when none existed. GUI cancellation MUST provide the same guarantees.
- Successful CLI `process` MAY replace an existing destination only after the new image is complete.

## Publication

The writer MUST keep rendered samples, container finalization, and retained metadata work on a temporary file beside the destination. It MUST publish only after all required writes and close operations succeed. A temporary file MUST use the selected image container even when its name has a temporary suffix.

It MUST NOT be visible at the final image path during rendering or writing.

A failed or short native write, failed container finalization, or failed atomic replacement MUST propagate as a command failure. The writer MUST remove its unpublished temporary file on handled failure or cancellation. It MUST NOT report a damaged or incomplete image as saved.

A metadata warning permitted by the existing metadata contract MAY remain a warning only when the image container is complete and readable. This contract does not guarantee cleanup after forced process termination, power-loss durability, or a transaction across image, `--raw-out`, and diagnostic files.

CLI image completion notices and published dimensions MUST be recorded only after final image publication. A later auxiliary-file failure MAY return a failed command with an already committed image; it MUST NOT produce an overall-success notice. Existing telemetry-report persistence exceptions remain owned by [telemetry](../telemetry/spec.md#local-surfaces).

## Publication acceptance

A failed atomic replacement MUST be tested separately. Each failure MUST repeat with an absent destination and an existing byte-for-byte sentinel: the command fails, the final destination stays absent or unchanged, and its unpublished temporary file is removed.

Use an isolated native failure mechanism on non-POSIX platforms to exercise image-data writes and container finalization.

Exercise metadata retention through the temporary-file path. Preserve an existing permitted metadata-warning case without treating an unreadable image as a metadata-only failure.

Exercise platform-native atomic replacement on Linux, Windows, and macOS before claiming cross-platform acceptance.

