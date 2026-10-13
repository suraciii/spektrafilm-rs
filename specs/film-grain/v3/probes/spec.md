# Grain Dye diagnostic probes

## Scope

These probes are engineering controls for active Grain Dye. They do not change the Grain Dye field model, select a new default, or establish physical stock accuracy. Other grain engines and inactive Grain Dye ignore their variables. Default renders retain fixed masses and density-average readout. An absent variable keeps the current default path.

The probes must fail before field allocation when their route, stage, or factor conditions are unsupported. Transient probe outputs, source derivatives, run logs, numerical reports, and images remain outside the repository.

## Spectral aperture probe

The probe measures the effect of reading formed Grain Dye density through a finer output lattice before film-print exposure. It is an engineering control for the print route.

The probe is enabled when `SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR` is an integer from `2` through `24`.

The probe supports the `input > film > print > scan` route with `io.scan_film = false`, injection at or before `log_e_film`, and collection at `rgb_out`. It rejects film scans, later injection taps, and other collection taps. Enlarger diffusion, scanner blur, unsharp mask, and print glare must be inactive.

### Readout

The probe keeps the developed source, Dye parameters, random seed, global cell addresses, source edge extension, formation support, layer allocation, Poisson counts, and cell pitch unchanged.

It multiplies the requested Dye output dimensions by the factor and divides both output footprint pitches by the factor. Grain Dye then reads the same global formed field at the finer footprints.

The print spectral stage runs on the finer CMY readout with the existing film profile, print profile, illuminant, filtration, normalization, preflash, exposure, and selected enlarger LUT. The returned linear print exposure is reconstructed from the stage log output, averaged over each factor-by-factor output block, and converted back to the stage log representation before print development.

The probe performs no averaging in encoded RGB. It does not average CMY before the print spectral transfer more than the finer Dye footprints already do.

### Interpretation

The probe is a supersampling approximation to spectral aperture integration. It is not exact fractional cell-overlap integration. It cannot recover structure inside a formed field cell, and it does not establish physical scanner PSF or film-stock calibration.

The factor comparison must use one fixed input and one fixed seed. Report linear print exposure differences before reporting final RGB differences. Grain strength must remain unchanged.

### Failure behavior

The probe must fail before field allocation when its route or stage conditions are unsupported. It must reject non-integer, non-finite, zero, negative, or out-of-range factors. It must preserve the existing error behavior when disabled.

The probe restores the requested output pitch after exposure averaging. Crops select fine-grid readout footprints in the same global film coordinates. The probe does not run downstream spatial optics.

### Promotion boundary

A production spectral-aperture implementation must replace supersampling with exact fractional overlap of route-specific linear spectral exposures over the formed cell field. The probe does not satisfy that production contract by itself.

## Formed-cell optical reading

`SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL=1` requests linear spectral response on each formed CMY field cell followed by exact fractional cell-area integration. The optical response belongs to the print or scanner stage. Formation, conditional targets, seed streams and global coordinates remain in the grain model.

Exactness refers to the piecewise-constant formed field. It does not recover within-cell variations. Field refinement must be assessed separately from output-footprint refinement.

The initial reader supports color-negative print exposure and direct film capture, including negative conversion after integration. Print collection may stop at `log_e_print`, `cmy_print` or `rgb_out`; film scanning collects `rgb_out`. Injection must occur at or before `log_e_film`. Density-only collection at `cmy_film` is rejected for an optical request.

The reader rejects simultaneous supersampling (`SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR`), enlarger/scanner LUTs, scanner luminance remap, spatial optics, scanner point/white/black corrections, per-frame automatic scanner setup, and magazine finishing. Rejections occur before field allocation. Zero spatial optics is an explicit experiment limit, not evidence for the production routes with optical filtering.

The optical reader bounds each band to at most 1,048,576 field or source cells. Existing spectral backends may allocate wavelength tensors within a band; this bound does not establish acceptable full-frame time or peak memory.

## Random event mass

`SPEKTRAFILM_GRAIN_V3_MASS_LAW` selects `fixed`, `two_class`, or `gamma4`. An absent selector uses `fixed`; invalid values fail. These settings are diagnostic environment variables, not saved recipe fields or GUI controls.

For the two random-mark candidates, `q = E[W²] = 1.25`, event rate is multiplied by q and baseline event mass is divided by q. This preserves conditional density mean, variance and common-kernel covariance. It changes higher cumulants, empty-cell probability and mean optical transmission.

`two_class` uses independent thinned Poisson draws with compensated masses 0.4 and 1.2 at rates 0.625 times the original rate each. `gamma4` draws the compensated Poisson count K and then one Gamma total with shape 4K and scale 0.2. It does not enumerate individual events. Neither candidate has physical calibration.

Expectation mode replaces total random mass with the original conditional mean for every law. Rate validation applies after compensation. Unrepresentable samples fail; they are not clipped or replaced with normal approximations.

## Evidence

Comparisons must identify source, precision, physical pitch, support, event area, seed ensemble, optical route and disabled effects. An approximation must be compared to an independent optical integral of the same model. Candidate appearance changes must be reported separately from approximation errors.
