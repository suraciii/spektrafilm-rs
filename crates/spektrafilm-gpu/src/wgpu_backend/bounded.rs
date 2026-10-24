//! Bounded storage partitions for pointwise device work. Partitions change
//! ownership and dispatch sizes only; numerical coordinates stay with callers.
use super::*;

pub(super) fn storage_binding_limit(backend: &WgpuBackend) -> Result<u64, String> {
    let limits = backend.device.limits();
    let physical = limits.max_buffer_size.min(u64::from(limits.max_storage_buffer_binding_size));
    match std::env::var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES") {
        Ok(value) => {
            let requested = value.parse::<u64>().map_err(|_| {
                "SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES must be a positive byte count".to_owned()
            })?;
            if requested == 0 {
                return Err("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES must be positive".to_owned());
            }
            Ok(physical.min(requested))
        }
        Err(std::env::VarError::NotPresent) => Ok(physical),
        Err(error) => Err(format!("invalid GPU binding limit: {error}")),
    }
}

pub(super) fn chunk_elements(limit: u64, bytes_per_element: u64) -> Result<usize, String> {
    let count = (limit / bytes_per_element).min(u64::from(u32::MAX));
    if count == 0 {
        return Err(format!("GPU minimum working set needs {bytes_per_element} bytes per binding; limit is {limit}"));
    }
    Ok(count as usize)
}

/// A coefficient lattice split without evaluating or reordering its samples.
/// Global element offsets, not rounded image rectangles, identify each page.
pub(super) struct DevicePage {
    pub first: u32,
    pub len: u32,
    pub buffer: wgpu::Buffer,
}

impl WgpuBackend {
    pub(super) fn upload_pages_with_limit(&self, values: &[f32], label: &str, limit: u64) -> Result<Vec<DevicePage>, String> {
        let elements = chunk_elements(limit, 4)?;
        if values.len() > u32::MAX as usize {
            return Err(format!("{label}: lattice exceeds device u32 indexing"));
        }
        Ok(values.chunks(elements).enumerate().map(|(page, data)| DevicePage {
            first: (page * elements) as u32,
            len: data.len() as u32,
            buffer: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE,
            }),
        }).collect())
    }

    /// Pointwise RGB dispatch with bounded uploads and one terminal result per
    /// chunk. Coefficients are image-independent and each must fit one binding.
    pub(super) fn dispatch_rgb_chunks<P: bytemuck::Pod>(
        &self,
        image: &ImageBuf,
        shader: &'static str,
        coefficients: &[&[f32]],
        params: impl Fn(u32) -> P,
        name: &'static str,
    ) -> Result<ImageBuf, String> {
        let limit = storage_binding_limit(self)?;
        let pixels = chunk_elements(limit, 12)?;
        for coefficient in coefficients {
            if std::mem::size_of_val(*coefficient) as u64 > limit {
                return Err(format!("{name}: minimum coefficient binding exceeds {limit} bytes"));
            }
        }
        let mut output = Vec::with_capacity(image.data.len());
        for samples in image.data.chunks(pixels * 3) {
            let input = scalars_to_f32(samples);
            let n = (samples.len() / 3) as u32;
            let uniform = params(n);
            let mut bindings = Vec::with_capacity(coefficients.len() + 3);
            bindings.push(GpuBuffer::uniform(bytemuck::bytes_of(&uniform)));
            bindings.push(GpuBuffer::storage_ro(bytemuck::cast_slice(&input)));
            for coefficient in coefficients {
                bindings.push(GpuBuffer::storage_ro(bytemuck::cast_slice(coefficient)));
            }
            bindings.push(GpuBuffer::storage_rw(&vec![0; input.len() * 4]));
            let result = self.dispatch_compute(shader, &bindings, n, bindings.len() - 1, name)
                .ok_or_else(|| format!("{name}: bounded device dispatch could not represent its working set"))?;
            output.extend(f32_to_scalars(result));
        }
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(ImageBuf::from_data(image.width, image.height, output))
    }
}

#[cfg(test)]
mod tests {
    use super::chunk_elements;

    #[test]
    fn partition_keeps_last_pixel_when_limit_is_not_rgb_aligned() {
        let pixels = chunk_elements(101, 12).unwrap();
        assert_eq!(pixels, 8);
        let lengths: Vec<_> = (0..35).collect::<Vec<_>>().chunks(pixels).map(<[_]>::len).collect();
        assert_eq!(lengths, [8, 8, 8, 8, 3]);
        assert_eq!(lengths.iter().sum::<usize>(), 35);
        assert!(chunk_elements(11, 12).is_err());
    }
}
