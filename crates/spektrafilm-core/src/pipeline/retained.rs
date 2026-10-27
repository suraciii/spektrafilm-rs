//! Retained production render: the whole image route over device carriers.
//!
//! `specs/gpu-execution/design.md` (§Device residency) requires the operation
//! to retain device ownership from input numerical preparation through film
//! and scan work, output composition, and the final saving or display
//! conversion. This module is the runtime half of that: it resolves the same
//! parameters, geometry, and stage order the host route uses, then hands each
//! stage a retained carrier instead of host samples.
//!
//! The host route stays authoritative and unchanged. Only an initially
//! selected CPU backend may use that route; failure after GPU selection is
//! reported without restarting the operation on the host.

use super::*;
use crate::WorkflowRoute;
use spektrafilm_backend::{
    ConvertColorspaceGpuParams, DeviceImage, InputGeometryGpuParams, Precision, RetainedImageMeta,
    SampleDomain,
};

/// One retained render request.
#[derive(Debug, Clone, Copy)]
pub struct RetainedRender {
    /// Quarter turns applied by the device geometry stage, counter-clockwise.
    pub rotation_quarter_turns: u32,
    /// Preview long-edge cap. Standard negative conversion renders the full
    /// working frame and downsamples only the finished `rgb_out`; other routes
    /// apply the triangle preview resample before pipeline execution.
    pub preview_max_size: Option<u32>,
}

impl RetainedRender {
    /// Full-resolution render with no deferred rotation.
    pub const FULL: Self = Self {
        rotation_quarter_turns: 0,
        preview_max_size: None,
    };

    /// A full-resolution render with a deferred rotation.
    pub fn rotated(quarter_turns: u32) -> Self {
        Self {
            rotation_quarter_turns: quarter_turns,
            preview_max_size: None,
        }
    }

    /// A preview render at `max_size`, measured on the full source.
    pub fn preview(max_size: u32) -> Self {
        Self {
            rotation_quarter_turns: 0,
            preview_max_size: Some(max_size),
        }
    }
}

impl Pipeline {
    /// Render one image and return its retained result.
    ///
    /// The route is: upload the encoded source once, meter it there, apply the
    /// exposure scale, resolve the working geometry on the device, run the
    /// resident chain, then blend the prepared source when the request asks
    /// for a mix. Every step consumes the previous carrier, so no stage
    /// materializes an image.
    ///
    /// `Ok(None)` is reserved for an initially selected CPU backend.
    pub fn process_retained(
        &self,
        image: &ImageBuf,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<DeviceImage>, String> {
        self.process_retained_with_taps(image, request, backend, None, None)
    }

    /// Upload a host source once and retain explicit diagnostic boundaries.
    pub fn process_retained_with_taps(
        &self,
        image: &ImageBuf,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<Option<DeviceImage>, String> {
        if !backend.is_gpu() {
            return Ok(None);
        }
        let cancellation_backend = self
            .cancellation
            .as_ref()
            .map(|token| {
                backend
                    .with_cancellation(Some(token.clone()))
                    .ok_or_else(|| "selected GPU cannot bind operation cancellation".to_owned())
            })
            .transpose()?;
        let backend = cancellation_backend.as_deref().unwrap_or(backend);
        self.validate_retained_request(backend)?;
        let input_space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)?;
        let carrier = backend.retain_image(
            image,
            &RetainedImageMeta {
                width: image.width,
                height: image.height,
                domain: if self.params.io.input_cctf_decoding {
                    SampleDomain::Encoded
                } else {
                    SampleDomain::Linear
                },
                precision: Precision::host(),
                space: input_space,
                pixel_pitch_um: None,
            },
        )?;
        let (inject, collect) = self.retained_taps(inject, collect)?;
        self.process_retained_applied(&carrier, request, backend, inject, collect)
    }

    /// Continue a device-produced input (including RAW) without materializing
    /// or uploading its samples. Stored colour and transfer state must match
    /// the source declaration, just as for a host input.
    pub fn process_device_retained(
        &self,
        image: &DeviceImage,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
    ) -> Result<DeviceImage, String> {
        self.process_device_retained_with_taps(image, request, backend, None, None)
    }

    /// Device-source equivalent of `process_with_taps`; explicit boundaries
    /// override the persistent tap parameters without materializing samples.
    pub fn process_device_retained_with_taps(
        &self,
        image: &DeviceImage,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<DeviceImage, String> {
        if !backend.is_gpu() || !image.is_retained() {
            return self.fail("device input requires its selected GPU backend");
        }
        let cancellation_backend = self
            .cancellation
            .as_ref()
            .map(|token| {
                backend
                    .with_cancellation(Some(token.clone()))
                    .ok_or_else(|| "selected GPU cannot bind operation cancellation".to_owned())
            })
            .transpose()?;
        let backend = cancellation_backend.as_deref().unwrap_or(backend);
        self.validate_retained_request(backend)?;
        backend.image_payload(image)?;
        let (inject, collect) = self.retained_taps(inject, collect)?;
        self.process_retained_applied(image, request, backend, inject, collect)?
            .ok_or_else(|| "selected GPU declined its retained input".to_owned())
    }

    fn validate_retained_request(&self, backend: &dyn ComputeBackend) -> Result<(), String> {
        if self.tc_lut.is_some() {
            let prepared = self.prepared_tc_lut.as_ref().ok_or_else(|| {
                "selected GPU requires device-prepared spectral calibration; construct with Runtime::new_with_backend or Pipeline::new_with_spectral_backend, or rebuild with with_params_with_backend using the selected backend".to_owned()
            })?;
            if backend.preparation_device_id() != Some(prepared.values.backend_id()) {
                return Err("spectral calibration belongs to another device; rebuild with with_params_with_backend using the selected backend".to_owned());
            }
        }
        // The same validation the host route performs, in the same order.
        crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
        crate::stages::grain_fotufilm::validate_supported(&self.film, &self.params)?;
        self.check_cancelled()?;
        Ok(())
    }

    fn retained_taps(
        &self,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<(Tap, Tap), String> {
        let inject = match inject {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .inject
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbIn),
        };
        let collect = match collect {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .collect
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbOut),
        };
        let order = self.tap_order();
        for tap in [inject, collect] {
            if !order.contains(&tap) {
                return Err(format!(
                    "tap {tap:?} is not part of this topology (scan_film = {})",
                    self.params.io.scan_film
                ));
            }
        }
        if collect.position() < inject.position() {
            return Err(format!(
                "no node path reaches tap {collect:?} from {inject:?}"
            ));
        }
        if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 && inject != Tap::RgbIn {
            return Err(
                "output_mix.amount below 1 requires rgb_in injection for final rgb_out collection"
                    .into(),
            );
        }
        Self::validate_grain_v3_taps(false, inject, collect, self.params.io.scan_film)?;
        Ok((inject, collect))
    }

    /// Retained body shared by uploaded rasters and device-produced sources.
    fn process_retained_applied(
        &self,
        image: &DeviceImage,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        inject: Tap,
        collect: Tap,
    ) -> Result<Option<DeviceImage>, String> {
        let defer_preview = self.params.scanner.requires_full_frame();
        let working_preview = if defer_preview {
            None
        } else {
            request.preview_max_size
        };
        if inject == collect {
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                image.width(),
                image.height(),
                &self.params.io,
                request.rotation_quarter_turns as i32,
                Some(
                    if collect == Tap::RgbOut {
                        request.preview_max_size
                    } else {
                        working_preview
                    }
                    .unwrap_or(u32::MAX),
                ),
                image.pixel_pitch_um().unwrap_or_else(|| {
                    crate::resizing::pixel_size_um(
                        self.params.camera.film_format_mm,
                        image.width(),
                        image.height(),
                    )
                }),
            )?;
            let result = self.retained_geometry_stage(image, &geometry, backend)?;
            if let Some(context) = backend.observation_context() {
                context.set_working_dimensions(result.width(), result.height());
                context.set_path(spektrafilm_backend::telemetry::ExecutionPath::GpuResident);
            }
            return Ok(Some(result));
        }

        let route = self.params.workflow.route_enum();
        if inject == Tap::RgbIn && collect == Tap::RgbOut {
            self.ensure_gpu_coverage(backend)?;
        }
        // A convert-film route's numerical content is the scanner inverse
        // solver, which the resident chain runs as its `ScanInverse` head
        // (`resident_front_pass`), so the route keeps its retained entry
        // exactly like the film routes do.
        let input_space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)?;
        let output_space =
            spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)?;
        let source_pitch = crate::resizing::pixel_size_um(
            self.params.camera.film_format_mm,
            image.width(),
            image.height(),
        );

        let expected_domain = if self.params.io.input_cctf_decoding {
            SampleDomain::Encoded
        } else {
            SampleDomain::Linear
        };
        if inject == Tap::RgbIn
            && (image.domain() != expected_domain || image.space().name != input_space.name)
        {
            return self.fail("retained input colour state does not match its source declaration");
        }
        let mut meta = image.meta();
        meta.pixel_pitch_um = Some(source_pitch);
        meta.domain = match inject {
            Tap::LogEFilm | Tap::LogEPrint => SampleDomain::LogExposure,
            Tap::CmyFilm | Tap::CmyPrint => SampleDomain::Density,
            _ => image.domain(),
        };
        let mut carrier = image.with_meta(meta);

        // The output-mix source is the delivered input, not the exposed or
        // geometried working image. Cloning retains immutable sample ownership.
        let mix_input = carrier.clone();

        // 1. Meter the full source, then apply the scale to the uploaded
        //    samples. Both operate before any geometry, which is the reference
        //    order: `expose_full_input` scales the encoded image and
        //    `rescale_working` runs afterwards. The input-only route runs no
        //    camera, so it has no exposure to apply, and a convert-film
        //    route's image is already a scanner capture, which
        //    `expose_full_input` returns unchanged.
        if inject == Tap::RgbIn
            && route != WorkflowRoute::Input
            && !route.is_convert_film()
            && self.params.camera.auto_exposure
        {
            let ae_ev = self.meter_retained(&carrier, backend)?;
            carrier = match backend.scale_retained(&carrier, 2.0f64.powf(ae_ev) as f32)? {
                Some(scaled) => scaled,
                None => {
                    return self.declines_or_fails(
                        backend,
                        "the selected backend cannot scale a retained image exposure",
                    );
                }
            };
        }

        // 2. Working geometry on the device. Dye and Fotufilm develop the full
        //    source: their field readout owns the crop and the output lattice,
        //    so the generic geometry stage stays identity for them.
        let full_source_fields = route.exposes_film()
            && (crate::stages::grain_v3::active(&self.params)
                || crate::stages::grain_fotufilm::active(&self.params));
        let mut working_pitch = source_pitch;
        let mut applied_geometry: Vec<InputGeometryGpuParams> = Vec::new();
        if inject == Tap::RgbIn && !full_source_fields {
            // The reference applies geometry in two distinct orders. A preview
            // resizes the full frame first and every later crop or upscale is
            // resolved inside that preview frame; a full render crops and
            // rescales the full source directly. Both run on the carried
            // samples, so the second stage reuses the first one's buffer.
            if let Some(max_size) = working_preview {
                // The preview frame is the reference's working image, so its
                // own magnification defines the pitch every later stage sees.
                // The long-edge rule is symmetric, so the bound of the rotated
                // frame is the rotation of the bound.
                let (preview_width, preview_height) = crate::resizing::preview_dimensions(
                    carrier.width(),
                    carrier.height(),
                    max_size,
                );
                let preview_pitch = crate::resizing::pixel_size_um(
                    self.params.camera.film_format_mm,
                    preview_width,
                    preview_height,
                );
                // The preview stage's own rule (rotate, then bound the long
                // edge with the triangle reference, no crop) is the same one
                // `resolve_device_geometry` applies for `Some(max_size)`.
                let (preview, _) = crate::resizing::resolve_device_geometry(
                    carrier.width(),
                    carrier.height(),
                    &self.params.io,
                    request.rotation_quarter_turns as i32,
                    Some(max_size),
                    preview_pitch,
                )?;
                carrier = self.retained_geometry_stage(&carrier, &preview, backend)?;
                applied_geometry.push(preview);
                working_pitch = preview_pitch;
                if self.params.io.upscale_factor != 1.0 {
                    working_pitch /= self.params.io.upscale_factor as f64;
                }
                let (params, _dimensions) = crate::resizing::resolve_device_geometry(
                    carrier.width(),
                    carrier.height(),
                    &self.params.io,
                    0,
                    None,
                    working_pitch,
                )?;
                carrier = self.retained_geometry_stage(&carrier, &params, backend)?;
                working_pitch = params.pixel_pitch_um;
                applied_geometry.push(params);
            } else {
                let (params, _dimensions) = crate::resizing::resolve_device_geometry(
                    carrier.width(),
                    carrier.height(),
                    &self.params.io,
                    request.rotation_quarter_turns as i32,
                    None,
                    if self.params.io.upscale_factor != 1.0 {
                        source_pitch / self.params.io.upscale_factor as f64
                    } else {
                        source_pitch
                    },
                )?;
                carrier = self.retained_geometry_stage(&carrier, &params, backend)?;
                working_pitch = params.pixel_pitch_um;
                applied_geometry.push(params);
            }
        }
        if inject == Tap::RgbIn && full_source_fields {
            // Optical grain owns crop/upscale, but input orientation and an
            // ordinary preview still precede development of its full source.
            let (width, height) =
                working_preview.map_or((carrier.width(), carrier.height()), |max_size| {
                    crate::resizing::preview_dimensions(carrier.width(), carrier.height(), max_size)
                });
            working_pitch =
                crate::resizing::pixel_size_um(self.params.camera.film_format_mm, width, height);
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                carrier.width(),
                carrier.height(),
                &self.params.io,
                request.rotation_quarter_turns as i32,
                Some(working_preview.unwrap_or(u32::MAX)),
                working_pitch,
            )?;
            carrier = self.retained_geometry_stage(&carrier, &geometry, backend)?;
            applied_geometry.push(geometry);
            if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 {
                // The optical field owns result geometry, not the source
                // branch. Replay its requested crop/upscale after orientation
                // and preview, without applying either to field conditioning.
                let (geometry, _) = crate::resizing::resolve_device_geometry(
                    carrier.width(),
                    carrier.height(),
                    &self.params.io,
                    0,
                    None,
                    working_pitch / self.params.io.upscale_factor as f64,
                )?;
                applied_geometry.push(geometry);
            }
        }
        if inject != Tap::RgbIn {
            // Preview and deferred orientation precede even an injected
            // diagnostic image; camera metering, crop and upscale do not.
            let (width, height) =
                working_preview.map_or((carrier.width(), carrier.height()), |max_size| {
                    crate::resizing::preview_dimensions(carrier.width(), carrier.height(), max_size)
                });
            working_pitch =
                crate::resizing::pixel_size_um(self.params.camera.film_format_mm, width, height);
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                carrier.width(),
                carrier.height(),
                &self.params.io,
                request.rotation_quarter_turns as i32,
                Some(working_preview.unwrap_or(u32::MAX)),
                working_pitch,
            )?;
            carrier = self.retained_geometry_stage(&carrier, &geometry, backend)?;
        }
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(carrier.width(), carrier.height());
        }

        if collect == Tap::RgbPre {
            if let Some(context) = backend.observation_context() {
                context.set_path(spektrafilm_backend::telemetry::ExecutionPath::GpuResident);
            }
            return Ok(Some(carrier));
        }

        let mut result = if route == WorkflowRoute::Input {
            let result = self.retained_input_route(&carrier, input_space, output_space, backend)?;
            if let Some(context) = backend.observation_context() {
                context.set_path(spektrafilm_backend::telemetry::ExecutionPath::GpuResident);
            }
            result
        } else {
            // Auto-exposure already ran above. Convert-film instead starts
            // at the scanner inverse solve, with no RGB-to-raw front pass.
            if inject.position() <= Tap::RgbPre.position()
                && self.tc_lut.is_none()
                && self.mallett_core.is_none()
                && !route.is_convert_film()
            {
                return self.declines_or_fails(
                    backend,
                    "no RGB-to-raw front pass was resolved for the retained route",
                );
            }
            match self.try_gpu_resident_retained_with_taps(
                &carrier,
                backend,
                working_pitch,
                0.0,
                if inject == Tap::RgbIn {
                    Tap::RgbPre
                } else {
                    inject
                },
                collect,
            )? {
                Some(result) => result,
                None => {
                    return self.declines_or_fails(
                        backend,
                        "the selected GPU declined the retained stage chain",
                    );
                }
            }
        };

        // 4. The final mix reads the prepared source from the same upload.
        if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 {
            let source = self.retained_output_mix_source(
                &mix_input,
                &applied_geometry,
                input_space,
                output_space,
                backend,
            )?;
            result = match backend.mix_output_retained(
                &result,
                &source,
                &spektrafilm_backend::OutputMixGpuParams {
                    amount: self.params.output_mix.amount as f32,
                    encoded: self.params.io.output_cctf_encoding,
                    cctf_kind: spektrafilm_backend::cctf_kind(output_space.cctf),
                },
            )? {
                Some(mixed) => mixed,
                None => {
                    return self.declines_or_fails(
                        backend,
                        "the selected backend cannot blend two retained images",
                    );
                }
            };
        }
        if defer_preview
            && collect == Tap::RgbOut
            && let Some(max_size) = request.preview_max_size
        {
            let (width, height) =
                crate::resizing::preview_dimensions(result.width(), result.height(), max_size);
            let pitch = result.pixel_pitch_um().unwrap_or(working_pitch)
                * result.width().max(result.height()) as f64
                / width.max(height) as f64;
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                result.width(),
                result.height(),
                &self.params.io,
                0,
                Some(max_size),
                pitch,
            )?;
            result = self.retained_geometry_stage(&result, &geometry, backend)?;
        }
        Ok(Some(result))
    }

    /// Run one device geometry stage over the carried samples.
    ///
    /// A stage that resolves to nothing — no rotation, no crop, no resize — is
    /// skipped rather than dispatched: the carrier is already the working
    /// raster, and a backend that reports `None` for an unchanged request
    /// means "no work", not "cannot serve".
    pub(super) fn retained_geometry_stage(
        &self,
        carrier: &DeviceImage,
        params: &InputGeometryGpuParams,
        backend: &dyn ComputeBackend,
    ) -> Result<DeviceImage, String> {
        let identity =
            params.rotation_quarter_turns == 0 && params.crop.is_none() && params.resize.is_none();
        if identity {
            return Ok(carrier.clone());
        }
        match backend.input_geometry(carrier, params)? {
            Some(staged) => Ok(staged),
            None => {
                self.fail("the selected backend cannot apply working geometry to a retained image")
            }
        }
    }

    /// The input-only route over retained samples: convert to the output space
    /// and apply the terminal CMYK appearance. Geometry already ran, matching
    /// the reference order (crop and rescale, then the colour conversion).
    ///
    /// The pitch travels with the carrier: the geometry stage ran before this
    /// route, and the conversion and appearance passes retag their results from
    /// the carrier they consumed, so the returned carrier states the resolved
    /// working pitch — preview-derived when a preview bound was set, and the
    /// upload pitch when geometry was skipped for a full-source grain field.
    /// Nothing here reads a pitch that predates the resolved geometry.
    fn retained_input_route(
        &self,
        carrier: &DeviceImage,
        input_space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        output_space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        backend: &dyn ComputeBackend,
    ) -> Result<DeviceImage, String> {
        let matrix = spektrafilm_math::colorspace::conversion_matrix(input_space, output_space);
        let params = ConvertColorspaceGpuParams {
            src_cctf: spektrafilm_backend::cctf_kind(input_space.cctf),
            dst_cctf: spektrafilm_backend::cctf_kind(output_space.cctf),
            decode: self.params.io.input_cctf_decoding,
            encode: self.params.io.output_cctf_encoding,
            matrix: std::array::from_fn(|i| std::array::from_fn(|j| matrix[i][j] as f32)),
        };
        let domain = if self.params.io.output_cctf_encoding {
            SampleDomain::Encoded
        } else {
            SampleDomain::Linear
        };
        let mut converted =
            match backend.convert_colorspace_retained(carrier, &params, output_space, domain)? {
                Some(converted) => converted,
                None => {
                    return self.fail(
                    "the selected backend cannot convert a retained image between colour spaces",
                );
                }
            };
        if self.params.cmyk.active && self.params.cmyk.strength > 0.0 {
            let (to_lab, from_lab) = crate::cmyk::CmykTransform::new(output_space).gpu_matrices();
            let f32m = |m: &[[f64; 3]; 3]| {
                std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] as f32))
            };
            let cmyk = spektrafilm_backend::CmykGpuParams {
                strength: self.params.cmyk.strength.clamp(0.0, 1.0) as f32,
                encoded: self.params.io.output_cctf_encoding,
                cctf_kind: spektrafilm_backend::cctf_kind(output_space.cctf),
                to_lab: f32m(&to_lab),
                from_lab: f32m(&from_lab),
            };
            converted = match backend.cmyk_retained(&converted, &cmyk)? {
                Some(applied) => applied,
                None => {
                    return self.fail(
                        "the selected backend cannot apply the CMYK appearance to a retained image",
                    );
                }
            };
        }
        Ok(converted)
    }

    /// Prepare the untouched source branch: preview the stored transport,
    /// decode, apply requested crop/upscale, then convert primaries. Optical
    /// readouts defer result geometry but the source uses the delivered grid.
    pub(super) fn retained_output_mix_source(
        &self,
        mix_input: &DeviceImage,
        geometry: &[InputGeometryGpuParams],
        input_space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        output_space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        backend: &dyn ComputeBackend,
    ) -> Result<DeviceImage, String> {
        // CPU preview bounds the encoded source before pipeline ingress.
        // Unlike crop/spline geometry, that triangle stage precedes decode.
        let (preview, geometry) =
            geometry.split_at(usize::from(geometry.first().is_some_and(|stage| {
                stage.resize.is_some_and(|resize| {
                    resize.method == spektrafilm_backend::InputResizeMethod::PreviewTriangle
                })
            })));
        let mut preview_input = mix_input.clone();
        for params in preview {
            preview_input = self.retained_geometry_stage(&preview_input, params, backend)?;
        }
        // 1. Decode the source transport (identity matrix: transfer only).
        let mut carrier = if self.params.io.input_cctf_decoding {
            let decode = ConvertColorspaceGpuParams {
                src_cctf: spektrafilm_backend::cctf_kind(input_space.cctf),
                dst_cctf: spektrafilm_backend::cctf_kind(input_space.cctf),
                decode: true,
                encode: false,
                matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            };
            match backend.convert_colorspace_retained(
                &preview_input,
                &decode,
                input_space,
                SampleDomain::Linear,
            )? {
                Some(decoded) => decoded,
                None => {
                    return self
                        .fail("the selected backend cannot decode the prepared output-mix source");
                }
            }
        } else {
            preview_input
        };
        // 2. Requested source geometry, including deferred optical readout crop.
        for params in geometry {
            carrier = self.retained_geometry_stage(&carrier, params, backend)?;
        }
        // 3. Primaries of the output space, still linear.
        let matrix = spektrafilm_math::colorspace::conversion_matrix(input_space, output_space);
        let convert = ConvertColorspaceGpuParams {
            src_cctf: spektrafilm_backend::cctf_kind(input_space.cctf),
            dst_cctf: spektrafilm_backend::cctf_kind(output_space.cctf),
            decode: false,
            encode: false,
            matrix: std::array::from_fn(|i| std::array::from_fn(|j| matrix[i][j] as f32)),
        };
        match backend.convert_colorspace_retained(
            &carrier,
            &convert,
            output_space,
            SampleDomain::Linear,
        )? {
            Some(converted) => Ok(converted),
            None => self.fail("the selected backend cannot prepare the retained output-mix source"),
        }
    }

    /// Meter the retained source, keeping the bounded EV on the host.
    fn meter_retained(
        &self,
        carrier: &DeviceImage,
        backend: &dyn ComputeBackend,
    ) -> Result<f64, String> {
        let space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
            .expect("validated input color space");
        let device = backend.meter_autoexposure_retained(
            carrier,
            &spektrafilm_backend::MeterAutoexposureParams {
                width: carrier.width(),
                height: carrier.height(),
                max_size: crate::stages::filming::MAX_SIZE as u32,
                y_row: space.matrix_rgb_to_xyz[1],
                cctf_kind: self
                    .params
                    .io
                    .input_cctf_decoding
                    .then(|| spektrafilm_backend::cctf_kind(space.cctf)),
                method: self.params.camera.auto_exposure_method.clone(),
                mid_grey: crate::stages::filming::MID_GREY,
            },
        )?;
        match device {
            Some(ev) => Ok(ev),
            None => self.fail(
                "the selected backend cannot execute full-source auto-exposure metering on a \
                 retained image",
            ),
        }
    }

    /// A stage the retained route cannot run: an explicit GPU request fails by
    /// name. Automatic selection never reaches this on the retained route
    /// unless the backend is a GPU the operator selected, so the failure is
    /// the honest report rather than a silent host fence.
    fn fail<T>(&self, reason: &str) -> Result<T, String> {
        Err(format!("retained render: {reason}"))
    }

    /// A selected GPU cannot decline into the CPU reference route.
    fn declines_or_fails<T>(
        &self,
        _backend: &dyn ComputeBackend,
        reason: &str,
    ) -> Result<Option<T>, String> {
        Err(format!(
            "selected GPU cannot run the complete computation scope ({reason}); \
             retry with an explicit CPU request"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline() -> Pipeline {
        let data = crate::test_support::data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.io.scan_film = true;
        Pipeline::new(film, print, params)
    }

    #[test]
    fn retained_taps_preserve_override_precedence_and_topology() {
        let mut pipeline = pipeline();
        pipeline.params.taps.inject = Some("log_e_film".into());
        pipeline.params.taps.collect = Some("cmy_film".into());
        assert_eq!(
            pipeline.retained_taps(None, None).unwrap(),
            (Tap::LogEFilm, Tap::CmyFilm)
        );
        assert_eq!(
            pipeline
                .retained_taps(Some(Tap::RgbIn), Some(Tap::RgbOut))
                .unwrap(),
            (Tap::RgbIn, Tap::RgbOut)
        );
        assert!(
            pipeline
                .retained_taps(Some(Tap::CmyPrint), Some(Tap::RgbOut))
                .unwrap_err()
                .contains("not part of this topology")
        );
        assert!(
            pipeline
                .retained_taps(Some(Tap::CmyFilm), Some(Tap::LogEFilm))
                .unwrap_err()
                .contains("no node path")
        );
        pipeline.params.output_mix.amount = 0.5;
        assert!(
            pipeline
                .retained_taps(Some(Tap::LogEFilm), Some(Tap::RgbOut))
                .unwrap_err()
                .contains("requires rgb_in injection")
        );
    }

    #[test]
    fn automatic_selected_gpu_decline_never_restarts_on_cpu() {
        struct AutomaticGpu;
        impl ComputeBackend for AutomaticGpu {
            fn name(&self) -> &str {
                "automatic GPU"
            }
            fn is_gpu(&self) -> bool {
                true
            }
            fn colorspace_convert(
                &self,
                _: &ImageBuf,
                _: &[[f32; 3]; 3],
            ) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host conversion")
            }
            fn cctf_encode_srgb(&self, _: &ImageBuf) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host transfer encoding")
            }
            fn cctf_decode_srgb(&self, _: &ImageBuf) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host transfer decoding")
            }
            fn gaussian_blur(&self, _: &ImageBuf, _: f32) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host blur")
            }
            fn table_lookup(
                &self,
                _: &ImageBuf,
                _: &[f32],
                _: &[[f32; 3]],
            ) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host lookup")
            }
            fn lut3d_interp(
                &self,
                _: &ImageBuf,
                _: &spektrafilm_backend::Lut3D,
            ) -> Result<ImageBuf, String> {
                panic!("a decline must not execute host lattice interpolation")
            }
        }
        let error = pipeline()
            .declines_or_fails::<DeviceImage>(&AutomaticGpu, "missing RAW stage")
            .unwrap_err();
        assert!(error.contains("missing RAW stage"));
        assert!(error.contains("explicit CPU request"));
    }
}
