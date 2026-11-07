//! Bounded native-viewer presentation geometry; image samples stay in carriers.
/// Endpoint warnings applied only to photographic viewer pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerClipping {
    #[default]
    Off,
    Overlay {
        shadows: bool,
        highlights: bool,
    },
    Shadows,
    Highlights,
}

impl ViewerClipping {
    pub fn active(self) -> bool {
        !matches!(
            self,
            Self::Off
                | Self::Overlay {
                    shadows: false,
                    highlights: false,
                }
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ViewerComposeGpuParams {
    pub width: u32,
    pub height: u32,
    pub rect: [f32; 4],
    pub border: [f32; 4],
    pub paper_rect: [f32; 4],
    pub image_rect: [f32; 4],
    pub gray: [f32; 3],
    pub interpolation: u32,
    pub full_source: bool,
    pub output_layer: bool,
    pub fade: f32,
    pub clipping: ViewerClipping,
}

/// One GUI viewer resample request: the long-edge cap the viewer applies to a
/// full-resolution image before it composes viewport pixels.
///
/// The resample is the viewer's only image-sized reduction, so a device route
/// must not compute it on the host. The kernel is separable bilinear over the
/// exact source rectangle each output pixel covers, matching the host
/// `capped_raster`; the returned pixels are disposable viewer data, never a
/// retained or saved sample.
///
/// The host owns the destination size: it derives the long-edge factor and the
/// rounded destination dimensions in f64, so one authoritative rule decides
/// how many pixels the viewer keeps, and a device backend cannot disagree
/// with the viewer about the raster geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewerResampleGpuParams {
    /// Source width in pixels.
    pub width: u32,
    /// Source height in pixels.
    pub height: u32,
    /// Destination width in pixels, computed host-side.
    pub out_width: u32,
    /// Destination height in pixels, computed host-side.
    pub out_height: u32,
}

/// One GUI viewer display-colour transform request: native image samples to
/// encoded sRGB viewing pixels.
///
/// This is the viewer's display transform (`display_rgb`), not the saving
/// conversion: the matrix is the source space's display matrix and the
/// destination transfer is always sRGB. The result is clamped to 0..=1 for the
/// bounded display surface, exactly as the host viewer does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayTransformGpuParams {
    /// Source CCTF discriminant, used only when `decode` is set.
    pub src_cctf: u32,
    /// Whether to decode the source transfer before the matrix.
    pub decode: bool,
    /// Source → encoded sRGB matrix.
    pub matrix: [[f32; 3]; 3],
    /// Whether to clamp the encoded result to 0..=1.
    pub clamp: bool,
}

/// Fixed RGB8 display-transform representations prepared by the host CMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ViewerIccKind {
    Identity = 0,
    /// Three packed 256-byte channel tables.
    Curves = 1,
    /// Twelve Q14 matrix/offset words, 768 Q14 input-shaper words, then
    /// three packed 16385-byte output-shaper tables.
    MatrixShaper = 2,
    /// Exact RGB8-domain map, lexicographic RGB order, packed RGB bytes.
    /// Unlike a sampled 3D LUT, this representation has no interpolation error.
    ExactRgb8 = 3,
}

impl ViewerIccKind {
    pub fn word_count(self) -> usize {
        match self {
            Self::Identity => 1,
            Self::Curves => 3 * 256 / 4,
            Self::MatrixShaper => 12 + 3 * 256 + (3 * 16385usize).div_ceil(4),
            Self::ExactRgb8 => 3 * 256 * 256 * 256 / 4,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Identity),
            1 => Some(Self::Curves),
            2 => Some(Self::MatrixShaper),
            3 => Some(Self::ExactRgb8),
            _ => None,
        }
    }
}

/// Immutable, image-independent LittleCMS preparation for fixed GPU application.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerIccGpuParams<'a> {
    /// Content identity of the profiles, conversion semantics, CMM and format.
    pub identity: [u8; 32],
    pub kind: ViewerIccKind,
    pub words: &'a [u32],
}

/// One GUI viewer magnified full-source composition request: the bounded
/// viewport raster the magnified view produces from the full-resolution
/// source.
///
/// The magnified view samples the source at every visible viewport pixel, so
/// this is the viewer's per-pixel composition rather than a reduction. The
/// request carries the viewport geometry the host already resolved, the
/// interpolation mode, and the resolved display and ICC transforms, so the
/// device evaluates the same per-pixel arithmetic the host composition does.
/// A viewport pixel outside the image rectangle stays transparent, exactly as
/// the host leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerFullSourceGpuParams<'a> {
    /// Viewport texture size in pixels.
    pub tex_width: u32,
    /// Viewport texture size in pixels.
    pub tex_height: u32,
    /// Viewport rectangle minimum, in points.
    pub rect_min: [f32; 2],
    /// Viewport rectangle size, in points.
    pub rect_size: [f32; 2],
    /// Image rectangle minimum, in points.
    pub image_min: [f32; 2],
    /// Image rectangle size, in points.
    pub image_size: [f32; 2],
    /// Interpolation mode discriminant, matching the viewer's `Interpolation`
    /// order: nearest, linear, cubic, spline16, spline36, lanczos, blackman.
    pub interpolation: u32,
    /// Whether the display colour transform applies at all. When it does not,
    /// the host views raw output-space values.
    pub transform: bool,
    /// Source CCTF discriminant, used only when `transform` is set.
    pub src_cctf: u32,
    /// Whether to decode the source transfer before the display matrix.
    pub decode: bool,
    /// Source → encoded sRGB display matrix.
    pub matrix: [[f32; 3]; 3],
    /// Whether the request carries the resolved ICC pipeline.
    pub icc: bool,
    /// Whether the ICC request carries a matrix stage.
    pub icc_matrix: bool,
    /// ICC matrix in the reference's 1.14 fixed point, used only when
    /// `icc_matrix` is set.
    pub icc_matrix_fixed14: [[i32; 3]; 3],
    /// ICC matrix offset in 1.14 fixed point, used only when `icc_matrix` is
    /// set.
    pub icc_offset_fixed14: [i32; 3],
    /// ICC first-shaper tables, one per channel, used only when `icc` is set.
    pub icc_first: &'a [Vec<i32>],
    /// ICC second-shaper tables, one per channel, used only when `icc` is set.
    pub icc_second: &'a [Vec<u32>],
}
