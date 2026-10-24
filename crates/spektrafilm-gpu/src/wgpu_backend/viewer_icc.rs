//! Reused prepared ICC resources and one fixed RGB8 application kernel.
use super::*;
use spektrafilm_backend::{DeviceImage, SampleDomain, ViewerIccGpuParams, ViewerIccKind};

const MAX_PREPARED_ENTRIES: usize = 16;
const MAX_PREPARED_BYTES: usize = 96 * 1024 * 1024;

#[derive(Default)]
pub(super) struct IccDeviceCache {
    entries: Vec<([u8; 32], ViewerIccKind, usize, wgpu::Buffer)>,
}
impl IccDeviceCache {
    fn get(&mut self, device: &ObservedDevice, p: &ViewerIccGpuParams<'_>) -> wgpu::Buffer {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.0 == p.identity && entry.1 == p.kind)
        {
            let entry = self.entries.remove(index);
            let buffer = entry.3.clone();
            self.entries.insert(0, entry);
            return buffer;
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("icc_prepared_data"),
            contents: bytemuck::cast_slice(p.words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.entries
            .insert(0, (p.identity, p.kind, p.words.len() * 4, buffer.clone()));
        while self.entries.len() > MAX_PREPARED_ENTRIES
            || self.entries.iter().map(|entry| entry.2).sum::<usize>() > MAX_PREPARED_BYTES
        {
            self.entries.pop();
        }
        buffer
    }
}

impl WgpuBackend {
    pub(super) fn viewer_icc_gpu(
        &self,
        image: &DeviceImage,
        p: &ViewerIccGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if p.words.len() != p.kind.word_count() {
            return Err("viewer ICC prepared-data length is invalid".into());
        }
        let bytes = std::mem::size_of_val(p.words) as u64;
        if bytes > super::bounded::storage_binding_limit(self)? {
            return Err("viewer ICC prepared data exceeds device binding limits".into());
        }
        let source = self.buffer_payload(image)?;
        let data = self.pipe.viewer_icc_cache.lock().get(&self.device, p);
        let mut output = Vec::with_capacity(source.pages.len());
        for page in &source.pages {
            self.check_cancelled()?;
            let buffer = self.viewer_dispatch(
                include_str!("viewer_icc_apply.wgsl"),
                &[page.n_pixels, p.kind as u32, 0, 0],
                &[&page.buffer, &data],
                page.n_pixels,
                "viewer_icc",
            )?;
            output.push(super::paged::ImagePage {
                first_pixel: page.first_pixel,
                n_pixels: page.n_pixels,
                buffer,
            });
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            output,
            image.width(),
            image.height(),
            SampleDomain::Encoded,
            image.space(),
            image.pixel_pitch_um(),
        ))
    }

    pub(super) fn viewer_icc_samples_gpu(
        &self,
        rgb: &[[f32; 3]],
        p: &ViewerIccGpuParams<'_>,
    ) -> Result<Vec<[f32; 3]>, String> {
        if rgb.is_empty() {
            return Ok(Vec::new());
        }
        let width =
            u32::try_from(rgb.len()).map_err(|_| "ICC sample count exceeds device index range")?;
        let image = ImageBuf::from_data(
            width,
            1,
            f32_to_scalars(rgb.iter().flatten().copied().collect()),
        );
        let source = self.upload_retained(
            &image,
            &spektrafilm_backend::RetainedImageMeta {
                width,
                height: 1,
                domain: SampleDomain::Encoded,
                precision: spektrafilm_backend::Precision::F32,
                space: spektrafilm_math::colorspace::resolve("sRGB")?,
                pixel_pitch_um: None,
            },
        )?;
        let transformed = self.viewer_icc_gpu(&source, p)?;
        let samples = self.materialize_buffer(&transformed)?;
        Ok(samples
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_icc_quantization_cache_reuse_and_eviction() {
        let backend = crate::test_support::adapter();
        let input = [
            [f32::NAN, 0.5, f32::INFINITY],
            [f32::NEG_INFINITY, -0.1, 1.1],
            [0.25, 0.75, f32::NAN],
        ];
        let expected: Vec<_> = input
            .iter()
            .map(|pixel| pixel.map(|value| (value.clamp(0.0, 1.0) * 255.0) as u8))
            .collect();
        let evaluate = |params: &ViewerIccGpuParams<'_>| {
            backend
                .viewer_icc_samples_gpu(&input, params)
                .unwrap()
                .into_iter()
                .map(|pixel| pixel.map(|value| (value * 255.0).round() as u8))
                .collect::<Vec<_>>()
        };
        let mut words = Vec::with_capacity(ViewerIccKind::Curves.word_count());
        for _ in 0..3 {
            for first in (0..256).step_by(4) {
                words.push(u32::from_le_bytes(std::array::from_fn(|i| {
                    (first + i) as u8
                })));
            }
        }
        for key in 0..MAX_PREPARED_ENTRIES as u8 {
            let params = ViewerIccGpuParams {
                identity: [key; 32],
                kind: ViewerIccKind::Curves,
                words: &words,
            };
            assert_eq!(evaluate(&params), expected);
        }
        let hot = ViewerIccGpuParams {
            identity: [0; 32],
            kind: ViewerIccKind::Curves,
            words: &words,
        };
        assert_eq!(evaluate(&hot), expected);
        let next = ViewerIccGpuParams {
            identity: [99; 32],
            kind: ViewerIccKind::Curves,
            words: &words,
        };
        assert_eq!(evaluate(&next), expected);
        let cache = backend.pipe.viewer_icc_cache.lock();
        assert_eq!(cache.entries.len(), MAX_PREPARED_ENTRIES);
        assert!(cache.entries.iter().any(|entry| entry.0 == [0; 32]));
        assert!(!cache.entries.iter().any(|entry| entry.0 == [1; 32]));
        assert!(cache.entries.iter().map(|entry| entry.2).sum::<usize>() <= MAX_PREPARED_BYTES);
        drop(cache);
        let invalid = ViewerIccGpuParams {
            words: &words[..1],
            ..hot
        };
        assert!(
            backend
                .viewer_icc_samples_gpu(&input, &invalid)
                .unwrap_err()
                .contains("length")
        );
        // Three distinct exhaustive maps must evict by bytes even though the
        // number of maps is far below the entry-count limit. Reuse host words.
        let exact = vec![0; ViewerIccKind::ExactRgb8.word_count()];
        for key in 200..203 {
            let params = ViewerIccGpuParams {
                identity: [key; 32],
                kind: ViewerIccKind::ExactRgb8,
                words: &exact,
            };
            assert_eq!(evaluate(&params), vec![[0; 3]; input.len()]);
        }
        let cache = backend.pipe.viewer_icc_cache.lock();
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(
            cache.entries.iter().map(|entry| entry.2).sum::<usize>(),
            MAX_PREPARED_BYTES
        );
        assert!(!cache.entries.iter().any(|entry| entry.0 == [200; 32]));
    }
}
