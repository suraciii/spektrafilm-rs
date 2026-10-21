# Dye grain

This context defines the domain language for the film-coordinate Dye grain model and its optical calibration evidence.

## Language

**Dye field**: The conditional film-coordinate CMY dye realization formed from developed record-density targets and read through output footprints. It is a modeled material response, not encoded-RGB noise.

**Dye support**: The normalized spatial operator that distributes conditional dye mass in film coordinates. Support controls spatial formation and is independent of event area and measured RMS.

**Dye event area**: The mass represented by one stochastic dye event. It controls count variance through the conditional count law; it is not a microscopic crystal area.

**Optical grain measurement**: A fixed-protocol measurement of a Dye realization after declared receiver, aperture, wavelength, density, and viewing conditions. Its RMS is evidence for that protocol, not a universal stock constant.

**Dye calibration record**: A versioned record that binds a stock or profile, algorithm revision, effective parameters, measurement protocol, seed population, fit result, holdout result, and evidence status. It does not authorize per-image normalization or cross-stock reuse.
