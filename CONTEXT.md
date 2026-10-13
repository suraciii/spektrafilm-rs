# Film simulation

SpektraFilm models how a digital photograph forms a film image and how that image becomes a digital output or a photographic print.

## Language

**Film record density**: The optical density formed in one developed film record. It describes the developed material, before an output receiver interprets it.

**Scanned medium**: The developed film or photographic paper read for digital output. Its native polarity is independent of the delivered image's polarity.

**Scan receiver**: The spectral response and reference readings used to acquire transmitted or reflected light from a scanned medium.
_Avoid_: Scanner look, output color space

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

**Digital output**: The color space, transfer encoding, geometry, and file representation used to deliver the image.
_Avoid_: Scanned medium, negative conversion
