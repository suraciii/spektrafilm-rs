//! Native retained-image analysis. Only bounded count grids and one derived
//! pixel cross the device boundary; source samples stay resident.
use super::*;
use spektrafilm_backend::analysis::{
    AnalysisData, AnalysisMode, AnalysisPixel, AnalysisRequest, BINS,
};
use spektrafilm_backend::{DeviceImage, SampleDomain};

const COMMON: &str = concat!(
    include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
    include_str!("../../../spektrafilm-shaders/wgsl/retained_analysis_common.wgsl"),
    include_str!("../../../spektrafilm-shaders/wgsl/retained_analysis.wgsl"),
);
const PIXEL: &str = concat!(
    include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
    include_str!("../../../spektrafilm-shaders/wgsl/retained_analysis_common.wgsl"),
    include_str!("../../../spektrafilm-shaders/wgsl/retained_analysis_pixel.wgsl"),
);
const CHUNK: u32 = 262_144;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    page_first: u32,
    start: u32,
    count: u32,
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    mode: u32,
    zoom: u32,
    spatial: u32,
    decode: u32,
    cctf: u32,
    column_offset: u32,
    totals_offset: u32,
    pad: u32,
    rows: [[f32; 4]; 3],
    y_row: [f32; 4],
}

fn params(image: &DeviceImage) -> Result<Params, String> {
    let decode = match image.domain() {
        SampleDomain::Linear => 0,
        SampleDomain::Encoded => 1,
        _ => return Err("analysis requires RGB samples".into()),
    };
    let matrix = if image.space().name == "sRGB" {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    } else {
        image.space().display_matrix()
    };
    let y = spektrafilm_math::colorspace::SRGB_TO_XYZ_F64[1];
    Ok(Params {
        page_first: 0,
        start: 0,
        count: 0,
        width: image.width(),
        x0: 0,
        y0: 0,
        x1: image.width(),
        y1: image.height(),
        mode: 0,
        zoom: 1,
        spatial: 0,
        decode,
        cctf: spektrafilm_backend::cctf_kind(image.space().cctf),
        column_offset: 0,
        totals_offset: 0,
        pad: 0,
        rows: std::array::from_fn(|i| {
            [
                matrix[i][0] as f32,
                matrix[i][1] as f32,
                matrix[i][2] as f32,
                0.0,
            ]
        }),
        y_row: [y[0] as f32, y[1] as f32, y[2] as f32, 0.0],
    })
}

impl WgpuBackend {
    fn analysis_dispatch(
        &self,
        source: &wgpu::Buffer,
        output: &wgpu::Buffer,
        p: &Params,
        pixel: bool,
    ) -> Result<(), String> {
        self.check_cancelled()?;
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("analysis_parameters"),
                contents: bytemuck::bytes_of(p),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            if pixel { PIXEL } else { COMMON },
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("analysis"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_analysis"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(if pixel { 1 } else { p.count.div_ceil(256) }, 1, 1);
        }
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        Ok(())
    }

    pub(super) fn analyze_retained_gpu(
        &self,
        image: &DeviceImage,
        request: &AnalysisRequest,
    ) -> Result<AnalysisData, String> {
        self.check_cancelled()?;
        let source = self.buffer_payload(image)?;
        let mut p = params(image)?;
        let [x0, y0, x1, y1] = request.region;
        if x0 >= x1 || y0 >= y1 || x1 > image.width() || y1 > image.height() {
            return Err("analysis region is outside the image or empty".into());
        }
        if !matches!(request.chroma_zoom, 1 | 2 | 4) {
            return Err("analysis chroma zoom must be 1, 2, or 4".into());
        }
        p.x0 = x0;
        p.y0 = y0;
        p.x1 = x1;
        p.y1 = y1;
        p.spatial = (x1 - x0).min(BINS as u32);
        p.zoom = request.chroma_zoom;
        p.mode = match request.mode {
            AnalysisMode::Histogram => 0,
            AnalysisMode::Waveform | AnalysisMode::Parade => 1,
            AnalysisMode::Vectorscope => 2,
        };
        let cells = match p.mode {
            1 => 4 * p.spatial * 256,
            2 => 256 * 256,
            _ => 0,
        };
        p.column_offset = 1024 + cells;
        p.totals_offset = p.column_offset + p.spatial;
        let size = u64::from(p.totals_offset + 5) * 4;
        let counts = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("analysis_counts"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // Count selected pixels before a linear image boundary. A page's two
        // boundaries then give a contiguous range in compact region order,
        // even when the page starts or ends in the middle of a selected row.
        let selected_before = |boundary: u64| {
            let width = u64::from(image.width());
            let row = boundary / width;
            let column = boundary % width;
            let region_width = u64::from(x1 - x0);
            let rows = row.saturating_sub(u64::from(y0)).min(u64::from(y1 - y0));
            let partial = if row >= u64::from(y0) && row < u64::from(y1) {
                column.saturating_sub(u64::from(x0)).min(region_width)
            } else {
                0
            };
            (rows * region_width + partial) as u32
        };
        for page in &source.pages {
            self.check_cancelled()?;
            p.page_first = page.first_pixel;
            let mut start = selected_before(u64::from(page.first_pixel));
            let last = selected_before(u64::from(page.first_pixel) + u64::from(page.n_pixels));
            while start < last {
                self.check_cancelled()?;
                p.start = start;
                p.count = (last - start).min(CHUNK);
                self.analysis_dispatch(&page.buffer, &counts, &p, false)?;
                start += p.count;
            }
        }
        let counts: Vec<u32> = self.pipe.readback_buffer_cancellable(&counts, size)?;
        let hist = std::array::from_fn(|channel| {
            counts[channel * BINS..(channel + 1) * BINS]
                .iter()
                .map(|&n| u64::from(n))
                .collect()
        });
        let grid = || {
            counts[1024..p.column_offset as usize]
                .iter()
                .map(|&n| u64::from(n))
                .collect()
        };
        let total = u64::from(x1 - x0) * u64::from(y1 - y0);
        let offset = p.totals_offset as usize;
        Ok(AnalysisData {
            histogram: hist,
            waveform: if p.mode == 1 { grid() } else { Vec::new() },
            column_population: counts[p.column_offset as usize..offset]
                .iter()
                .map(|&n| u64::from(n))
                .collect(),
            vectorscope: if p.mode == 2 { grid() } else { Vec::new() },
            spatial_bins: p.spatial,
            total,
            finite: u64::from(counts[offset]),
            invalid: u64::from(counts[offset + 1]),
            below: u64::from(counts[offset + 2]),
            above: u64::from(counts[offset + 3]),
            outside_window: u64::from(counts[offset + 4]),
        })
    }

    pub(super) fn inspect_analysis_pixel_gpu(
        &self,
        image: &DeviceImage,
        x: u32,
        y: u32,
    ) -> Result<AnalysisPixel, String> {
        self.check_cancelled()?;
        if x >= image.width() || y >= image.height() {
            return Err("analysis pixel is outside the image".into());
        }
        let source = self.buffer_payload(image)?;
        let mut p = params(image)?;
        let absolute = y * image.width() + x;
        let page = source
            .pages
            .iter()
            .find(|page| {
                absolute >= page.first_pixel && absolute - page.first_pixel < page.n_pixels
            })
            .ok_or("analysis pixel has no retained page")?;
        p.start = absolute - page.first_pixel;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("analysis_pixel"),
            size: 28,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.analysis_dispatch(&page.buffer, &output, &p, true)?;
        let values: Vec<f32> = self.pipe.readback_buffer_cancellable(&output, 28)?;
        Ok(AnalysisPixel {
            rgb: [values[0] as f64, values[1] as f64, values[2] as f64],
            tone: values[3] as f64,
            ab: [values[4] as f64, values[5] as f64],
            valid: values[6] != 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::analysis::{analyze_host, inspect_host};
    use spektrafilm_backend::{ComputeBackend, Precision, RetainedImageMeta};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn meta(
        width: u32,
        height: u32,
        space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        domain: SampleDomain,
    ) -> RetainedImageMeta {
        RetainedImageMeta {
            width,
            height,
            space,
            domain,
            precision: Precision::host(),
            pixel_pitch_um: None,
        }
    }

    #[test]
    fn native_pages_preserve_roi_histogram_and_pixel_center_waveform() {
        let gpu = crate::test_support::adapter();
        let width = 257;
        let mut values = vec![0.0f32; width as usize * 3];
        values[3] = spektrafilm_math::colorspace::srgb_decode_f64(0.501) as f32;
        values[9..12].copy_from_slice(&[-0.1, 1.2, 0.5]);
        values[12..15].copy_from_slice(&[f32::NAN, 0.0, 0.0]);
        values[18..21].copy_from_slice(&[1.0, 1.0, 1.0]);
        values[450..453].copy_from_slice(&[0.3, 0.2, 0.1]);
        let host = ImageBuf::from_data(width, 1, f32_to_scalars(values.clone()));
        let m = meta(
            width,
            1,
            &spektrafilm_math::colorspace::SRGB,
            SampleDomain::Linear,
        );
        let pages = values
            .chunks(3 * 73)
            .enumerate()
            .map(|(i, samples)| super::super::paged::ImagePage {
                first_pixel: i as u32 * 73,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("analysis_test_page"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let retained = gpu.retain_image_pages(pages, width, 1, m.domain, m.space, None);
        for mode in [
            AnalysisMode::Histogram,
            AnalysisMode::Waveform,
            AnalysisMode::Parade,
        ] {
            let request = AnalysisRequest {
                region: [0, 0, width, 1],
                mode,
                chroma_zoom: 1,
            };
            let actual = gpu.analyze_retained(&retained, &request).unwrap();
            let expected = analyze_host(&host, m, &request, None).unwrap();
            assert_eq!(
                (
                    actual.total,
                    actual.finite,
                    actual.invalid,
                    actual.below,
                    actual.above
                ),
                (257, 256, 1, 1, 1)
            );
            assert_eq!(actual.histogram, expected.histogram);
            assert_eq!(actual.waveform, expected.waveform);
            assert_eq!(actual.column_population, expected.column_population);
            assert_eq!(actual.column_population[128], 2);
            assert_eq!(actual.histogram[1][128], 1);
        }
        let request = AnalysisRequest {
            region: [3, 0, 7, 1],
            mode: AnalysisMode::Histogram,
            chroma_zoom: 1,
        };
        let subset = gpu.analyze_retained(&retained, &request).unwrap();
        assert_eq!(
            (
                subset.total,
                subset.finite,
                subset.invalid,
                subset.below,
                subset.above
            ),
            (4, 3, 1, 1, 1)
        );
        assert!(!gpu.inspect_analysis_pixel(&retained, 4, 0).unwrap().valid);
        let inspected = gpu.inspect_analysis_pixel(&retained, 150, 0).unwrap();
        let expected = inspect_host(&host, m, 150, 0).unwrap();
        assert!(inspected.valid && (inspected.tone - expected.tone).abs() < 2e-5);
        assert!(gpu.inspect_analysis_pixel(&retained, width, 0).is_err());
    }

    #[test]
    fn multirow_regions_match_host_across_partial_pages() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (31, 9);
        let colors = [[0.7f32, 0.2, 0.1], [0.1, 0.6, 0.3], [0.2, 0.1, 0.8]];
        let mut values: Vec<f32> = (0..width * height)
            .flat_map(|i| colors[(i % 3) as usize])
            .collect();
        let invalid = ((3 * width + 8) * 3) as usize;
        values[invalid] = f32::NAN;
        let host = ImageBuf::from_data(width, height, f32_to_scalars(values.clone()));
        let m = meta(
            width,
            height,
            &spektrafilm_math::colorspace::SRGB,
            SampleDomain::Linear,
        );
        let pages = values
            .chunks(3 * 17)
            .enumerate()
            .map(|(i, samples)| super::super::paged::ImagePage {
                first_pixel: i as u32 * 17,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("analysis_test_page"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let retained = gpu.retain_image_pages(pages, width, height, m.domain, m.space, None);
        for region in [
            [0, 0, width, height],
            [8, 1, 9, 8],
            [5, 2, 25, 8],
            [16, 0, 19, 1],
            [29, 1, 31, 8],
        ] {
            for mode in [
                AnalysisMode::Histogram,
                AnalysisMode::Waveform,
                AnalysisMode::Parade,
                AnalysisMode::Vectorscope,
            ] {
                let request = AnalysisRequest {
                    region,
                    mode,
                    chroma_zoom: 2,
                };
                let actual = gpu.analyze_retained(&retained, &request).unwrap();
                let expected = analyze_host(&host, m, &request, None).unwrap();
                assert_eq!(actual.histogram, expected.histogram);
                assert_eq!(actual.waveform, expected.waveform);
                assert_eq!(actual.vectorscope, expected.vectorscope);
                assert_eq!(actual.column_population, expected.column_population);
                assert_eq!(actual.spatial_bins, expected.spatial_bins);
                assert_eq!(
                    (
                        actual.total,
                        actual.finite,
                        actual.invalid,
                        actual.below,
                        actual.above,
                        actual.outside_window,
                    ),
                    (
                        expected.total,
                        expected.finite,
                        expected.invalid,
                        expected.below,
                        expected.above,
                        expected.outside_window,
                    ),
                );
            }
        }
    }

    #[test]
    fn device_conversion_vectorscope_and_inspection_match_native_interpretation() {
        let gpu = crate::test_support::adapter();
        let host = ImageBuf::from_data(
            3,
            1,
            f32_to_scalars(vec![0.7, 0.2, 0.1, 0.1, 0.6, 0.3, 0.2, 0.1, 0.8]),
        );
        for (space, domain) in [
            (&spektrafilm_math::colorspace::SRGB, SampleDomain::Encoded),
            (
                &spektrafilm_math::colorspace::DISPLAY_P3,
                SampleDomain::Linear,
            ),
        ] {
            let m = meta(3, 1, space, domain);
            let image = gpu.retain_image(&host, &m).unwrap();
            for x in 0..3 {
                let expected = inspect_host(&host, m, x, 0).unwrap();
                let actual = gpu.inspect_analysis_pixel(&image, x, 0).unwrap();
                assert!(actual.valid);
                for c in 0..3 {
                    assert!((actual.rgb[c] - expected.rgb[c]).abs() < 2e-5);
                }
                assert!((actual.tone - expected.tone).abs() < 2e-5);
                for c in 0..2 {
                    assert!((actual.ab[c] - expected.ab[c]).abs() < 2e-5);
                }
            }
            for zoom in [1, 2, 4] {
                let request = AnalysisRequest {
                    region: [0, 0, 3, 1],
                    mode: AnalysisMode::Vectorscope,
                    chroma_zoom: zoom,
                };
                let expected = analyze_host(&host, m, &request, None).unwrap();
                let actual = gpu.analyze_retained(&image, &request).unwrap();
                assert_eq!(actual.histogram, expected.histogram);
                assert_eq!(actual.vectorscope, expected.vectorscope);
                assert_eq!(actual.outside_window, expected.outside_window);
                assert_eq!(
                    actual.vectorscope.iter().sum::<u64>() + actual.outside_window,
                    3
                );
                assert_eq!(actual.column_population, [1, 1, 1]);
            }
        }
    }

    #[test]
    fn encoded_signal_bin_centers_cover_all_256_intervals() {
        let gpu = crate::test_support::adapter();
        let values: Vec<f32> = (0..256)
            .flat_map(|i| {
                let v = (i as f32 + 0.5) / 256.0;
                [v, v, v]
            })
            .collect();
        let host = ImageBuf::from_data(256, 1, f32_to_scalars(values));
        let m = meta(
            256,
            1,
            &spektrafilm_math::colorspace::SRGB,
            SampleDomain::Encoded,
        );
        let image = gpu.retain_image(&host, &m).unwrap();
        let request = AnalysisRequest {
            region: [0, 0, 256, 1],
            mode: AnalysisMode::Histogram,
            chroma_zoom: 1,
        };
        let result = gpu.analyze_retained(&image, &request).unwrap();
        for channel in 0..4 {
            assert!(result.histogram[channel].iter().all(|&count| count == 1));
        }
        assert_eq!((result.finite, result.below, result.above), (256, 0, 0));
    }

    #[test]
    fn neutral_samples_remain_at_vectorscope_center() {
        let gpu = crate::test_support::adapter();
        let host = ImageBuf::from_data(
            3,
            1,
            f32_to_scalars(vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.25, 1.0, 1.0, 1.0]),
        );
        let m = meta(
            3,
            1,
            &spektrafilm_math::colorspace::SRGB,
            SampleDomain::Linear,
        );
        let image = gpu.retain_image(&host, &m).unwrap();
        let request = AnalysisRequest {
            region: [0, 0, 3, 1],
            mode: AnalysisMode::Vectorscope,
            chroma_zoom: 4,
        };
        let result = gpu.analyze_retained(&image, &request).unwrap();
        assert!(result.vectorscope[128 * 256 + 128] >= 1);
        let central: u64 = [127, 128]
            .into_iter()
            .flat_map(|y| [127, 128].into_iter().map(move |x| y * 256 + x))
            .map(|i| result.vectorscope[i])
            .sum();
        assert_eq!(central, 3);
        for x in 0..3 {
            let pixel = gpu.inspect_analysis_pixel(&image, x, 0).unwrap();
            assert!(pixel.valid && pixel.ab.iter().all(|v| v.abs() < 2e-6));
        }
    }

    #[test]
    fn cancelled_analysis_readback_does_not_poison_reuse() {
        let gpu = crate::test_support::explicit_adapter();
        let host = ImageBuf::from_data(1, 1, vec![0.25 as _; 3]);
        let m = meta(
            1,
            1,
            &spektrafilm_math::colorspace::SRGB,
            SampleDomain::Linear,
        );
        let image = gpu.retain_image(&host, &m).unwrap();
        let request = AnalysisRequest {
            region: [0, 0, 1, 1],
            mode: AnalysisMode::Histogram,
            chroma_zoom: 1,
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        gpu.pipe.test_on_readback({
            let cancellation = cancellation.clone();
            move || cancellation.store(true, Ordering::Relaxed)
        });
        let operation = gpu.with_cancellation(Some(cancellation)).unwrap();
        assert_eq!(
            operation.analyze_retained(&image, &request).unwrap_err(),
            "operation cancelled"
        );
        gpu.pipe.test_on_readback(|| {});
        assert_eq!(gpu.analyze_retained(&image, &request).unwrap().finite, 1);
    }
}
