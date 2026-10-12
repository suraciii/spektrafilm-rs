"""Frozen-mask appearance and spatial measurements; deltas are candidate minus baseline.

Spatial evidence is retained before scalar summaries. Linear-sRGB diagnostics use
unclipped CAT02-converted samples, while Lab and official LDR-FLIP use bounded
encoded sRGB. Probe coordinates are supplied by the protocol, never fitted.
"""
from pathlib import Path
from importlib.metadata import version
import hashlib

import numpy as np
from scipy import ndimage, signal
from PIL import Image

try:
    from .image_data import ImageDataError, load_image, display_rgb, rgb_to_lab, linear_rgb
except ImportError:
    from image_data import ImageDataError, load_image, display_rgb, rgb_to_lab, linear_rgb

METRIC_VERSION = "spektrafilm-appearance-spatial-v3"
COLOR_TRANSFORM = "colour-science-CAT02-srgb-D65-2deg-v1"
LUMA = np.array([0.2126, 0.7152, 0.0722])


def ciede2000(lab1, lab2):
    import colour
    return colour.delta_E(np.asarray(lab1), np.asarray(lab2), method="CIE 2000")


delta_e00 = ciede2000


def rectangle_mask(geometry, dimensions):
    """Decode a frozen rectangle or lossless, hash-verified mask."""
    width, height = dimensions
    if isinstance(geometry, dict) and "mask" in geometry:
        geometry = geometry["mask"]
    if isinstance(geometry, dict) and "path" in geometry:
        path = Path(geometry["path"])
        if geometry.get("width") != width or geometry.get("height") != height:
            raise ImageDataError("invalid_region", "Declared mask dimensions differ from master")
        if hashlib.sha256(path.read_bytes()).hexdigest() != geometry.get("sha256"):
            raise ImageDataError("invalid_region", "Mask hash differs from declaration")
        if path.suffix.lower() == ".npy":
            values = np.load(path, allow_pickle=False)
        elif path.suffix.lower() in (".tif", ".tiff"):
            import tifffile
            values = tifffile.imread(path)
        elif path.suffix.lower() == ".png":
            with Image.open(path) as image:
                values = np.asarray(image)
        else:
            raise ImageDataError("invalid_region", "Mask must be NPY, PNG, or TIFF")
        if values.shape != (height, width) or not np.isfinite(values).all():
            raise ImageDataError("invalid_region", "Mask must be a finite single-channel master-sized array")
        mask = values != 0
    else:
        rectangle = geometry.get("rectangle") if isinstance(geometry, dict) else geometry
        if not isinstance(rectangle, (list, tuple)) or len(rectangle) != 4 or any(type(x) is not int for x in rectangle):
            raise ImageDataError("invalid_region", "Expected integer [x,y,width,height]")
        x, y, w, h = rectangle
        if x < 0 or y < 0 or w <= 0 or h <= 0 or x + w > width or y + h > height:
            raise ImageDataError("invalid_region", "Rectangle is outside master")
        mask = np.zeros((height, width), dtype=bool)
        mask[y:y+h, x:x+w] = True
    if not mask.any():
        raise ImageDataError("invalid_region", "Mask has no selected pixels")
    return mask


def numeric_health(image):
    samples = image.samples
    finite = np.isfinite(samples)
    extrema = []
    for channel, label in enumerate("RGB"):
        values = samples[..., channel][finite[..., channel]]
        row = {"channel": label, "sample_count": int(values.size), "availability": "available" if values.size else "unavailable"}
        if values.size:
            row.update(min=float(values.min()), max=float(values.max()))
        else:
            row["reason"] = "no_finite_samples"
        extrema.append(row)
    return {"dimensions": image.dimensions, "sample_count": int(samples.size),
            "finite_sample_count": int(finite.sum()), "nonfinite_sample_count": int((~finite).sum()),
            "nonfinite_pixel_count": int((~finite.all(-1)).sum()), "all_finite": bool(finite.all()),
            "channel_extrema": extrema, "below_zero_fraction": float(np.mean(finite & (samples < 0))),
            "above_one_fraction": float(np.mean(finite & (samples > 1))),
            "range_fraction_denominator": "all_native_channel_samples"}


def _crop(mask):
    y, x = np.nonzero(mask)
    return (slice(y.min(), y.max()+1), slice(x.min(), x.max()+1))


def _official_flip(baseline, candidate, ppd):
    import flip_evaluator
    if not np.isfinite(ppd) or ppd <= 0:
        raise ImageDataError("invalid_protocol", "FLIP pixels_per_degree must be positive")
    values, _, used = flip_evaluator.evaluate(
        np.ascontiguousarray(np.clip(baseline, 0, 1), dtype=np.float32),
        np.ascontiguousarray(np.clip(candidate, 0, 1), dtype=np.float32), "LDR",
        inputsRGB=True, applyMagma=False, parameters={"ppd": float(ppd)})
    values = np.asarray(values, dtype=np.float64)
    if values.ndim == 3 and values.shape[-1] == 1:
        values = values[..., 0]
    if values.shape != baseline.shape[:2] or not np.isfinite(values).all():
        raise ImageDataError("computation_failure", "Official FLIP returned invalid map")
    return values, used


class _Evidence:
    def __init__(self, protocol, out_dir):
        self.protocol = protocol
        self.out_dir = Path(out_dir)
        self.result = {"measurements": [], "findings": [], "artifacts": [], "numeric_health": {},
                       "execution_status": "completed", "metric_version": METRIC_VERSION,
                       "color_transform": COLOR_TRANSFORM}

    def failure(self, region, metric, code, message, evidence=(), execution=True):
        self.result["findings"].append({"id": f"{region}:{metric}:{code}", "case_id": self.protocol.get("case_id"),
            "region_id": region, "metric": metric, "decision": "unavailable", "code": code,
            "message": message, "evidence": list(evidence)})
        if execution:
            self.result["execution_status"] = "failed"

    def scalar(self, region, metric, value, count, domain, units, estimator, refs=(), reason=None):
        row = {"case_id": self.protocol.get("case_id"), "seed_population": self.protocol.get("seed_population", []),
            "region_id": region, "metric": metric, "metric_version": METRIC_VERSION, "domain": domain,
            "units": units, "estimator": estimator, "sample_count": int(count), "artifact_refs": list(refs),
            "availability": "available" if reason is None else "unavailable"}
        if reason is None and value is not None and np.isfinite(value):
            row["value"] = float(value)
        else:
            row.update(availability="unavailable", reason=reason or "computation_failure")
            if reason is None:
                self.failure(region, metric, "computation_failure", "Scalar is nonfinite", refs)
        self.result["measurements"].append(row)

    def artifact(self, name, values, role, **metadata):
        values = np.asarray(values)
        if not np.isfinite(values).all():
            raise ImageDataError("computation_failure", "Artifact contains nonfinite samples: " + name)
        self.out_dir.mkdir(parents=True, exist_ok=True)
        filename = hashlib.sha256(name.encode()).hexdigest()[:16] + ".npy"
        path = self.out_dir / filename
        with path.open("xb") as stream:
            np.save(stream, values, allow_pickle=False)
        identifier = name + "_artifact"
        self.result["artifacts"].append({"id": identifier, "path": filename,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "media_type": "application/x-npy",
            "role": role, "shape": list(values.shape), "dtype": str(values.dtype),
            "coordinate_space": "native_render", **metadata})
        return identifier


def _summary(evidence, region, name, values, refs, domain, units, mean=False):
    values = np.asarray(values)
    stats = {"p95": np.quantile(values, .95, method="linear"),
             "p99": np.quantile(values, .99, method="linear"), "max": np.max(values)}
    stats["mean" if mean else "median"] = np.mean(values) if mean else np.median(values)
    for label, value in stats.items():
        evidence.scalar(region, name+"."+label, value, values.size, domain, units,
                        "region pixel " + label + "; quantile method=linear", refs)


def _paired_scalars(evidence, region, prefix, sides, count, domain, units, estimator, refs):
    for name in sides[0]:
        left, right = sides[0][name], sides[1][name]
        for side, value in (("baseline", left), ("candidate", right), ("delta", None if left is None or right is None else right-left)):
            evidence.scalar(region, f"{prefix}.{side}.{name}", value, count, domain, units,
                            estimator + "; delta=candidate-baseline", refs,
                            reason=("zero_variance" if "correlation" in name or "frequency_centroid" in name else
                                    "no_half_height_crossing" if "half_height" in name else
                                    "no_mtf50_crossing" if "mtf50" in name else "insufficient_evidence") if value is None else None)


def _grain(evidence, region, mask, samples, settings, flat):
    crop = _crop(mask)
    local_mask = mask[crop]
    border = int(settings.get("residual", {}).get("border_px", 0))
    if border:
        local_mask = ndimage.binary_erosion(local_mask, iterations=border, border_value=0)
    count = int(local_mask.sum())
    if count < 4:
        raise ImageDataError("insufficient_samples", "Grain requires at least four selected residual pixels")
    residual_settings = settings.get("residual", {})
    method = residual_settings.get("method", "flat_mean" if flat else "gaussian_high_pass")
    if method == "flat_mean" and not flat:
        method = "gaussian_high_pass"
    radius = float(residual_settings.get("high_pass_radius_px", 3))
    window_name = settings.get("spectrum", {}).get("window", "hann").lower()
    metrics = []
    refs = []
    retained = []
    for side, image in zip(("baseline", "candidate"), samples):
        image = image[crop]
        if method == "flat_mean":
            residual = image - np.mean(image[local_mask], axis=0)
        elif method == "gaussian_high_pass":
            residual = image - ndimage.gaussian_filter(image, (radius, radius, 0), mode="reflect")
        else:
            raise ImageDataError("invalid_protocol", "Unknown grain detrending method")
        channels = np.concatenate((residual, (residual @ LUMA)[..., None]), axis=-1)
        valid = channels[local_mask]
        stats = {"rms."+label: np.sqrt(np.mean(valid[:, index]**2)) for index, label in enumerate(("R", "G", "B", "luminance"))}
        chroma = residual - (residual @ LUMA)[..., None]
        stats["rms.chromatic"] = np.sqrt(np.mean(chroma[local_mask]**2))
        covariance = np.cov(residual[local_mask], rowvar=False, ddof=0)
        for i, a in enumerate("RGB"):
            for j in range(i+1, 3):
                b = "RGB"[j]
                stats[f"covariance.{a}{b}"] = covariance[i, j]
                denominator = np.sqrt(covariance[i, i]*covariance[j, j])
                stats[f"correlation.{a}{b}"] = covariance[i, j]/denominator if denominator > 0 else None
        height, width = local_mask.shape
        if window_name == "hann":
            window = signal.windows.hann(height, sym=False)[:, None] * signal.windows.hann(width, sym=False)[None, :]
        elif window_name in ("boxcar", "rectangular", "none"):
            window = np.ones(local_mask.shape)
        else:
            raise ImageDataError("invalid_protocol", "Grain window must be hann or boxcar")
        window *= local_mask
        normalization = np.sum(window**2)
        if normalization == 0:
            raise ImageDataError("insufficient_samples", "Grain window has zero energy")
        fft = np.fft.fft2(channels*window[..., None], axes=(0, 1))
        psd = np.fft.fftshift(np.abs(fft)**2/normalization, axes=(0, 1))
        fx, fy = np.fft.fftshift(np.fft.fftfreq(width)), np.fft.fftshift(np.fft.fftfreq(height))
        metadata = {"domain": "linear_srgb", "units": "sample squared / cycles per pixel",
                    "window": window_name, "normalization": "abs(FFT(residual*mask*window))^2 / sum((mask*window)^2)",
                    "valid_dimensions": [width, height], "valid_sample_count": count,
                    "residual": {"method": method, "sigma_px": radius if method == "gaussian_high_pass" else None, "border_px": border},
                    "channels": ["R", "G", "B", "luminance"]}
        refs.append(evidence.artifact(f"grain.{region}.{side}.psd2d", psd, "grain_spectrum", **metadata))
        refs.append(evidence.artifact(f"grain.{region}.{side}.frequency_x", fx, "spectrum_coordinates", units="cycles/pixel"))
        refs.append(evidence.artifact(f"grain.{region}.{side}.frequency_y", fy, "spectrum_coordinates", units="cycles/pixel"))
        radial_frequency = np.hypot(fy[:, None], fx[None, :])
        edges = np.linspace(0, np.sqrt(.5), max(3, min(width, height)//2+1))
        rows = []
        for lo, hi in zip(edges[:-1], edges[1:]):
            annulus = (radial_frequency >= lo) & (radial_frequency < hi)
            if annulus.any():
                rows.append([(lo+hi)/2, *np.mean(psd[annulus], axis=0), int(annulus.sum())])
        radial = np.asarray(rows)
        refs.append(evidence.artifact(f"grain.{region}.{side}.radial_psd", radial, "grain_radial_spectrum", columns=["frequency_cycles_per_pixel", "R", "G", "B", "luminance", "bin_count"], **metadata))
        autocorrelation = np.fft.fftshift(np.fft.ifft2(np.abs(fft)**2, axes=(0, 1)).real, axes=(0, 1))/normalization
        refs.append(evidence.artifact(f"grain.{region}.{side}.autocovariance", autocorrelation, "grain_autocovariance", units="sample squared", lag_origin_px=[width//2, height//2]))
        center_power = autocorrelation[height//2, width//2]
        valid_channels = center_power > 0
        if valid_channels.any():
            acf = autocorrelation[..., valid_channels]/center_power[valid_channels]
            refs.append(evidence.artifact(f"grain.{region}.{side}.acf", acf, "grain_autocorrelation",
                units="normalized correlation", channels=np.asarray(["R", "G", "B", "luminance"])[valid_channels].tolist(),
                lag_origin_px=[width//2, height//2], estimator="windowed cyclic autocovariance / zero lag"))
        retained.append({"psd2d": psd, "radial_psd": radial, "autocovariance": autocorrelation, "residual": np.where(local_mask[..., None], residual, 0)})
        refs.append(evidence.artifact(f"grain.{region}.{side}.residual", np.where(local_mask[..., None], residual, 0), "grain_residual", domain="linear_srgb", mask_count=count))
        stats["psd.luminance_total"] = np.sum(psd[..., 3])/psd.shape[0]/psd.shape[1]
        power = radial[:, 4]
        stats["frequency_centroid_cycles_per_pixel"] = float(np.sum(radial[:, 0]*power)/power.sum()) if power.sum() > 0 else None
        metrics.append(stats)
    for name in retained[0]:
        difference = retained[1][name]-retained[0][name]
        if name == "radial_psd":
            difference[:, 0] = retained[0][name][:, 0]
            difference[:, -1] = retained[0][name][:, -1]
        refs.append(evidence.artifact(f"grain.{region}.delta.{name}", difference, "grain_signed_difference",
            domain="linear_srgb", estimator="candidate minus baseline "+name, residual_method=method))
    for name in metrics[0]:
        units = "correlation" if name.startswith("correlation") else "cycles/pixel" if "frequency" in name else "sample squared" if name.startswith(("covariance", "psd")) else "linear-sRGB sample RMS"
        _paired_scalars(evidence, region, "grain."+method, [{name: x[name]} for x in metrics], count, "linear_srgb", units,
                        f"{method} residual; border={border}px; population covariance ddof=0; RMS=sqrt(mean(residual^2))", refs)


def _halation(evidence, region, mask, samples, settings):
    required = ("center_px", "core_radius_px", "background_inner_radius_px", "background_outer_radius_px", "extent_px")
    if any(key not in settings for key in required):
        raise ImageDataError("missing_probe_geometry", "Halation requires frozen center, core, background annulus, and extent")
    y, x = np.indices(mask.shape)
    cx, cy = settings["center_px"]
    radius = np.hypot(x-cx, y-cy)
    core_radius = float(settings["core_radius_px"])
    extent = float(settings["extent_px"])
    bin_width = float(settings.get("bin_width_px", 1))
    background = mask & (radius >= settings["background_inner_radius_px"]) & (radius <= settings["background_outer_radius_px"])
    spill = mask & (radius > core_radius) & (radius <= extent)
    core = mask & (radius <= core_radius)
    if not background.any() or not spill.any() or not core.any():
        raise ImageDataError("invalid_probe_geometry", "Halation core, background, and spill must contain frozen pixels")
    edges = np.arange(0, extent+bin_width, bin_width)
    profiles = []
    stats = []
    refs = []
    for side, image in zip(("baseline", "candidate"), samples):
        excess = image - np.median(image[background], axis=0)
        for axis, displacement in (("x", x-cx), ("y", y-cy)):
            rows_normal = []
            # Frozen center strips preserve source alignment across both sides.
            transverse = np.abs(y-cy) if axis == "x" else np.abs(x-cx)
            for lo in np.arange(-extent, extent, bin_width):
                strip = mask & (transverse <= bin_width/2) & (displacement >= lo) & (displacement < lo+bin_width)
                if strip.any():
                    rgb = np.mean(excess[strip], axis=0)
                    rows_normal.append([lo+bin_width/2, *rgb, rgb[0]-rgb[1], int(strip.sum())])
            if rows_normal:
                refs.append(evidence.artifact(f"halation.{region}.{side}.normal_{axis}", np.asarray(rows_normal),
                    "halation_edge_spill_profile", columns=["signed_displacement_px", "R_excess", "G_excess", "B_excess", "red_minus_green", "count"],
                    domain="linear_srgb", estimator="frozen center strip; per-channel median annulus background subtraction", source_core_radius_px=core_radius))
        rows = []
        for lo, hi in zip(edges[:-1], edges[1:]):
            annulus = mask & (radius >= lo) & (radius < hi)
            if annulus.any():
                rgb = np.mean(excess[annulus], axis=0)
                rows.append([(lo+hi)/2, *rgb, rgb[0]-rgb[1], int(annulus.sum())])
        profile = np.asarray(rows)
        profiles.append(profile)
        refs.append(evidence.artifact(f"halation.{region}.{side}.radial", profile, "halation_profile", columns=["radius_px", "R_excess", "G_excess", "B_excess", "red_minus_green", "count"], domain="linear_srgb", background="per-channel median frozen annulus", source_core_radius_px=core_radius))
        values = {"core_clipped_fraction": np.mean((image[core] >= 1).any(-1))}
        for channel, label in enumerate("RGB"):
            outer = profile[:, 0] > core_radius
            coordinates, amplitudes = profile[outer, 0], profile[outer, channel+1]
            peak_index = int(np.argmax(amplitudes))
            peak = amplitudes[peak_index]
            half = np.flatnonzero(amplitudes[peak_index:] <= peak/2)
            values[f"{label}.peak_amplitude"] = peak
            values[f"{label}.peak_radius_px"] = coordinates[peak_index]
            values[f"{label}.half_height_width_px"] = coordinates[peak_index+half[0]]-coordinates[peak_index] if peak > 0 and half.size else None
            values[f"{label}.integrated_spill"] = np.sum(excess[..., channel][spill])
        values["red_minus_green.integrated_spill"] = np.sum((excess[..., 0]-excess[..., 1])[spill])
        stats.append(values)
    delta = profiles[1].copy()
    delta[:, 1:5] -= profiles[0][:, 1:5]
    refs.append(evidence.artifact(f"halation.{region}.delta.radial", delta, "halation_profile", columns=["radius_px", "R_delta", "G_delta", "B_delta", "red_minus_green_delta", "count"], domain="linear_srgb"))
    for name in stats[0]:
        units = "pixels" if name.endswith("_px") else "fraction" if "fraction" in name else "linear-sRGB sample pixels" if "spill" in name else "linear-sRGB sample"
        count = int(core.sum()) if name == "core_clipped_fraction" else int(spill.sum())
        estimator = "fraction of frozen core pixels with any linear-sRGB channel >= 1" if name == "core_clipped_fraction" else "frozen radial bins; peak outside source core; half-height first outward crossing; signed excess integral"
        _paired_scalars(evidence, region, "halation", [{name: row[name]} for row in stats], count, "linear_srgb", units, estimator, refs)


def _edge(evidence, region, mask, samples, settings):
    if any(key not in settings for key in ("axis", "input_contrast", "input_exposure_stops")):
        raise ImageDataError("missing_probe_geometry", "Edge needs frozen normal axis, input contrast, and exposure")
    axis = settings["axis"]
    if axis not in ("x", "y"):
        raise ImageDataError("invalid_protocol", "Edge axis must be x or y")
    crop = _crop(mask)
    local = mask[crop]
    reduce_axis = 0 if axis == "x" else 1
    counts = local.sum(axis=reduce_axis)
    selected = counts > 0
    coordinates = np.arange(local.shape[1] if axis == "x" else local.shape[0])[selected]
    if coordinates.size < 8:
        raise ImageDataError("insufficient_samples", "Edge requires eight normal profile samples")
    plateaus = max(2, int(coordinates.size*float(settings.get("plateau_fraction", .2))))
    metrics = []
    refs = []
    esf_sides = []
    for side, image in zip(("baseline", "candidate"), samples):
        luminance = image[crop] @ LUMA
        profile = np.sum(luminance*local, axis=reduce_axis)[selected]/counts[selected]
        dark, bright = np.median(profile[:plateaus]), np.median(profile[-plateaus:])
        amplitude = bright-dark
        if amplitude == 0:
            raise ImageDataError("zero_edge_contrast", "Edge plateau luminances are equal")
        esf = (profile-dark)/amplitude
        lsf = np.gradient(esf, coordinates)
        window = signal.windows.hann(lsf.size, sym=False)
        spectrum = np.abs(np.fft.rfft(lsf*window))
        if spectrum[0] == 0:
            raise ImageDataError("zero_edge_contrast", "Edge LSF DC is zero")
        frequency = np.fft.rfftfreq(lsf.size, d=1)
        sfr = spectrum/spectrum[0]
        esf_sides.append(esf)
        refs.append(evidence.artifact(f"edge.{region}.{side}.esf", np.column_stack((coordinates, profile, esf, lsf)), "edge_response", columns=["normal_position_px", "linear_luminance", "normalized_ESF", "LSF_per_pixel"], domain="linear_srgb", input_contrast=settings["input_contrast"], input_exposure_stops=settings["input_exposure_stops"]))
        refs.append(evidence.artifact(f"edge.{region}.{side}.sfr", np.column_stack((frequency, sfr)), "edge_sfr", columns=["cycles_per_pixel", "SFR"], estimator="axis-normal mask-weighted ESF; gradient LSF; periodic Hann; abs(rFFT); normalize DC", interpretation="end-to-end nonlinear response, not independent emulsion MTF"))
        crossing = np.flatnonzero(sfr <= .5)
        mtf50 = None
        if crossing.size and crossing[0] > 0:
            k = crossing[0]
            mtf50 = frequency[k-1]+(.5-sfr[k-1])*(frequency[k]-frequency[k-1])/(sfr[k]-sfr[k-1])
        metrics.append({"overshoot": max(0, np.max(esf)-1), "undershoot": max(0, -np.min(esf)), "mtf50_cycles_per_pixel": mtf50, "plateau_contrast": amplitude})
    refs.append(evidence.artifact(f"edge.{region}.delta.esf", np.column_stack((coordinates, esf_sides[1]-esf_sides[0])), "edge_response", columns=["normal_position_px", "candidate_minus_baseline_normalized_ESF"]))
    for name in metrics[0]:
        units = "cycles/pixel" if "mtf" in name else "linear luminance" if name == "plateau_contrast" else "normalized edge amplitude"
        _paired_scalars(evidence, region, "edge", [{name: m[name]} for m in metrics], int(mask.sum()), "linear_srgb", units,
                        "fixed axis ESF; endpoint median plateaus; Hann-windowed gradient LSF SFR; linear first 0.5 crossing", refs)


def _tone(evidence, region, mask, labs, samples, protocol, refs):
    quantiles = [np.quantile(lab[..., 0][mask], [.05, .25, .5, .75, .95], method="linear") for lab in labs]
    for side, values in (("baseline", quantiles[0]), ("candidate", quantiles[1]), ("delta", quantiles[1]-quantiles[0])):
        for percent, value in zip((5, 25, 50, 75, 95), values):
            evidence.scalar(region, f"tone.l_star.{side}.q{percent:02d}", value, mask.sum(), "bounded_srgb_d65_lab", "L*", "frozen-region lightness pixel quantile method=linear", refs)
    for scale in protocol.get("region_sampling_scales", []):
        metrics = []
        contrast_refs = []
        for side, image in zip(("baseline", "candidate"), samples):
            luminance = image @ LUMA
            residual = luminance-ndimage.gaussian_filter(luminance, float(scale), mode="reflect")
            contrast_refs.append(evidence.artifact(f"tone.{region}.{side}.local_contrast.{scale}", residual, "local_contrast_map", domain="linear_srgb_luminance", units="linear luminance", sigma_px=float(scale)))
            metrics.append({"rms": np.sqrt(np.mean(residual[mask]**2))})
        _paired_scalars(evidence, region, f"tone.local_contrast.sigma_{scale}", metrics, mask.sum(), "linear_srgb_luminance", "linear luminance RMS", "Gaussian high-pass reflect; sigma="+str(scale)+"px; selected frozen pixels RMS", contrast_refs)
    points = protocol.get("probe", {}).get("tone_response", [])
    if not points or region != "full_image":
        return
    coordinates = np.asarray([point["exposure_stops"] for point in points], dtype=float)
    if len(points) < 2 or np.any(np.diff(coordinates) <= 0):
        raise ImageDataError("invalid_protocol", "Tone exposures must be strictly increasing with at least two points")
    responses = []
    curve_refs = []
    for side, image, lab in zip(("baseline", "candidate"), samples, labs):
        curve = []
        for point in points:
            selected = rectangle_mask(point["geometry"], [mask.shape[1], mask.shape[0]]) & mask
            if not selected.any():
                raise ImageDataError("invalid_probe_geometry", "Tone response point has no selected pixels")
            curve.append([np.median((image @ LUMA)[selected]), np.median(lab[..., 0][selected]), int(selected.sum())])
        response = np.asarray(curve)
        responses.append(response)
        curve_refs.append(evidence.artifact(f"tone.{region}.{side}.response", np.column_stack((coordinates, response)), "tone_response", columns=["input_exposure_stops", "linear_luminance", "L_star", "pixel_count"], estimator="frozen patch median"))
    curve_refs.append(evidence.artifact(f"tone.{region}.delta.response", np.column_stack((coordinates, responses[1][:, :2]-responses[0][:, :2])), "tone_response", columns=["input_exposure_stops", "delta_linear_luminance", "delta_L_star"]))
    for index in range(len(points)-1):
        interval = coordinates[index+1]-coordinates[index]
        for channel, label in enumerate(("linear_luminance", "l_star")):
            slope = [(response[index+1, channel]-response[index, channel])/interval for response in responses]
            _paired_scalars(evidence, region, f"tone.response.interval_{index}.{label}", [{"slope": value} for value in slope], 2, "linear_srgb_luminance" if channel == 0 else "bounded_srgb_d65_lab", "linear luminance/stop" if channel == 0 else "L*/stop", f"secant slope exposure interval [{coordinates[index]},{coordinates[index+1]}] stops", curve_refs)


def select_hotspots(values, settings, metric="flip", artifact_refs=()):
    """Rank tiles, merge selected touching rectangles, then rerank their means."""
    values = np.asarray(values)
    if values.ndim != 2 or not np.isfinite(values).all():
        raise ImageDataError("computation_failure", "Hotspot map must be finite HxW")
    count = int(settings.get("count", 0))
    minimum = int(settings.get("min_area_px", 1))
    side = int(np.ceil(np.sqrt(minimum)))
    height, width = values.shape
    records = []
    for y in range(0, height, side):
        for x in range(0, width, side):
            w, h = min(side, width-x), min(side, height-y)
            if w*h >= minimum:
                records.append([x, y, w, h])
    def ranking(rectangle):
        x, y, w, h = rectangle
        return (-float(np.mean(values[y:y+h, x:x+w])), y, x)
    records.sort(key=ranking)
    merged = records[:count]
    changed = True
    while changed:
        changed = False
        for first in range(len(merged)):
            x, y, w, h = merged[first]
            for second in range(first+1, len(merged)):
                a, b, c, d = merged[second]
                # Positive overlap in one axis and touching/overlap in the other.
                if ((x <= a+c and a <= x+w and y < b+d and b < y+h) or
                    (y <= b+d and b <= y+h and x < a+c and a < x+w)):
                    left, top = min(x, a), min(y, b)
                    merged[first] = [left, top, max(x+w, a+c)-left, max(y+h, b+d)-top]
                    merged.pop(second)
                    changed = True
                    break
            if changed:
                break
    merged.sort(key=ranking)
    return [{"id": f"automatic_{index+1:03d}", "geometry": rectangle, "rectangle": rectangle,
             "ranking_metric": metric, "ranking_value": -ranking(rectangle)[0],
             "selection_method": "rank square tiles by mean descending, ties y/x; select count; merge touching or overlapping rectangles to fixed point; rerank merged rectangle means",
             "artifact_refs": list(artifact_refs)} for index, rectangle in enumerate(merged)]


def aggregate_seed_measurements(seed_results):
    """Equal-weight seed mean and ddof=1 deviation, retaining original rows."""
    groups = {}
    expected = {}
    for result in seed_results:
        if "seed" in result:
            expected.setdefault(result["case_id"], []).append(result["seed"])
    for result in seed_results:
        for row in result.get("measurements", []):
            key = (row["case_id"], row["region_id"], row["metric"], row["domain"], row["units"], row["estimator"])
            groups.setdefault(key, []).append(row)
    output = []
    for key, rows in sorted(groups.items(), key=lambda item: str(item[0])):
        seeds = [seed for row in rows for seed in row.get("seed_population", [])]
        complete = set(seeds) == set(expected.get(key[0], seeds))
        available = complete and all(row["availability"] == "available" for row in rows)
        values = np.asarray([row["value"] for row in rows if row["availability"] == "available"])
        for suffix in ("seed_mean", "seed_std"):
            row = dict(rows[0])
            row.update(metric=key[2]+"."+suffix, seed_population=seeds, sample_count=len(rows), seed_count=len(rows),
                       estimator="equal-weight per-seed arithmetic mean" if suffix == "seed_mean" else "cross-seed sample standard deviation ddof=1",
                       artifact_refs=list(dict.fromkeys(ref for source in rows for ref in source.get("artifact_refs", []))),
                       per_seed_measurements=rows)
            row["expected_seed_population"] = expected.get(key[0], seeds)
            row.pop("value", None)
            row.pop("reason", None)
            if not available:
                row.update(availability="unavailable", reason="missing_seed_measurement" if not complete else "unavailable_seed_measurement")
            elif suffix == "seed_std" and len(rows) < 2:
                row.update(availability="unavailable", reason="insufficient_seeds")
            else:
                row.update(availability="available", value=float(np.mean(values) if suffix == "seed_mean" else np.std(values, ddof=1)))
            output.append(row)
    return output


def compare_images(baseline, candidate, regions, protocol, out_dir):
    evidence = _Evidence(protocol, out_dir)
    populations = [{"id": "full_image", "measurements": protocol.get("measurements", ["numeric", "color", "tone"])}, *(regions or [])]
    try:
        images = [load_image(baseline), load_image(candidate)]
        evidence.result["numeric_health"] = {side: numeric_health(image) for side, image in zip(("baseline", "candidate"), images)}
        if images[0].dimensions != images[1].dimensions:
            raise ImageDataError("dimension_mismatch", "Masters differ in dimensions; no resize performed")
        if any(not health["all_finite"] for health in evidence.result["numeric_health"].values()):
            raise ImageDataError("nonfinite_samples", "Native master contains nonfinite samples")
        encoded = [display_rgb(image) for image in images]
        if any(not np.isfinite(image).all() for image in encoded):
            raise ImageDataError("computation_failure", "Display transform produced nonfinite values")
        linear = [linear_rgb(image) for image in images]
        labs = [rgb_to_lab(np.clip(image, 0, 1)) for image in encoded]
        evidence.result["display_health"] = {side: {"below_zero_fraction": float(np.mean(image < 0)), "above_one_fraction": float(np.mean(image > 1)), "clipped_sample_fraction": float(np.mean((image < 0) | (image > 1))), "sample_count": int(image.size), "domain": "unclipped_encoded_srgb"} for side, image in zip(("baseline", "candidate"), encoded)}
        signed = labs[1]-labs[0]
        maps = {"delta_e00": ciede2000(*labs), "delta_l_star": signed[..., 0], "delta_a_star": signed[..., 1], "delta_b_star": signed[..., 2], "delta_c_star": np.hypot(labs[1][..., 1], labs[1][..., 2])-np.hypot(labs[0][..., 1], labs[0][..., 2])}
        map_refs = {name: evidence.artifact(name, values, "difference_map", metric=name, domain="bounded_srgb_d65_lab", units={"delta_e00": "delta E 00", "delta_l_star": "L*", "delta_a_star": "a*", "delta_b_star": "b*", "delta_c_star": "C*"}[name], dimensions=images[0].dimensions) for name, values in maps.items()}
        if any("flip" in population.get("measurements", []) for population in populations):
            maps["flip"], used = _official_flip(*encoded, protocol["pixels_per_degree"])
            map_refs["flip"] = evidence.artifact("flip", maps["flip"], "difference_map", metric="flip", domain="bounded_encoded_srgb", units="FLIP error", pixels_per_degree=protocol["pixels_per_degree"], implementation="NVlabs flip-evaluator "+version("flip-evaluator"), output_size=images[0].dimensions)
            evidence.result["flip_parameters"] = {"ppd": float(protocol["pixels_per_degree"])}
        seen = set()
        for population in populations:
            region = population["id"]
            selected = population.get("measurements", [])
            try:
                if region in seen:
                    raise ImageDataError("invalid_region", "Duplicate or reserved full_image region")
                seen.add(region)
                mask = np.ones(images[0].samples.shape[:2], dtype=bool) if region == "full_image" else rectangle_mask(population["geometry"], images[0].dimensions)
                count = int(mask.sum())
                for family in selected:
                    if family == "numeric":
                        same = (images[0].color_space, images[0].transfer) == (images[1].color_space, images[1].transfer)
                        delta = np.asarray(images[1].samples[mask], dtype=float)-np.asarray(images[0].samples[mask], dtype=float)
                        maximum = np.max(np.abs(delta))
                        rmse = 0 if maximum == 0 else maximum*np.sqrt(np.mean((delta/maximum)**2))
                        for name, value in (("max_abs_difference", maximum), ("rmse", rmse)):
                            evidence.scalar(region, "numeric."+name, value, delta.size, "native:"+str(images[0].color_space)+":"+str(images[0].transfer), "native channel sample", "aligned native channel "+name, reason=None if same else "color_declaration_mismatch")
                        for side, image in zip(("baseline", "candidate"), images):
                            health = numeric_health(type(image)(image.samples[mask].reshape(1, count, 3), image.color_space, image.transfer))
                            for name in ("nonfinite_sample_count", "nonfinite_pixel_count", "below_zero_fraction", "above_one_fraction"):
                                evidence.scalar(region, f"numeric.{side}.{name}", health[name], count if name.endswith("pixel_count") else count*3, "native", "fraction" if name.endswith("fraction") else "count", "native all-channel sample population")
                    elif family == "color":
                        _summary(evidence, region, "delta_e00", maps["delta_e00"][mask], [map_refs["delta_e00"]], "bounded_srgb_d65_lab", "delta E 00")
                        for name, units in (("delta_l_star", "L*"), ("delta_a_star", "a*"), ("delta_b_star", "b*"), ("delta_c_star", "C*")):
                            for statistic, function in (("mean", np.mean), ("median", np.median)):
                                evidence.scalar(region, name+"."+statistic, function(maps[name][mask]), count, "bounded_srgb_d65_lab", units, "signed pixel "+statistic+" candidate minus baseline", [map_refs[name]])
                        for threshold in protocol.get("delta_e_thresholds", []):
                            evidence.scalar(region, f"delta_e00.fraction_above_{threshold}", np.mean(maps["delta_e00"][mask] > threshold), count, "bounded_srgb_d65_lab", "fraction", "pixel count above threshold="+str(threshold)+" / valid selected pixels", [map_refs["delta_e00"]])
                    elif family == "flip":
                        _summary(evidence, region, "flip", maps["flip"][mask], [map_refs["flip"]], "bounded_encoded_srgb", "FLIP error", mean=True)
                    elif family == "tone":
                        _tone(evidence, region, mask, labs, linear, protocol, [map_refs["delta_l_star"]])
                    elif family == "grain":
                        _grain(evidence, region, mask, linear, protocol["grain"], population.get("role") == "flat_field" or protocol.get("probe", {}).get("kind") == "flat_field")
                    elif family == "halation":
                        _halation(evidence, region, mask, linear, protocol.get("halation", {}))
                    elif family == "edge":
                        _edge(evidence, region, mask, linear, protocol.get("edge", {}))
                    else:
                        raise ImageDataError("invalid_protocol", "Unknown family: "+family)
            except (ImageDataError, ValueError, KeyError, OSError, FloatingPointError) as error:
                code = error.code if isinstance(error, ImageDataError) else "computation_failure"
                evidence.scalar(region, family if "family" in locals() else "region", None, 0, "declared_measurement", "unavailable", "selected family execution", reason=code)
                evidence.failure(region, family if "family" in locals() else "region", code, str(error), list(map_refs.values()), execution=code not in ("missing_probe_geometry", "insufficient_samples", "zero_edge_contrast"))
        ranking = "flip" if "flip" in maps else "delta_e00"
        evidence.result["automatic_regions"] = select_hotspots(maps[ranking], protocol.get("hotspots", {}), ranking, [map_refs[ranking]])
    except Exception as error:
        code = error.code if isinstance(error, ImageDataError) else "computation_failure"
        evidence.failure("full_image", "image", code, str(error))
        for population in populations:
            for family in population.get("measurements", []):
                evidence.scalar(population["id"], family, None, 0, "declared_measurement", "unavailable",
                                "selected family boundary failure", reason=code)
    return evidence.result
