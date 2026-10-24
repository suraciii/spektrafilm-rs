//! Retained RGB storage split at whole-pixel boundaries. Page offsets are
//! absolute source coordinates; copies never materialize image intermediates.
use super::film_chain::device_image::BufferPayload;
use super::*;

#[derive(Clone)]
pub(in crate::wgpu_backend) struct ImagePage {
    pub first_pixel: u32,
    pub n_pixels: u32,
    pub buffer: wgpu::Buffer,
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn allocate_image_pages(
        &self,
        width: u32,
        height: u32,
        label: &str,
    ) -> Result<Vec<ImagePage>, String> {
        let count = u64::from(width) * u64::from(height);
        if count > u64::from(u32::MAX) {
            return Err("retained image exceeds device pixel indexing".into());
        }
        let capacity =
            super::bounded::chunk_elements(super::bounded::storage_binding_limit(self)?, 12)?
                as u32;
        let mut pages = Vec::new();
        let mut first = 0;
        while first < count as u32 {
            self.check_cancelled()?;
            let n_pixels = capacity.min(count as u32 - first);
            pages.push(ImagePage {
                first_pixel: first,
                n_pixels,
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: u64::from(n_pixels) * 12,
                    mapped_at_creation: false,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                }),
            });
            first += n_pixels;
        }
        Ok(pages)
    }

    /// Copy an arbitrary contiguous range of RGB samples into a bounded buffer.
    pub(in crate::wgpu_backend) fn copy_samples_to(
        &self,
        encoder: &mut ObservedEncoder,
        source: &BufferPayload,
        first_sample: u64,
        sample_count: u64,
        destination: &wgpu::Buffer,
        destination_sample: u64,
    ) -> Result<(), String> {
        if first_sample
            .checked_add(sample_count)
            .is_none_or(|end| end > source.bytes / 4)
            || destination_sample
                .checked_add(sample_count)
                .is_none_or(|end| end > destination.size() / 4)
        {
            return Err("retained sample copy is outside its source or destination".into());
        }
        copy_intersections(
            encoder,
            &source.pages,
            first_sample,
            destination,
            destination_sample,
            sample_count,
            false,
        );
        Ok(())
    }

    /// Scatter a bounded result into absolute output-page sample coordinates.
    pub(in crate::wgpu_backend) fn copy_samples_from(
        &self,
        encoder: &mut ObservedEncoder,
        source: &wgpu::Buffer,
        source_sample: u64,
        destination: &[ImagePage],
        destination_sample: u64,
        sample_count: u64,
    ) -> Result<(), String> {
        let end = destination
            .last()
            .map_or(0, |page| u64::from(page.first_pixel + page.n_pixels) * 3);
        if source_sample
            .checked_add(sample_count)
            .is_none_or(|end| end > source.size() / 4)
            || destination_sample
                .checked_add(sample_count)
                .is_none_or(|value| value > end)
        {
            return Err("retained sample scatter is outside its source or destination".into());
        }
        copy_intersections(
            encoder,
            destination,
            destination_sample,
            source,
            source_sample,
            sample_count,
            true,
        );
        Ok(())
    }

    /// Extract a packed rectangle while keeping its source offsets absolute.
    // Keep the copy endpoints and rectangle explicit at this transport boundary.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_backend) fn copy_window_to(
        &self,
        encoder: &mut ObservedEncoder,
        source: &BufferPayload,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        destination: &wgpu::Buffer,
    ) -> Result<(), String> {
        if x.checked_add(width).is_none_or(|end| end > source.width)
            || y.checked_add(height).is_none_or(|end| end > source.height)
            || u64::from(width) * u64::from(height) * 12 > destination.size()
        {
            return Err("retained window is outside its source or destination".into());
        }
        for row in 0..height {
            self.copy_samples_to(
                encoder,
                source,
                (u64::from(y + row) * u64::from(source.width) + u64::from(x)) * 3,
                u64::from(width) * 3,
                destination,
                u64::from(row) * u64::from(width) * 3,
            )?;
        }
        Ok(())
    }
}

fn copy_intersections(
    encoder: &mut ObservedEncoder,
    pages: &[ImagePage],
    first: u64,
    contiguous: &wgpu::Buffer,
    contiguous_first: u64,
    count: u64,
    scatter: bool,
) {
    let end = first + count;
    for page in pages {
        let page_first = u64::from(page.first_pixel) * 3;
        let begin = first.max(page_first);
        let finish = end.min(page_first + u64::from(page.n_pixels) * 3);
        if begin >= finish {
            continue;
        }
        let page_offset = (begin - page_first) * 4;
        let buffer_offset = (contiguous_first + begin - first) * 4;
        let bytes = (finish - begin) * 4;
        if scatter {
            encoder.copy_buffer_to_buffer(
                contiguous,
                buffer_offset,
                &page.buffer,
                page_offset,
                bytes,
            );
        } else {
            encoder.copy_buffer_to_buffer(
                &page.buffer,
                page_offset,
                contiguous,
                buffer_offset,
                bytes,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_pages_copy_unaligned_samples_and_cross_row_windows() {
        let gpu = crate::test_support::adapter();
        let values: Vec<f32> = (0..5 * 3 * 3).map(|i| i as f32 * 0.125).collect();
        let pages: Vec<_> = values
            .chunks(12)
            .enumerate()
            .map(|(i, samples)| ImagePage {
                first_pixel: i as u32 * 4,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("retained_page_fixture"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                    }),
            })
            .collect();
        let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
        let image = gpu.retain_image_pages(
            pages,
            5,
            3,
            spektrafilm_backend::SampleDomain::Linear,
            space,
            None,
        );
        let payload = gpu.buffer_payload(&image).unwrap();
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("retained_window_fixture"),
            size: 3 * 2 * 12,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.copy_window_to(&mut encoder, payload, 1, 1, 3, 2, &output)
            .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.pipe.readback_buffer(&output, 72);
        let expected: Vec<_> = values[18..27]
            .iter()
            .chain(&values[33..42])
            .copied()
            .collect();
        assert_eq!(actual, expected);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        // Start/end are inside RGB pixels; both copies cross physical pages.
        gpu.copy_samples_to(&mut encoder, payload, 2, 17, &output, 1)
            .unwrap();
        gpu.copy_samples_from(&mut encoder, &output, 1, &payload.pages, 23, 17)
            .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        let result = gpu.materialize_buffer(&image).unwrap();
        let mut expected = values.clone();
        expected[23..40].copy_from_slice(&values[2..19]);
        assert_eq!(scalars_to_f32(&result.data), expected);
    }
}
