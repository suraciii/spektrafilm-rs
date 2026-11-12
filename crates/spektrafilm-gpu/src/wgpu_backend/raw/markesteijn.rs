//! Parallel Markesteijn windows. Each dispatch reads an immutable set of
//! direction planes; queue order establishes visibility at stage boundaries.
use super::*;

const SHADER: &str = include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_markesteijn.wgsl");
const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const UNIFORM: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;

impl WgpuBackend {
    pub(super) fn raw_markesteijn_gpu(
        &self,
        frame: &RawFrame,
        image: &RawBuffer,
        cfa: &RawBuffer,
    ) -> Result<(Vec<RawDispatch>, RawBuffer), String> {
        self.check_cancelled()?;
        if frame.width == 0 || frame.height == 0 {
            return Err("Markesteijn requires nonempty dimensions".into());
        }
        if frame.cfa_rows != 6 || frame.cfa_columns != 6 {
            return Err("Markesteijn requires a 6-by-6 X-Trans CFA".into());
        }
        let n = u64::from(frame.width) * u64::from(frame.height);
        let bytes = n * 8 * 16;
        if let Some(reason) = required_buffer_exceeds_limits(bytes, &self.device.limits()) {
            return Err(format!("Markesteijn direction planes: {reason}"));
        }
        let planes = [
            RawBuffer::zeros(&self.device, "raw_markesteijn_a", bytes),
            RawBuffer::zeros(&self.device, "raw_markesteijn_b", bytes),
        ];
        let aux = RawBuffer::zeros(&self.device, "raw_markesteijn_aux", n * 32);
        let scores = RawBuffer::zeros(&self.device, "raw_markesteijn_scores", n * 24 * 4);
        let metadata = RawBuffer::zeros(&self.device, "raw_markesteijn_tables", 147 * 4);
        let mut passes = Vec::new();
        let mut source = 0;
        let mut add = |stage: u32, pass: u32, direction: u32, swap: bool| {
            let params = RawBuffer::upload(
                &self.device,
                "raw_markesteijn_params",
                bytemuck::cast_slice(&[frame.width, frame.height, stage, pass, direction, 0, 0, 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            passes.push(self.raw_dispatch(
                SHADER,
                &[
                    &params,
                    image,
                    cfa,
                    &metadata,
                    &planes[source],
                    &planes[source ^ 1],
                    &aux,
                    &scores,
                ],
                &[UNIFORM, WRITE, READ, WRITE, READ, WRITE, WRITE, WRITE],
                if stage == 0 { 1 } else { n as u32 },
                "raw_markesteijn",
            ));
            if swap {
                source ^= 1;
            }
        };
        // Stage IDs mirror raw_markesteijn.wgsl: tables, copy, bounds, green;
        // copy refined directions, recalc, solitary, opposite chroma, 2x2;
        // derivatives, shared threshold/votes, sums, final accumulation.
        add(0, 0, 0, false);
        add(1, 0, 0, true);
        add(2, 0, 0, false);
        add(3, 0, 0, true);
        for pass in 0..3 {
            if pass == 1 {
                add(4, pass, 0, true);
            }
            if pass > 0 {
                add(5, pass, 0, true);
            }
            for direction in 0..6 {
                add(6, pass, direction, true);
            }
            add(7, pass, 0, true);
            add(8, pass, 0, true);
        }
        for stage in 9..=12 {
            add(stage, 0, 0, false);
        }
        Ok((passes, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_markesteijn_rejects_bad_cfa_and_cancelled_work() {
        use std::sync::{Arc, atomic::AtomicBool};
        let gpu = crate::test_support::adapter();
        let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
        frame.width = 40;
        frame.height = 40;
        frame.cfa_rows = 6;
        frame.cfa_columns = 6;
        let image = RawBuffer::zeros(&gpu.device, "bad_xtrans_image", 40 * 40 * 16);
        let cfa = RawBuffer::zeros(&gpu.device, "bad_xtrans_cfa", 36 * 4);
        let (passes, status) = gpu.raw_markesteijn_gpu(&frame, &image, &cfa).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for pass in passes {
            pass.encode(&mut encoder, "bad_xtrans");
        }
        gpu.queue.submit(Some(encoder.finish()));
        let status = gpu
            .pipe
            .readback_buffer_as::<i32>(&status.buffer, 147 * 4)
            .unwrap();
        assert_ne!(status[146], 0);
        let mut cancelled = gpu.derived_explicit();
        let signal = Arc::new(AtomicBool::new(true));
        cancelled.pipe = cancelled.pipe.with_cancellation(Some(signal.clone()));
        cancelled.cancellation = Some(signal);
        assert_eq!(
            cancelled
                .raw_markesteijn_gpu(&frame, &image, &cfa)
                .err()
                .unwrap(),
            "operation cancelled"
        );
        frame.cfa_rows = 2;
        assert!(gpu.raw_markesteijn_gpu(&frame, &image, &cfa).is_err());
    }

    #[test]
    fn parallel_markesteijn_matches_scalar_interior() {
        let gpu = crate::test_support::adapter();
        let pattern: [u8; 36] = [
            1, 0, 1, 1, 2, 1, 2, 1, 2, 0, 1, 0, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 0, 1, 0, 2, 1,
            2, 1, 2, 1, 1, 0, 1,
        ];
        let size = 64usize;
        for shift in 0..36 {
            let pattern: Vec<u8> = (0..36)
                .map(|i| pattern[((i / 6 + shift / 6) % 6) * 6 + (i % 6 + shift % 6) % 6])
                .collect();
            let tables = super::super::xtrans_reference::Tables::new(&pattern, 6, 6);
            for fixture in 0..4 {
                let mut input = vec![0u32; size * size * 4];
                for y in 0..size {
                    for x in 0..size {
                        let c = pattern[(y % 6) * 6 + x % 6] as usize;
                        input[(y * size + x) * 4 + c] = [1000, 2000, 3000][c]
                            + match fixture {
                                0 => 0,
                                1 => (x * 2 + y * 3) as u32,
                                2 => {
                                    if x * 2 + y > size {
                                        7000
                                    } else {
                                        0
                                    }
                                }
                                _ => ((x * 997 + y * 1731 + x * y * 103 + c * 193) % 50000) as u32,
                            };
                    }
                }
                let floating: Vec<f32> = input.iter().map(|v| *v as f32 / 65536.0).collect();
                let expected = super::super::xtrans_reference::demosaic_xtrans(
                    &pattern, 6, 6, &floating, size, size, 3, &tables,
                );
                let codes: Vec<u32> = pattern.iter().map(|v| u32::from(*v)).collect();
                let cfa = RawBuffer::upload(
                    &gpu.device,
                    "markesteijn_test_cfa",
                    bytemuck::cast_slice(&codes),
                    wgpu::BufferUsages::STORAGE,
                );
                let image = RawBuffer::upload(
                    &gpu.device,
                    "markesteijn_test_image",
                    bytemuck::cast_slice(&input),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                );
                let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
                frame.width = size as u32;
                frame.height = size as u32;
                frame.cfa_rows = 6;
                frame.cfa_columns = 6;
                let (passes, status) = gpu.raw_markesteijn_gpu(&frame, &image, &cfa).unwrap();
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                for pass in passes {
                    pass.encode(&mut encoder, "markesteijn_test");
                }
                gpu.queue.submit(Some(encoder.finish()));
                let status = gpu
                    .pipe
                    .readback_buffer_as::<i32>(&status.buffer, 147 * 4)
                    .unwrap();
                assert_eq!(status[146], 0, "phase {shift}");
                let actual = gpu
                    .pipe
                    .readback_buffer_as::<u32>(&image.buffer, (size * size * 16) as u64)
                    .unwrap();
                for y in 17..size - 17 {
                    for x in 17..size - 17 {
                        for c in 0..3 {
                            let i = (y * size + x) * 4 + c;
                            assert!(
                                (actual[i] as f32 - (expected[i] * 65536.0).clamp(0.0, 65535.0))
                                    .abs()
                                    <= 1.0,
                                "phase={shift} fixture={fixture} ({x},{y},{c}) {} != {}",
                                actual[i],
                                expected[i] * 65536.0
                            );
                        }
                    }
                }
            }
        }
    }
}
