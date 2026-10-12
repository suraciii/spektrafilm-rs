# Grain V3 Diagnostic Experiments

## Scope

These opt-in experiments compare optical readout and random event mass. They do not select a new default or establish physical stock accuracy. Default renders retain fixed masses and density-average readout.

## Formed-cell optical reading

For active Grain V3, `SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL=1` requests linear spectral response on each formed CMY field cell followed by exact fractional cell-area integration. The optical response belongs to the print or scanner stage. Formation, conditional targets, seed streams and global coordinates remain in the grain model.

Exactness refers to the piecewise-constant formed field. It does not recover within-cell variations. Field refinement must be assessed separately from output-footprint refinement.

The initial reader supports color-negative print exposure and direct film capture, including positive scan interpretation after integration. Print collection may stop at `log_e_print`, `cmy_print` or `rgb_out`; film scanning collects `rgb_out`. Injection must occur at or before `log_e_film`. Density-only collection at `cmy_film` is rejected for an optical request.

The reader rejects simultaneous supersampling (`SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR`), enlarger/scanner LUTs, scanner luminance remap, spatial optics, scanner point/white/black corrections, and magazine finishing. Rejections occur before field allocation. Zero spatial optics is an explicit experiment limit, not evidence for the production routes with optical filtering.

The optical reader bounds each band to at most 1,048,576 field or source cells. Existing spectral backends may allocate wavelength tensors within a band; this bound does not establish acceptable full-frame time or peak memory.

## Random mass

For active Grain V3, `SPEKTRAFILM_GRAIN_V3_MASS_LAW` selects `fixed`, `two_class`, or `gamma4`. An absent selector uses `fixed`; invalid values fail. These settings are diagnostic environment variables, not saved recipe fields or GUI controls.

For the two random-mark candidates, `q = E[W²] = 1.25`, event rate is multiplied by q and baseline event mass is divided by q. This preserves conditional density mean, variance and common-kernel covariance. It changes higher cumulants, empty-cell probability and mean optical transmission.

`two_class` uses independent thinned Poisson draws with compensated masses 0.4 and 1.2 at rates 0.625 times the original rate each. `gamma4` draws the compensated Poisson count K and then one Gamma total with shape 4K and scale 0.2. It does not enumerate individual events. Neither candidate has physical calibration.

Expectation mode replaces total random mass with the original conditional mean for every law. Rate validation applies after compensation. Unrepresentable samples fail; they are not clipped or replaced with normal approximations.

## Evidence

Transient probes, source derivatives, run logs, numerical reports and images remain outside the repository. Comparisons must identify source, precision, physical pitch, support, event area, seed ensemble, optical route and disabled effects. An approximation must be compared to an independent optical integral of the same model. Candidate appearance changes must be reported separately from approximation errors.
