//! Retained-output tonal distribution. Source samples never cross the boundary.
//! Exact f32 bin transitions are the reference's own binary64 bisection,
//! resolved once on the host and retained with the backend identity, including
//! observed derivatives. Binning itself is integer and normalization is f32.
use super::*;
use spektrafilm_backend::DeviceImage;

/// First f32 word whose sRGB-encoded value rounds into the next bin. This is
/// the reference's own binary64 bisection, resolved on the host because the
/// table is bounded and image-independent.
fn histogram_boundaries_host() -> [u32; 63] {
    std::array::from_fn(|index| {
        let mut low = 0u32;
        let mut high = 1.0f32.to_bits();
        while low < high {
            let middle = low + (high - low) / 2;
            let encoded =
                spektrafilm_math::colorspace::srgb_encode_f64(f32::from_bits(middle) as f64);
            if (encoded * 63.0).round() as usize > index {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low
    })
}

impl WgpuBackend {
    fn histogram_boundaries_gpu(&self) -> Result<&wgpu::Buffer, String> {
        self.check_cancelled()?;
        if let Some(boundaries) = self.pipe.histogram_boundaries.get() {
            return Ok(boundaries);
        }
        let words = histogram_boundaries_host();
        let boundaries = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("histogram_boundaries"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | if cfg!(test) {
                        wgpu::BufferUsages::COPY_SRC
                    } else {
                        wgpu::BufferUsages::empty()
                    },
            });
        Ok(self.pipe.histogram_boundaries.get_or_init(|| boundaries))
    }

    fn histogram_page(
        &self,
        source: &wgpu::Buffer,
        pixels: u32,
        boundaries: &wgpu::Buffer,
        counts: &wgpu::Buffer,
        encoder: &mut super::observation::ObservedEncoder,
    ) -> Result<(), String> {
        if pixels == 0 {
            return Ok(());
        }
        let limit = self.device.limits().max_compute_workgroups_per_dimension;
        let groups = pixels.div_ceil(256);
        let x = groups.min(limit);
        let y = groups.div_ceil(x);
        if y > limit {
            return Err("histogram dispatch exceeds device limits".into());
        }
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("histogram_params"),
                contents: bytemuck::cast_slice(&[pixels, 0, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/retained_histogram.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let entries: Vec<_> = [&params, source, boundaries, counts]
            .into_iter()
            .enumerate()
            .map(|(i, buffer)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("histogram"),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("histogram"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(x, y, 1);
        Ok(())
    }

    pub(super) fn retained_histogram_gpu(
        &self,
        image: &DeviceImage,
    ) -> Result<[[f32; 64]; 3], String> {
        self.check_cancelled()?;
        #[cfg(test)]
        self.pipe.test_operation("retained_histogram")?;
        let scope = self.device.context.scope(
            "retained_histogram",
            ObservationKind::Stage,
            self.device.context.purpose(),
        );
        scope.context().record_executor(Executor::Gpu, None);
        let source = self.buffer_payload(image)?;
        let boundaries = self.histogram_boundaries_gpu()?;
        let counts = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("histogram_counts"),
            size: 192 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("histogram_bins"),
            size: 192 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        for page in &source.pages {
            self.check_cancelled()?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.histogram_page(
                &page.buffer,
                page.n_pixels,
                boundaries,
                &counts,
                &mut encoder,
            )?;
            self.check_cancelled()?;
            self.submit_commands(Some(encoder.finish()))?;
        }
        self.check_cancelled()?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let pipeline = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/retained_histogram_normalize.wgsl"),
            &[
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("histogram_normalize"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("histogram_normalize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        let result: Vec<f32> = self.pipe.readback_buffer_cancellable(&output, 192 * 4)?;
        Ok(std::array::from_fn(|channel| {
            std::array::from_fn(|bin| result[channel * 64 + bin])
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::{ComputeBackend, Precision, RetainedImageMeta, SampleDomain};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn histogram_readback_cancellation_discards_bins_without_poisoning_reuse() {
        let gpu = crate::test_support::explicit_adapter();
        let host = ImageBuf::from_data(
            3,
            1,
            vec![
                0.0 as _, 0.0 as _, 0.0 as _, 0.5 as _, 0.5 as _, 0.5 as _, 1.0 as _, 1.0 as _,
                1.0 as _,
            ],
        );
        let image = gpu
            .retain_image(
                &host,
                &RetainedImageMeta {
                    width: 3,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let reference = gpu.retained_histogram(&image).unwrap();
        assert_eq!(reference[0].iter().sum::<f32>(), 3.0);
        let cancellation = Arc::new(AtomicBool::new(false));
        gpu.pipe.test_on_readback({
            let cancellation = cancellation.clone();
            move || cancellation.store(true, Ordering::Relaxed)
        });
        let operation = gpu.with_cancellation(Some(cancellation)).unwrap();
        assert_eq!(
            operation.retained_histogram(&image).unwrap_err(),
            "operation cancelled"
        );
        gpu.pipe.test_on_readback(|| {});
        assert_eq!(gpu.retained_histogram(&image).unwrap(), reference);
        assert_eq!(
            operation
                .with_cancellation(None)
                .unwrap()
                .retained_histogram(&image)
                .unwrap(),
            reference
        );
        assert_eq!(image.to_host(gpu.backend()).unwrap().data, host.data);
    }

    use spektrafilm_math::colorspace::srgb_encode_f64;

    #[test]
    fn histogram_device_boundaries_match_every_reference_word_and_share_backend_identity() {
        let gpu = crate::test_support::adapter();
        let boundaries = gpu.histogram_boundaries_gpu().unwrap();
        let actual: Vec<u32> = gpu.pipe.readback_buffer_as(boundaries, 63 * 4);
        let expected: [u32; 63] = std::array::from_fn(|index| {
            let mut low = 0u32;
            let mut high = 1.0f32.to_bits();
            while low < high {
                let middle = low + (high - low) / 2;
                let encoded = srgb_encode_f64(f32::from_bits(middle) as f64);
                if (encoded * 63.0).round() as usize > index {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            low
        });
        assert_eq!(actual.as_slice(), expected.as_slice());
        let derived = gpu.derived_explicit();
        assert!(std::ptr::eq(
            boundaries,
            derived.histogram_boundaries_gpu().unwrap()
        ));
        assert!(std::ptr::eq(
            boundaries,
            gpu.histogram_boundaries_gpu().unwrap()
        ));
    }
}
