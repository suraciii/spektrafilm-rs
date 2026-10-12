# Grain V3 Spectral Aperture Probe

## Purpose

The probe measures the effect of reading formed Grain V3 density through a finer output lattice before film-print exposure. It is an engineering control for the print route. It does not change the Grain V3 field model or the default product behavior.

## Scope

The probe is enabled for active Grain V3 when `SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR` is set to an integer from `2` through `24`. Other grain engines and inactive Grain V3 ignore the variable.

The probe supports the `input > film > print > scan` route with `io.scan_film = false`, injection at or before `log_e_film`, and collection at `rgb_out`. It rejects film scans, later injection taps, and other collection taps. Enlarger diffusion, scanner blur, unsharp mask, and print glare must be inactive.

The default path remains the existing density readout when the variable is absent.

## Readout

The probe keeps the developed source, V3 parameters, random seed, global cell addresses, source edge extension, formation support, layer allocation, Poisson counts, and cell pitch unchanged.

It multiplies the requested V3 output dimensions by the factor and divides both output footprint pitches by the factor. Grain V3 then reads the same global formed field at the finer footprints.

The print spectral stage runs on the finer CMY readout with the existing film profile, print profile, illuminant, filtration, normalization, preflash, exposure, and selected enlarger LUT. The returned linear print exposure is reconstructed from the stage log output, averaged over each factor-by-factor output block, and converted back to the stage log representation before print development.

The probe performs no averaging in encoded RGB. It does not average CMY before the print spectral transfer more than the finer V3 footprints already do.

## Interpretation

The probe is a supersampling approximation to spectral aperture integration. It is not exact fractional cell-overlap integration. It cannot recover structure inside a formed field cell, and it does not establish physical scanner PSF or film-stock calibration.

The factor comparison must use one fixed input and one fixed seed. Report linear print exposure differences before reporting final RGB differences. Grain strength must remain unchanged.

## Failure behavior

The probe must fail before field allocation when its route or stage conditions are unsupported. It must reject non-integer, non-finite, zero, negative, or out-of-range factors. It must preserve the existing error behavior when disabled.

The probe restores the requested output pitch after exposure averaging. Crops select fine-grid readout footprints in the same global film coordinates. The probe does not run downstream spatial optics.

## Promotion boundary

A production spectral-aperture implementation must replace supersampling with exact fractional overlap of route-specific linear spectral exposures over the formed cell field. The probe does not satisfy that production contract by itself.
