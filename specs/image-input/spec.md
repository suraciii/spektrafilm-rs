# Image input

## Contract

- The application accepts supported raster images and camera RAW files through one normalized image boundary.
- Encoded raster input is decoded with its declared color space and transfer function. RAW input is returned as linear ACES2065-1 after camera orientation, selected white balance, and optional Lensfun correction.
- RAW white balance supports as-shot, daylight, tungsten, and finite custom temperature and tint. Unsupported or invalid camera settings fail clearly.
- Auto-exposure meters the complete source before crop and resize. Crop and resize then produce the working image and preserve the source pixel pitch for spatial stages.
- Crop bounds, dimensions, and resize factors must be finite and valid. Empty crops are errors.
- Input loading does not apply film, print, grain, or output encoding; those belong to the film chain and output capabilities.
