# Image input design

- Image adapters produce a common floating image buffer plus source color metadata and retained source metadata.
- The RAW adapter owns LibRaw decoding, camera orientation, white balance, and Lensfun correction. It emits linear ACES2065-1 and does not emit a second encoded-sRGB path.
- Geometry processing owns crop, resize, antialiasing, and pixel-pitch propagation. Pipeline stages consume its result rather than recomputing geometry.
- Source decoding, exposure metering, geometry, and film exposure remain ordered operations. GUI preview limits apply to disposable preview rasters, not to the full-resolution source used by Scan or Export.
- Input adapters must not contain GUI state or output-writer policy.
