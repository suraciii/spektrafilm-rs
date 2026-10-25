//! Presentation adapter for fixed internal labels used by existing control APIs.
//! Canonical paths, edit labels and widget salts never pass through this mapping.
use std::borrow::Cow;

fn key(label: &str) -> Option<&'static str> {
    match label {
        "Photograph toolbar" => Some("controls.owner.photograph_toolbar"),
        "Result actions" => Some("controls.owner.result_actions"),
        "Work list" => Some("photo_work.list.title"),
        "Retained image" => Some("controls.section.retained_image"),
        "Photograph records" => Some("controls.section.photograph_records"),
        "Export queue" => Some("photo.queue.title"),
        "Preset discovery" => Some("controls.section.preset_discovery"),
        "Continue saving retained pixels" => Some("result.save.continue"),
        "Save adjustments…" => Some("photo.record.save"),
        "Reopen adjustments…" => Some("photo.record.reopen"),
        "Photographs…" => Some("photo.work_list"),
        "Add photographs…" => Some("photo_work.list.add"),
        "Previous photograph" => Some("photo_work.list.previous"),
        "Next photograph" => Some("photo_work.list.next"),
        "Remove entry" => Some("photo_work.list.remove"),
        "Choose an adjustment scope…" => Some("photo_work.list.choose_scope"),
        "Copy scope to selected destinations" => Some("photo_work.list.copy"),
        "Choose output directory…" => Some("photo_work.list.choose_directory"),
        "Allow replacement of existing output files" => Some("photo_work.list.replace"),
        "Submit selected photographs" => Some("photo_work.list.submit"),
        "Cancel remaining queue entries" => Some("photo.queue.cancel_remaining"),
        "Cancel this export" => Some("photo.queue.cancel"),
        "Retry submitted snapshot" => Some("photo.queue.retry_snapshot"),
        "All looks" => Some("surfaces.presets.discovery_all"),
        "Favorites" => Some("surfaces.presets.discovery_favorites"),
        "Recently used" => Some("surfaces.presets.discovery_recent"),
        "Clear search and filters" => Some("surfaces.presets.discovery_reset"),
        "retained image" => Some("controls.condition.retained_image"),
        "stale Save confirmation is open" => Some("controls.condition.stale_save"),
        "adjacent work list entry" => Some("controls.condition.work_adjacent"),
        "work list entry; removes membership, not files" => Some("controls.condition.work_remove"),
        "loaded source; selected destinations; explicit scope" => {
            Some("controls.condition.work_copy")
        }
        "selected destinations; output directory" => Some("controls.condition.work_submit"),
        "pending queued exports" => Some("controls.condition.queue_pending"),
        "pending queue entry; first eligible entry" => Some("controls.condition.queue_cancel"),
        "failed queue entry; first eligible entry" => Some("controls.condition.queue_retry"),
        "favorites retain built-in or user group identity" => {
            Some("controls.condition.preset_favorites")
        }
        "no matching presets" => Some("controls.condition.preset_empty"),
        " • applied" => Some("surfaces.presets.suffix_applied"),
        " • missing" => Some("surfaces.presets.suffix_missing"),
        " • modified" => Some("surfaces.presets.suffix_modified"),
        "%{cause}; %{detail}" => Some("surfaces.status.normalization"),
        "%{format} · %{depth} · %{space} · %{encoding} · %{compression}" => {
            Some("surfaces.export.summary_tiff")
        }
        "%{format} · %{depth} · %{space} · linear · ZIP" => Some("surfaces.export.summary_exr"),
        "%{format} · 8 bit · %{space} · encoded" => Some("surfaces.export.summary_png"),
        "%{format} · 8 bit · %{space} · quality %{quality} · %{subsampling}" => {
            Some("surfaces.export.summary_jpeg")
        }
        "%{kind} in flight · %{mode} (captured)" => Some("surfaces.diagnostics.in_flight"),
        "%{kind} queued · %{mode} (captured)" => Some("surfaces.diagnostics.queued"),
        "%{label}: %{reason}; stored values retained." => {
            Some("controls.grain.unavailable_profile")
        }
        "%{percent}% fit" => Some("surfaces.viewer.zoom_fit"),
        "%{role}: %{name}" => Some("surfaces.presets.role"),
        "%{stage} stage" => Some("surfaces.section.stage"),
        "1.0 = full simulation · 0.0 = original" => Some("controls.output.blend_guide"),
        "16 bit" => Some("surfaces.export.bit16"),
        "32 bit" => Some("surfaces.export.bit32"),
        "3x3 device-correction matrix (9 numbers, row-major) applied to the input before inversion, to undo the scanner/camera colour rendering vs the standard observer over the film dyes (the cross-channel cast that the scan illuminant and base cannot reach). Use 'Blind calibration' to fit it from the current image." => {
            Some("controls.convert.calibration_help")
        }
        "8 bit" => Some("surfaces.export.bit8"),
        "Action" => Some("controls.metadata.action"),
        "Add a bias to the auto-exposure of the camera." => {
            Some("controls.scanner.camera_exposure_help")
        }
        "Add grain to the negative" => Some("controls.grain.enable_help"),
        "Add readout stray light after lens blur and before scanner correction. This does not change print exposure or developed density." => {
            Some("controls.help.glare.enable")
        }
        "Additive offset on the family's halo warmth axis. Positive = warm outer halo / cool inner halo. Energy-preserving per channel. 0 = use family default." => {
            Some("controls.help.diffusion.warmth")
        }
        "Advanced. Multiplier on the bloom lambda. 1.0 = use family default." => {
            Some("controls.help.diffusion.bloom_size")
        }
        "Advanced. Multiplier on the bloom weight; the three group weights are renormalized to sum to 1. 1.0 = use family default." => {
            Some("controls.help.diffusion.bloom_intensity")
        }
        "Advanced. Multiplier on the core lambda. 1.0 = use family default." => {
            Some("controls.help.diffusion.core_size")
        }
        "Advanced. Multiplier on the core weight; the three group weights are renormalized to sum to 1. 1.0 = use family default." => {
            Some("controls.help.diffusion.core_intensity")
        }
        "Advanced. Multiplier on the halo lambda. 1.0 = use family default." => {
            Some("controls.help.diffusion.halo_size")
        }
        "Advanced. Multiplier on the halo weight; the three group weights are renormalized to sum to 1. 1.0 = use family default." => {
            Some("controls.help.diffusion.halo_intensity")
        }
        "Aligns the scan's overall brightness to the model (gain = 2^ev). Brighter input -> less recovered film density." => {
            Some("controls.convert.exposure_compensation_help")
        }
        "Amount" => Some("controls.grain.amount"),
        "Analogue" => Some("controls.grain.analogue"),
        "Applied preset '%{name}'" => Some("surfaces.status.preset_applied"),
        "Apply lens corrections" => Some("controls.scanner.lens_correction_help"),
        "Apply saving CCTF encoding" => Some("controls.label.apply_saving_cctf_encoding"),
        "Apply the copied four values to this frame's own automatic setup." => {
            Some("controls.scanner.paste_correction_help")
        }
        "Apply the hanatos2025 bandpass adaptation window when reconstructing spectra." => {
            Some("controls.upsampling.adaptation_window_help")
        }
        "Apply the hanatos2025 surface adaptation polynomial when reconstructing spectra." => {
            Some("controls.upsampling.adaptation_surface_help")
        }
        "Apply the inverse color component transfer function (CCTF) of the input color space, so the declared encoded samples become linear before the pipeline reads them." => {
            Some("controls.scanner.input_cctf_decoding_help")
        }
        "Apply the output color space transfer encoding to runtime output. This is independent of retained Save encoding and display presentation." => {
            Some("controls.output.output_cctf_help")
        }
        "Authored effective RMS optical-density target; not measured stock calibration." => {
            Some("controls.grain.fotufilm_rms_density_help")
        }
        "Authored effective population clump radius in film micrometers." => {
            Some("controls.grain.fotufilm_cloud_radius_help")
        }
        "Authored effective relative population weight per layer." => {
            Some("controls.grain.fotufilm_layer_weights_help")
        }
        "Authored share; transfers weight to or from sublayer 4." => {
            Some("controls.help.shares.transfer")
        }
        "Auto" => Some("controls.grain.auto"),
        "Auto normalizes the print exposure so the developed midgray density prints to the paper midgray and follows camera exposure compensation, like a color analyzer or minilab. Manual applies the print exposure time alone, like a test strip." => {
            Some("controls.scanner.print_exposure_mode_help")
        }
        "Backend" => Some("controls.label.backend"),
        "Base" => Some("controls.base.print_section"),
        "Base density: scale and per-channel masks" => Some("surfaces.summary.print_base"),
        "Base density: scale, tilt, and per-channel masks" => Some("surfaces.summary.base"),
        "Base statistical event area in square film micrometers; controls grain strength independently of dye support." => {
            Some("controls.grain.event_area_help")
        }
        "Base statistical particle area in square film micrometers; smaller values give finer grain. Spektra reads this value; Dye uses its own event area." => {
            Some("controls.grain.particle_area_help")
        }
        "Bit depth" => Some("controls.label.bit_depth"),
        "Bit depth: 8 bit — required by JPEG and PNG" => Some("surfaces.export.depth_fixed"),
        "Blend all three film sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset." => {
            Some("controls.chemistry.film_developer_exhaustion_help")
        }
        "Blend all three print sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset." => {
            Some("controls.chemistry.print_developer_exhaustion_help")
        }
        "Blend the complete result with the decoded, geometrically aligned prepared source in linear output RGB. 1 returns the complete result; 0 returns the prepared source before virtual camera exposure. For a captured negative, zero returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent." => {
            Some("controls.output.simulation_blend_help")
        }
        "Blur (viewing glare)" => Some("controls.label.blur_viewing_glare"),
        "Brightest-pixels percentile used by 'Detect base' to sample the clear / unexposed film (99 = brightest 1%). Detect base fits the film Base so that clear film maps to density 0 (unexposed -> neutral)." => {
            Some("controls.convert.base_percentile_help")
        }
        "Built-in" => Some("surfaces.presets.builtin"),
        "Built-in feature profile; selecting it changes only Halation profile leaves." => {
            Some("controls.halation.profile_help")
        }
        "Built-in feature profile; selecting it changes only the declared profile scope." => {
            Some("controls.grain.profile_help")
        }
        "CCTF is the color component transfer function. Apply the saving transfer encoding to the samples Export writes; off writes linear samples. This file conversion is independent of the viewer-only display transform." => {
            Some("surfaces.export.cctf_help")
        }
        "CMYK ink style" => Some("controls.cmyk.section"),
        "CMYK ink style appearance strength." => Some("controls.cmyk.strength_help"),
        "CPU %{stage}: %{reason}" => Some("surfaces.diagnostics.cpu_reason"),
        "CPU: highest precision, slower. GPU: faster, f32 precision." => {
            Some("surfaces.export.backend_help")
        }
        "Camera" => Some("controls.label.camera"),
        "Camera diffusion" => Some("controls.label.camera_diffusion"),
        "Camera taking filter held in front of the lens. Its measured spectral transmittance multiplies the incoming light, so it both darkens and color-casts the exposure (e.g. Hoya Y2/YA3 yellow, R1 red, X0/X1 UV/haze). 'none' = no filter." => {
            Some("controls.scanner.color_filter_help")
        }
        "Cancel" => Some("surfaces.action.cancel"),
        "Cancel export" => Some("surfaces.export.cancel"),
        "Cancel using the export worker; an output not yet published is discarded." => {
            Some("surfaces.export.cancel_help")
        }
        "Cannot reveal %{path}: %{detail}" => Some("surfaces.presets.reveal_error"),
        "Capture illuminant inactive on this route; stored values are retained." => {
            Some("controls.scanner.capture_inactive")
        }
        "Center of the crop region in relative coordinates in x, y (0-1)" => {
            Some("controls.crop.center_help")
        }
        "Changes the exposure time set in the virtual enlarger" => {
            Some("controls.scanner.print_exposure_help")
        }
        "Channel correlation" => Some("controls.grain.channel_correlation"),
        "Choice" => Some("controls.metadata.choice"),
        "Choose an ICC profile for the display transform applied to the viewer raster only. It never reaches retained, saved or exported pixels." => {
            Some("surfaces.display.select_help")
        }
        "Chroma" => Some("controls.grain.chroma"),
        "Chromatic grain amount." => Some("controls.grain.chroma_help"),
        "Clear preset search" => Some("surfaces.presets.clear_search"),
        "Clear search and restore ordinary inspector disclosure." => {
            Some("surfaces.search.clear_help")
        }
        "Close settings" => Some("surfaces.settings.close"),
        "Cloud radius (µm)" => Some("controls.grain.cloud_radius"),
        "Coating density scale" => Some("controls.grain.coating_density_scale"),
        "Collapse or expand Presets" => Some("surfaces.presets.toggle"),
        "Color" => Some("controls.grain.color"),
        "Color space and transfer encoding of the file Export writes. This is file conversion, distinct from the output color space of the runtime digital output and from the display transform, which is viewer-only." => {
            Some("surfaces.export.space_help")
        }
        "Color space and transfer encoding of the runtime digital output. It describes what the simulation produces, which is distinct from the saving color space used for the file and from the display transform, which is viewer-only." => {
            Some("controls.output.output_color_space_help")
        }
        "Color space of the file Save writes. Save writes the retained output in this space without rerendering; Export renders and writes independently with its own saving color space." => {
            Some("controls.output.saving_color_space_help")
        }
        "Color space of the input image, will be internally converted to sRGB and negative values clipped" => {
            Some("controls.scanner.input_color_space_help")
        }
        "Color-manage the input RGB image." => Some("surfaces.route.input"),
        "Commercial filter stop: 0, 1/8=0.125, 1/4=0.25, 1/2=0.5, 1, 2. Maps internally to the (p_s, p_a) deflected/absorbed photon fractions." => {
            Some("controls.help.diffusion.strength")
        }
        "Common exposure contrast around the fixed reference exposure. Above 1 is harder; below 1 is softer. Channel differences and automatic setup are unchanged." => {
            Some("controls.scanner.contrast_help")
        }
        "Compress input chromaticities toward the visible spectral locus before spectral upsampling. Off passes input through unchanged." => {
            Some("controls.gamut.input_enable_help")
        }
        "Compression" => Some("controls.label.compression"),
        "Compression: JPEG lossy — use JPEG quality and subsampling" => {
            Some("surfaces.export.compression_jpeg")
        }
        "Compression: PNG lossless — fixed by the PNG writer" => {
            Some("surfaces.export.compression_png")
        }
        "Compression: ZIP — required by the EXR writer" => {
            Some("surfaces.export.compression_exr")
        }
        "Contrast" => Some("controls.scanner.contrast"),
        "Controls how quickly the highlight boost ramps in, from 0 to 1." => {
            Some("controls.halation.boost_range_help")
        }
        "Conversion style inactive until Negative to positive is selected; stored values are retained." => {
            Some("controls.scanner.conversion_inactive")
        }
        "Convert" => Some("controls.label.convert"),
        "Convert the input and scan the film with its base removed." => {
            Some("surfaces.route.minus_base")
        }
        "Convert the input to a negative capture, print it photographically, then scan the paper." => {
            Some("surfaces.route.convert_print")
        }
        "Convert the input to a negative capture, then scan the film." => {
            Some("surfaces.route.convert_scan")
        }
        "Copy only the four operator values for this session, not automatic setup, film density or capture settings." => {
            Some("controls.scanner.copy_correction_help")
        }
        "Copy operator correction" => Some("controls.scanner.copy_correction"),
        "Could not apply the Halation profile: %{error}" => {
            Some("controls.halation.profile_apply_error")
        }
        "Could not apply the grain profile: %{error}" => Some("controls.grain.profile_apply_error"),
        "Couplers" => Some("controls.couplers.section"),
        "Crop and upscale" => Some("controls.crop.section"),
        "Crop image to a fraction of the original size to preview details at full scale" => {
            Some("controls.crop.crop_help")
        }
        "Current look: %{group} / %{name} • %{state}" => Some("surfaces.presets.current"),
        "Custom" => Some("controls.scanner.custom"),
        "Custom-only controls are inactive under a named profile; stored overrides are retained. Select Custom to edit them." => {
            Some("controls.grain.named_profile_custom_controls")
        }
        "Cyan-channel film base density shift around 650 nm (1.0 = neutral; adds (value - 1) density)." => {
            Some("controls.base.cyan_film_help")
        }
        "Cyan-channel multiplicative scale of the print base min around 610 nm (1.0 = neutral)." => {
            Some("controls.base.cyan_print_help")
        }
        "DIR couplers: inhibition and interlayer response" => Some("surfaces.summary.couplers"),
        "DIR inhibition from the B layer onto the R and G layers respectively (g_B->R, g_B->G)." => {
            Some("controls.couplers.gamma_blue_to_red_green_help")
        }
        "DIR inhibition from the G layer onto the R and B layers respectively (g_G->R, g_G->B)." => {
            Some("controls.couplers.gamma_green_to_red_blue_help")
        }
        "DIR inhibition from the R layer onto the G and B layers respectively (g_R->G, g_R->B)." => {
            Some("controls.couplers.gamma_red_to_green_blue_help")
        }
        "Decay constant of the scatter exponential tail per channel [R,G,B], in micrometers (internally approximated by a sum of Gaussians). Controls extended low-level spread within the emulsion." => {
            Some("controls.halation.scatter_tail_help")
        }
        "Delete" => Some("surfaces.action.delete"),
        "Delete '%{name}' from My Presets?" => Some("surfaces.presets.delete_prompt"),
        "Delete preset?" => Some("surfaces.presets.delete_title"),
        "Deleted preset '%{name}'" => Some("surfaces.status.preset_deleted"),
        "Delete…" => Some("surfaces.presets.delete_action"),
        "Density curves: development time, gamma, developer exhaustion" => {
            Some("surfaces.summary.chemistry")
        }
        "Density unsharp-mask amount." => Some("controls.grain.unsharp_amount_help"),
        "Density unsharp-mask radius." => Some("controls.grain.unsharp_sigma_help"),
        "Detail of the inscribed-hull compression boundary = number of FFT modes kept. HIGHER keeps more locus detail (more cusp chroma) but re-admits polygon kinks; LOWER is smoother, rounding the spectral tips inward toward a circle. Note: this is modes-kept, not a blur width — larger means LESS smoothing." => {
            Some("controls.gamut.hull_detail_help")
        }
        "Development time for a BW development-time family: selects the density curve and base+fog to render. '—' uses the representative middle development; ignored for single-curve and color stocks." => {
            Some("surfaces.profiles.time_help")
        }
        "Development time inactive: %{reason}; stored selection retained." => {
            Some("surfaces.chemistry.inactive")
        }
        "Development time: %{time} min · material-defined (no selectable family)" => {
            Some("surfaces.chemistry.material_time")
        }
        "Diagnostic details unavailable." => Some("surfaces.diagnostics.unavailable"),
        "Diagnostic report publication error: %{error}" => {
            Some("surfaces.status.publication_error")
        }
        "Diagnostic report save error: %{error}" => Some("surfaces.status.report_error"),
        "Diagnostics" => Some("surfaces.diagnostics.title"),
        "Dialog directory persistence error: %{error}" => Some("surfaces.status.directory_error"),
        "Digital scan sharpening after interpretation and output gamut processing: [sigma in pixels, amount]." => {
            Some("controls.scanner.unsharp_mask_help")
        }
        "Disable print viewing glare for a print scan; toggle again to restore its previous state." => {
            Some("surfaces.scanner.print_help")
        }
        "Discard these export settings. The retained output is unchanged." => {
            Some("surfaces.export.discard_help")
        }
        "Display" => Some("controls.label.display"),
        "Display ICC profile cleared" => Some("surfaces.status.display_cleared"),
        "Display ICC profile: %{path}" => Some("surfaces.status.display_profile"),
        "Display ICC: %{profile}" => Some("surfaces.display.profile"),
        "Display ICC: platform profile or sRGB fallback" => Some("surfaces.display.fallback"),
        "Display transform: %{error}" => Some("surfaces.display.error"),
        "Display transform: ICC failed (%{error}); viewing sRGB preview" => {
            Some("surfaces.display.failed")
        }
        "Display transform: ICC failed; using the sRGB preview; viewing sRGB preview" => {
            Some("surfaces.display.invalid_profile")
        }
        "Display transform: active (%{path})" => Some("surfaces.display.active"),
        "Display transform: disabled; viewing output-space values" => {
            Some("surfaces.display.disabled")
        }
        "Display transform: missing sRGB preview; viewing output-space values" => {
            Some("surfaces.display.missing_preview")
        }
        "Display transform: sRGB preview; no display profile" => {
            Some("surfaces.display.no_profile")
        }
        "Duplicate to My Presets…" => Some("surfaces.presets.duplicate_builtin"),
        "Duplicate…" => Some("surfaces.presets.duplicate"),
        "Dye" => Some("controls.grain.dye"),
        "Dye negative" => Some("controls.grain.dye_negative"),
        "Dye reversal" => Some("controls.grain.dye_reversal"),
        "Dye support (µm)" => Some("controls.grain.dye_support"),
        "Dye-cloud blur in micrometers." => Some("controls.grain.dye_cloud_blur_help"),
        "EXR uses linear color and ZIP compression; encoding is fixed off." => {
            Some("surfaces.export.exr_fixed")
        }
        "Edit the first three shares; the fourth is the remaining share so the authored total stays one. All four shares must be positive." => {
            Some("controls.help.shares.help")
        }
        "Effective Fotufilm grain amount." => Some("controls.grain.fotufilm_amount_help"),
        "Effective Fotufilm particle size multiplier." => Some("controls.grain.fotufilm_size_help"),
        "Effective Fotufilm particle softness." => Some("controls.grain.fotufilm_softness_help"),
        "Effective chromatic grain amount." => Some("controls.grain.fotufilm_color_help"),
        "Effective coating density scale." => Some("controls.grain.fotufilm_coating_density_help"),
        "Effective fog density." => Some("controls.grain.fotufilm_fog_density_help"),
        "Effective grain amount per emulsion layer." => {
            Some("controls.grain.fotufilm_layer_amounts_help")
        }
        "Effective grain material." => Some("controls.grain.fotufilm_material_help"),
        "Effective population radius span." => Some("controls.grain.fotufilm_radius_span_help"),
        "Effective relative particle size per layer." => {
            Some("controls.grain.fotufilm_layer_size_ratio_help")
        }
        "Enable CMYK ink style" => Some("controls.cmyk.enable"),
        "Enable DIR coupler inhibition." => Some("controls.couplers.enable_help"),
        "Enable DIR couplers" => Some("controls.couplers.enable"),
        "Enable film base" => Some("controls.base.enable_film"),
        "Enable film base-density (film base + fog / orange mask) tuning." => {
            Some("controls.base.enable_film_help")
        }
        "Enable film density curves" => Some("controls.chemistry.enable_film_curves"),
        "Enable film density-curve morphing." => Some("controls.chemistry.enable_film_curves_help"),
        "Enable grain" => Some("controls.grain.enable"),
        "Enable halation" => Some("controls.halation.enable"),
        "Enable halation and in-emulsion scatter." => Some("controls.halation.enable_help"),
        "Enable input gamut compress" => Some("controls.gamut.input_enable"),
        "Enable print base" => Some("controls.base.enable_print"),
        "Enable print base-density tuning." => Some("controls.base.enable_print_help"),
        "Enable print density curves" => Some("controls.chemistry.enable_print_curves"),
        "Enable print density-curve morphing." => {
            Some("controls.chemistry.enable_print_curves_help")
        }
        "Enable print glare" => Some("controls.label.enable_print_glare"),
        "Enable the independent CMYK ink style processing stage; retains its strength while disabled." => {
            Some("controls.cmyk.enable_help")
        }
        "Engine" => Some("controls.grain.engine"),
        "Enlarger" => Some("controls.label.enlarger"),
        "Enlarger diffusion" => Some("controls.label.enlarger_diffusion"),
        "Enlarger inactive on this route; stored values are retained." => {
            Some("controls.scanner.enlarger_inactive")
        }
        "Enlarger optics: core, tail, and warmth" => Some("surfaces.summary.enlarger_diffusion"),
        "Enter any shared catalog illuminant or BB<kelvin> (1667..=25000)." => {
            Some("controls.convert.custom_illuminant_help")
        }
        "Estimated output size: ≈ %{size}" => Some("surfaces.export.size"),
        "Estimated output size: ≈ %{size} before compression" => {
            Some("surfaces.export.size_compressed")
        }
        "Event area (µm²)" => Some("controls.grain.event_area"),
        "Experimental local source-response compression for halation only. Zero preserves the linear response; higher values make secondary bright sources contribute more nearly like the brightest source without changing the base film raw exposure." => {
            Some("controls.halation.source_compression_help")
        }
        "Explicit Grain Procedural phase timer; 0 uses the seed-derived phase." => {
            Some("controls.grain.timer_help")
        }
        "Exponential tail scale in um for long-range coupler spread. Larger values extend the rare long-distance inhibition halo beyond the Gaussian core." => {
            Some("controls.couplers.diffusion_tail_help")
        }
        "Export" => Some("surfaces.action.export"),
        "Export file…" => Some("surfaces.export.submit"),
        "Export options" => Some("surfaces.export.title"),
        "File and container" => Some("surfaces.export.section_file"),
        "Color and computation" => Some("surfaces.export.section_color"),
        "Summary and actions" => Some("surfaces.export.section_summary"),
        "Exported preset to %{path}" => Some("surfaces.status.preset_exported"),
        "Export…" => Some("surfaces.presets.export"),
        "Expose film and scan the selected film medium." => Some("surfaces.route.film_scan"),
        "Expose film, print it photographically, then scan the paper." => {
            Some("surfaces.route.film_print_scan")
        }
        "Factory reset error: %{error}" => Some("surfaces.status.factory_error"),
        "Film" => Some("controls.label.film"),
        "Film Format" => Some("controls.scanner.film_format"),
        "Film Resolution" => Some("controls.grain.film_resolution"),
        "Film Type" => Some("controls.grain.film_type"),
        "Film base" => Some("controls.label.film_base"),
        "Film chemistry" => Some("controls.chemistry.film_section"),
        "Film material" => Some("controls.label.film_material"),
        "Film polarity for procedural grain." => Some("controls.grain.film_type_help"),
        "Film profile changed" => Some("surfaces.status.film_changed"),
        "Film stock to simulate" => Some("surfaces.profiles.film_help"),
        "Film: %{name}" => Some("surfaces.header.film"),
        "Finished exposure" => Some("controls.scanner.finished_exposure"),
        "Finished-positive exposure in stops. Positive values brighten the result after automatic setup, before the fixed tone curve; this does not change camera exposure or film density." => {
            Some("controls.scanner.finished_exposure_help")
        }
        "Fit and recenter the viewer. Retained and written pixels are unchanged." => {
            Some("surfaces.viewer.reset_help")
        }
        "Fit the correction matrix from the current vibrant image; review the fitted values afterwards." => {
            Some("controls.convert.blind_calibration_help")
        }
        "Fit the retained image in the viewer, leaving pan unchanged. Viewer only." => {
            Some("surfaces.viewer.fit_help")
        }
        "Fog density" => Some("controls.grain.fog_density"),
        "Format" => Some("controls.label.format"),
        "Fotufilm Film" => Some("controls.grain.fotufilm"),
        "Fotufilm advanced" => Some("controls.grain.fotufilm_advanced"),
        "Fraction of coupler diffusion energy assigned to the long tail. 0 = pure Gaussian spread, higher values add a longer-range inhibition shoulder." => {
            Some("controls.couplers.diffusion_tail_weight_help")
        }
        "Frame Long Edge" => Some("controls.scanner.frame_long_edge"),
        "GPU passes and measurement availability" => Some("surfaces.diagnostics.gpu"),
        "GPU timing" => Some("surfaces.diagnostics.gpu_timing"),
        "GUI parameters" => Some("controls.label.gui_parameters"),
        "Gamma applied to the film density curves during development; 1.0 keeps the profile curves unchanged. Requires an actual film-development route." => {
            Some("controls.chemistry.film_density_curve_gamma_help")
        }
        "Gamma applied to the paper density curves during print development; 1.0 keeps the profile curves unchanged. Requires an admitted print-development route." => {
            Some("controls.chemistry.print_density_curve_gamma_help")
        }
        "Gamma factor applied to the fast sub-layer." => {
            Some("controls.chemistry.gamma_factor_fast_help")
        }
        "Gamma factor applied to the mid and slow sub-layers." => {
            Some("controls.chemistry.gamma_factor_slow_help")
        }
        "Gaussian sigma in pixels on linear transmitted or reflected readout light, before negative conversion, gamut processing, sharpening and encoding. Identity points retain this optical order." => {
            Some("controls.scanner.lens_blur_help")
        }
        "Gaussian sigma of the dye support in film micrometers; it conditions the field, which is read in film coordinates." => {
            Some("controls.grain.dye_support_help")
        }
        "General" => Some("surfaces.settings.general"),
        "Geometry: crop toggle, center, size, upscale factor" => {
            Some("controls.label.geometry_crop_toggle_center_size_upscale_factor")
        }
        "Glare" => Some("controls.label.glare"),
        "Global coupled gamma multiplier for the film density curves." => {
            Some("controls.chemistry.film_gamma_factor_help")
        }
        "Global coupled gamma multiplier for the print density curves." => {
            Some("controls.chemistry.print_gamma_factor_help")
        }
        "Global multiplier on the DIR coupler inhibition matrix. 1.0 leaves the per-channel gammas as-is." => {
            Some("controls.couplers.inhibition_amount_help")
        }
        "Grain" => Some("controls.grain.section"),
        "Grain particle size." => Some("controls.grain.size_help"),
        "Green–magenta tint" => Some("controls.scanner.tint"),
        "Halation" => Some("controls.halation.section"),
        "Halation Profiles" => Some("controls.halation.profiles"),
        "Halation is inactive; stored profile values retained and available when enabled." => {
            Some("controls.halation.inactive_profile")
        }
        "High-level halation strength multiplier (1.0 = physical defaults). Scales the per-channel halation amplitudes." => {
            Some("controls.halation.halation_amount_help")
        }
        "High-level scatter size multiplier (1.0 = physical defaults). Scales both core and tail sigmas." => {
            Some("controls.halation.scatter_spatial_scale_help")
        }
        "High-level scatter strength. 1.0 = full physical scatter, 0.0 = no scatter. Scales the fraction of light that undergoes in-emulsion scattering." => {
            Some("controls.halation.scatter_amount_help")
        }
        "Highlight grain amount." => Some("controls.grain.highlights_help"),
        "Highlight scatter and back-reflection; source response and RGB controls" => {
            Some("surfaces.summary.halation")
        }
        "Highlights" => Some("controls.grain.highlights"),
        "Hold automatic comparison" => Some("controls.scanner.hold_comparison"),
        "Hold the primary mouse button or focused Space to view this same frame with default operator values. Release to restore the corrected view. Saved and exported corrections are unchanged." => {
            Some("controls.scanner.hold_comparison_help")
        }
        "How the print exposure follows camera exposure compensation. follow_mode tracks it in auto mode and does not track it in manual; never disables tracking in auto; always applies the relative compensation factor in manual." => {
            Some("controls.scanner.print_tracking_help")
        }
        "If enabled, divide by (1 + sum of bounce amplitudes) so mid-grey is preserved. If disabled, halation is purely additive and subtly lifts shadows as well as highlights." => {
            Some("controls.halation.renormalize_help")
        }
        "Import" => Some("surfaces.presets.import"),
        "Import Raw" => Some("controls.label.import_raw"),
        "Import failed. You can import a renamed copy. %{detail}" => {
            Some("surfaces.presets.import_rename")
        }
        "Imported preset to %{path}" => Some("surfaces.status.preset_imported"),
        "Inactive" => Some("controls.scanner.inactive"),
        "Inactive controls · %{reason} · stored values retained" => {
            Some("surfaces.section.inactive")
        }
        "Inactive until this engine is selected; stored values are retained." => {
            Some("controls.grain.inactive_engine")
        }
        "Inactive: requires a negative film, an eligible negative-to-positive route and standard conversion style. Stored corrections are retained." => {
            Some("controls.scanner.operator_inactive")
        }
        "Independent preflash Magenta shift around the calibrated neutral filter pack, in Kodak CC units." => {
            Some("controls.scanner.preflash_m_shift_help")
        }
        "Independent preflash Yellow shift around the calibrated neutral filter pack, in Kodak CC units." => {
            Some("controls.scanner.preflash_y_shift_help")
        }
        "Input" => Some("controls.label.input"),
        "Original" => Some("surfaces.viewer.input"),
        "Input and capture" => Some("controls.label.input_and_capture"),
        "Input decoding: color space and cctf" => {
            Some("controls.label.input_decoding_color_space_and_cctf")
        }
        "Input gamut compress" => Some("controls.gamut.input_section"),
        "Input gamut compression: algorithm, knee, hull detail" => {
            Some("surfaces.summary.input_gamut")
        }
        "Interpolation used to display the output layer in the viewer. It resamples viewer pixels only; retained, saved and exported pixels are unchanged. The input layer always uses nearest and the paper back always uses spline36." => {
            Some("surfaces.viewer.interpolation_help")
        }
        "Invalid document" => Some("surfaces.presets.invalid_document"),
        "Invalid preset name '%{name}': use 1–64 portable filename characters, without a carrier extension or reserved device name." => {
            Some("surfaces.presets.name_guidance")
        }
        "Invalid preset name. %{detail}" => Some("surfaces.presets.invalid_name"),
        "Issues" => Some("surfaces.presets.issues"),
        "JPEG and PNG use encoded color; encoding is fixed on." => {
            Some("surfaces.export.encoded_fixed")
        }
        "JPEG format" => Some("controls.condition.jpeg"),
        "JPEG quality" => Some("surfaces.export.jpeg_quality"),
        "JPEG quality and subsampling apply only to JPEG; stored values are retained." => {
            Some("surfaces.export.jpeg_inactive")
        }
        "JPEG quality applies only to JPEG. The stored quality is retained for the next JPEG export." => {
            Some("surfaces.export.jpeg_quality_help")
        }
        "JPEG subsampling" => Some("controls.label.jpeg_subsampling"),
        "JPEG uses lossy compression, including at quality 100. Use PNG, TIFF or EXR to preserve image detail." => {
            Some("surfaces.export.jpeg_warning")
        }
        "JPEG: lossy 8-bit for web. PNG: lossless 8-bit. TIFF: 8/16-bit integer or 32-bit float, ZIP or uncompressed. EXR: 16/32-bit linear for compositing." => {
            Some("surfaces.export.format_help")
        }
        "Key controls: halation amount (strength) · halation spatial scale (size) · scatter amount (highlight glow)" => {
            Some("controls.halation.key_controls")
        }
        "Key controls: inhibition samelayer (contrast) · inhibition interlayer (saturation) · inhibition amount (global)" => {
            Some("controls.couplers.key_controls")
        }
        "Largest nonnested phase: %{phase} · %{ms} ms" => Some("surfaces.diagnostics.phase"),
        "Layer amounts" => Some("controls.grain.layer_amounts"),
        "Layer size ratio" => Some("controls.grain.layer_size_ratio"),
        "Layer weights" => Some("controls.grain.layer_weights"),
        "Leave at daylight (D65): it is the colorimetric reference the rest of the pipeline assumes, and should not be changed. Do not use it to neutralize a colour cast — fix white balance downstream with the enlarger filters instead. (custom exposes temperature/tint for special cases.)" => {
            Some("controls.scanner.white_balance_help")
        }
        "Lens blur (readout optics)" => Some("controls.scanner.lens_blur"),
        "Lens correction applied (%{info})" => Some("surfaces.status.lens_applied"),
        "Lens correction not applied" => Some("surfaces.status.lens_not_applied"),
        "Light used to acquire the imported negative RGB. This only changes capture inversion, not the fixed product readout light." => {
            Some("controls.convert.scan_illuminant_help")
        }
        "Linear flash exposure and independent Yellow and Magenta shifts" => {
            Some("surfaces.summary.preflash")
        }
        "Linear multiplier of the integrated spectral flash, not EV. Zero disables flash and retains its filters. For negative-to-photographic-paper printing, flash can bring weak highlight exposure differences into a responsive part of the paper curve; it cannot recover absent detail. Excess may lower paper white, add fog, alter midtones and change color. Other print materials retain their own polarity." => {
            Some("controls.scanner.preflash_exposure_help")
        }
        "Load an image before rotating." => Some("surfaces.status.rotate_requires_image"),
        "Load error: %{error}" => Some("surfaces.status.load_error"),
        "Load from file" => Some("surfaces.state.load_file"),
        "Loaded %{width} × %{height} (%{mp} MP) in %{ms} ms" => Some("surfaces.status.loaded"),
        "Loaded %{width} × %{height} (%{mp} MP) in %{ms} ms; %{lens}" => {
            Some("surfaces.status.loaded_raw")
        }
        "Loaded GUI state from %{path}" => Some("surfaces.status.state_loaded"),
        "Loaded GUI state from %{path}; %{detail}" => Some("surfaces.status.state_loaded_detail"),
        "Located control" => Some("controls.label.located_control"),
        "Logical bytes are not physical bus traffic. Host waits are not shader time." => {
            Some("surfaces.diagnostics.transfers_help")
        }
        "Logical transfers and work counters" => Some("surfaces.diagnostics.transfers"),
        "Look, film or print stock" => Some("surfaces.presets.search_hint"),
        "M filter shift of the color enlarger from a neutral position, in Kodak CC units" => {
            Some("controls.scanner.print_m_shift_help")
        }
        "Magenta-channel film base density shift around 555 nm (1.0 = neutral; adds (value - 1) density)." => {
            Some("controls.base.magenta_film_help")
        }
        "Magenta-channel multiplicative scale of the print base min around 530 nm (1.0 = neutral)." => {
            Some("controls.base.magenta_print_help")
        }
        "Material" => Some("controls.grain.material"),
        "Maximum highlight boost in stops." => Some("controls.halation.boost_ev_help"),
        "Mean red/blue minus green exposure difference in EV. Positive values add magenta; negative values add green." => {
            Some("controls.scanner.tint_help")
        }
        "Metering method used by the virtual camera's auto-exposure." => {
            Some("controls.scanner.metering_method_help")
        }
        "Method to upsample the spectral resolution of the image, hanatos2025 works on the full visible locus, mallett2019 works only on sRGB (will clip input)." => {
            Some("controls.upsampling.method_help")
        }
        "Micro-structure scale and clump size." => Some("controls.grain.micro_structure_help"),
        "Midtone grain amount." => Some("controls.grain.midtones_help"),
        "Midtones" => Some("controls.grain.midtones"),
        "Minimum grain density shared by Spektra and Dye." => {
            Some("controls.grain.density_min_help")
        }
        "Multiplier on the cross-layer (off-diagonal) inhibition. Controls saturation enhancement from interlayer DIR effects." => {
            Some("controls.couplers.inhibition_interlayer_help")
        }
        "Multiplier on the image-plane PSF widths (all per-group lambdas). Adjust for image-format / print-size differences." => {
            Some("controls.help.diffusion.scale")
        }
        "Multiplier on the same-layer (diagonal) inhibition. Controls overall contrast / gamma reduction within each RGB layer." => {
            Some("controls.couplers.inhibition_samelayer_help")
        }
        "My Presets" => Some("surfaces.presets.mine"),
        "My Presets already contains '%{name}'. Choose another name." => {
            Some("surfaces.presets.duplicate_name")
        }
        "NEGATIVE film: per-channel Langmuir saturation of DIR inhibitor release, normalized to each layer's d_max (R, G, B). Lower = earlier, stronger roll-off at high density (gentler shoulder, tames high-amount breakage); large values approach the linear model." => {
            Some("controls.couplers.langmuir_donor_help")
        }
        "Name in My Presets" => Some("surfaces.presets.name_label"),
        "Named positive-conversion tone of the negative-to-positive interpretation. `standard` measures the whole frame for automatic exposure and color balance before the frozen tone; any other name is a declared style you place in data/scan-styles/, never a fitted surface. The selector lists the styles found there. A style applies only with scan output set to Negative to positive." => {
            Some("controls.scanner.conversion_style_help")
        }
        "Negative" => Some("controls.scanner.negative"),
        "Negative to positive" => Some("controls.scanner.negative_to_positive"),
        "No adjustment to redo" => Some("surfaces.history.no_redo"),
        "No adjustment to undo" => Some("surfaces.history.no_undo"),
        "No completed reports. Collection applies to the next operation." => {
            Some("surfaces.diagnostics.empty")
        }
        "No control matches “%{query}”." => Some("surfaces.search.no_match"),
        "No controls differ from the factory baseline." => Some("surfaces.search.none_modified"),
        "No image loaded" => Some("surfaces.header.no_source"),
        "No image loaded\nDrop an image here, or choose Open in the header" => {
            Some("surfaces.viewer.empty")
        }
        "No matching look presets. Clear preset search to show all looks." => {
            Some("surfaces.presets.no_matches")
        }
        "No matching looks in this group." => Some("surfaces.presets.group_no_matches"),
        "No modified controls match “%{query}”." => Some("surfaces.search.no_modified_match"),
        "No observations in this scope." => Some("surfaces.diagnostics.no_observations"),
        "No output" => Some("surfaces.histogram.empty"),
        "No saved looks. Use Save preset to save the current look." => {
            Some("surfaces.presets.empty")
        }
        "Noise" => Some("controls.grain.noise"),
        "None" => Some("surfaces.export.none"),
        "Normalized size of the crop region in x, y (0,1), as fraction of the long side." => {
            Some("controls.crop.size_help")
        }
        "Number of emulsion sub-layers the composite density is split into when sublayers active is off; each sub-layer draws its own grain realization. Spektra-only." => {
            Some("controls.grain.sublayer_count_help")
        }
        "Number of multi-bounce Gaussians summed in the halation pass. Subsequent bounces use sqrt(k)-spaced widths. Typical: 2-3." => {
            Some("controls.halation.bounce_count_help")
        }
        "Numeric" => Some("controls.metadata.numeric"),
        "Off" => Some("surfaces.diagnostics.off"),
        "Only modified" => Some("surfaces.settings.modified"),
        "Open" => Some("surfaces.action.open"),
        "Open image — RGB and supported RAW formats use their existing import paths." => {
            Some("surfaces.action.open_help")
        }
        "Operator correction" => Some("controls.scanner.operator_correction"),
        "OptionalNumeric" => Some("controls.metadata.optionalnumeric"),
        "Output" => Some("controls.label.output"),
        "Modified" => Some("surfaces.viewer.output"),
        "Output gamut compress" => Some("controls.gamut.output_section"),
        "Output gamut compression: algorithm, knee" => Some("surfaces.summary.output_gamut"),
        "Overall grain amount." => Some("controls.grain.amount_help"),
        "Overall multiplier on the film base density." => Some("controls.base.film_scale_help"),
        "Overall multiplier on the print base density." => Some("controls.base.print_scale_help"),
        "POSITIVE/reversal film: per-channel receiver-side Langmuir saturation (R, G, B), normalized to the arrived-inhibitor ceiling (the receiver analogue of langmuir_donor_k_rgb's d_max normalization). The donor (silver release) stays linear; this rolls off the receiving layer's response under a pushed amount. Lower = harder, more crash-proof knee (alters the operating-point look); large values approach linear." => {
            Some("controls.couplers.langmuir_receiver_help")
        }
        "PSF family. pro_mist / glimmerglass / cinebloom are transparent (energy-preserving); black_pro_mist absorbs a fraction of the deflected light, lifting shadows by reducing local contrast." => {
            Some("controls.help.diffusion.family")
        }
        "Paper" => Some("controls.scanner.paper"),
        "Paper back" => Some("surfaces.viewer.paper_back"),
        "Pass intervals are not operation GPU duration. Unavailable values are not zero." => {
            Some("surfaces.diagnostics.gpu_help")
        }
        "Paste operator correction" => Some("controls.scanner.paste_correction"),
        "Per-bounce amplitude decay ratio (rho). Physical range 0.3-0.7. Controls how fast the halation energy falls off between bounces." => {
            Some("controls.halation.bounce_decay_help")
        }
        "Per-channel gamma factor applied to the blue channel." => {
            Some("controls.chemistry.gamma_factor_blue_help")
        }
        "Per-channel gamma factor applied to the green channel." => {
            Some("controls.chemistry.gamma_factor_green_help")
        }
        "Per-channel gamma factor applied to the red channel." => {
            Some("controls.chemistry.gamma_factor_red_help")
        }
        "Per-channel grain uniformity shared by Spektra and Dye." => {
            Some("controls.grain.uniformity_help")
        }
        "Per-channel multiplier on the particle area; the default makes the blue record coarser. Spektra reads this value; Dye uses the granularity preset instead." => {
            Some("controls.grain.particle_scale_help")
        }
        "Per-channel same-layer DIR gamma (R, G, B). Effective gamma reduction of each layer's density curve." => {
            Some("controls.couplers.gamma_samelayer_help")
        }
        "Per-channel relative Dye strength: event-area scale k = (G / 6)^2, not a certified optical RMS target. Zero retains the reference-scale fallback; upstream Spektra retains this value as a no-op. Distinct from Procedural Amount." => {
            Some("controls.grain.rms_granularity_help")
        }
        "Percentage of the glare light (typically 0.1-0.25)" => Some("controls.help.glare.percent"),
        "Permutation of film density channels. Selecting a channel swaps its prior position so no record is duplicated or dropped." => {
            Some("controls.help.channels.permutation")
        }
        "Phase durations (inclusive; nested costs are not additive)" => {
            Some("surfaces.diagnostics.phases")
        }
        "Physical back-reflection size multiplier; scales every channel's first-bounce spread without coupling its strength." => {
            Some("controls.halation.halation_spatial_scale_help")
        }
        "Pixel payload at the input's full resolution. This container compresses the samples, so the file is smaller; the exact size depends on image content." => {
            Some("surfaces.export.size_compressed_help")
        }
        "Pixel payload at the input's full resolution. This container stores the samples uncompressed, so the file matches apart from headers." => {
            Some("surfaces.export.size_help")
        }
        "Pixels from render #%{id} · %{width} × %{height} · diagnostics %{state}" => {
            Some("surfaces.diagnostics.source")
        }
        "Positive" => Some("controls.grain.positive"),
        "Positive scale per component: Spektra uses relative particle scales per emulsion sublayer; Dye uses numerical curve components, changing event mass and rate, not separate cloud-kernel widths." => {
            Some("controls.grain.particle_scale_sublayers_help")
        }
        "Post-blur sigma." => Some("controls.grain.blur_help"),
        "Post-scan Procedural appearance resolution method: 0 = Optical Resolution; 1 = Fast Blur. This does not change scanner readout optics." => {
            Some("controls.grain.resolution_type_help")
        }
        "Preflash" => Some("controls.label.preflash"),
        "Preflash inactive on this route; stored values are retained." => {
            Some("controls.scanner.preflash_inactive")
        }
        "Preserve medium polarity" => Some("controls.scanner.preserve_polarity"),
        "Preserve medium polarity keeps the scanned medium's native scan polarity: a negative film scan reads the negative image on the film, and a photographic-paper or slide scan already reads a positive image. Negative to positive interprets a developed negative as a positive image by inverting the film's own density records and applying the fixed positive tone response." => {
            Some("controls.scanner.scan_output_help")
        }
        "Preset error: %{error}" => Some("surfaces.status.preset_error"),
        "Preset library could not be read. %{detail}" => Some("surfaces.presets.scan_error"),
        "Preset name" => Some("surfaces.presets.name_title"),
        "Preset operation failed. %{detail}" => Some("surfaces.presets.error"),
        "Preset unavailable at %{path}. %{detail}" => Some("surfaces.presets.fault"),
        "Presets" => Some("surfaces.presets.title"),
        "Print" => Some("controls.label.print"),
        "Print base" => Some("controls.label.print_base"),
        "Print chemistry" => Some("controls.chemistry.print_section"),
        "Print illuminant to simulate. Default is the bundled TH-KG3." => {
            Some("controls.scanner.print_illuminant_help")
        }
        "Print material" => Some("controls.label.print_material"),
        "Print stock to simulate" => Some("surfaces.profiles.print_help"),
        "Print viewing glare" => Some("controls.label.print_viewing_glare"),
        "Print viewing glare: percent, roughness, blur" => {
            Some("controls.label.print_viewing_glare_percent_roughness_blur")
        }
        "Print: %{name}" => Some("surfaces.header.print"),
        "Procedural" => Some("controls.grain.procedural"),
        "Procedural Grain Profiles" => Some("controls.grain.procedural_profiles"),
        "Procedural grain processing mode." => Some("controls.grain.processing_mode_help"),
        "Processing Mode" => Some("controls.grain.processing_mode"),
        "Profile" => Some("controls.metadata.profile"),
        "Profile selection error: %{error}" => Some("surfaces.status.profile_error"),
        "Profile selection error: '%{name}' is not a film profile" => {
            Some("surfaces.status.profile_not_film")
        }
        "Profile selection error: '%{name}' is not a print profile" => {
            Some("surfaces.status.profile_not_print")
        }
        "Protected range above midgray for the boost onset in stops." => {
            Some("controls.halation.protect_ev_help")
        }
        "RAW decode settings match the accepted source. Camera exposure is a separate simulation control." => {
            Some("surfaces.raw.current")
        }
        "RAW decode settings pending; the accepted source still uses the previous settings." => {
            Some("surfaces.raw.pending")
        }
        "RAW decoding is inactive for this source. Settings are retained for the next RAW import." => {
            Some("surfaces.raw.inactive")
        }
        "RAW source" => Some("controls.condition.raw"),
        "RAW source; custom white balance; explicit Reprocess RAW" => {
            Some("controls.condition.raw_custom")
        }
        "RAW source; explicit Reprocess RAW" => Some("controls.condition.raw_reprocess"),
        "RMS density" => Some("controls.grain.rms_density"),
        "Radius span" => Some("controls.grain.radius_span"),
        "Read-only zoom status: the fitted zoom, or the exact source-pixel-to-device-pixel percentage. Viewer presentation only." => {
            Some("surfaces.viewer.zoom_help")
        }
        "Readout stray light: percent, roughness, blur" => Some("surfaces.summary.glare"),
        "Real negative capture: exposure, base estimation, calibration, channel swaps" => {
            Some("surfaces.summary.convert")
        }
        "Recommended: Spektra (material grain) · Procedural (post-scan, lightweight)" => {
            Some("controls.grain.recommended_engines")
        }
        "Red-minus-blue exposure difference in EV. Positive values are warmer; negative values are cooler. This is not Kelvin and does not change capture white balance." => {
            Some("controls.scanner.warmth_help")
        }
        "Redo" => Some("surfaces.action.redo"),
        "Redo %{label}" => Some("surfaces.history.redo"),
        "Refresh" => Some("surfaces.presets.refresh"),
        "Refreshed preset library" => Some("surfaces.status.presets_refreshed"),
        "Reinhard knee (threshold, limit, power) on normalized chroma C/C_max. Default (0.0, 1.0, 6.0) rolls off smoothly from the center with the asymptote on the output cube edge." => {
            Some("controls.gamut.output_knee_help")
        }
        "Reinhard knee (threshold, limit, power). Default (0.815, 1.0, 1.2) — ACES RGC pair: the 0.815 threshold leaves in-gamut colours untouched, the 1.2 power gives a slow tail so deep-imaginary colours keep their gradation, and limit 1.0 asymptotes on the boundary." => {
            Some("controls.gamut.input_knee_help")
        }
        "Rename…" => Some("surfaces.presets.rename"),
        "Render the original working resolution with the current runtime controls and retain it as the viewer's output. Spatial and stochastic effects follow their current accepted controls, including any explicit deactivation settings." => {
            Some("surfaces.action.scan_help")
        }
        "Replace startup controls with the current state. Adjustment Undo does not reverse this file change." => {
            Some("surfaces.state.save_default_help")
        }
        "Replace the current controls and session settings with factory state and remove the saved startup default; confirmation required." => {
            Some("surfaces.state.restore_factory_help")
        }
        "Replaced startup controls with current GUI state; adjustment Undo does not reverse this file change" => {
            Some("surfaces.status.startup_saved")
        }
        "Replaces the current controls and session settings with the factory state, and removes the saved startup default. Adjustment Undo does not reverse the startup-file change." => {
            Some("surfaces.state.restore_warning")
        }
        "Reprocess RAW" => Some("surfaces.raw.reprocess"),
        "Requested: %{requested} · Selected: %{selected} · Route: %{route}" => {
            Some("surfaces.diagnostics.backend")
        }
        "Reset the four operator values only; keep this frame's own automatic setup." => {
            Some("controls.scanner.restore_automatic_help")
        }
        "Resident route declined: %{reason}" => Some("surfaces.diagnostics.declined"),
        "Resolution Type" => Some("controls.grain.resolution_type"),
        "Resolution component of the post-scan Procedural appearance algorithm; this does not set scanner lens resolution." => {
            Some("controls.grain.film_resolution_help")
        }
        "Restore" => Some("surfaces.action.restore"),
        "Restore %{section} to the factory state" => Some("surfaces.section.restore"),
        "Restore automatic" => Some("controls.scanner.restore_automatic"),
        "Restore factory default" => Some("surfaces.state.restore_factory"),
        "Restore factory default?" => Some("surfaces.state.restore_title"),
        "Restored factory default GUI state" => Some("surfaces.status.factory_restored"),
        "Retained frame is not current · controls changed" => Some("surfaces.state.stale_frame"),
        "Reveal and scroll to this stage in the continuous inspector." => {
            Some("surfaces.search.stage_help")
        }
        "Revert" => Some("surfaces.presets.revert"),
        "Rotate the retained source 90° clockwise; this is not a viewer transform." => {
            Some("surfaces.source.rotate_cw_help")
        }
        "Rotate the retained source 90° counter-clockwise. Preview, Scan, and Export render this source." => {
            Some("surfaces.source.rotate_ccw_help")
        }
        "Rotated input %{degrees}°" => Some("surfaces.status.rotated"),
        "Roughness of the glare light (0-1)" => Some("controls.help.glare.roughness"),
        "Route: %{route}" => Some("surfaces.header.route"),
        "Modified becomes available when automatic Preview or Scan completes." => {
            Some("surfaces.viewer.output_disabled")
        }
        "SAVE" => Some("surfaces.action.save_retained"),
        "SCAN" => Some("surfaces.action.scan"),
        "Sample the brightest clear film and fit film Base plus exposure so unexposed film maps to density zero." => {
            Some("controls.convert.detect_base_help")
        }
        "Save" => Some("surfaces.action.save"),
        "Save current as default" => Some("surfaces.state.save_default"),
        "Save current look as a preset" => Some("surfaces.presets.save_help"),
        "Save current to file" => Some("surfaces.state.save_file"),
        "Save preset…" => Some("surfaces.presets.save"),
        "Save report…" => Some("surfaces.diagnostics.save"),
        "Saved GUI state to %{path}" => Some("surfaces.status.state_saved"),
        "Saved diagnostic report." => Some("surfaces.status.report_saved"),
        "Saved preset '%{name}' to %{path}" => Some("surfaces.status.preset_saved"),
        "Saving color space" => Some("surfaces.export.color_space"),
        "Scale image size up to increase resolution" => Some("controls.crop.upscale_help"),
        "Scan" => Some("controls.label.scan"),
        "Scanner" => Some("controls.label.scanner"),
        "Scanner inactive on this route; stored values are retained." => {
            Some("controls.scanner.scanner_inactive")
        }
        "Scanner sequence: scan output polarity, then readout optics and digital sharpening." => {
            Some("controls.scanner.sequence")
        }
        "Search controls" => Some("surfaces.search.label"),
        "Search presets" => Some("surfaces.presets.search"),
        "Select the JPEG chroma subsampling." => Some("surfaces.export.subsampling_help"),
        "Select the TIFF compression method." => Some("surfaces.export.compression_help"),
        "Select the bit depth." => Some("surfaces.export.depth_help"),
        "Select the grain engine: Spektra for material grain in developed film density, Procedural for post-scan grain appearance in encoded RGB, Dye for material grain formed in a film-coordinate dye field, Fotufilm Film for effective-appearance film population grain model." => {
            Some("controls.grain.engine_help")
        }
        "Session" => Some("controls.label.general"),
        "Session only. Changes apply to the next accepted operation; they do not render." => {
            Some("surfaces.diagnostics.session")
        }
        "Sets the physical film scale for grain and other film effects. Does not crop, resize, or change the image's aspect ratio. 35mm uses a 36 mm frame long edge; 120 uses a representative 56 mm; 4×5 uses 127 mm." => {
            Some("controls.scanner.film_format_help")
        }
        "Settings" => Some("surfaces.settings.title"),
        "Shadow grain amount." => Some("controls.grain.shadows_help"),
        "Shadows" => Some("controls.grain.shadows"),
        "Shared by Spektra and Dye; inactive until either engine is selected, stored values are retained." => {
            Some("controls.grain.inactive_shared_material")
        }
        "Shared Poisson-event rate fraction among active records within each numerical component, not realized CMY or final RGB correlation. Preserves each record's marginal count distribution; correlation depends on record-rate ratios." => {
            Some("controls.grain.channel_correlation_help")
        }
        "Shared material controls" => Some("controls.grain.shared_material"),
        "Show controls that differ from their factory baseline, independently of an applied preset." => {
            Some("surfaces.search.modified_help")
        }
        "Show directory" => Some("surfaces.presets.show_directory"),
        "Show file" => Some("surfaces.presets.show_file"),
        "Show the retained image at %{percent}% — one source pixel per device pixel at 100%. Viewer only; retained and written pixels are unchanged." => {
            Some("surfaces.viewer.zoom_exact_help")
        }
        "Show the retained input, decoded from its input color space and transfer decoding. The viewer layer does not change the retained image itself." => {
            Some("surfaces.viewer.input_help")
        }
        "Show the retained output of the latest Preview or Scan. The viewer layer does not change the retained image itself." => {
            Some("surfaces.viewer.output_help")
        }
        "Sigma in nm for Gaussian blur applied to reconstructed spectra." => {
            Some("controls.upsampling.gaussian_blur_help")
        }
        "Sigma in um for the diffusion of the couplers, (5-20 um), controls sharpness and affects saturation." => {
            Some("controls.couplers.diffusion_size_help")
        }
        "Sigma of gaussian blur in pixels for the glare" => Some("controls.help.glare.blur"),
        "Sigma of the first halation bounce per channel [R,G,B], in micrometers. Set by the base thickness (40-80 um for typical cine/still bases)." => {
            Some("controls.halation.first_sigma_help")
        }
        "Sigma of the scatter core Gaussian per channel [R,G,B], in micrometers. Controls fine-scale sharpness loss in the emulsion." => {
            Some("controls.halation.scatter_core_help")
        }
        "Silver" => Some("controls.grain.silver"),
        "Simulation blend" => Some("controls.output.simulation_blend"),
        "Size" => Some("controls.grain.size"),
        "Slide" => Some("controls.scanner.slide"),
        "Softness" => Some("controls.grain.softness"),
        "Software compute adapter — not a hardware GPU." => Some("surfaces.diagnostics.software"),
        "Solve main enlarger M/Y shifts so the current film base midgray prints neutral. Writes the Enlarger controls." => {
            Some("controls.convert.neutralize_filters_help")
        }
        "Source rotation" => Some("controls.label.source_rotation"),
        "Source rotation: %{degrees}°" => Some("surfaces.source.rotation"),
        "Source: %{source}" => Some("surfaces.header.source"),
        "Spectral reconstruction: method, adaptation, blur" => {
            Some("controls.label.spectral_reconstruction_method_adaptation_blur")
        }
        "Spectral tilt of the film base, pivoting at 555 nm (1.0 = +0.1 density at 650 nm; negative tilts the other way)." => {
            Some("controls.base.tilt_help")
        }
        "Spectral upsampling" => Some("controls.upsampling.section"),
        "Spektra" => Some("controls.grain.spektra"),
        "Split the developed density into the film profile's emulsion sublayers instead of applying grain to the composite density. Spektra-only; Dye always uses numerical curve components, and Procedural and Fotufilm ignore it." => {
            Some("controls.grain.sublayers_active_help")
        }
        "Stages and CPU reasons" => Some("surfaces.diagnostics.stages"),
        "Start an independent export: render the full-resolution input with the current runtime controls and these saving settings, then write the file. Export never reads or writes the retained output, so it is a different operation from Save." => {
            Some("surfaces.export.submit_help")
        }
        "Startup save error: %{error}" => Some("surfaces.status.startup_error"),
        "State load error: %{error}" => Some("surfaces.status.state_load_error"),
        "State save error: %{error}" => Some("surfaces.status.state_save_error"),
        "Stop using the selected ICC profile and fall back to the platform profile or sRGB. Retained, saved and exported pixels are unchanged." => {
            Some("surfaces.display.clear_help")
        }
        "Sublayer 1" => Some("controls.label.sublayer_1"),
        "Sublayer 2" => Some("controls.label.sublayer_2"),
        "Sublayer 3" => Some("controls.label.sublayer_3"),
        "Sublayer 4 (remainder)" => Some("controls.label.sublayer_4_remainder"),
        "Sublayer 4 is read-only: one minus the first three shares." => {
            Some("controls.grain.fourth_sublayer_remainder")
        }
        "Sublayer shares" => Some("controls.label.sublayer_shares"),
        "Summary" => Some("surfaces.diagnostics.summary"),
        "Summary: %{summary}" => Some("surfaces.export.summary"),
        "TIFF editable; JPEG/PNG encoded, EXR linear" => Some("controls.condition.transfer"),
        "TIFF format; EXR fixed ZIP" => Some("controls.condition.compression"),
        "Taking-lens diffusion: core, tail, and warmth" => {
            Some("surfaces.summary.camera_diffusion")
        }
        "Temperature in Kelvin for the custom whitebalance, not used for the other white balance settings" => {
            Some("controls.scanner.temperature_help")
        }
        "Text" => Some("controls.metadata.text"),
        "Timer" => Some("controls.grain.timer"),
        "Tint value for the custom white balance, not used for the other white balance settings" => {
            Some("controls.scanner.raw_tint_help")
        }
        "Toggle" => Some("controls.metadata.toggle"),
        "Toggle the diffusion filter (Pro-Mist family)." => Some("controls.help.diffusion.enable"),
        "Tonal distribution" => Some("surfaces.histogram.title"),
        "Total back-reflection halation amplitude per channel [R,G,B] (0-1). Typical red channel: weak AH 0.02-0.08, no AH 0.08-0.25. The blue channel is usually near zero." => {
            Some("controls.halation.halation_strength_help")
        }
        "Tuple" => Some("controls.metadata.tuple"),
        "Unconnected" => Some("controls.metadata.unconnected"),
        "Undo" => Some("surfaces.action.undo"),
        "Undo %{label}" => Some("surfaces.history.undo"),
        "Unsharp mask (digital sharpening)" => Some("controls.scanner.unsharp_mask"),
        "Update" => Some("surfaces.presets.update"),
        "Update, rename, duplicate, export, reveal or delete %{name}" => {
            Some("surfaces.presets.actions_help")
        }
        "Updated preset '%{name}'" => Some("surfaces.status.preset_updated"),
        "Use the CAT16 chromatic adaptation transform for the input RGB to film-space adaptation that feeds the Hanatos spectral upsampling. Off selects CAT02, the pre-0.3.4 behavior." => {
            Some("controls.upsampling.use_cat16_help")
        }
        "Use the auto-exposure feature of the virtual camera." => {
            Some("controls.scanner.auto_exposure_help")
        }
        "Use the visible spectral locus or its smoothed inscribed hull as the input gamut boundary." => {
            Some("controls.gamut.boundary_help")
        }
        "Viewer" => Some("controls.label.viewer"),
        "Viewer input error: %{error}" => Some("surfaces.status.viewer_error"),
        "Viewer only: the display transform and its ICC profile never reach retained, saved or exported pixels." => {
            Some("surfaces.display.isolation")
        }
        "Viewer-only display conversion; does not change retained or written pixels." => {
            Some("surfaces.display.transform_help")
        }
        "Viewer-only transition between accepted outputs." => {
            Some("surfaces.viewer.crossfade_help")
        }
        "Viewer-only white border, as a fraction of the image long edge." => {
            Some("surfaces.display.padding_help")
        }
        "Viewer-only wipe between input and output; retained and written pixels are unchanged." => {
            Some("surfaces.viewer.reveal_help")
        }
        "Viewer-only: show the virtual paper-back rendering generated from paper texture, watermark, and noise. It is not an editable material stage and changes no retained, saved or exported pixels." => {
            Some("surfaces.viewer.paper_back_help")
        }
        "Warmth" => Some("controls.scanner.warmth"),
        "Weight of the scatter tail Gaussian per channel [R,G,B] (0-1). Tail weight + core weight = 1. Higher values put more scattered light into the long tail." => {
            Some("controls.halation.scatter_tail_weight_help")
        }
        "What these settings will write: format, bit depth, saving color space, and the transfer encoding (CCTF, the color component transfer function, or linear) plus compression. Save writes the retained output with the current simulation saving settings instead, without rerendering." => {
            Some("surfaces.export.summary_help")
        }
        "Which physical path the image takes through the pipeline: input color-manages RGB. input > film > scan exposes and scans film; negative film can retain its negative polarity or use Negative to positive, while slide film remains positive. input > film > print > scan exposes film, prints photographically, and scans positive paper. input > convert-film > print > scan converts the input to negative capture, prints photographically, and scans positive paper. input > convert-film > scan scans the negative capture with its film base; it can preserve negative polarity or convert Negative to positive. input > convert-film > scan-minus-base removes the film base and keeps negative polarity. CMYK ink style is independent in every workflow." => {
            Some("surfaces.route.help")
        }
        "Workflow" => Some("controls.label.workflow"),
        "Workflow route changed" => Some("surfaces.status.route_changed"),
        "Workflow: 1) Detect base → 2) Blind calibration (vibrant frame) → 3) Neutralize filters" => {
            Some("controls.convert.workflow")
        }
        "Write the retained output to an image file without rerendering: the pixels are the latest Preview or Scan result, converted to the saving color space and transfer encoding. Export renders and writes independently." => {
            Some("surfaces.action.save_help")
        }
        "Y filter shift of the color enlarger from a neutral position, in Kodak CC units" => {
            Some("controls.scanner.print_y_shift_help")
        }
        "Yellow-channel film base density shift around 460 nm (1.0 = neutral; adds (value - 1) density)." => {
            Some("controls.base.yellow_film_help")
        }
        "Yellow-channel multiplicative scale of the print base min around 445 nm (1.0 = neutral)." => {
            Some("controls.base.yellow_print_help")
        }
        "actual Film development route; captured negatives and the input route are not redeveloped" => {
            Some("controls.condition.film_development")
        }
        "admitted print-development route, including recovered-negative printing" => {
            Some("controls.condition.print_development")
        }
        "algorithm" => Some("controls.gamut.algorithm"),
        "always" => Some("controls.scanner.always"),
        "amount" => Some("controls.label.amount"),
        "applied" => Some("surfaces.presets.applied"),
        "apply cctf decoding" => Some("controls.scanner.input_cctf_decoding"),
        "as_shot" => Some("controls.scanner.as_shot"),
        "auto" => Some("controls.scanner.auto"),
        "auto exposure" => Some("controls.scanner.auto_exposure"),
        "auto exposure method" => Some("controls.scanner.metering_method"),
        "available only when the Procedural engine is selected" => {
            Some("controls.grain.procedural_profile_unavailable")
        }
        "average" => Some("controls.scanner.metering_average"),
        "base percentile" => Some("controls.convert.base_percentile"),
        "blind calibration" => Some("controls.convert.blind_calibration"),
        "bloom intensity" => Some("controls.label.bloom_intensity"),
        "bloom size" => Some("controls.label.bloom_size"),
        "blur" => Some("controls.grain.blur"),
        "boost ev" => Some("controls.halation.boost_ev"),
        "boost range" => Some("controls.halation.boost_range"),
        "boundary" => Some("controls.gamut.boundary"),
        "calibration" => Some("controls.convert.calibration"),
        "camera.auto_exposure = true" => Some("controls.condition.auto_exposure"),
        "camera.diffusion_filter.active = true; strength and spatial_scale positive" => {
            Some("controls.condition.camera_diffusion")
        }
        "capture illuminant" => Some("controls.scanner.capture_illuminant"),
        "captured-negative Convert route" => Some("controls.condition.captured_negative"),
        "ccw rotate" => Some("surfaces.source.rotate_ccw"),
        "center_weighted" => Some("controls.scanner.metering_center"),
        "channel correlation" => Some("controls.grain.channel_correlation"),
        "channel permutation" => Some("controls.label.channel_permutation"),
        "chroma" => Some("controls.grain.chroma"),
        "clear" => Some("surfaces.action.clear"),
        "clear filter" => Some("surfaces.search.clear"),
        "cloud radius (µm)" => Some("controls.grain.cloud_radius"),
        "clump" => Some("controls.label.clump"),
        "cmyk.active = true" => Some("controls.condition.cmyk_active"),
        "cmyk.strength > 0" => Some("controls.condition.cmyk_strength"),
        "coating density scale" => Some("controls.grain.coating_density_scale"),
        "collected" => Some("surfaces.diagnostics.collected"),
        "color" => Some("controls.grain.color"),
        "color component transfer function" => Some("controls.output.cctf_expansion"),
        "color filter" => Some("controls.scanner.color_filter"),
        "confirmation required" => Some("controls.condition.confirm"),
        "contrast" => Some("controls.scanner.contrast"),
        "conversion style" => Some("controls.scanner.conversion_style"),
        "copy operator correction" => Some("controls.scanner.copy_correction"),
        "core Halation feature-profile scope; inactive values retained" => {
            Some("controls.condition.halation_scope")
        }
        "core intensity" => Some("controls.label.core_intensity"),
        "core size" => Some("controls.label.core_size"),
        "crop" => Some("controls.crop.crop"),
        "crop center" => Some("controls.crop.center"),
        "crop size" => Some("controls.crop.size"),
        "crossfade transition" => Some("surfaces.viewer.crossfade"),
        "custom" => Some("surfaces.presets.custom"),
        "custom BB<kelvin>" => Some("controls.convert.custom_illuminant_hint"),
        "custom scan illuminant" => Some("controls.convert.custom_scan_illuminant"),
        "cw rotate" => Some("surfaces.source.rotate_cw"),
        "cyan" => Some("controls.base.cyan"),
        "daylight" => Some("controls.scanner.daylight"),
        "density curve gamma" => Some("controls.chemistry.density_curve_gamma"),
        "density min" => Some("controls.grain.density_min"),
        "detect base" => Some("controls.convert.detect_base"),
        "developer exhaustion" => Some("controls.chemistry.developer_exhaustion"),
        "development time" => Some("controls.label.development_time"),
        "diffusion size um" => Some("controls.couplers.diffusion_size"),
        "diffusion tail um" => Some("controls.couplers.diffusion_tail"),
        "diffusion tail weight" => Some("controls.couplers.diffusion_tail_weight"),
        "dimensionless" => Some("controls.unit.dimensionless"),
        "duration unavailable" => Some("surfaces.diagnostics.duration_missing"),
        "dye support (µm)" => Some("controls.grain.dye_support"),
        "dye-cloud blur" => Some("controls.grain.dye_cloud_blur"),
        "enable CMYK ink style" => Some("controls.cmyk.enable"),
        "enable dir couplers" => Some("controls.couplers.enable"),
        "enable film base" => Some("controls.base.enable_film"),
        "enable film density curves" => Some("controls.chemistry.enable_film_curves"),
        "enable grain" => Some("controls.grain.enable"),
        "enable halation" => Some("controls.halation.enable"),
        "enable input gamut compress" => Some("controls.gamut.input_enable"),
        "enable print base" => Some("controls.base.enable_print"),
        "enable print density curves" => Some("controls.chemistry.enable_print_curves"),
        "engine" => Some("controls.grain.engine"),
        "enlarger.diffusion_filter.active = true; strength and spatial_scale positive; io.scan_film = false" => {
            Some("controls.condition.enlarger_diffusion")
        }
        "enlarger.preflash_exposure > 0; zero disables flash and retains filter settings" => {
            Some("controls.condition.preflash")
        }
        "enum" => Some("controls.metadata.enum"),
        "event area (µm²)" => Some("controls.grain.event_area"),
        "exposure compensation ev" => Some("controls.convert.exposure_compensation"),
        "exposure ×" => Some("controls.unit.exposure"),
        "film profile" => Some("controls.label.film_profile"),
        "film resolution" => Some("controls.grain.film_resolution"),
        "film type" => Some("controls.grain.film_type"),
        "film-exposure route that converts scene RGB to film exposure" => {
            Some("controls.condition.capture_illuminant")
        }
        "film_channel_swap" => Some("controls.convert.film_channel_swap"),
        "film_render.chemistry.active = true; supported fitted density_curves_model required" => {
            Some("controls.condition.film_chemistry")
        }
        "film_render.dir_couplers.active = true" => Some("controls.condition.couplers"),
        "film_render.grain.active = true; stochastic effects enabled" => {
            Some("controls.condition.grain")
        }
        "film_render.grain.engine = dye; supported stock, format and route" => {
            Some("controls.condition.dye")
        }
        "film_render.grain.engine = fotufilm" => Some("controls.condition.fotufilm"),
        "film_render.grain.engine = procedural" => Some("controls.condition.procedural"),
        "film_render.grain.engine = spektra" => Some("controls.condition.spektra"),
        "film_render.grain.engine = spektra or dye" => Some("controls.condition.spektra_dye"),
        "film_render.grain.sublayers_active = false" => Some("controls.condition.no_sublayers"),
        "film_render.grain.sublayers_active = true; film profile supplies sublayer density curves" => {
            Some("controls.condition.sublayers")
        }
        "film_render.grain.v2_profile = custom; select custom to use this control" => {
            Some("controls.condition.custom")
        }
        "film_render.halation.active = true" => Some("controls.condition.halation"),
        "film_render.halation.boost_ev != 0; LUT mode disabled" => Some("controls.condition.boost"),
        "filter family" => Some("controls.label.filter_family"),
        "finished exposure" => Some("controls.scanner.finished_exposure"),
        "fit" => Some("surfaces.viewer.fit"),
        "fog density" => Some("controls.grain.fog_density"),
        "follow_mode" => Some("controls.scanner.follow_mode"),
        "format-dependent" => Some("controls.condition.format"),
        "format-dependent color spaces" => Some("controls.condition.color_space"),
        "gamma factor" => Some("controls.chemistry.gamma_factor"),
        "gamma factor blue" => Some("controls.chemistry.gamma_factor_blue"),
        "gamma factor fast" => Some("controls.chemistry.gamma_factor_fast"),
        "gamma factor green" => Some("controls.chemistry.gamma_factor_green"),
        "gamma factor red" => Some("controls.chemistry.gamma_factor_red"),
        "gamma factor slow" => Some("controls.chemistry.gamma_factor_slow"),
        "gamma interlayer b to rg" => Some("controls.couplers.gamma_blue_to_red_green"),
        "gamma interlayer g to rb" => Some("controls.couplers.gamma_green_to_red_blue"),
        "gamma interlayer r to gb" => Some("controls.couplers.gamma_red_to_green_blue"),
        "gamma samelayer rgb" => Some("controls.couplers.gamma_samelayer"),
        "gray 18% canvas" => Some("controls.label.gray_18_canvas"),
        "green–magenta tint" => Some("controls.scanner.tint"),
        "halation amount" => Some("controls.halation.halation_amount"),
        "halation bounce decay" => Some("controls.halation.bounce_decay"),
        "halation first sigma um" => Some("controls.halation.first_sigma"),
        "halation n bounces" => Some("controls.halation.bounce_count"),
        "halation renormalize" => Some("controls.halation.renormalize"),
        "halation spatial scale" => Some("controls.halation.halation_spatial_scale"),
        "halation strength" => Some("controls.halation.halation_strength"),
        "halo intensity" => Some("controls.label.halo_intensity"),
        "halo size" => Some("controls.label.halo_size"),
        "halo warmth" => Some("controls.label.halo_warmth"),
        "hanatos2025 adaptation surface" => Some("controls.upsampling.adaptation_surface"),
        "hanatos2025 adaptation window" => Some("controls.upsampling.adaptation_window"),
        "highlight_weighted" => Some("controls.scanner.metering_highlight"),
        "highlights" => Some("controls.grain.highlights"),
        "hold automatic comparison" => Some("controls.scanner.hold_comparison"),
        "hull detail" => Some("controls.gamut.hull_detail"),
        "inherit (film profile)" => Some("controls.scanner.capture_inherit"),
        "inhibition amount" => Some("controls.couplers.inhibition_amount"),
        "inhibition interlayer" => Some("controls.couplers.inhibition_interlayer"),
        "inhibition samelayer" => Some("controls.couplers.inhibition_samelayer"),
        "input color space" => Some("controls.scanner.input_color_space"),
        "inscribed_hull" => Some("controls.gamut.inscribed_hull"),
        "io.crop = true" => Some("controls.condition.crop"),
        "io.scan_film = false; print_render.glare.active = true; percent > 0; stochastic effects enabled" => {
            Some("controls.condition.glare")
        }
        "knee" => Some("controls.gamut.knee"),
        "langmuir donor k rgb" => Some("controls.couplers.langmuir_donor"),
        "langmuir receiver k rgb" => Some("controls.couplers.langmuir_receiver"),
        "layer amounts" => Some("controls.grain.layer_amounts"),
        "layer size ratio" => Some("controls.grain.layer_size_ratio"),
        "layer weights" => Some("controls.grain.layer_weights"),
        "lens blur (readout optics)" => Some("controls.scanner.lens_blur"),
        "lens correction" => Some("controls.scanner.lens_correction"),
        "limit" => Some("controls.label.limit"),
        "linear" => Some("surfaces.export.linear"),
        "linear exposure multiplier" => Some("controls.unit.linear_exposure_multiplier"),
        "loaded captured negative" => Some("controls.condition.loaded_negative"),
        "loaded source" => Some("controls.condition.loaded"),
        "loaded source; immutable submitted request" => Some("controls.condition.submitted"),
        "locus" => Some("controls.gamut.locus"),
        "magenta" => Some("controls.base.magenta"),
        "manual" => Some("controls.scanner.manual"),
        "material" => Some("controls.grain.material"),
        "matrix" => Some("controls.scanner.metering_matrix"),
        "max size of the long edge of the preview image in pixels" => {
            Some("controls.display.preview_size_help")
        }
        "maximum" => Some("controls.metadata.maximum"),
        "median" => Some("controls.scanner.metering_median"),
        "micro structure" => Some("controls.grain.micro_structure"),
        "midtones" => Some("controls.grain.midtones"),
        "minimum" => Some("controls.metadata.minimum"),
        "missing" => Some("surfaces.presets.missing"),
        "modified" => Some("surfaces.presets.modified"),
        "mult usm amount" => Some("controls.grain.unsharp_amount"),
        "mult usm sigma" => Some("controls.grain.unsharp_sigma"),
        "multi_zone" => Some("controls.scanner.metering_multizone"),
        "negative film; eligible negative-to-positive route; standard conversion style" => {
            Some("controls.condition.operator_route")
        }
        "neutralize print filters" => Some("controls.convert.neutralize_filters"),
        "never" => Some("controls.scanner.never"),
        "none" => Some("controls.scanner.none"),
        "normalized scanner controls: scan output" => Some("surfaces.status.normalized"),
        "normalized scanner controls: scan output; scanner controls inactive for this route; stored selections retained" => {
            Some("surfaces.status.normalized_inactive")
        }
        "not applicable / not reached" => Some("surfaces.diagnostics.not_applicable"),
        "not collected" => Some("surfaces.diagnostics.not_collected"),
        "nullable" => Some("controls.metadata.nullable"),
        "off" => Some("controls.gamut.off"),
        "off: disable output gamut compression. aces_rgc (default): per-channel ACES RGC v1.3, matches Resolve/Nuke/OCIO and is the cheapest per pixel. cam16ucs: CAM16-UCS chroma reduction with the smoothest constant-hue behavior (heaviest). oklch: perceptual chroma reduction in OkLab, preserves hue + lightness. oklrab/jzazbz: alternative perceptual spaces." => {
            Some("controls.gamut.output_algorithm_help")
        }
        "on" => Some("controls.label.on"),
        "output cctf encoding" => Some("controls.output.output_cctf"),
        "output color space" => Some("controls.output.output_color_space"),
        "output interpolation" => Some("surfaces.viewer.interpolation"),
        "partial" => Some("controls.scanner.metering_partial"),
        "particle area (µm²)" => Some("controls.grain.particle_area"),
        "particle scale" => Some("controls.grain.particle_scale"),
        "particle scale sublayers" => Some("controls.grain.particle_scale_sublayers"),
        "paste operator correction" => Some("controls.scanner.paste_correction"),
        "percent" => Some("controls.label.percent"),
        "pixels" => Some("controls.unit.pixels"),
        "power" => Some("controls.label.power"),
        "preflash exposure" => Some("controls.scanner.preflash_exposure"),
        "preflash m filter shift" => Some("controls.scanner.preflash_m_shift"),
        "preflash y filter shift" => Some("controls.scanner.preflash_y_shift"),
        "preserved upstream no-op" => Some("controls.condition.no_op"),
        "preset actions" => Some("surfaces.presets.actions"),
        "preset actions for %{name}" => Some("surfaces.presets.actions_name"),
        "preview max size" => Some("controls.display.preview_size"),
        "print exposure" => Some("controls.scanner.print_exposure"),
        "print exposure ev tracking" => Some("controls.scanner.print_tracking"),
        "print exposure mode" => Some("controls.scanner.print_exposure_mode"),
        "print illuminant" => Some("controls.scanner.print_illuminant"),
        "print m filter shift" => Some("controls.scanner.print_m_shift"),
        "print profile" => Some("controls.label.print_profile"),
        "print y filter shift" => Some("controls.scanner.print_y_shift"),
        "print_channel_swap" => Some("controls.convert.print_channel_swap"),
        "print_render.density_curves_morph.active = true; supported fitted density_curves_model required" => {
            Some("controls.condition.print_chemistry")
        }
        "processing mode" => Some("controls.grain.processing_mode"),
        "protect ev" => Some("controls.halation.protect_ev"),
        "radius span" => Some("controls.grain.radius_span"),
        "relative" => Some("controls.unit.relative"),
        "reset" => Some("surfaces.state.reset"),
        "reset %{section}" => Some("surfaces.section.reset_name"),
        "reset view" => Some("surfaces.viewer.reset"),
        "resolution type" => Some("controls.grain.resolution_type"),
        "restore automatic" => Some("controls.scanner.restore_automatic"),
        "retained output; viewer-only" => Some("controls.condition.retained"),
        "reveal transition" => Some("surfaces.viewer.reveal"),
        "rms density" => Some("controls.grain.rms_density"),
        "rms granularity" => Some("controls.grain.rms_granularity"),
        "roughness" => Some("controls.label.roughness"),
        "sRGB for web · Adobe RGB for print · ACES2065-1 for archival" => {
            Some("surfaces.export.space_hint")
        }
        "saving cctf encoding" => Some("controls.output.saving_cctf"),
        "saving color space" => Some("controls.output.saving_color_space"),
        "scale" => Some("controls.base.scale"),
        "scan for print" => Some("controls.label.scan_for_print"),
        "scan illuminant" => Some("controls.convert.scan_illuminant"),
        "scan output" => Some("controls.scanner.scan_output"),
        "scanner selections retained; scanner controls inactive for this route; stored selections retained" => {
            Some("surfaces.status.scanner_inactive")
        }
        "scatter amount" => Some("controls.halation.scatter_amount"),
        "scatter core um" => Some("controls.halation.scatter_core"),
        "scatter spatial scale" => Some("controls.halation.scatter_spatial_scale"),
        "scatter tail um" => Some("controls.halation.scatter_tail"),
        "scatter tail weight" => Some("controls.halation.scatter_tail_weight"),
        "select ICC profile…" => Some("surfaces.display.select_profile"),
        "selected ICC profile" => Some("controls.condition.display_profile"),
        "selected material has no supported fitted density_curves_model; chemistry values remain stored and inactive" => {
            Some("controls.condition.missing_model")
        }
        "selected report" => Some("controls.condition.report"),
        "shadows" => Some("controls.grain.shadows"),
        "sigma" => Some("controls.label.sigma"),
        "simulation blend" => Some("controls.output.simulation_blend"),
        "size" => Some("controls.grain.size"),
        "softness" => Some("controls.grain.softness"),
        "source compression" => Some("controls.halation.source_compression"),
        "spatial effects enabled" => Some("controls.condition.spatial"),
        "spatial scale" => Some("controls.label.spatial_scale"),
        "spectral gaussian blur" => Some("controls.upsampling.gaussian_blur"),
        "spectral upsampling" => Some("controls.upsampling.method"),
        "standard negative_to_positive on an eligible negative film scan route" => {
            Some("controls.condition.scan_operator")
        }
        "step" => Some("controls.metadata.step"),
        "stored corrections retained while inactive; clipboard and comparison are session-only" => {
            Some("controls.condition.operator_retained")
        }
        "strength" => Some("controls.cmyk.strength"),
        "sub-layers" => Some("controls.grain.sublayer_count"),
        "sublayer shares" => Some("controls.grain.sublayer_shares"),
        "sublayers active" => Some("controls.grain.sublayers_active"),
        "temperature" => Some("controls.scanner.temperature"),
        "threshold" => Some("controls.label.threshold"),
        "tilt" => Some("controls.base.tilt"),
        "timer" => Some("controls.grain.timer"),
        "tint" => Some("controls.scanner.raw_tint"),
        "tungsten" => Some("controls.scanner.tungsten"),
        "type" => Some("controls.metadata.type"),
        "unavailable" => Some("surfaces.diagnostics.value_unavailable"),
        "uniformity" => Some("controls.grain.uniformity"),
        "unit" => Some("controls.metadata.unit"),
        "unsharp mask (digital sharpening)" => Some("controls.scanner.unsharp_mask"),
        "unsubmitted export draft" => Some("controls.condition.draft"),
        "upscale factor" => Some("controls.crop.upscale_factor"),
        "use cat16" => Some("controls.upsampling.use_cat16"),
        "use display transform" => Some("controls.label.use_display_transform"),
        "viewer output layer only" => Some("controls.condition.viewer_output"),
        "viewer-only" => Some("controls.condition.viewer"),
        "warmth" => Some("controls.scanner.warmth"),
        "when engine = spektra: film_render.grain.sublayers_active = true; film profile supplies sublayer density curves" => {
            Some("controls.condition.spektra_sublayers")
        }
        "white balance" => Some("controls.scanner.white_balance"),
        "white padding" => Some("controls.label.white_padding"),
        "workflow" => Some("controls.label.workflow"),
        "working dimensions unavailable" => Some("surfaces.diagnostics.dimensions_missing"),
        "xy: radial compression in CIE 1931 chromaticity toward the spectral locus / inscribed hull (ACES RGC family, preserves dominant wavelength). oklch: perceptual chroma reduction in OkLab. off: disable input gamut compression." => {
            Some("controls.gamut.input_algorithm_help")
        }
        "yellow" => Some("controls.base.yellow"),
        "zoom %{zoom}" => Some("surfaces.viewer.zoom"),
        _ => None,
    }
}

pub(crate) fn localized<'a>(ctx: &egui::Context, label: &'a str) -> Cow<'a, str> {
    localized_for(crate::i18n::locale(ctx), label)
}

pub(crate) fn localized_for<'a>(locale: crate::i18n::Locale, label: &'a str) -> Cow<'a, str> {
    if let Some(key) = key(label) {
        return crate::i18n::text_for(locale, key);
    }
    match label {
        "Layer amounts 0" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_amounts"),
            component = Cow::Borrowed("0")
        ),
        "Layer amounts 1" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_amounts"),
            component = Cow::Borrowed("1")
        ),
        "Layer amounts 2" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_amounts"),
            component = Cow::Borrowed("2")
        ),
        "Layer size ratio 0" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_size_ratio"),
            component = Cow::Borrowed("0")
        ),
        "Layer size ratio 1" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_size_ratio"),
            component = Cow::Borrowed("1")
        ),
        "Layer size ratio 2" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_size_ratio"),
            component = Cow::Borrowed("2")
        ),
        "Layer weights 0" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_weights"),
            component = Cow::Borrowed("0")
        ),
        "Layer weights 1" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_weights"),
            component = Cow::Borrowed("1")
        ),
        "Layer weights 2" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.layer_weights"),
            component = Cow::Borrowed("2")
        ),
        "Unsharp mask (digital sharpening) amount" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.scanner.unsharp_mask"),
            component = crate::i18n::text_for(locale, "controls.label.amount")
        ),
        "Unsharp mask (digital sharpening) sigma" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.scanner.unsharp_mask"),
            component = crate::i18n::text_for(locale, "controls.label.sigma")
        ),
        "crop center x" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.crop.center"),
            component = Cow::Borrowed("x")
        ),
        "crop center y" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.crop.center"),
            component = Cow::Borrowed("y")
        ),
        "crop size x" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.crop.size"),
            component = Cow::Borrowed("x")
        ),
        "crop size y" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.crop.size"),
            component = Cow::Borrowed("y")
        ),
        "density min B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.density_min"),
            component = Cow::Borrowed("B")
        ),
        "density min G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.density_min"),
            component = Cow::Borrowed("G")
        ),
        "density min R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.density_min"),
            component = Cow::Borrowed("R")
        ),
        "gamma interlayer b to rg G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_blue_to_red_green"),
            component = Cow::Borrowed("G")
        ),
        "gamma interlayer b to rg R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_blue_to_red_green"),
            component = Cow::Borrowed("R")
        ),
        "gamma interlayer g to rb B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_green_to_red_blue"),
            component = Cow::Borrowed("B")
        ),
        "gamma interlayer g to rb R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_green_to_red_blue"),
            component = Cow::Borrowed("R")
        ),
        "gamma interlayer r to gb B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_red_to_green_blue"),
            component = Cow::Borrowed("B")
        ),
        "gamma interlayer r to gb G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_red_to_green_blue"),
            component = Cow::Borrowed("G")
        ),
        "gamma samelayer rgb B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_samelayer"),
            component = Cow::Borrowed("B")
        ),
        "gamma samelayer rgb G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_samelayer"),
            component = Cow::Borrowed("G")
        ),
        "gamma samelayer rgb R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.gamma_samelayer"),
            component = Cow::Borrowed("R")
        ),
        "halation first sigma um B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.first_sigma"),
            component = Cow::Borrowed("B")
        ),
        "halation first sigma um G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.first_sigma"),
            component = Cow::Borrowed("G")
        ),
        "halation first sigma um R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.first_sigma"),
            component = Cow::Borrowed("R")
        ),
        "halation strength B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.halation_strength"),
            component = Cow::Borrowed("B")
        ),
        "halation strength G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.halation_strength"),
            component = Cow::Borrowed("G")
        ),
        "halation strength R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.halation_strength"),
            component = Cow::Borrowed("R")
        ),
        "knee limit" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.gamut.knee"),
            component = crate::i18n::text_for(locale, "controls.label.limit")
        ),
        "knee power" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.gamut.knee"),
            component = crate::i18n::text_for(locale, "controls.label.power")
        ),
        "knee threshold" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.gamut.knee"),
            component = crate::i18n::text_for(locale, "controls.label.threshold")
        ),
        "langmuir donor k rgb B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_donor"),
            component = Cow::Borrowed("B")
        ),
        "langmuir donor k rgb G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_donor"),
            component = Cow::Borrowed("G")
        ),
        "langmuir donor k rgb R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_donor"),
            component = Cow::Borrowed("R")
        ),
        "langmuir receiver k rgb B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_receiver"),
            component = Cow::Borrowed("B")
        ),
        "langmuir receiver k rgb G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_receiver"),
            component = Cow::Borrowed("G")
        ),
        "langmuir receiver k rgb R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.couplers.langmuir_receiver"),
            component = Cow::Borrowed("R")
        ),
        "micro structure blur" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.micro_structure"),
            component = crate::i18n::text_for(locale, "controls.grain.blur")
        ),
        "micro structure clump" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.micro_structure"),
            component = crate::i18n::text_for(locale, "controls.label.clump")
        ),
        "particle scale 0" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale"),
            component = Cow::Borrowed("0")
        ),
        "particle scale 1" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale"),
            component = Cow::Borrowed("1")
        ),
        "particle scale 2" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale"),
            component = Cow::Borrowed("2")
        ),
        "particle scale sublayers 0" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale_sublayers"),
            component = Cow::Borrowed("0")
        ),
        "particle scale sublayers 1" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale_sublayers"),
            component = Cow::Borrowed("1")
        ),
        "particle scale sublayers 2" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.particle_scale_sublayers"),
            component = Cow::Borrowed("2")
        ),
        "rms granularity B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.rms_granularity"),
            component = Cow::Borrowed("B")
        ),
        "rms granularity G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.rms_granularity"),
            component = Cow::Borrowed("G")
        ),
        "rms granularity R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.rms_granularity"),
            component = Cow::Borrowed("R")
        ),
        "scatter core um B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_core"),
            component = Cow::Borrowed("B")
        ),
        "scatter core um G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_core"),
            component = Cow::Borrowed("G")
        ),
        "scatter core um R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_core"),
            component = Cow::Borrowed("R")
        ),
        "scatter tail um B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail"),
            component = Cow::Borrowed("B")
        ),
        "scatter tail um G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail"),
            component = Cow::Borrowed("G")
        ),
        "scatter tail um R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail"),
            component = Cow::Borrowed("R")
        ),
        "scatter tail weight B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail_weight"),
            component = Cow::Borrowed("B")
        ),
        "scatter tail weight G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail_weight"),
            component = Cow::Borrowed("G")
        ),
        "scatter tail weight R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.halation.scatter_tail_weight"),
            component = Cow::Borrowed("R")
        ),
        "uniformity B" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.uniformity"),
            component = Cow::Borrowed("B")
        ),
        "uniformity G" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.uniformity"),
            component = Cow::Borrowed("G")
        ),
        "uniformity R" => rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = crate::i18n::text_for(locale, "controls.grain.uniformity"),
            component = Cow::Borrowed("R")
        ),
        // Names supplied by profiles, standards and users retain their identity.
        _ => Cow::Borrowed(label),
    }
}
