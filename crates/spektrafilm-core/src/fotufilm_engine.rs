use crate::params::{RuntimeParams, Tap};
use spektrafilm_fotufilm::{Session, Stage};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::{
    colorspace,
    image::ImageBuf,
    precision::{from_f32, to_f32, to_f64},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// Complete source graph. Delivery geometry is applied after full-frame source execution.
#[derive(Clone)]
pub(crate) struct FotufilmEngine {
    pub params: RuntimeParams,
    session: Session,
}

impl FotufilmEngine {
    pub fn new(params: RuntimeParams, data_dir: &Path) -> Result<Self, String> {
        params.validate()?;
        params.validate_color()?;
        let session = Session::new(data_dir, params.fotufilm.clone())?;
        Ok(Self { params, session })
    }

    pub fn process(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ImageBuf, String> {
        let inject = inject
            .or(self
                .params
                .taps
                .inject
                .as_deref()
                .map(Tap::parse)
                .transpose()?)
            .unwrap_or(Tap::RgbIn);
        let collect = collect
            .or(self
                .params
                .taps
                .collect
                .as_deref()
                .map(Tap::parse)
                .transpose()?)
            .unwrap_or(Tap::RgbOut);
        if !matches!(inject, Tap::RgbIn | Tap::CmyFilm)
            || !matches!(collect, Tap::RgbIn | Tap::CmyFilm | Tap::RgbOut)
        {
            return Err("Fotufilm taps support rgb_in (declared input), cmy_film (source gross developed density), and rgb_out; legacy log-exposure/print-density taps have no equivalent in the original graph".into());
        }
        if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("render canceled".into());
        }
        if inject == collect {
            return Ok(image);
        }
        if collect == Tap::RgbIn {
            return Err("Fotufilm cannot collect an earlier tap than the injected stage".into());
        }
        let mut observation = backend.observation_context().map(|context| {
            let scope = context.scope(
                "fotufilm_source_cpu_binary32",
                spektrafilm_gpu::telemetry::ObservationKind::Stage,
                spektrafilm_gpu::telemetry::Purpose::Image,
            );
            scope.context().record_executor(
                spektrafilm_gpu::telemetry::Executor::Cpu,
                Some(spektrafilm_gpu::telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        let requested_stage = self
            .params
            .fotufilm
            .native
            .get("stage")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("full");
        let density_input =
            inject == Tap::CmyFilm || (inject == Tap::RgbIn && requested_stage == "print");
        let source = if density_input {
            image.data.iter().copied().map(to_f32).collect()
        } else {
            convert(
                &image,
                colorspace::resolve(&self.params.io.input_color_space)?,
                &colorspace::ITU_R_BT2020,
                self.params.io.input_cctf_decoding,
                false,
            )
        };
        let stage = if collect == Tap::CmyFilm {
            Stage::Negative
        } else if inject == Tap::CmyFilm {
            Stage::Print
        } else {
            Stage::Full
        };
        let result = if collect == Tap::RgbOut && inject == Tap::RgbIn {
            self.session.render(
                image.width,
                image.height,
                &source,
                self.params.random_seed,
                cancellation,
            )
        } else {
            self.session.render_stage(
                image.width,
                image.height,
                &source,
                self.params.random_seed,
                cancellation,
                stage,
            )
        };
        if let Some(scope) = observation.as_mut() {
            scope.set_complete(result.is_ok());
        }
        let mut output = ImageBuf::from_data(
            image.width,
            image.height,
            result?.into_iter().map(from_f32).collect(),
        );
        let output_stage = if collect == Tap::CmyFilm {
            "negative"
        } else if inject == Tap::CmyFilm {
            "print"
        } else {
            requested_stage
        };
        if collect == Tap::RgbOut && output_stage != "negative" {
            let source_space = if output_stage == "texture" {
                &colorspace::ITU_R_BT2020
            } else {
                &colorspace::DISPLAY_P3
            };
            let converted = convert(
                &output,
                source_space,
                colorspace::resolve(&self.params.io.output_color_space)?,
                false,
                self.params.io.output_cctf_encoding,
            );
            output.data = converted.into_iter().map(from_f32).collect();
        }
        if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("render canceled".into());
        }
        if output_stage == "negative" {
            return Ok(output);
        }
        if self.params.io.crop {
            output = crate::resizing::crop_image(
                &output,
                self.params.io.crop_center,
                self.params.io.crop_size,
            )?;
        }
        let mut io = self.params.io.clone();
        io.crop = false;
        Ok(
            crate::resizing::crop_and_rescale(&output, &io, self.params.camera.film_format_mm)?
                .0
                .into_owned(),
        )
    }
}

fn convert(
    image: &ImageBuf,
    source: &colorspace::RgbColorSpace,
    destination: &colorspace::RgbColorSpace,
    decode: bool,
    encode: bool,
) -> Vec<f32> {
    let matrix = colorspace::conversion_matrix(source, destination);
    let identity = source.name == destination.name;
    let mut output = Vec::with_capacity(image.data.len());
    for pixel in image.data.as_chunks::<3>().0 {
        let mut rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        if decode {
            rgb = rgb.map(|v| colorspace::cctf_decode(v, source.cctf));
        }
        if !identity {
            rgb = matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]);
        }
        if encode {
            rgb = rgb.map(|v| colorspace::cctf_encode(v, destination.cctf));
        }
        output.extend(rgb.map(|v| v as f32));
    }
    output
}
