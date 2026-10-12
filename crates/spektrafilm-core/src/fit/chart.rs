//! Chart layout, patch tables, chart assembly, and patch sampling.

use serde::{Deserialize, Serialize};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

/// A half-open `[start, end)` patch index range. Serialized as a two-element array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "[usize; 2]", into = "[usize; 2]")]
pub struct IndexRange {
    pub start: usize,
    pub end: usize,
}

impl From<[usize; 2]> for IndexRange {
    fn from(value: [usize; 2]) -> Self {
        Self {
            start: value[0],
            end: value[1],
        }
    }
}

impl From<IndexRange> for [usize; 2] {
    fn from(value: IndexRange) -> Self {
        [value.start, value.end]
    }
}

impl IndexRange {
    /// Patch indices of this range in ascending order.
    pub fn indices(self) -> impl Iterator<Item = usize> {
        self.start..self.end
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range selects no patch.
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// One ordered list of linear RGB patch values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchTable {
    pub patches: Vec<[f64; 3]>,
}

/// Patch grid geometry and the patch index sets a fit consumes.
///
/// Patches are placed in row-major order in a grid with `gap_px` between and
/// around the patches. `color` selects the chromatic patches, `gray` the gray
/// row, and `fit_gray` the gray patches the fit scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartLayout {
    pub rows: usize,
    pub cols: usize,
    pub patch_px: usize,
    pub gap_px: usize,
    pub box_px: usize,
    pub color: IndexRange,
    pub gray: IndexRange,
    pub fit_gray: IndexRange,
}

impl ChartLayout {
    pub fn patch_count(&self) -> usize {
        self.rows * self.cols
    }

    /// Chart image size in pixels.
    pub fn image_size(&self) -> (u32, u32) {
        let width = self.cols * self.patch_px + (self.cols + 1) * self.gap_px;
        let height = self.rows * self.patch_px + (self.rows + 1) * self.gap_px;
        (width as u32, height as u32)
    }

    fn patch_origin(&self, index: usize) -> (usize, usize) {
        let row = index / self.cols;
        let col = index % self.cols;
        (
            self.gap_px + row * (self.patch_px + self.gap_px),
            self.gap_px + col * (self.patch_px + self.gap_px),
        )
    }

    /// Reject a layout that cannot address its own patch set.
    pub fn validate(&self) -> Result<(), String> {
        if self.rows == 0 || self.cols == 0 {
            return Err("chart layout rows and cols must be positive".into());
        }
        if self.patch_px == 0 {
            return Err("chart layout patch_px must be positive".into());
        }
        if self.box_px == 0 || !self.box_px.is_multiple_of(2) || self.box_px > self.patch_px {
            return Err(format!(
                "chart layout box_px must be positive, even, and at most patch_px ({}), got {}",
                self.patch_px, self.box_px
            ));
        }
        let count = self.patch_count();
        for (name, range) in [
            ("color", self.color),
            ("gray", self.gray),
            ("fit_gray", self.fit_gray),
        ] {
            if range.is_empty() {
                return Err(format!(
                    "chart layout {name} range [{}, {}) must be non-empty",
                    range.start, range.end
                ));
            }
            if range.end > count {
                return Err(format!(
                    "chart layout {name} range [{}, {}) exceeds the patch count {count}",
                    range.start, range.end
                ));
            }
        }
        Ok(())
    }
}

/// Assemble the chart image from a patch table without any color management.
pub fn assemble(layout: &ChartLayout, patches: &[[f64; 3]]) -> Result<ImageBuf, String> {
    let (width, height) = layout.image_size();
    let mut image = ImageBuf::new(width, height);
    for (index, patch) in patches.iter().enumerate() {
        let (y0, x0) = layout.patch_origin(index);
        for y in y0..y0 + layout.patch_px {
            for x in x0..x0 + layout.patch_px {
                let base = ((y as u32 * width + x as u32) * 3) as usize;
                for (channel, value) in patch.iter().enumerate() {
                    image.data[base + channel] = from_f64(*value);
                }
            }
        }
    }
    Ok(image)
}

/// Mean a centered box per patch. The box never touches a gap or a neighbor.
pub fn sample(layout: &ChartLayout, image: &ImageBuf) -> Result<Vec<[f64; 3]>, String> {
    let (width, height) = layout.image_size();
    if image.width != width || image.height != height {
        return Err(format!(
            "sampled image is {}x{}, chart layout expects {width}x{height}",
            image.width, image.height
        ));
    }
    let half = layout.box_px / 2;
    let mut out = Vec::with_capacity(layout.patch_count());
    for index in 0..layout.patch_count() {
        let (y0, x0) = layout.patch_origin(index);
        let (cy, cx) = (y0 + layout.patch_px / 2, x0 + layout.patch_px / 2);
        let mut sum = [0.0f64; 3];
        let mut count = 0.0f64;
        for y in cy - half..cy + half {
            for x in cx - half..cx + half {
                let base = ((y as u32 * width + x as u32) * 3) as usize;
                for (channel, value) in sum.iter_mut().enumerate() {
                    *value += to_f64(image.data[base + channel]);
                }
                count += 1.0;
            }
        }
        out.push(sum.map(|value| value / count));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> ChartLayout {
        serde_json::from_str(
            r#"{"rows":5,"cols":9,"patch_px":72,"gap_px":12,"box_px":24,
                "color":[0,36],"gray":[36,45],"fit_gray":[37,44]}"#,
        )
        .expect("layout parses")
    }

    #[test]
    fn layout_round_trips_through_json() {
        let parsed = layout();
        assert_eq!(parsed.patch_count(), 45);
        assert_eq!(parsed.image_size(), (768, 432));
        assert_eq!((parsed.color.start, parsed.color.end), (0, 36));
        assert_eq!(
            parsed.fit_gray.indices().collect::<Vec<_>>(),
            (37..44).collect::<Vec<_>>()
        );
        let text = serde_json::to_string(&parsed).expect("layout serializes");
        let round: ChartLayout = serde_json::from_str(&text).expect("layout reparses");
        assert_eq!(round.fit_gray.end, 44);
    }

    #[test]
    fn layout_rejects_unaddressable_geometry() {
        let mut odd_box = layout();
        odd_box.box_px = 25;
        assert!(odd_box.validate().unwrap_err().contains("even"));
        let mut oversized = layout();
        oversized.color = IndexRange { start: 0, end: 46 };
        assert!(oversized.validate().unwrap_err().contains("exceeds"));
        let mut empty = layout();
        empty.fit_gray = IndexRange { start: 40, end: 40 };
        assert!(empty.validate().unwrap_err().contains("non-empty"));
    }

    #[test]
    fn assemble_and_sample_recover_the_patch_table() {
        let mut grid = layout();
        grid.rows = 2;
        grid.cols = 3;
        grid.box_px = 8;
        grid.color = IndexRange { start: 0, end: 6 };
        grid.gray = IndexRange { start: 0, end: 1 };
        grid.fit_gray = IndexRange { start: 0, end: 1 };
        let patches: Vec<[f64; 3]> = (0..6)
            .map(|index| [index as f64 / 10.0, 0.25 + index as f64 / 20.0, 0.5])
            .collect();
        let image = assemble(&grid, &patches).expect("chart assembles");
        let sampled = sample(&grid, &image).expect("chart samples");
        assert_eq!(sampled.len(), 6);
        for (got, want) in sampled.iter().zip(&patches) {
            for channel in 0..3 {
                assert!(
                    (got[channel] - want[channel]).abs() < 1e-12,
                    "{got:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn sampling_rejects_a_foreign_image_size() {
        let grid = layout();
        let too_small = ImageBuf::new(10, 10);
        assert!(sample(&grid, &too_small).unwrap_err().contains("expects"));
    }
}
