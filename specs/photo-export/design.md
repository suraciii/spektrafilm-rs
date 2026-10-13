# Photo export design

- Rendering, saving conversion, and native writing are separate boundaries. The renderer retains floating output in its execution domain; [GPU residency](../gpu-execution/design.md#device-residency) governs computation and final host handoff. One shared writer handles CLI and GUI publication.
- Export chooses a backend and re-renders at export geometry. Save starts from retained output and applies only the requested saving conversion.
- Simulation color settings and saving color settings are distinct. A writer option never changes film-chain gamut mapping.
- Writers validate format-specific constraints before opening the destination, write to a temporary path, and publish only after the write succeeds.
- Cancellation is checked before rendering and around publishable work. A native write that cannot be interrupted completes into a temporary path and is removed on cancellation.

## Transaction boundary

The [publication contract](spec.md#publication) belongs to the shared image writer. `process` must not bypass it by calling the native writer on the final destination. Structured render must retain its refusal to replace an existing output. GUI and CLI adapters may choose replacement policy but must not duplicate container-writing rules.

Native image data writes and close results must both be checked. Metadata read warnings after a write must not conceal a native write failure. The publication result must distinguish a committed image with a permitted metadata warning from an unpublished image error. Observers must retain the caller's final destination identity rather than expose an implementation temporary path as the output.

The native file stream must detect short writes and buffered flush or close failures even when a container plugin reports success. A metadata warning requires a readable, complete image container. The writer may validate the container after a metadata failure, but must not repeat image decoding on the successful write path.


Same-directory staging permits platform-native atomic replacement without a cross-filesystem move. Removing the old destination before renaming would destroy failure preservation and must not be used. A platform that cannot atomically replace the selected destination must fail while preserving it. The writer must not add copy-based fallback publication.

Each image transaction must own a unique temporary file. Handled errors must clean up only that file. Source metadata retention and format choice must survive staging. The image transaction must not absorb `--raw-out` or telemetry persistence into a multi-file commit.

