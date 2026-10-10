# Magazine print color

## Product model

Magazine print is an optional finishing treatment for a completed RGB image. It is not a film stock, a photographic paper, or a base workflow. Finished RGB means an output-referred image whose exposure and source rendering are already established.

The initial color appearance must follow the selected vintage-magazine numerical study: a monotonic tone curve, selective red and blue color weight, restrained green chroma, and a small warm near-neutral highlight response. It must not claim historical press calibration or reproduce every magazine.

## Enablement

- The GUI must expose a `Magazine print` checkbox next to the base workflow controls. It must be off in the factory state.
- Enabling or disabling Magazine print must not change the base workflow, film stock, photographic paper, scan polarity, or source exposure.
- The existing `input` workflow must be labeled `Finished RGB` for this use. It must perform source-to-output color management without film simulation, photographic printing, metering, or new film grain.
- Magazine print must accept the completed RGB output of every supported base workflow. It must not require a photographic print stage.
- The GUI must show the resulting base workflow and enabled finishing treatment together before rendering.
- A single color Strength control must use the range 0 to 1. Its enabled default must be 1. Disabling the treatment must retain the selected Strength.
- Presets and saved GUI state must retain the base workflow, enablement, and Strength independently. Preview, Scan, and Export must use the same effective finishing settings.
- `input > film > scan > magazine` must not remain a selectable base workflow. Existing saved configurations that use it must receive an explicit migration error that identifies `input > film > scan` plus Magazine print enablement as the replacement.

## Color contract

- The feature must accept output-referred RGB with explicit primaries, white point, and transfer state. Linear encoding must not imply scene-referred exposure.
- The feature must run once after the base workflow and its scanner-domain effects, before output publication.
- The feature must preserve image dimensions and spatial detail positions. It must not re-run exposure, film development, or grain generation.
- All pixels must use the same color rule. The feature must not detect faces or use manually selected image regions. Low-chroma restraint must not be described as guaranteed skin protection.
- Strength must blend the source and full-strength appearance in an explicitly defined common linear RGB space. Disabled processing and Strength 0 must return the base result exactly.
- The feature must preserve the caller's output primaries and transfer state. It must not apply a second transfer encoding.
- Output must remain RGB. The feature must not produce CMYK separations, press-ready files, or a physical proof.
- Color Strength must not add halftone grids, plate registration, or paper texture.

## Print structure boundary

The study's lightweight halftone appearance is a separate optional finishing effect. It must be off by default and must not alter color Strength. Its production controls and scale contract require a separate spatial-effect specification before implementation. The fixed-pixel study is not a scale rule for arbitrary output sizes.

## Acceptance

- The same completed RGB buffer and finishing settings must produce the same result through standalone and appended entry points.
- Finished RGB processing must ignore the resolved film and photographic-paper profiles: changing either stock must not change its output. The CLI keeps its existing profile-resolution flags for this workflow.
- Tests must cover bypass, Strength endpoints, neutral-ramp ordering, finite output, and declared color-space and transfer-state handling.
- Visual acceptance must compare the selected study and production output on the same fixed Velvia and Portra sources, neutral and chromatic probes, and additional photographs with varied skin colors and illumination. Region measurements must not participate in rendering.
- Backend acceptance must compare the same images and settings against measured numerical budgets. A backend must not silently bypass the treatment.
