# Product Vision

SpektraFilm exists to simulate analog color film and the print-and-scan chain spectrally, through measured spectral data rather than color presets. It serves photographers, filmmakers, and colorists who want to render digital images as film with physical accuracy.

A separate LUT workflow provides non-spectral color transforms for workflows that do not need spectral material simulation. It does not replace the spectral film chain or claim physical film accuracy.

Spectral film renderings can be exported as complete LUT files. A LUT file is imported explicitly when the LUT workflow is wanted; export never applies it or creates a preset.

The preset library carries Film, LUT, and Effects presets; applying one selects the mode it describes. A LUT result is a color transform, not a spectral claim.

## Product boundary

SpektraFilm is an independent product. Other film-simulation products are structural reference only; their feature inventories, control names, defaults, and presets are not product scope.

The product surface is the set of capabilities declared by its specifications. A new effect requires an explicit product decision and an owning specification; it MUST NOT be added because another product provides it.

Within that surface, physical film, print, and scan processes MAY remain single-track. A generic creative effect MUST be available in both methods, as a shared model or as separate Film and RGB models.
