//! Four-channel integer RAW storage. Copies preserve full-image coordinates;
//! only bounded windows are exposed to numerical storage bindings.
use super::*;

pub(super) struct RawPage {
    pub first_pixel: u32,
    pub n_pixels: u32,
    pub buffer: wgpu::Buffer,
}
pub(super) struct RawPlane {
    pub width: u32,
    pub height: u32,
    pub pages: Vec<RawPage>,
}
impl WgpuBackend {
    pub(super) fn raw_allocate_plane(
        &self,
        width: u32,
        height: u32,
        label: &str,
    ) -> Result<RawPlane, String> {
        self.check_cancelled()?;
        let count = u64::from(width) * u64::from(height);
        if count > u64::from(u32::MAX) {
            return Err("RAW image exceeds device pixel indexing".into());
        }
        let limit = super::super::bounded::storage_binding_limit(self)?;
        let capacity = (limit / 16).min(u64::from(u32::MAX)) as u32;
        if capacity == 0 {
            return Err("Device cannot bind one RAW pixel".into());
        }
        let mut pages = Vec::new();
        let mut first = 0u32;
        while first < count as u32 {
            self.check_cancelled()?;
            let n_pixels = capacity.min(count as u32 - first);
            pages.push(RawPage {
                first_pixel: first,
                n_pixels,
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: u64::from(n_pixels) * 16,
                    mapped_at_creation: false,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                }),
            });
            first += n_pixels;
        }
        self.check_cancelled()?;
        Ok(RawPlane {
            width,
            height,
            pages,
        })
    }
}
impl RawPlane {
    pub(super) fn copy_window_to(
        &self,
        encoder: &mut ObservedEncoder,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dest: &wgpu::Buffer,
    ) -> Result<(), String> {
        self.check(x, y, w, h)?;
        if u64::from(w) * u64::from(h) * 16 > dest.size() {
            return Err("RAW window exceeds destination allocation".into());
        }
        for row in 0..h {
            self.copy_range(
                encoder,
                u64::from(y + row) * u64::from(self.width) + u64::from(x),
                w,
                dest,
                u64::from(row) * u64::from(w),
                false,
            );
        }
        Ok(())
    }
    // Source and destination windows have independent origins and source row stride.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_window_from(
        &self,
        encoder: &mut ObservedEncoder,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        source: &wgpu::Buffer,
        source_width: u32,
        source_x: u32,
        source_y: u32,
    ) -> Result<(), String> {
        self.check(x, y, w, h)?;
        if source_x.checked_add(w).is_none_or(|end| end > source_width)
            || (u64::from(source_y + h - 1) * u64::from(source_width) + u64::from(source_x + w))
                * 16
                > source.size()
        {
            return Err("RAW window exceeds source allocation".into());
        }
        for row in 0..h {
            self.copy_range(
                encoder,
                u64::from(y + row) * u64::from(self.width) + u64::from(x),
                w,
                source,
                u64::from(source_y + row) * u64::from(source_width) + u64::from(source_x),
                true,
            );
        }
        Ok(())
    }
    fn check(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), String> {
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|v| v > self.width)
            || y.checked_add(h).is_none_or(|v| v > self.height)
        {
            return Err("RAW window is outside the full image".into());
        }
        Ok(())
    }
    pub(super) fn copy_range(
        &self,
        encoder: &mut ObservedEncoder,
        first: u64,
        count: u32,
        other: &wgpu::Buffer,
        other_first: u64,
        scatter: bool,
    ) {
        let end = first + u64::from(count);
        for page in &self.pages {
            let page_first = u64::from(page.first_pixel);
            let begin = first.max(page_first);
            let finish = end.min(page_first + u64::from(page.n_pixels));
            if begin >= finish {
                continue;
            }
            let page_offset = (begin - page_first) * 16;
            let other_offset = (other_first + begin - first) * 16;
            let bytes = (finish - begin) * 16;
            if scatter {
                encoder.copy_buffer_to_buffer(
                    other,
                    other_offset,
                    &page.buffer,
                    page_offset,
                    bytes,
                );
            } else {
                encoder.copy_buffer_to_buffer(
                    &page.buffer,
                    page_offset,
                    other,
                    other_offset,
                    bytes,
                );
            }
        }
    }
}
