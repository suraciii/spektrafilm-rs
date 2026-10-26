# Dye grain

This context defines the domain language for the film-coordinate Dye grain model and its optical calibration evidence.

## Language

**Dye field**: The conditional film-coordinate record-density realization formed from developed targets and read through output footprints. It represents color dye records or an effective monochrome record, not encoded-RGB noise.

**Single-record BW field**: One stochastic material-density realization for a monochrome profile. It describes record layout, not identified silver chemistry or stock calibration.

**Dye support**: The normalized spatial operator that distributes conditional dye mass in film coordinates. Support controls spatial formation and is independent of event area and measured RMS.

**Dye event area**: The mass represented by one stochastic dye event. It controls count variance through the conditional count law; it is not a microscopic crystal area.

**Record contribution**: The integrated density response of one effective stochastic record event, in density times area (`D·µm²`). It is not geometric grain area or a calibrated material constant.

**Optical grain measurement**: A fixed-protocol measurement of a Dye realization after declared receiver, aperture, wavelength, density, and viewing conditions. Its RMS is evidence for that protocol, not a universal stock constant.

**Absolute spectral density**: The total optical density of the passive film base and dye records at one wavelength. It is nonnegative. It differs from receiver density measured relative to a declared reference.

**Mean-density uncertainty**: The estimated standard error of the mean receiver density across independent seed realizations. It is not the uncertainty of the measured RMS or a bound on systematic error.

**Dye calibration record**: A versioned record that binds a stock or profile, algorithm revision, effective parameters, measurement protocol, seed population, fit result, holdout result, and evidence status. It does not authorize per-image normalization or cross-stock reuse.
