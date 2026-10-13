# Photo export

## Contract

- Export re-renders the image at the requested output resolution and writes PNG, JPEG, TIFF, or OpenEXR.
- CPU f64 is the reference export backend and the default. Explicit WGPU export uses f32 and fails if no usable adapter exists; unsupported stages may use the faithful CPU route.
- Save writes retained floating output without rerunning the film simulation. It may convert to a separately selected saving color space and transfer function before writing.
- Existing material controls and [output mix](../material-response/spec.md#output-mix) must be captured in the immutable Export snapshot. Export must evaluate preflash and chemistry at their existing eligible exposure and development stages and preserve scanner calibration and inverse semantics. Finished RGB must not run material processing. Save writes the retained completed result without repeating material processing, Magazine appearance, output gamut processing, grain, or output mix. Saving color conversion follows the completed output mix in linear output RGB.
- PNG and JPEG require 8-bit output. TIFF supports 8-bit, 16-bit, and float output; OpenEXR supports half and float output. Integer output clips to [0,1]; float output preserves negative and super-white values.
- JPEG quality is 1–100 with 4:4:4 or 4:2:0 sampling. TIFF and OpenEXR support ZIP or no compression where the format allows it.
- Invalid format, bit depth, compression, color, or geometry combinations fail before final publication.
- Image publication must be atomic for CLI processing, GUI Save, and Export. A failed write or publication must leave an existing destination unchanged. Failure must leave no completed destination when none existed. GUI cancellation must provide the same guarantees. Successful CLI `process` may replace an existing destination only after the new image is complete.

## Publication

The writer must keep rendered samples, container finalization, and retained metadata work on a temporary file beside the destination. It must publish only after all required writes and close operations succeed. A temporary file must use the selected image container even when its name has a temporary suffix. It must not be visible at the final image path during rendering or writing.

A failed or short native write, failed container finalization, or failed atomic replacement must propagate as a command failure. The writer must remove its unpublished temporary file on handled failure or cancellation. It must not report a damaged or incomplete image as saved. A metadata warning permitted by the existing metadata contract may remain a warning only when the image container is complete and readable. This contract does not guarantee cleanup after forced process termination, power-loss durability, or a transaction across image, `--raw-out`, and diagnostic files.

CLI image completion notices and published dimensions must be recorded only after final image publication. A later auxiliary-file failure may return a failed command with an already committed image; it must not produce an overall-success notice. Existing telemetry-report persistence exceptions remain owned by [telemetry](../telemetry/spec.md#local-surfaces).

## Publication acceptance

Run the real `process` entry point for PNG, JPEG, TIFF, and OpenEXR. Verify readable output, dimensions, sample storage, and retained metadata after successful publication. Test successful replacement of an existing destination.

Inject write failures during image data and container finalization. On POSIX, a controlled file-size limit may exercise the native short-write path. Use an isolated failure mechanism on other platforms. Test a failed atomic replacement separately. Repeat each failure with an absent destination and an existing byte-for-byte sentinel. The command must fail, leave the final destination absent or unchanged, and remove its unpublished temporary file. Stderr must report the failed boundary without an image completion notice. Normal stdout must remain empty. A parser error or an invalid destination rejected before writing does not prove native write failure handling.

Exercise metadata retention through the temporary-file path. Preserve an existing permitted metadata-warning case without treating an unreadable image as a metadata-only failure. Repeat GUI cancellation acceptance when changing the shared writer. Exercise platform-native atomic replacement on Linux, Windows, and macOS before claiming cross-platform acceptance.

