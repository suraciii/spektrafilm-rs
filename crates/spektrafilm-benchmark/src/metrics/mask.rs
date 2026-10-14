use crate::common::Result;
use serde_json::Value;
use std::path::Path;

use super::err;

#[derive(Clone, Copy)]
pub struct Crop {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
pub fn crop(mask: &[bool], width: usize, height: usize) -> Result<Crop> {
    let mut left = width;
    let mut right = 0;
    let mut top = height;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left == width {
        return Err(err("insufficient_samples", "Region has no selected pixels"));
    }
    Ok(Crop {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}
pub fn cropped_mask(mask: &[bool], width: usize, c: Crop) -> Vec<bool> {
    (c.y..c.y + c.height)
        .flat_map(|y| {
            mask[y * width + c.x..y * width + c.x + c.width]
                .iter()
                .copied()
        })
        .collect()
}
pub fn erode(mask: &mut Vec<bool>, width: usize, height: usize, iterations: usize) {
    if iterations == 0 {
        return;
    }
    let mut temporary = vec![false; mask.len()];
    for _ in 0..iterations {
        for y in 0..height {
            for x in 0..width {
                temporary[y * width + x] = x > 0
                    && y > 0
                    && x + 1 < width
                    && y + 1 < height
                    && mask[y * width + x]
                    && mask[y * width + x - 1]
                    && mask[y * width + x + 1]
                    && mask[(y - 1) * width + x]
                    && mask[(y + 1) * width + x];
            }
        }
        std::mem::swap(mask, &mut temporary);
    }
}
pub fn geometry_mask(geometry: &Value, width: usize, height: usize) -> Result<Vec<bool>> {
    // The integration owner rewrites mask paths to retained absolute paths.
    crate::documents::geometry_mask(geometry, width, height, Path::new("/"))
}
