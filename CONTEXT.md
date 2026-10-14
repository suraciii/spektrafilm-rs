# Film simulation

SpektraFilm models how a digital photograph forms a film image and how that image becomes a digital output or a photographic print.

## Language

Real operations of exposure, development, photographic printing, and film digitization own the unqualified vocabulary of this language. A modeled convention must carry a distinguishing qualifier and must not occupy an unqualified term of a real operation. The product models the real operational workflow first; a modeled convention extends it.

**Record density**: The optical density formed in one developed record of a film or a photographic paper. It describes the developed material, before an output receiver interprets it.
_Avoid_: Scanned density, receiver capture

**Scanned medium**: The developed film or photographic paper read for digital output. Its native polarity is independent of the delivered image's polarity.

**Scan receiver**: The spectral response and reference readings used to acquire transmitted or reflected light from a scanned medium.
_Avoid_: Scanner look, output color space

**Receiver capture**: The RGB readings a scan receiver acquires from a scanned medium. A negative conversion may interpret a receiver capture instead of record densities.
_Avoid_: Scan output, scanned medium

**Scanned density**: The density derived from one channel of a scan capture, relative to that channel's clear reference. A slide scan reads its point range on scanned densities.
_Avoid_: Record density, receiver capture

**Monochrome**: A film stock that forms one silver image. It uses the silver material law whether its profile is negative or positive. A chromogenic monochrome stock forms dye records and requires an explicit dye material law.
_Avoid_: Black and white correction, scanner white and black correction

**Negative conversion**: The interpretation of a developed negative as a positive image. It includes a color interpretation and a base positive tone response.
_Avoid_: Positive scan, scanner finish

**Exposure reconstruction**: A negative conversion that uses the selected film's response to estimate relative exposure from record densities or receiver measurements.
_Avoid_: Reference conversion

**Reference conversion**: A negative conversion that uses a common receiver and positive response instead of reconstructing exposure through the selected film's characteristic curves.
_Avoid_: Unprocessed film color, physical ground truth

**Digital Reference**: The Scan output treatment that uses reference conversion to present simulated film under a common digital output model. The model is a reference convention, not a measured scanner.

**Reference Exposure**: A fixed placement of the reference conversion's tone response. Its exposure and tone placement do not depend on the photograph's content.

**Image correction**: A brightness, color balance, or contrast adjustment to the delivered image. It may be manual or derived from the photograph.

**Automatic scan setup**: Image correction derived from a photograph's measurements for its digital delivery.
_Avoid_: Receiver calibration, film development

**Scanner finishing**: An optional color and tone appearance applied after negative conversion.
_Avoid_: Negative conversion, scan receiver

**Lab Scan**: A modeled minilab-style digital delivery of a simulated negative. Receiver acquisition, conversion, automatic scan setup, and scanner finishing are separate parts of that delivery.
_Avoid_: Measured Frontier profile, measured Noritsu profile

**Scanner white and black correction**: The endpoint correction of a positive scan. A luminance remap maps the scanned medium's white and black reference luminances onto target white and black levels, with the matched film or print exposure correction. It applies to a photographic-paper or slide film scan, not a negative film scan.
_Avoid_: Black and white correction, monochrome

**Film-scan black and white points**: The per-channel window positions on a scanned film's point range: the relative log-exposure range of the derived inverse for a negative conversion, and the shared scanned-density range for a slide film scan. Points remap linearly without clipping.
_Avoid_: Scanner white and black correction, output levels

**Digital output**: The color space, transfer encoding, geometry, and file representation used to deliver the image.
_Avoid_: Scanned medium, negative conversion
