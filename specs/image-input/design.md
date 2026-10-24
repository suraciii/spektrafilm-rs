# Image input design

- Image adapters expose normalized floating samples, source color metadata, and retained source metadata. Storage and host handoff follow [GPU device residency](../gpu-execution/design.md#device-residency).
- The RAW adapter owns decoding semantics, camera orientation, white balance, and Lensfun correction. Native unpacking and numerical execution MUST follow [GPU preparation boundaries](../gpu-execution/design.md#preparation-boundary).
- Its normalized result is linear ACES2065-1, not a second encoded-sRGB path.
- Geometry processing owns crop, resize, antialiasing, and pixel-pitch propagation. Pipeline stages consume its result rather than recomputing geometry.
- Source decoding, exposure metering, geometry, and film exposure remain ordered operations. GUI preview limits apply to disposable preview rasters, not to the full-resolution source used by Scan or Export.
- Input adapters MUST NOT contain GUI state or output-writer policy.
- The capture illuminant is resolved once per render from the selected film profile's `reference_illuminant` and the explicit `camera.capture_illuminant` leaf, following [Image input capture illuminant](spec.md#capture-illuminant). An unset leaf keeps the profile value; an explicit leaf replaces it for the camera-side conversion into film exposure only.
- The RAW adapter MUST NOT supply a scene-light value from metadata, and no stage MAY derive one from image content. Its spectral selection reuses the shared illuminant service the scan and enlarger illuminants already use, so there is one illuminant vocabulary and one validator.
