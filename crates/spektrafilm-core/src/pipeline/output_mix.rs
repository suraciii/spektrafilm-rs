//! Pipeline output-mix preparation and composition.

use super::*;

impl Pipeline {
    /// Prepare the already decoded/RAW-corrected input before virtual camera
    /// exposure. Decode its configured transport before spatial resampling;
    /// Dye and Fotufilm defer result geometry but use this same requested grid.
    pub(super) fn prepare_output_mix_source(&self, image: &ImageBuf) -> Result<ImageBuf, String> {
        use spektrafilm_math::colorspace::{cctf_decode, conversion_matrix, convert_rgb, resolve};
        let input_space = resolve(&self.params.io.input_color_space)?;
        let output_space = resolve(&self.params.io.output_color_space)?;
        let decoded = if self.params.io.input_cctf_decoding {
            let mut decoded = image.clone();
            decoded.data.par_iter_mut().for_each(|value| {
                *value = from_f64(cctf_decode(to_f64(*value), input_space.cctf));
            });
            Some(decoded)
        } else {
            None
        };
        let linear = decoded.as_ref().unwrap_or(image);
        let (working, _) = crate::resizing::crop_and_rescale(
            linear,
            &self.params.io,
            self.params.camera.film_format_mm,
        )?;
        let mut source = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => decoded.unwrap_or_else(|| image.clone()),
        };
        let matrix = conversion_matrix(input_space, output_space);
        source.data.par_chunks_exact_mut(3).for_each(|pixel| {
            let converted = convert_rgb(
                [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])],
                input_space,
                false,
                output_space,
                false,
                &matrix,
            );
            for channel in 0..3 {
                pixel[channel] = from_f64(converted[channel]);
            }
        });
        Ok(source)
    }

    /// The only final mix: both operands use linear output primaries, and the
    /// returned buffer uses the requested output transfer without clipping.
    pub(super) fn compose_output_mix(
        &self,
        mut result: ImageBuf,
        source: Option<ImageBuf>,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let amount = self.params.output_mix.amount;
        if amount == 1.0 {
            return Ok(result);
        }
        let t = Instant::now();
        let _mix = crate::stages::StageObservation::cpu(
            backend,
            "output_mix",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        let mut source = source.ok_or_else(|| {
            "output_mix.amount below 1 requires the prepared rgb_in source".to_owned()
        })?;
        if result.width != source.width || result.height != source.height {
            return Err("output_mix source and final output geometry differ".into());
        }
        use spektrafilm_math::colorspace::{cctf_decode, cctf_encode, resolve};
        let space = resolve(&self.params.io.output_color_space)?;
        let encoded = self.params.io.output_cctf_encoding;
        let output = if amount == 0.0 {
            // Do not touch the processed operand at the exact source endpoint.
            if encoded {
                source.data.par_iter_mut().for_each(|value| {
                    *value = from_f64(cctf_encode(to_f64(*value), space.cctf));
                });
            }
            source
        } else {
            result
                .data
                .par_iter_mut()
                .zip(source.data.par_iter())
                .for_each(|(result, source)| {
                    let result_linear = if encoded {
                        cctf_decode(to_f64(*result), space.cctf)
                    } else {
                        to_f64(*result)
                    };
                    let linear = (1.0 - amount) * to_f64(*source) + amount * result_linear;
                    *result = from_f64(if encoded {
                        cctf_encode(linear, space.cctf)
                    } else {
                        linear
                    });
                });
            result
        };
        record_stage_timing(&mut timings, "output_mix", t);
        Ok(output)
    }
}
