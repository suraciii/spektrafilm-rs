//! Device-owned `convertFloatToInt` followed by `adjust_bl` for float RAW.
//!
//! LibRaw's integer unpack route already exports folded black metadata. Float
//! unpack must instead preserve the unfurled values: scaling a folded sum would
//! move the native unsigned truncation across an addition. A single bounded
//! metadata invocation prepares the same controls for either storage layout;
//! the page ingress converts samples inline without a second image allocation.
use super::*;
use spektrafilm_backend::RawSensorLayout;

const U: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;
const R: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const W: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const PREPARE: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_float_helpers.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_float_prepare.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FloatMetadata {
    geometry: [u32; 4],
    levels: [u32; 4],
    channels: [u32; 4],
    white: [f32; 4],
    control: [f32; 4],
}

pub(super) struct RawFloatPlan {
    pub(super) black: RawBuffer,
    pub(super) repeat: RawBuffer,
    pub(super) scale_params: RawBuffer,
    pub(super) conversion: RawBuffer,
}

impl WgpuBackend {
    /// Prepare only bounded metadata; numerical values stay on the device.
    pub(super) fn prepare_raw_float_gpu(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
    ) -> Result<RawFloatPlan, String> {
        self.check_cancelled()?;
        let map_count = u64::from(sensor.black_map_rows) * u64::from(sensor.black_map_columns);
        // cblack has LIBRAW_CBLACK_SIZE=4104 entries, six preceding the map.
        if map_count != sensor.black_map.len() as u64 || map_count > 4098 {
            return Err("RAW repeating black metadata exceeds LibRaw's cblack bounds".into());
        }
        let floating = matches!(
            sensor.layout,
            RawSensorLayout::FloatCfa | RawSensorLayout::Float3 | RawSensorLayout::Float4
        );
        let metadata = FloatMetadata {
            geometry: [
                sensor.filters,
                sensor.black_map_rows,
                sensor.black_map_columns,
                u32::from(floating),
            ],
            levels: [
                sensor.black,
                sensor.maximum,
                u32::from(sensor.fixed_maximum),
                u32::from(params.auto_white_balance),
            ],
            channels: sensor.channel_black,
            white: params.white_balance,
            control: [
                sensor.float_maximum,
                params.adjust_maximum_threshold,
                0.0,
                0.0,
            ],
        };
        let metadata = RawBuffer::upload(
            &self.device,
            "raw_float_metadata",
            bytemuck::bytes_of(&metadata),
            wgpu::BufferUsages::UNIFORM,
        );
        let empty_map = [0u32];
        let source_repeat = RawBuffer::upload(
            &self.device,
            "raw_unfurled_black_map",
            bytemuck::cast_slice(if sensor.black_map.is_empty() {
                &empty_map
            } else {
                &sensor.black_map
            }),
            wgpu::BufferUsages::STORAGE,
        );
        let plan = RawFloatPlan {
            black: RawBuffer::zeros(&self.device, "raw_normalized_channel_black", 16),
            repeat: RawBuffer::zeros(
                &self.device,
                "raw_normalized_black_map",
                map_count.max(1) * 4,
            ),
            scale_params: RawBuffer {
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("raw_normalized_scale_params"),
                    size: 32,
                    mapped_at_creation: false,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::UNIFORM
                        | wgpu::BufferUsages::COPY_SRC,
                }),
            },
            conversion: RawBuffer::zeros(&self.device, "raw_float_conversion", 16),
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.raw_dispatch(
            PREPARE,
            &[
                &metadata,
                &source_repeat,
                &plan.black,
                &plan.repeat,
                &plan.scale_params,
                &plan.conversion,
            ],
            &[U, R, W, W, W, W],
            1,
            "raw_float_prepare",
        )
        .encode(&mut encoder, "raw_float_prepare");
        self.raw_submit(encoder)?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare(
        backend: &WgpuBackend,
        metadata: FloatMetadata,
        map: &[u32],
    ) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
        let meta = RawBuffer::upload(
            &backend.device,
            "float_test_metadata",
            bytemuck::bytes_of(&metadata),
            wgpu::BufferUsages::UNIFORM,
        );
        let source = RawBuffer::upload(
            &backend.device,
            "float_test_map",
            bytemuck::cast_slice(map),
            wgpu::BufferUsages::STORAGE,
        );
        let black = RawBuffer::zeros(&backend.device, "float_test_black", 16);
        let repeat = RawBuffer::zeros(&backend.device, "float_test_repeat", map.len() as u64 * 4);
        let scale = RawBuffer::zeros(&backend.device, "float_test_scale", 32);
        let conversion = RawBuffer::zeros(&backend.device, "float_test_conversion", 16);
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        backend
            .raw_dispatch(
                PREPARE,
                &[&meta, &source, &black, &repeat, &scale, &conversion],
                &[U, R, W, W, W, W],
                1,
                "float_test",
            )
            .encode(&mut encoder, "float_test");
        backend.queue.submit(Some(encoder.finish()));
        (
            backend
                .pipe
                .readback_buffer_as::<u32>(&black.buffer, 16)
                .unwrap(),
            backend
                .pipe
                .readback_buffer_as::<u32>(&repeat.buffer, map.len() as u64 * 4)
                .unwrap(),
            backend
                .pipe
                .readback_buffer_as::<u32>(&scale.buffer, 32)
                .unwrap(),
            backend
                .pipe
                .readback_buffer_as::<u32>(&conversion.buffer, 16)
                .unwrap(),
        )
    }

    fn metadata(maximum: u32, float_maximum: f32) -> FloatMetadata {
        FloatMetadata {
            geometry: [0x94949494, 2, 2, 1],
            levels: [3, maximum, 1, 0],
            channels: [1, 3, 5, 7],
            white: [2.0, 1.0, 1.5, 1.0],
            control: [float_maximum, 0.75, 0.0, 0.0],
        }
    }

    #[test]
    fn float_black_truncates_before_native_bayer_fold() {
        let backend = crate::test_support::adapter();
        let (black, repeat, scale, conversion) =
            prepare(&backend, metadata(32766, 32768.0), &[1, 3, 5, 7]);
        // multiplier=16383/32768. Individual truncations are deliberately
        // different from scaling black+channel+repeat as a single sum.
        assert_eq!(black, [1, 3, 6, 6]);
        assert_eq!(repeat, [0, 0, 0, 0]);
        assert_eq!(scale[4], 16382);
        assert_eq!(conversion, [(16383.0f32 / 32768.0).to_bits(), 1, 1, 16383]);
        assert_eq!(
            &scale[..4],
            &[
                2.0f32.to_bits(),
                1.0f32.to_bits(),
                1.5f32.to_bits(),
                1.0f32.to_bits()
            ]
        );
    }

    #[test]
    fn float_scaling_thresholds_are_inclusive_and_metadata_participates() {
        let backend = crate::test_support::adapter();
        for maximum in [4096u32, 32767] {
            let (black, _, scale, conversion) =
                prepare(&backend, metadata(maximum, 1.0), &[1, 3, 5, 7]);
            assert_eq!(black, [5, 9, 15, 15]);
            assert_eq!(scale[4], maximum - 5);
            assert_eq!(conversion, [1.0f32.to_bits(), 1, 5, maximum]);
        }
        let (_, _, _, conversion) = prepare(&backend, metadata(1, 1.0), &[1, 3, 5, 7]);
        assert_eq!(conversion[0], 16383.0f32.to_bits());
        assert_eq!(conversion[3], 16383);
    }

    #[test]
    fn float_repeating_map_moves_common_part_after_scaling() {
        let backend = crate::test_support::adapter();
        let mut meta = metadata(32768, 1.0);
        meta.geometry[1] = 3;
        let (black, repeat, scale, conversion) = prepare(&backend, meta, &[3, 5, 7, 9, 11, 13]);
        assert_eq!(black, [2, 3, 4, 5]);
        assert_eq!(repeat, [0, 1, 2, 3, 4, 5]);
        assert_eq!(scale[4], 16381);
        assert_eq!(conversion[2], 2);
    }

    #[test]
    fn float_samples_use_native_truncation_not_rounding() {
        let backend = crate::test_support::adapter();
        let input = [-1.0f32, 0.0, 1.0, 2.0, 3.0, 32768.0];
        let values = RawBuffer::upload(
            &backend.device,
            "float_sample_input",
            bytemuck::cast_slice(&input),
            wgpu::BufferUsages::STORAGE,
        );
        let words = [(16383.0f32 / 32768.0).to_bits(), 1u32, 0, 16383];
        let conversion = RawBuffer::upload(
            &backend.device,
            "float_sample_conversion",
            bytemuck::cast_slice(&words),
            wgpu::BufferUsages::STORAGE,
        );
        let output = RawBuffer::zeros(
            &backend.device,
            "float_sample_output",
            input.len() as u64 * 4,
        );
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        backend
            .raw_dispatch(
                concat!(
                    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_float_helpers.wgsl"),
                    include_str!(
                        "../../../../spektrafilm-shaders/wgsl/raw/raw_float_samples_probe.wgsl"
                    )
                ),
                &[&values, &conversion, &output],
                &[R, R, W],
                input.len() as u32,
                "float_sample_probe",
            )
            .encode(&mut encoder, "float_sample_probe");
        backend.queue.submit(Some(encoder.finish()));
        assert_eq!(
            backend
                .pipe
                .readback_buffer_as::<u32>(&output.buffer, input.len() as u64 * 4)
                .unwrap(),
            [0, 0, 0, 0, 1, 16383]
        );
    }

    #[test]
    fn integer_metadata_is_not_folded_twice() {
        let backend = crate::test_support::adapter();
        let mut meta = metadata(65535, 0.0);
        meta.geometry[3] = 0;
        meta.channels = [5, 9, 13, 17];
        meta.levels[0] = 5;
        let (black, repeat, scale, conversion) = prepare(&backend, meta, &[0, 1, 2, 3]);
        assert_eq!(black, [5, 9, 13, 17]);
        assert_eq!(repeat, [0, 1, 2, 3]);
        assert_eq!(scale[4], 65530);
        assert_eq!(conversion, [1.0f32.to_bits(), 0, 5, 65535]);
    }
}
