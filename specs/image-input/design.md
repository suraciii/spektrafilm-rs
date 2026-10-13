# Image input design

- Image adapters expose normalized floating samples, source color metadata, and retained source metadata. Storage and host handoff follow [GPU device residency](../gpu-execution/design.md#device-residency).
- The RAW adapter owns decoding semantics, camera orientation, white balance, and Lensfun correction. Native unpacking and numerical execution must follow [GPU preparation boundaries](../gpu-execution/design.md#preparation-boundary). Its normalized result is linear ACES2065-1, not a second encoded-sRGB path.
- Geometry processing owns crop, resize, antialiasing, and pixel-pitch propagation. Pipeline stages consume its result rather than recomputing geometry.
- Source decoding, exposure metering, geometry, and film exposure remain ordered operations. GUI preview limits apply to disposable preview rasters, not to the full-resolution source used by Scan or Export.
- Input adapters must not contain GUI state or output-writer policy.
