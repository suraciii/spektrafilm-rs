# Photo export

## Contract

- Export re-renders the image at the requested output resolution and writes PNG, JPEG, TIFF, or OpenEXR.
- CPU f64 is the reference export backend and the default. Explicit WGPU export uses f32 and fails if no usable adapter exists; unsupported stages may use the faithful CPU route.
- Save writes retained floating output without rerunning the film simulation. It may convert to a separately selected saving color space and transfer function before writing.
- PNG and JPEG require 8-bit output. TIFF supports 8-bit, 16-bit, and float output; OpenEXR supports half and float output. Integer output clips to [0,1]; float output preserves negative and super-white values.
- JPEG quality is 1–100 with 4:4:4 or 4:2:0 sampling. TIFF and OpenEXR support ZIP or no compression where the format allows it.
- Invalid format, bit depth, compression, color, or geometry combinations fail before final publication.
- Export publication is atomic. Cancellation or failure leaves no completed destination file.
