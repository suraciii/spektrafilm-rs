//! Shared prepared-image I/O. Loading and writing retain sample interpretation;
//! saving-space conversion is an explicit operation separate from simulation.
use std::{ffi::{CString, CStr, c_char, c_void}, path::Path, sync::Arc};
use spektrafilm_math::{image::ImageBuf, precision::{from_f64, to_f64}};

#[derive(Debug, thiserror::Error)]
pub enum ImageIoError {
    #[error("Unsupported image extension '{0}'; use JPEG, PNG, TIFF or EXR")]
    UnsupportedFormat(String),
    #[error("Unsupported depth {0}; select 8, 16 or 32 bits")]
    UnsupportedDepth(u8),
    #[error("EXR requires 16-bit half or 32-bit float, received {0} bits")]
    ExrDepth(u8),
    #[error("Invalid {format} export settings: {reason}")]
    InvalidExport { format: &'static str, reason: &'static str },
    #[error("Image path contains a null byte")]
    InvalidPath,
    #[error("Invalid RGB buffer dimensions or sample count")]
    InvalidImage,
    #[error("{0}")]
    InvalidColorSpace(String),
    #[error("Image {operation} failed for {path}: {message}")]
    Native { operation: &'static str, path: String, message: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth { Eight, Sixteen, ThirtyTwo }
impl BitDepth { pub fn bits(self) -> u8 { match self { Self::Eight => 8, Self::Sixteen => 16, Self::ThirtyTwo => 32 } } }
impl TryFrom<u8> for BitDepth {
    type Error = ImageIoError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value { 8 => Ok(Self::Eight), 16 => Ok(Self::Sixteen), 32 => Ok(Self::ThirtyTwo), _ => Err(ImageIoError::UnsupportedDepth(value)) }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegSubsampling { Yuv444, Yuv420 }
impl JpegSubsampling {
    pub fn as_str(self) -> &'static str { match self { Self::Yuv444 => "4:4:4", Self::Yuv420 => "4:2:0" } }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression { Zip, None }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ImageFormat { Jpeg = 0, Png = 1, Tiff = 2, Exr = 3 }
impl ImageFormat {
    pub fn detect(path: &Path) -> Result<Self, ImageIoError> {
        let ext = extension(path);
        match ext.as_str() { "jpg" | "jpeg" => Ok(Self::Jpeg), "png" => Ok(Self::Png), "tif" | "tiff" => Ok(Self::Tiff), "exr" => Ok(Self::Exr), _ => Err(ImageIoError::UnsupportedFormat(ext)) }
    }
}
fn extension(path: &Path) -> String { path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase() }
pub fn is_raw(path: &Path) -> bool {
    matches!(extension(path).as_str(), "dng"|"cr2"|"cr3"|"nef"|"nrw"|"arw"|"srf"|"sr2"|"raf"|"orf"|"rw2"|"pef"|"srw"|"x3f"|"iiq"|"3fr"|"crw"|"rwl"|"mrw"|"mef"|"kdc"|"ari"|"bay"|"dcr"|"drf"|"erf"|"fff"|"k25"|"mos"|"ptx")
}
#[derive(Debug)]
struct NativeMetadata(*mut c_void);
// Exiv2 metadata is immutable after read, shared by save calls as const data.
unsafe impl Send for NativeMetadata {}
unsafe impl Sync for NativeMetadata {}
impl Drop for NativeMetadata { fn drop(&mut self) { unsafe { sf_metadata_free(self.0) } } }
#[derive(Debug, Clone)]
pub struct ImageMetadata(Arc<NativeMetadata>);
pub struct LoadedImage { pub image: ImageBuf, pub metadata: Option<ImageMetadata> }
#[derive(Debug, Clone, Copy)]
pub struct SaveOptions<'a> {
    pub depth: BitDepth,
    pub color_space: &'a str,
    pub cctf_encoding: bool,
    pub jpeg_quality: Option<u8>,
    pub jpeg_subsampling: Option<JpegSubsampling>,
    pub compression: Option<Compression>,
}
#[derive(Debug, Default)]
pub struct SaveReport { pub metadata_warning: Option<String> }
unsafe extern "C" {
    fn sf_io_free(pointer: *mut c_void);
    fn sf_metadata_free(pointer: *mut c_void);
    fn sf_metadata_read(path: *const c_char) -> *mut c_void;
    fn sf_image_load(path: *const c_char, width: *mut u32, height: *mut u32, samples: *mut *mut f64, error: *mut *mut c_char) -> i32;
    fn sf_image_save(path: *const c_char, width: u32, height: u32, samples: *const f64, depth: i32, format: i32, jpeg_quality: i32, jpeg_subsampling: i32, compression: i32, color_space: *const c_char, icc: *const u8, icc_len: usize, error: *mut *mut c_char) -> i32;
    fn sf_metadata_write(path: *const c_char, source: *const c_void, width: u32, height: u32, space: *const c_char, encoded: bool, error: *mut *mut c_char) -> i32;
}
fn cpath(path: &Path) -> Result<CString, ImageIoError> { CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| ImageIoError::InvalidPath) }
fn take_error(pointer: *mut c_char) -> String {
    if pointer.is_null() { return "Native codec did not provide an error".into(); }
    let result = unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned();
    unsafe { sf_io_free(pointer.cast()) }; result
}
/// Metadata read failures are nonfatal, matching upstream read_image_metadata.
pub fn read_metadata(path: &Path) -> Option<ImageMetadata> {
    let path = cpath(path).ok()?;
    let pointer = unsafe { sf_metadata_read(path.as_ptr()) };
    (!pointer.is_null()).then(|| ImageMetadata(Arc::new(NativeMetadata(pointer))))
}
pub fn load(path: &Path) -> Result<LoadedImage, ImageIoError> {
    ImageFormat::detect(path)?;
    let name = cpath(path)?;
    let (mut width, mut height, mut samples, mut error) = (0, 0, std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe { sf_image_load(name.as_ptr(), &mut width, &mut height, &mut samples, &mut error) } == 0 {
        return Err(ImageIoError::Native { operation: "load", path: path.display().to_string(), message: take_error(error) });
    }
    let count = width as usize * height as usize * 3;
    let data = unsafe { std::slice::from_raw_parts(samples, count) }.iter().copied().map(from_f64).collect();
    unsafe { sf_io_free(samples.cast()) };
    Ok(LoadedImage { image: ImageBuf::from_data(width, height, data), metadata: read_metadata(path) })
}
/// Integer outputs clip to [0,1] and truncate after scaling, matching numpy
/// astype. PNG/JPEG always use uint8; float TIFF/EXR preserve unbounded samples.
/// Pixel failures are errors; metadata failures report a warning after writing.
pub fn save(path: &Path, image: &ImageBuf, options: SaveOptions<'_>, metadata: Option<&ImageMetadata>) -> Result<SaveReport, ImageIoError> {
    save_inner(path, image, options, metadata)
}

/// Compatibility helper for callers that need an explicit JPEG quality.
pub fn save_jpeg_quality(
    path: &Path,
    image: &ImageBuf,
    mut options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    quality: u8,
) -> Result<SaveReport, ImageIoError> {
    options.jpeg_quality = Some(quality);
    save_inner(path, image, options, metadata)
}

fn save_inner(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
) -> Result<SaveReport, ImageIoError> {
    let format = ImageFormat::detect(path)?;
    let format_name = match format { ImageFormat::Jpeg => "JPEG", ImageFormat::Png => "PNG", ImageFormat::Tiff => "TIFF", ImageFormat::Exr => "EXR" };
    match format {
        ImageFormat::Jpeg | ImageFormat::Png if options.depth != BitDepth::Eight =>
            return Err(ImageIoError::InvalidExport { format: format_name, reason: "only 8-bit output is supported" }),
        ImageFormat::Exr if options.depth == BitDepth::Eight => return Err(ImageIoError::ExrDepth(8)),
        _ => {}
    }
    if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) && !options.cctf_encoding {
        return Err(ImageIoError::InvalidExport { format: format_name, reason: "JPEG and PNG require encoded output" });
    }
    if format == ImageFormat::Exr && options.cctf_encoding {
        return Err(ImageIoError::InvalidExport { format: "EXR", reason: "EXR output must be linear" });
    }
    if format == ImageFormat::Exr && !matches!(options.color_space, "sRGB" | "ACES2065-1") {
        return Err(ImageIoError::InvalidExport { format: "EXR", reason: "EXR color space must be sRGB or ACES2065-1" });
    }
    if format != ImageFormat::Jpeg && options.jpeg_quality.is_some() {
        return Err(ImageIoError::InvalidExport { format: format_name, reason: "JPEG quality is only valid for JPEG" });
    }
    if format != ImageFormat::Jpeg && options.jpeg_subsampling.is_some() {
        return Err(ImageIoError::InvalidExport { format: format_name, reason: "JPEG subsampling is only valid for JPEG" });
    }
    if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) && options.compression.is_some() {
        return Err(ImageIoError::InvalidExport { format: format_name, reason: "compression is not configurable for JPEG or PNG" });
    }
    if format == ImageFormat::Exr && matches!(options.compression, Some(Compression::None)) {
        return Err(ImageIoError::InvalidExport { format: "EXR", reason: "EXR compression is always zip" });
    }
    if let Some(quality) = options.jpeg_quality {
        if !(1..=100).contains(&quality) {
            return Err(ImageIoError::InvalidExport { format: "JPEG", reason: "quality must be between 1 and 100" });
        }
    }
    if image.width == 0 || image.height == 0 || image.data.len() != image.width as usize * image.height as usize * 3 { return Err(ImageIoError::InvalidImage); }
    let name = cpath(path)?;
    let space = CString::new(options.color_space).map_err(|_| ImageIoError::InvalidPath)?;
    let icc = icc_profile(options.color_space, options.cctf_encoding).unwrap_or(&[]);
    #[cfg(feature = "precision-f64")]
    let data = image.data.as_slice();
    #[cfg(not(feature = "precision-f64"))]
    let data: Vec<f64> = image.data.iter().copied().map(to_f64).collect();
    let mut error = std::ptr::null_mut();
    let jpeg_subsampling = match options.jpeg_subsampling.unwrap_or(JpegSubsampling::Yuv444) { JpegSubsampling::Yuv444 => 444, JpegSubsampling::Yuv420 => 420 };
    let compression = match options.compression.unwrap_or(Compression::Zip) { Compression::Zip => 1, Compression::None => 0 };
    if unsafe { sf_image_save(name.as_ptr(), image.width, image.height, data.as_ptr(), options.depth.bits().into(), format as i32, options.jpeg_quality.map_or(0, i32::from), jpeg_subsampling, compression, space.as_ptr(), icc.as_ptr(), icc.len(), &mut error) } == 0 {
        return Err(ImageIoError::Native { operation: "save", path: path.display().to_string(), message: take_error(error) });
    }
    let mut report = SaveReport::default();
    if format != ImageFormat::Exr {
        let source = metadata.map_or(std::ptr::null(), |m| m.0.0.cast_const());
        if unsafe { sf_metadata_write(name.as_ptr(), source, image.width, image.height, space.as_ptr(), options.cctf_encoding, &mut error) } == 0 { report.metadata_warning = Some(take_error(error)); }
    }
    Ok(report)
}

/// Convert retained simulation output into an independently selected saving
/// space. No clipping or gamut mapping occurs here; the output boundary owns
/// integer clipping. Decode once, adapt white with CAT02, then encode once.
pub fn convert_image(image: &ImageBuf, source: &str, source_encoded: bool, destination: &str, destination_encoded: bool) -> Result<ImageBuf, ImageIoError> {
    use spektrafilm_math::colorspace::{resolve, conversion_matrix, convert_rgb};
    use rayon::prelude::*;
    let source = resolve(source).map_err(ImageIoError::InvalidColorSpace)?;
    let destination = resolve(destination).map_err(ImageIoError::InvalidColorSpace)?;
    // Pinned Python skips the transform entirely when the saving space and
    // encoding already match the output layer (controller.py save guard).
    // Re-running the 4-digit sRGB round trip would shift pixels by ~2e-5.
    if source.name == destination.name && source_encoded == destination_encoded {
        return Ok(image.clone());
    }
    let matrix = conversion_matrix(source, destination);
    let mut output = image.clone();
    output.data.par_chunks_exact_mut(3).for_each(|pixel| {
        let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        let converted = convert_rgb(rgb, source, source_encoded, destination, destination_encoded, &matrix);
        for channel in 0..3 { pixel[channel] = from_f64(converted[channel]); }
    });
    Ok(output)
}
/// The pinned upstream mapping intentionally has no linear P3 profile.
pub fn icc_profile(space: &str, encoded: bool) -> Option<&'static [u8]> {
    macro_rules! profile { ($path:literal) => { Some(include_bytes!(concat!("../../../data/icc/", $path)).as_slice()) }; }
    match (space, encoded) {
        ("sRGB", true) => profile!("ellelstone/sRGB-elle-V2-srgbtrc.icc"),
        ("sRGB", false) => profile!("ellelstone/sRGB-elle-V2-g10.icc"),
        ("Adobe RGB (1998)", true) => profile!("ellelstone/ClayRGB-elle-V2-g22.icc"),
        ("Adobe RGB (1998)", false) => profile!("ellelstone/ClayRGB-elle-V2-g10.icc"),
        ("ProPhoto RGB", true) => profile!("ellelstone/LargeRGB-elle-V2-g18.icc"),
        ("ProPhoto RGB", false) => profile!("ellelstone/LargeRGB-elle-V2-g10.icc"),
        ("ITU-R BT.2020", true) => profile!("ellelstone/Rec2020-elle-V2-rec709.icc"),
        ("ITU-R BT.2020", false) => profile!("ellelstone/Rec2020-elle-V2-g10.icc"),
        ("ACES2065-1", _) => profile!("ellelstone/ACES-elle-V2-g10.icc"),
        ("Display P3", true) => profile!("saucecontrol/DisplayP3-v2-micro.icc"),
        ("DCI-P3", true) => profile!("saucecontrol/DCI-P3-v4.icc"),
        _ => None,
    }
}
