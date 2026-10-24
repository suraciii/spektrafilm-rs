// Shared definitions for the device RAW pipeline.
//
// The kernels reproduce LibRaw's post-unpack numerical stages exactly:
// `raw2image`'s inline black subtraction, `scale_colors`' integer scaling,
// `pre_interpolate`'s green merge, `border_interpolate(5)`, AHD
// (`ahd_interpolate`), and `convert_to_rgb`. Sample arithmetic stays in the
// reference's integer domain (`u32` samples, 0..65535); only LibRaw's own
// float expressions are evaluated in f32.

struct RawFrame {
    width: u32,
    height: u32,
    cfa_rows: u32,
    cfa_columns: u32,
    black_rows: u32,
    black_columns: u32,
    /// Nonzero when `pre_interpolate` merges the second green.
    green_merge: u32,
    /// Nonzero when `scale_colors_loop` subtracts the repeating black map.
    has_black_map: u32,
    /// Row parity of the merged sites: `FC(1, 0) >> 1`.
    merge_row: u32,
    /// Column parity of the merged sites: `FC(row, 1) & 1`.
    merge_column: u32,
    source_width: u32,
    source_height: u32,
    channels: u32,
    fuji_width: u32,
    fuji_layout: u32,
    colors: u32,
    subtract_black_map: u32,
    mix_green: u32,
};

fn raw_pixel_count(frame: RawFrame) -> u32 {
    return frame.width * frame.height;
}

fn raw_cfa_index(x: u32, y: u32, frame: RawFrame) -> u32 {
    return (y % frame.cfa_rows) * frame.cfa_columns + (x % frame.cfa_columns);
}

/// The colour a pixel reports after `pre_interpolate` cleared the second
/// green's bit, i.e. `fcol` for every later stage.
///
/// Buffer-free: the caller supplies the code it already read out of `cfa`,
/// again so that this shared file names no per-stage buffer.
fn raw_color_from_code(code: u32, frame: RawFrame) -> u32 {
    if (frame.green_merge != 0u && code == 3u) {
        return 1u;
    }
    return code;
}

/// Whether `pre_interpolate`'s merge loop visits `(x, y)`. The loop visits a
/// fixed parity pair, which is *not* always the set of code-3 sites: for an
/// unusual packed pattern it can copy an empty channel 3 over a green sample,
/// and the device reproduces that result rather than a cleaner one.
fn raw_is_merge_site(x: u32, y: u32, frame: RawFrame) -> bool {
    return frame.green_merge != 0u && (y & 1u) == frame.merge_row && (x & 1u) == frame.merge_column;
}

/// `CLIP(x)`: `LIM((int)x, 0, 65535)`.
fn raw_clip(value: i32) -> u32 {
    return u32(clamp(value, 0, 65535));
}

/// `ULIM(x, y, z)`: clamp to the interval the two bounds define.
fn raw_ulim(value: i32, a: i32, b: i32) -> u32 {
    if (a < b) {
        return u32(clamp(value, a, b));
    }
    return u32(clamp(value, b, a));
}

/// Index of `image`'s sample for `(x, y)`'s `channel`. Buffer-free for the
/// same reason as the colour helpers: the caller reads `image` itself, so the
/// shared file names no per-stage buffer.
fn raw_sample_index(x: u32, y: u32, frame: RawFrame, channel: u32) -> u32 {
    return (y * frame.width + x) * 4u + channel;
}

/// `ahd_interpolate_r_and_b_in_rgb_and_convert_to_cielab`'s per-dispatch
/// parameters. Lives here because `raw_lab.wgsl` and the homogeneity stage
/// both need it, while `raw_ahd_rb.wgsl` is the stage that uploads it.
struct AhdPass {
    direction: u32,
    colors: u32,
    _pad0: u32,
    _pad1: u32,
};
