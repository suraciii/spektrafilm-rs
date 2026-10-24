# Photo export design

- Rendering, saving conversion, and native writing are separate boundaries. The renderer retains floating output in its execution domain; [GPU residency](../gpu-execution/design.md#device-residency) governs computation and final host handoff.
- One shared writer handles CLI and GUI publication.
- Export chooses a backend and re-renders at export geometry. Save starts from retained output and applies only the requested saving conversion.
- Simulation color settings and saving color settings are distinct. A writer option never changes film-chain gamut mapping.
- Writers validate format-specific constraints before opening the destination, write to a temporary path, and publish only after the write succeeds.
- Cancellation is checked before rendering and around publishable work. A native write that cannot be interrupted completes into a temporary path and is removed on cancellation.

## Transaction boundary

The [publication contract](spec.md#publication) belongs to the shared image writer. `process` MUST NOT bypass it by calling the native writer on the final destination. Structured render MUST retain its refusal to replace an existing output.

GUI and CLI adapters MAY choose replacement policy but MUST NOT duplicate container-writing rules.

Native image data writes and close results MUST both be checked. Metadata read warnings after a write MUST NOT conceal a native write failure. The publication result MUST distinguish a committed image with a permitted metadata warning from an unpublished image error.

Observers MUST retain the caller's final destination identity rather than expose an implementation temporary path as the output.

The native file stream MUST detect short writes and buffered flush or close failures even when a container plugin reports success. A metadata warning requires a readable, complete image container. The writer MAY validate the container after a metadata failure, but MUST NOT repeat image decoding on the successful write path.


Same-directory staging permits platform-native atomic replacement without a cross-filesystem move. Removing the old destination before renaming would destroy failure preservation and MUST NOT be used. A platform that cannot atomically replace the selected destination MUST fail while preserving it.

The writer MUST NOT add copy-based fallback publication.

Each image transaction MUST own a unique temporary file. Handled errors MUST clean up only that file. Source metadata retention and format choice MUST survive staging.

The image transaction MUST NOT absorb `--raw-out` or telemetry persistence into a multi-file commit.

