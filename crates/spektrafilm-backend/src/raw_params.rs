//! Device-independent RAW sensor samples and resolved processing controls.
//!
//! LibRaw owns file parsing and compressed-sample unpacking. These records
//! preserve the unpacked sample plane and the metadata needed to run its
//! selected post-unpack algorithm on a compute device. The CPU reference keeps
//! running LibRaw's own `dcraw_process`; a device route consumes these records
//! instead and must not call `dcraw_process`.

/// Unpacked sensor storage layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawSensorLayout {
    /// One CFA or single-colour sample per sensor site.
    Cfa,
    /// Three interleaved decoded channels per sensor site.
    Interleaved3,
    /// Four interleaved decoded channels per sensor site.
    Interleaved4,
    /// One floating-point sample per sensor site.
    FloatCfa,
    /// Three interleaved floating-point channels per sensor site.
    Float3,
    /// Four interleaved floating-point channels per sensor site.
    Float4,
}

impl RawSensorLayout {
    /// Samples per sensor site.
    pub fn channels(self) -> u32 {
        match self {
            Self::Cfa | Self::FloatCfa => 1,
            Self::Interleaved3 | Self::Float3 => 3,
            Self::Interleaved4 | Self::Float4 => 4,
        }
    }
}

/// The algorithm LibRaw's `user_qual = -1` dispatch selects for a sensor.
///
/// The dispatch mirrors `dcraw_process()` exactly: `quality = 2 + !fuji_width`,
/// then the `colors > 3` / `real_colors > 3` / `bad_bayer` /
/// `filters <= 1000` fallbacks, then the X-Trans and AHD branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawDemosaic {
    /// Three-colour Bayer AHD.
    BayerAhd,
    /// VNG for four-colour, irregular, 16×16, or non-Bayer CFA data.
    Vng,
    /// PPG for Fuji-rotated Bayer data.
    FujiPpg,
    /// Fuji X-Trans Markesteijn interpolation (three passes).
    XTrans,
    /// The sensor already carries colour samples; LibRaw does not interpolate.
    None,
}

impl RawDemosaic {
    /// A stable name for diagnostics, error messages, and test reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::BayerAhd => "bayer_ahd",
            Self::Vng => "vng",
            Self::FujiPpg => "fuji_ppg",
            Self::XTrans => "xtrans",
            Self::None => "none",
        }
    }
}

/// Unpacked LibRaw samples and file metadata.
///
/// Ordinary sensors store `width × height × layout.channels()` values, with
/// zero-filled sites beyond `copied_width × copied_height`. Fuji-rotated
/// sensors instead store the compact `copied_width × copied_height` source
/// rectangle: the device performs LibRaw's diamond placement before scaling.
#[derive(Clone, Debug)]
pub struct RawSensor {
    pub samples: Vec<f32>,
    pub layout: RawSensorLayout,
    /// Active image dimensions before camera orientation.
    pub width: u32,
    pub height: u32,
    /// Sites actually copied from the unpacked allocation.
    pub copied_width: u32,
    pub copied_height: u32,
    /// LibRaw's row stride in the unpacked allocation, in bytes.
    pub allocation_row_stride_bytes: u32,
    /// LibRaw packed CFA descriptor (`filters`).
    pub filters: u32,
    /// Periodic CFA lookup tile over active-area coordinates, values 0..=3.
    pub cfa: Vec<u8>,
    pub cfa_rows: u32,
    pub cfa_columns: u32,
    /// LibRaw Fuji-rotated sensor width; zero for ordinary layouts.
    pub fuji_width: u32,
    /// LibRaw Fuji layout discriminator.
    pub fuji_layout: bool,
    /// LibRaw orientation code in the `flip_index` bit layout: bit 4 swaps
    /// axes, bit 2 flips rows, bit 1 flips columns.
    pub flip: u32,
    /// Pixel aspect ratio used by LibRaw's optional stretch stage.
    pub pixel_aspect: f64,
    pub is_foveon: bool,
    /// Logical colour channels in the decoder metadata.
    pub colors: u32,
    /// Global black: folded for integer storage, unfurled for float storage
    /// until device `convertFloatToInt` and `adjust_bl` preparation.
    pub black: u32,
    /// LibRaw saturation level before `maximum -= black`.
    pub maximum: u32,
    /// Per-channel black: folded for integer storage, unfurled for float storage.
    pub channel_black: [u32; 4],
    /// Repeating black-map height, or zero when absent.
    pub black_map_rows: u32,
    /// Repeating black-map width, or zero when absent.
    pub black_map_columns: u32,
    /// Repeating black offsets from LibRaw's `cblack[6..]`.
    pub black_map: Vec<u32>,
    /// Camera white-balance multipliers (`cam_mul`).
    pub camera_multipliers: [f32; 4],
    /// Multipliers LibRaw parsed before `scale_colors` (`pre_mul`).
    pub default_multipliers: [f32; 4],
    /// LibRaw camera→linear-sRGB matrix (3 rows × 4 camera channels).
    pub rgb_camera: [[f32; 4]; 3],
    /// Camera→XYZ matrix normalized by D65 white, as `cielab()` builds it.
    pub xyz_camera: [[f32; 4]; 3],
    /// Eight-by-eight white-balance sample grid.
    pub white: [u16; 64],
    /// Whether the samples already have the shot white balance applied.
    pub as_shot_wb_applied: bool,
    /// Decoder requests averaging the two green channels after interpolation.
    pub mix_green: bool,
    /// Decoder declares a fixed saturation level (`LIBRAW_DECODER_FIXEDMAXC`).
    pub fixed_maximum: bool,
    /// The decoder marks zero CFA samples as missing, not measured black.
    pub zero_is_bad: bool,
    /// The decoder's output is already RGB; `convert_to_rgb` skips its matrix.
    pub raw_color: bool,
    /// Native X3F stage clears negative signed-ushort samples after black removal.
    pub signed_foveon: bool,
    /// Maximum reconstructed by the floating-point decoder before conversion.
    pub float_maximum: f32,
}

impl RawSensor {
    /// The CFA colour at active-area coordinate `(row, column)`.
    pub fn cfa_at(&self, row: u32, column: u32) -> u8 {
        if self.cfa_rows == 0 || self.cfa_columns == 0 {
            return 0;
        }
        self.cfa[((row % self.cfa_rows) * self.cfa_columns + (column % self.cfa_columns)) as usize]
    }

    /// LibRaw's `FC(row, col)` packed-pattern lookup.
    pub fn fc(&self, row: u32, column: u32) -> u32 {
        let shift = (((row << 1) & 14) | (column & 1)) << 1;
        (self.filters >> shift) & 3
    }
}

impl RawSensor {
    /// Dimensions after `fuji_rotate`, before colour conversion and stretch.
    pub fn rotated_dimensions(&self) -> Result<(u32, u32), String> {
        if self.fuji_width == 0 {
            return Ok((self.width, self.height));
        }
        let diagonal = self.fuji_width.checked_sub(1).ok_or("Invalid Fuji width")?;
        if diagonal >= self.height {
            return Err("Fuji diagonal exceeds the active image".into());
        }
        let step = 0.5f64.sqrt();
        Ok((
            (f64::from(diagonal) / step) as u32,
            (f64::from(self.height - diagonal) / step) as u32,
        ))
    }

    /// LibRaw stretches after camera-to-output conversion and before flip.
    pub fn stretched_dimensions(&self) -> Result<(u32, u32), String> {
        if !self.pixel_aspect.is_finite() || self.pixel_aspect <= 0.0 {
            return Err("RAW pixel aspect must be finite and positive".into());
        }
        let (mut width, mut height) = self.rotated_dimensions()?;
        if self.pixel_aspect < 1.0 {
            height = (f64::from(height) / self.pixel_aspect + 0.5) as u32;
        } else if self.pixel_aspect > 1.0 {
            width = (f64::from(width) * self.pixel_aspect + 0.5) as u32;
        }
        if width == 0 || height == 0 || width > u16::MAX.into() || height > u16::MAX.into() {
            return Err("RAW output dimensions exceed LibRaw's 16-bit bitmap boundary".into());
        }
        Ok((width, height))
    }

    /// Published bitmap dimensions, including the native flip bit layout.
    pub fn output_dimensions(&self) -> Result<(u32, u32), String> {
        let (width, height) = self.stretched_dimensions()?;
        Ok(if self.flip & 4 != 0 {
            (height, width)
        } else {
            (width, height)
        })
    }
}

/// Lensfun's image-independent, interpolated correction model. Each field is
/// one 16-byte shader uniform row; paired components preserve native double
/// normalization values without constructing a host correction raster.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawLensParams {
    pub dimensions: [u32; 4],
    /// Projection, geometry/vignetting enable bits, and native combined-coordinate SIMD semantics.
    pub flags: [u32; 4],
    pub normalization: [f32; 4],
    pub center: [f32; 4],
    pub distortion: [f32; 4],
    pub tca: [f32; 4],
    pub tca_tail: [f32; 4],
    pub vignette: [f32; 4],
    pub mapping: [f32; 4],
}

/// Processing controls resolved from `RawSensor` metadata and `RawOptions`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawGpuParams {
    /// The algorithm LibRaw's `user_qual = -1` dispatch selects.
    pub demosaic: RawDemosaic,
    /// LibRaw's `out_cam` for ACES AP0 output: camera RGB → linear ACES.
    pub camera_to_aces: [[f32; 4]; 3],
    /// CAT02 adaptation from the requested white point to 6504 K, in ACES XYZ.
    /// Identity when `adaptation_active` is false.
    pub adaptation: [[f32; 3]; 3],
    pub adaptation_active: bool,
    /// Custom green-tint multiplier; one for every other white-balance mode.
    pub tint: f32,
    /// LibRaw's default `adjust_maximum_thr`.
    pub adjust_maximum_threshold: f32,
    /// White-balance multipliers before normalization by the smallest one
    /// (`pre_mul` after `scale_colors`' selection).
    pub white_balance: [f32; 4],
    /// As-shot metadata has no camera multiplier; reduce native 8×8 blocks.
    pub auto_white_balance: bool,
    /// Saturation level after `maximum -= black`.
    pub maximum: u32,
    /// Whether `pre_interpolate` merges the second green into channel 1.
    pub green_merge: bool,
    /// Row parity of the sites `pre_interpolate` merges.
    pub green_merge_row: u32,
    /// Column parity of the sites `pre_interpolate` merges.
    pub green_merge_column: u32,
    /// Optional Lensfun model, applied after orientation and before custom WB.
    pub lens: Option<RawLensParams>,
}

impl RawGpuParams {
    /// LibRaw's `adjust_maximum()`: a measured sensor maximum between the
    /// threshold and the declared white level replaces that white level.
    pub fn adjusted_maximum(&self, data_maximum: u32) -> u32 {
        if data_maximum > 0
            && data_maximum < self.maximum
            && (data_maximum as f32) > (self.maximum as f32) * self.adjust_maximum_threshold
        {
            data_maximum
        } else {
            self.maximum
        }
    }

    /// The same plan with the saturation level a device reduction measured.
    pub fn with_measured_maximum(mut self, data_maximum: u32) -> Self {
        self.maximum = self.adjusted_maximum(data_maximum);
        self
    }

    /// The per-channel integer scale factor `scale_colors_loop` applies.
    ///
    /// `dmin` is the smallest multiplier, because `highlight == 0` forces
    /// `dmax = dmin` in LibRaw's `scale_colors`; `scale = (pre / dmin) ·
    /// 65535 / maximum`.
    pub fn scale_multipliers(&self) -> [f32; 4] {
        let mut dmin = f32::INFINITY;
        for value in self.white_balance {
            if value < dmin {
                dmin = value;
            }
        }
        if dmin.is_nan() || dmin <= 0.00001 || self.maximum == 0 {
            return [1.0; 4];
        }
        std::array::from_fn(|c| (self.white_balance[c] / dmin) * 65535.0 / self.maximum as f32)
    }
}
