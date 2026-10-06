use crate::precision::{Scalar, ZERO};
use rayon::prelude::*;

/// Contiguous HxWx3 image buffer, row-major, channel-interleaved.
///
/// Pixel type is `Scalar` — f32 by default, f64 with `--features precision-f64`.
#[derive(Clone)]
pub struct ImageBuf {
    pub width: u32,
    pub height: u32,
    pub data: Vec<Scalar>,
}

impl ImageBuf {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![ZERO; (width as usize) * (height as usize) * 3],
        }
    }

    pub fn from_data(width: u32, height: u32, data: Vec<Scalar>) -> Self {
        assert_eq!(data.len(), (width as usize) * (height as usize) * 3);
        Self {
            width,
            height,
            data,
        }
    }

    pub fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    #[inline]
    pub fn idx(&self, x: u32, y: u32, c: usize) -> usize {
        ((y as usize) * (self.width as usize) + (x as usize)) * 3 + c
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [Scalar; 3] {
        let i = self.idx(x, y, 0);
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, rgb: [Scalar; 3]) {
        let i = self.idx(x, y, 0);
        self.data[i] = rgb[0];
        self.data[i + 1] = rgb[1];
        self.data[i + 2] = rgb[2];
    }
    /// Return a new buffer rotated by a multiple of 90 degrees.
    ///
    /// Positive turns are counter-clockwise, matching NumPy's `rot90`
    /// convention used by the pinned Python GUI.
    pub fn rotated_quarter_turns(&self, quarter_turns: i32) -> Self {
        let turns = quarter_turns.rem_euclid(4);
        if turns == 0 {
            return self.clone();
        }
        let (width, height) = if turns % 2 == 0 {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        let mut rotated = Self::new(width, height);
        for y in 0..self.height {
            for x in 0..self.width {
                let (dst_x, dst_y) = match turns {
                    1 => (y, self.width - 1 - x),
                    2 => (self.width - 1 - x, self.height - 1 - y),
                    3 => (self.height - 1 - y, x),
                    _ => unreachable!(),
                };
                rotated.set(dst_x, dst_y, self.get(x, y));
            }
        }
        rotated
    }


    pub fn pixels(&self) -> impl Iterator<Item = &[Scalar]> {
        self.data.chunks_exact(3)
    }

    pub fn pixels_mut(&mut self) -> impl Iterator<Item = &mut [Scalar]> {
        self.data.chunks_exact_mut(3)
    }

    pub fn par_pixels(&self) -> rayon::slice::ChunksExact<'_, Scalar> {
        self.data.par_chunks_exact(3)
    }

    pub fn par_pixels_mut(&mut self) -> rayon::slice::ChunksExactMut<'_, Scalar> {
        self.data.par_chunks_exact_mut(3)
    }

    pub fn rows(&self) -> impl Iterator<Item = &[Scalar]> {
        self.data.chunks_exact((self.width as usize) * 3)
    }

    pub fn par_rows(&self) -> rayon::slice::ChunksExact<'_, Scalar> {
        self.data.par_chunks_exact((self.width as usize) * 3)
    }

    pub fn par_rows_mut(&mut self) -> rayon::slice::ChunksExactMut<'_, Scalar> {
        let w = (self.width as usize) * 3;
        self.data.par_chunks_exact_mut(w)
    }

    pub fn extract_channel(&self, c: usize) -> Vec<Scalar> {
        assert!(c < 3);
        self.data.iter().skip(c).step_by(3).copied().collect()
    }

    pub fn write_channel(&mut self, c: usize, chan: &[Scalar]) {
        assert!(c < 3);
        assert_eq!(chan.len(), self.pixel_count());
        for (i, &v) in chan.iter().enumerate() {
            self.data[i * 3 + c] = v;
        }
    }
}

impl std::fmt::Debug for ImageBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImageBuf({}x{}, {} pixels)",
            self.width,
            self.height,
            self.pixel_count()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::precision::from_f64;

    #[test]
    fn test_new_and_dimensions() {
        let img = ImageBuf::new(4, 3);
        assert_eq!(img.width, 4);
        assert_eq!(img.height, 3);
        assert_eq!(img.data.len(), 4 * 3 * 3);
        assert_eq!(img.pixel_count(), 12);
    }

    #[test]
    fn test_get_set_pixel() {
        let mut img = ImageBuf::new(2, 2);
        img.set(1, 0, [from_f64(0.5), from_f64(0.6), from_f64(0.7)]);
        let px = img.get(1, 0);
        assert_eq!(px, [from_f64(0.5), from_f64(0.6), from_f64(0.7)]);
    }

    #[test]
    fn test_pixels_iter() {
        let img = ImageBuf::from_data(
            2,
            1,
            vec![
                from_f64(1.0),
                from_f64(2.0),
                from_f64(3.0),
                from_f64(4.0),
                from_f64(5.0),
                from_f64(6.0),
            ],
        );
        let pixels: Vec<&[Scalar]> = img.pixels().collect();
        assert_eq!(pixels.len(), 2);
        assert_eq!(pixels[0], &[from_f64(1.0), from_f64(2.0), from_f64(3.0)]);
        assert_eq!(pixels[1], &[from_f64(4.0), from_f64(5.0), from_f64(6.0)]);
    }

    #[test]
    fn test_extract_write_channel() {
        let mut img = ImageBuf::from_data(
            2,
            1,
            vec![
                from_f64(1.0),
                from_f64(2.0),
                from_f64(3.0),
                from_f64(4.0),
                from_f64(5.0),
                from_f64(6.0),
            ],
        );
        let g = img.extract_channel(1);
        assert_eq!(g, vec![from_f64(2.0), from_f64(5.0)]);
        img.write_channel(1, &[from_f64(10.0), from_f64(20.0)]);
        assert_eq!(
            img.get(0, 0),
            [from_f64(1.0), from_f64(10.0), from_f64(3.0)]
        );
        assert_eq!(
            img.get(1, 0),
            [from_f64(4.0), from_f64(20.0), from_f64(6.0)]
        );
    }
    #[test]
    fn rotated_quarter_turns_match_numpy_orientation_for_non_square_image() {
        let mut image = ImageBuf::new(2, 3);
        for (index, pixel) in image.pixels_mut().enumerate() {
            pixel[0] = from_f64(index as f64);
            pixel[1] = from_f64(index as f64 + 10.0);
            pixel[2] = from_f64(index as f64 + 20.0);
        }
        let ccw = image.rotated_quarter_turns(1);
        assert_eq!((ccw.width, ccw.height), (3, 2));
        assert_eq!(ccw.get(0, 0), image.get(1, 0));
        assert_eq!(ccw.get(2, 0), image.get(1, 2));
        assert_eq!(ccw.get(0, 1), image.get(0, 0));
        assert_eq!(image.rotated_quarter_turns(-1).rotated_quarter_turns(1).data, image.data);
        assert_eq!(image.rotated_quarter_turns(4).data, image.data);
    }
}
