//! Shared prepared-image I/O. Loading and writing retain sample interpretation;
//! saving-space conversion is an explicit operation separate from simulation.
use spektrafilm_backend::telemetry::{
    IssueBoundary, IssueCategory, ObservationContext, ObservationKind, Purpose,
};
use spektrafilm_backend::{
    ComputeBackend, DeviceImage, OutputSampleData, OutputSampleGpuParams, OutputSampleKind,
    OutputSampleValues, SampleDomain,
};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f64, to_f64},
};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, thiserror::Error)]
pub enum ImageIoError {
    #[error("Unsupported image extension '{0}'; use JPEG, PNG, TIFF or EXR")]
    UnsupportedFormat(String),
    #[error("Unsupported depth {0}; select 8, 16 or 32 bits")]
    UnsupportedDepth(u8),
    #[error("EXR requires 16-bit half or 32-bit float, received {0} bits")]
    ExrDepth(u8),
    #[error("Invalid {format} export settings: {reason}")]
    InvalidExport {
        format: &'static str,
        reason: SaveOptionViolation,
    },
    #[error("Image path contains a null byte")]
    InvalidPath,
    #[error("Invalid RGB buffer dimensions or sample count")]
    InvalidImage,
    #[error("unknown RGB colour space {0:?}")]
    InvalidColorSpace(String),
    #[error("Saving conversion failed: {0}")]
    Convert(String),
    #[error("Converted samples are {got:?}, but {format} {depth}-bit output requires {expected:?}")]
    SampleKind {
        format: &'static str,
        depth: u8,
        expected: OutputSampleKind,
        got: OutputSampleKind,
    },
    #[error("Image {operation} failed for {path}: {message}")]
    Native {
        operation: &'static str,
        path: String,
        message: BridgeError,
    },
}

/// Authored bridge failures and genuine codec/OS detail travel independently.
/// Display preserves the original CLI diagnostic verbatim.
#[derive(Debug, Clone)]
pub struct BridgeError {
    original: String,
    failure_codes: u32,
    diagnostic_detail: String,
}
impl BridgeError {
    pub fn reasons(&self) -> impl Iterator<Item = BridgeReason> {
        BridgeReason::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, reason)| {
                (self.failure_codes & (1 << index) != 0).then_some(reason)
            })
    }
    pub fn diagnostic_detail(&self) -> &str {
        if self.failure_codes == 0 {
            &self.original
        } else {
            &self.diagnostic_detail
        }
    }
}
impl From<String> for BridgeError {
    fn from(value: String) -> Self {
        Self {
            original: value,
            failure_codes: 0,
            diagnostic_detail: String::new(),
        }
    }
}
impl From<&str> for BridgeError {
    fn from(value: &str) -> Self {
        value.to_owned().into()
    }
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.original)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeReason {
    OpenInput,
    TooSmall,
    Container,
    XmpInitialization,
    Dimensions,
    OpenOutput,
    OutputPlugin,
    CheckedWrites,
    OpenContainer,
    WriteData,
    FinalizeContainer,
    FinalizeFile,
    UnreadableAfterMetadata,
    InvalidAfterMetadata,
    IncompleteAfterMetadata,
    FinalizeRead,
    Timestamp,
    MissingDiagnostic,
}
impl BridgeReason {
    const ALL: [Self; 18] = [
        Self::OpenInput,
        Self::TooSmall,
        Self::Container,
        Self::XmpInitialization,
        Self::Dimensions,
        Self::OpenOutput,
        Self::OutputPlugin,
        Self::CheckedWrites,
        Self::OpenContainer,
        Self::WriteData,
        Self::FinalizeContainer,
        Self::FinalizeFile,
        Self::UnreadableAfterMetadata,
        Self::InvalidAfterMetadata,
        Self::IncompleteAfterMetadata,
        Self::FinalizeRead,
        Self::Timestamp,
        Self::MissingDiagnostic,
    ];
}

/// A complete unpublished image. Dropping it removes only its owned temporary file.
#[derive(Debug)]
pub struct PreparedImageOutput {
    temporary: tempfile::TempPath,
    destination: PathBuf,
    report: SaveReport,
}

impl PreparedImageOutput {
    pub fn report(&self) -> &SaveReport {
        &self.report
    }

    /// Atomically publish, optionally replacing an existing destination.
    pub fn publish(self, replace: bool) -> Result<SaveReport, ImageIoError> {
        let Self {
            temporary,
            destination,
            report,
        } = self;
        let result = if replace {
            temporary.persist(&destination)
        } else {
            temporary.persist_noclobber(&destination)
        };
        result.map_err(|error| ImageIoError::Native {
            operation: "publication",
            path: destination.display().to_string(),
            message: error.error.to_string().into(),
        })?;
        Ok(report)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Eight,
    Sixteen,
    ThirtyTwo,
}
impl BitDepth {
    pub fn bits(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
            Self::ThirtyTwo => 32,
        }
    }
}
impl TryFrom<u8> for BitDepth {
    type Error = ImageIoError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            8 => Ok(Self::Eight),
            16 => Ok(Self::Sixteen),
            32 => Ok(Self::ThirtyTwo),
            _ => Err(ImageIoError::UnsupportedDepth(value)),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegSubsampling {
    Yuv444,
    Yuv420,
}
impl JpegSubsampling {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yuv444 => "4:4:4",
            Self::Yuv420 => "4:2:0",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Zip,
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ImageFormat {
    Jpeg = 0,
    Png = 1,
    Tiff = 2,
    Exr = 3,
}

/// Canonical lowercase container tag shared by every CLI report, GUI label
/// and state document. The single spelling for each container lives here so a
/// caller cannot invent a second one.
impl ImageFormat {
    pub fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }

    /// Every extension this container owns, in loader order.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Tiff => &["tif", "tiff"],
            Self::Exr => &["exr"],
        }
    }

    /// The extension a writer appends when the destination carries none.
    pub fn extension(self) -> &'static str {
        self.extensions()[0]
    }
}

/// Container extensions the loader resolves, paired with their format. The
/// file dialog and `detect` read this one table, so a new container cannot be
/// offered by the dialog without being loadable, or the reverse.
const CONTAINER_FORMATS: &[(&str, ImageFormat)] = &[
    ("jpg", ImageFormat::Jpeg),
    ("jpeg", ImageFormat::Jpeg),
    ("png", ImageFormat::Png),
    ("tif", ImageFormat::Tiff),
    ("tiff", ImageFormat::Tiff),
    ("exr", ImageFormat::Exr),
];

/// RAW extensions LibRaw decodes.
const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw",
    "x3f", "iiq", "3fr", "crw", "rwl", "mrw", "mef", "kdc", "ari", "bay", "dcr", "drf", "erf",
    "fff", "k25", "mos", "ptx",
];

/// Every extension a container format owns, for a writer's file-dialog filter.
pub fn container_extensions() -> Vec<&'static str> {
    CONTAINER_FORMATS.iter().map(|(name, _)| *name).collect()
}

/// Every extension the input path accepts, containers first: the file-dialog
/// filter list.
pub fn accepted_extensions() -> Vec<&'static str> {
    container_extensions()
        .into_iter()
        .chain(RAW_EXTENSIONS.iter().copied())
        .collect()
}

impl ImageFormat {
    pub fn detect(path: &Path) -> Result<Self, ImageIoError> {
        let ext = extension(path);
        CONTAINER_FORMATS
            .iter()
            .find(|(name, _)| *name == ext)
            .map(|(_, format)| *format)
            .ok_or(ImageIoError::UnsupportedFormat(ext))
    }
}
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
pub fn is_raw(path: &Path) -> bool {
    RAW_EXTENSIONS.contains(&extension(path).as_str())
}
#[derive(Debug)]
struct NativeMetadata(*mut c_void);
// Exiv2 metadata is immutable after read, shared by save calls as const data.
unsafe impl Send for NativeMetadata {}
unsafe impl Sync for NativeMetadata {}
impl Drop for NativeMetadata {
    fn drop(&mut self) {
        unsafe { sf_metadata_free(self.0) }
    }
}
#[derive(Debug, Clone)]
pub struct ImageMetadata(Arc<NativeMetadata>);
pub struct LoadedImage {
    pub image: ImageBuf,
    pub metadata: Option<ImageMetadata>,
}
#[derive(Debug, Clone, Copy)]
pub struct SaveOptions<'a> {
    pub depth: BitDepth,
    pub color_space: &'a str,
    pub cctf_encoding: bool,
    pub jpeg_quality: Option<u8>,
    pub jpeg_subsampling: Option<JpegSubsampling>,
    pub compression: Option<Compression>,
}

/// One way a container/option pair breaks the writer contract. The rules live
/// here once; callers that reject map each variant to their own wording, and
/// the GUI export dialog maps them to the coerced selection instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOptionViolation {
    /// JPEG/PNG require 8-bit, EXR requires 16- or 32-bit.
    Depth,
    /// JPEG and PNG must write encoded samples.
    RequiresEncoded,
    /// EXR must write linear samples.
    RequiresLinear,
    /// EXR only carries sRGB or ACES2065-1.
    ExrColorSpace,
    /// JPEG quality is set for a container that does not carry it.
    JpegQualityNotApplicable,
    /// JPEG subsampling is set for a container that does not carry it.
    JpegSubsamplingNotApplicable,
    /// JPEG quality is outside 1..=100.
    JpegQualityRange,
    /// Compression is set for JPEG or PNG.
    CompressionNotConfigurable,
    /// EXR compression is always zip.
    ExrCompression,
}

impl std::fmt::Display for SaveOptionViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Depth => "only 8-bit output is supported",
            Self::RequiresEncoded => "JPEG and PNG require encoded output",
            Self::RequiresLinear => "EXR output must be linear",
            Self::ExrColorSpace => "EXR color space must be sRGB or ACES2065-1",
            Self::JpegQualityNotApplicable => "JPEG quality is only valid for JPEG",
            Self::JpegSubsamplingNotApplicable => "JPEG subsampling is only valid for JPEG",
            Self::CompressionNotConfigurable => "compression is not configurable for JPEG or PNG",
            Self::ExrCompression => "EXR compression is always zip",
            Self::JpegQualityRange => "quality must be between 1 and 100",
        })
    }
}

/// Every writer-contract violation of one [`SaveOptions`] value, in the order
/// the writer used to check them so the first error stays stable. `rendered_output`
/// selects the GUI save path, which preserves the selected transfer encoding for
/// every format instead of requiring linear EXR and encoded JPEG/PNG.
pub fn save_option_violations(
    format: ImageFormat,
    options: &SaveOptions<'_>,
    rendered_output: bool,
) -> Vec<SaveOptionViolation> {
    let mut violations = Vec::new();
    if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) && options.depth != BitDepth::Eight {
        violations.push(SaveOptionViolation::Depth);
    }
    if format == ImageFormat::Exr && options.depth == BitDepth::Eight {
        violations.push(SaveOptionViolation::Depth);
    }
    if !rendered_output
        && matches!(format, ImageFormat::Jpeg | ImageFormat::Png)
        && !options.cctf_encoding
    {
        violations.push(SaveOptionViolation::RequiresEncoded);
    }
    if !rendered_output && format == ImageFormat::Exr && options.cctf_encoding {
        violations.push(SaveOptionViolation::RequiresLinear);
    }
    if !rendered_output
        && format == ImageFormat::Exr
        && !matches!(options.color_space, "sRGB" | "ACES2065-1")
    {
        violations.push(SaveOptionViolation::ExrColorSpace);
    }
    if format != ImageFormat::Jpeg && options.jpeg_quality.is_some() {
        violations.push(SaveOptionViolation::JpegQualityNotApplicable);
    }
    if format != ImageFormat::Jpeg && options.jpeg_subsampling.is_some() {
        violations.push(SaveOptionViolation::JpegSubsamplingNotApplicable);
    }
    if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) && options.compression.is_some() {
        violations.push(SaveOptionViolation::CompressionNotConfigurable);
    }
    if format == ImageFormat::Exr && matches!(options.compression, Some(Compression::None)) {
        violations.push(SaveOptionViolation::ExrCompression);
    }
    if let Some(quality) = options.jpeg_quality
        && !(1..=100).contains(&quality)
    {
        violations.push(SaveOptionViolation::JpegQualityRange);
    }
    violations
}

/// The writer rules a container imposes on one [`SaveOptions`] value,
/// independent of the destination path. Callers that resolve a container from
/// a path (the CLI's `--output`) pass `ImageFormat::detect(path)`; callers that
/// already hold the container pass it directly. `rendered_output` selects the
/// GUI save path, which preserves the selected transfer encoding for every
/// format instead of requiring linear EXR and encoded JPEG/PNG.
pub fn validate_save_options(
    format: ImageFormat,
    options: &SaveOptions<'_>,
    rendered_output: bool,
) -> Result<(), ImageIoError> {
    let name = format.name();
    match save_option_violations(format, options, rendered_output).first() {
        None => Ok(()),
        Some(SaveOptionViolation::Depth) if format == ImageFormat::Exr => {
            Err(ImageIoError::ExrDepth(8))
        }
        Some(SaveOptionViolation::Depth) => Err(ImageIoError::InvalidExport {
            format: name,
            reason: SaveOptionViolation::Depth,
        }),
        Some(SaveOptionViolation::RequiresEncoded) => Err(ImageIoError::InvalidExport {
            format: name,
            reason: SaveOptionViolation::RequiresEncoded,
        }),
        Some(SaveOptionViolation::RequiresLinear) => Err(ImageIoError::InvalidExport {
            format: "EXR",
            reason: SaveOptionViolation::RequiresLinear,
        }),
        Some(SaveOptionViolation::ExrColorSpace) => Err(ImageIoError::InvalidExport {
            format: "EXR",
            reason: SaveOptionViolation::ExrColorSpace,
        }),
        Some(SaveOptionViolation::JpegQualityNotApplicable) => Err(ImageIoError::InvalidExport {
            format: name,
            reason: SaveOptionViolation::JpegQualityNotApplicable,
        }),
        Some(SaveOptionViolation::JpegSubsamplingNotApplicable) => {
            Err(ImageIoError::InvalidExport {
                format: name,
                reason: SaveOptionViolation::JpegSubsamplingNotApplicable,
            })
        }
        Some(SaveOptionViolation::CompressionNotConfigurable) => Err(ImageIoError::InvalidExport {
            format: name,
            reason: SaveOptionViolation::CompressionNotConfigurable,
        }),
        Some(SaveOptionViolation::ExrCompression) => Err(ImageIoError::InvalidExport {
            format: "EXR",
            reason: SaveOptionViolation::ExrCompression,
        }),
        Some(SaveOptionViolation::JpegQualityRange) => Err(ImageIoError::InvalidExport {
            format: "JPEG",
            reason: SaveOptionViolation::JpegQualityRange,
        }),
    }
}
#[derive(Debug, Default)]
pub struct SaveReport {
    pub metadata_warning: Option<BridgeError>,
}
unsafe extern "C" {
    fn sf_io_free(pointer: *mut c_void);
    fn sf_io_take_failure(codes: *mut u32, detail: *mut *mut c_char);
    fn sf_metadata_free(pointer: *mut c_void);
    fn sf_metadata_read(path: *const c_char) -> *mut c_void;
    fn sf_image_load(
        path: *const c_char,
        width: *mut u32,
        height: *mut u32,
        samples: *mut *mut f64,
        error: *mut *mut c_char,
    ) -> i32;
    fn sf_image_save(
        path: *const c_char,
        width: u32,
        height: u32,
        samples: *const f64,
        depth: i32,
        format: i32,
        jpeg_quality: i32,
        jpeg_subsampling: i32,
        compression: i32,
        color_space: *const c_char,
        export_metadata: bool,
        icc: *const u8,
        icc_len: usize,
        error: *mut *mut c_char,
    ) -> i32;
    fn sf_image_validate(path: *const c_char, error: *mut *mut c_char) -> i32;
    fn sf_image_save_samples(
        path: *const c_char,
        width: u32,
        height: u32,
        samples: *const c_void,
        sample_kind: i32,
        depth: i32,
        format: i32,
        jpeg_quality: i32,
        jpeg_subsampling: i32,
        compression: i32,
        color_space: *const c_char,
        export_metadata: bool,
        icc: *const u8,
        icc_len: usize,
        error: *mut *mut c_char,
    ) -> i32;
    fn sf_metadata_write(
        path: *const c_char,
        source: *const c_void,
        width: u32,
        height: u32,
        space: *const c_char,
        encoded: bool,
        error: *mut *mut c_char,
    ) -> i32;
}
fn cpath(path: &Path) -> Result<CString, ImageIoError> {
    CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| ImageIoError::InvalidPath)
}
fn take_string(pointer: *mut c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let result = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { sf_io_free(pointer.cast()) };
    result
}
fn take_error(pointer: *mut c_char) -> BridgeError {
    let mut failure_codes = 0;
    let mut detail = std::ptr::null_mut();
    // Capture and clear on the executing thread before freeing error storage
    // or making any other foreign call.
    unsafe { sf_io_take_failure(&mut failure_codes, &mut detail) };
    let mut original = take_string(pointer);
    let diagnostic_detail = take_string(detail);
    if original.is_empty() {
        original = "Native codec did not provide an error".into();
        failure_codes |= 1 << 17;
    }
    BridgeError {
        original,
        failure_codes,
        diagnostic_detail,
    }
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
    let (mut width, mut height, mut samples, mut error) =
        (0, 0, std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe {
        sf_image_load(
            name.as_ptr(),
            &mut width,
            &mut height,
            &mut samples,
            &mut error,
        )
    } == 0
    {
        return Err(ImageIoError::Native {
            operation: "load",
            path: path.display().to_string(),
            message: take_error(error),
        });
    }
    let count = width as usize * height as usize * 3;
    let data = unsafe { std::slice::from_raw_parts(samples, count) }
        .iter()
        .copied()
        .map(from_f64)
        .collect();
    unsafe { sf_io_free(samples.cast()) };
    Ok(LoadedImage {
        image: ImageBuf::from_data(width, height, data),
        metadata: read_metadata(path),
    })
}
/// Integer outputs clip to [0,1] and truncate after scaling, matching numpy
/// astype. PNG/JPEG always use uint8; float TIFF/EXR preserve unbounded samples.
/// Pixel failures are errors; metadata failures report a warning after writing.
pub fn save(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
) -> Result<SaveReport, ImageIoError> {
    save_inner(path, image, options, metadata, None)
}

/// Write pixels with metadata observations attributed to the caller's file-write phase.
pub fn save_observed(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    context: &ObservationContext,
) -> Result<SaveReport, ImageIoError> {
    save_inner(
        path,
        image,
        options,
        metadata,
        context.enabled().then_some(context),
    )
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
    save_inner(path, image, options, metadata, None)
}

/// Prepare a complete export without publishing it, for cancellation or no-replace adapters.
pub fn prepare_save_observed(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    context: &ObservationContext,
) -> Result<PreparedImageOutput, ImageIoError> {
    prepare_inner(
        path,
        image,
        options,
        metadata,
        false,
        context.enabled().then_some(context),
    )
}

fn save_inner(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    observation: Option<&ObservationContext>,
) -> Result<SaveReport, ImageIoError> {
    prepare_inner(path, image, options, metadata, false, observation)?.publish(true)
}

/// Pixel payload one writer transaction packs. The reference payload is the
/// host-converted floating image whose integer clipping and quantization the
/// native writer owns; a converted payload already carries those operations
/// and is only packed into the container.
enum WriterPixels<'a> {
    Reference(&'a ImageBuf),
    Converted(&'a OutputSampleData),
}

impl WriterPixels<'_> {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Reference(image) => (image.width, image.height),
            Self::Converted(samples) => (samples.width, samples.height),
        }
    }
}

/// Sample representation a container/depth pair requires at the writer
/// boundary. The mapping lives once, so a caller cannot request an integer
/// container from floating samples or half samples for an 8-bit TIFF.
pub fn output_sample_kind(
    format: ImageFormat,
    depth: BitDepth,
) -> Result<OutputSampleKind, ImageIoError> {
    match (format, depth) {
        (ImageFormat::Jpeg | ImageFormat::Png, _) => Ok(OutputSampleKind::U8),
        (ImageFormat::Tiff, BitDepth::Eight) => Ok(OutputSampleKind::U8),
        (ImageFormat::Tiff, BitDepth::Sixteen) => Ok(OutputSampleKind::U16),
        (ImageFormat::Tiff, BitDepth::ThirtyTwo) => Ok(OutputSampleKind::F32),
        (ImageFormat::Exr, BitDepth::Sixteen) => Ok(OutputSampleKind::Half),
        (ImageFormat::Exr, BitDepth::ThirtyTwo) => Ok(OutputSampleKind::F32),
        (ImageFormat::Exr, BitDepth::Eight) => Err(ImageIoError::ExrDepth(8)),
    }
}

fn sample_kind_code(kind: OutputSampleKind) -> i32 {
    match kind {
        OutputSampleKind::U8 => 0,
        OutputSampleKind::U16 => 1,
        OutputSampleKind::Half => 2,
        OutputSampleKind::F32 => 3,
    }
}

fn sample_values_ptr(values: &OutputSampleValues) -> *const c_void {
    match values {
        OutputSampleValues::U8(values) => values.as_ptr().cast(),
        OutputSampleValues::U16(values) | OutputSampleValues::Half(values) => {
            values.as_ptr().cast()
        }
        OutputSampleValues::F32(values) => values.as_ptr().cast(),
    }
}

fn sample_values_len(values: &OutputSampleValues) -> usize {
    match values {
        OutputSampleValues::U8(values) => values.len(),
        OutputSampleValues::U16(values) | OutputSampleValues::Half(values) => values.len(),
        OutputSampleValues::F32(values) => values.len(),
    }
}

fn prepare_inner(
    path: &Path,
    image: &ImageBuf,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    rendered_output: bool,
    observation: Option<&ObservationContext>,
) -> Result<PreparedImageOutput, ImageIoError> {
    prepare_pixels(
        path,
        WriterPixels::Reference(image),
        options,
        metadata,
        rendered_output,
        observation,
    )
}

fn prepare_samples_inner(
    path: &Path,
    samples: &OutputSampleData,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    rendered_output: bool,
    observation: Option<&ObservationContext>,
) -> Result<PreparedImageOutput, ImageIoError> {
    prepare_pixels(
        path,
        WriterPixels::Converted(samples),
        options,
        metadata,
        rendered_output,
        observation,
    )
}

fn prepare_pixels(
    path: &Path,
    pixels: WriterPixels<'_>,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    rendered_output: bool,
    observation: Option<&ObservationContext>,
) -> Result<PreparedImageOutput, ImageIoError> {
    let format = ImageFormat::detect(path)?;
    validate_save_options(format, &options, rendered_output)?;
    let (width, height) = pixels.dimensions();
    if width == 0 || height == 0 {
        return Err(ImageIoError::InvalidImage);
    }
    let count = width as usize * height as usize * 3;
    match &pixels {
        WriterPixels::Reference(image) if image.data.len() != count => {
            return Err(ImageIoError::InvalidImage);
        }
        WriterPixels::Converted(samples) => {
            let expected = output_sample_kind(format, options.depth)?;
            if samples.kind != expected {
                return Err(ImageIoError::SampleKind {
                    format: format.name(),
                    depth: options.depth.bits(),
                    expected,
                    got: samples.kind,
                });
            }
            if sample_values_len(&samples.values) != count {
                return Err(ImageIoError::InvalidImage);
            }
        }
        WriterPixels::Reference(_) => {}
    }
    cpath(path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let suffix = format!(
        ".{}",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("tmp")
    );
    let temporary = tempfile::Builder::new()
        .prefix(".spektrafilm-")
        .suffix(&suffix)
        .tempfile_in(parent)
        .map_err(|error| ImageIoError::Native {
            operation: "staging",
            path: path.display().to_string(),
            message: error.to_string().into(),
        })?
        .into_temp_path();
    let name = cpath(&temporary)?;
    let space = CString::new(options.color_space).map_err(|_| ImageIoError::InvalidPath)?;
    let icc = icc_profile(options.color_space, options.cctf_encoding).unwrap_or(&[]);
    let mut error = std::ptr::null_mut();
    let jpeg_subsampling = match options.jpeg_subsampling {
        Some(JpegSubsampling::Yuv444) => 444,
        Some(JpegSubsampling::Yuv420) => 420,
        None if rendered_output => 0,
        None => 444,
    };
    let compression = match options.compression.unwrap_or(Compression::Zip) {
        Compression::Zip => 1,
        Compression::None => 0,
    };
    let written = unsafe {
        match &pixels {
            WriterPixels::Reference(image) => {
                #[cfg(feature = "precision-f64")]
                let data = image.data.as_slice();
                #[cfg(not(feature = "precision-f64"))]
                let data: Vec<f64> = image.data.iter().copied().map(to_f64).collect();
                sf_image_save(
                    name.as_ptr(),
                    width,
                    height,
                    data.as_ptr(),
                    options.depth.bits().into(),
                    format as i32,
                    options.jpeg_quality.map_or(0, i32::from),
                    jpeg_subsampling,
                    compression,
                    space.as_ptr(),
                    !rendered_output,
                    icc.as_ptr(),
                    icc.len(),
                    &mut error,
                )
            }
            WriterPixels::Converted(samples) => sf_image_save_samples(
                name.as_ptr(),
                width,
                height,
                sample_values_ptr(&samples.values),
                sample_kind_code(samples.kind),
                options.depth.bits().into(),
                format as i32,
                options.jpeg_quality.map_or(0, i32::from),
                jpeg_subsampling,
                compression,
                space.as_ptr(),
                !rendered_output,
                icc.as_ptr(),
                icc.len(),
                &mut error,
            ),
        }
    };
    if written == 0 {
        return Err(ImageIoError::Native {
            operation: "save",
            path: path.display().to_string(),
            message: take_error(error),
        });
    }
    let mut report = SaveReport::default();
    if format != ImageFormat::Exr {
        let metadata_scope = observation
            .map(|context| context.scope("metadata", ObservationKind::Phase, Purpose::Image));
        let source = metadata.map_or(std::ptr::null(), |m| m.0.0.cast_const());
        if unsafe {
            sf_metadata_write(
                name.as_ptr(),
                source,
                width,
                height,
                space.as_ptr(),
                options.cctf_encoding,
                &mut error,
            )
        } == 0
        {
            report.metadata_warning = Some(take_error(error));
            error = std::ptr::null_mut();
            if unsafe { sf_image_validate(name.as_ptr(), &mut error) } == 0 {
                return Err(ImageIoError::Native {
                    operation: "metadata finalization",
                    path: path.display().to_string(),
                    message: take_error(error),
                });
            }
            if let Some(context) = observation {
                context.issue(IssueCategory::MetadataWarning, IssueBoundary::Metadata);
            }
        }
        drop(metadata_scope);
    }
    Ok(PreparedImageOutput {
        temporary,
        destination: path.to_owned(),
        report,
    })
}

/// Convert retained simulation output into an independently selected saving
/// space. No clipping or gamut mapping occurs here; the output boundary owns
/// integer clipping. Decode once, adapt white with CAT02, then encode once.
pub fn convert_image(
    image: &ImageBuf,
    source: &str,
    source_encoded: bool,
    destination: &str,
    destination_encoded: bool,
) -> Result<ImageBuf, ImageIoError> {
    use rayon::prelude::*;
    use spektrafilm_math::colorspace::{conversion_matrix, convert_rgb, resolve};
    let source = resolve(source).map_err(|_| ImageIoError::InvalidColorSpace(source.to_owned()))?;
    let destination = resolve(destination)
        .map_err(|_| ImageIoError::InvalidColorSpace(destination.to_owned()))?;
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
        let converted = convert_rgb(
            rgb,
            source,
            source_encoded,
            destination,
            destination_encoded,
            &matrix,
        );
        for channel in 0..3 {
            pixel[channel] = from_f64(converted[channel]);
        }
    });
    Ok(output)
}

/// What the writer packs: the host reference conversion (whose integer
/// clipping and quantization the native writer owns), or samples the device
/// already converted, clipped, and quantized.
pub enum SavingPixels {
    Reference(ImageBuf),
    Converted(OutputSampleData),
}

impl SavingPixels {
    pub fn width(&self) -> u32 {
        match self {
            Self::Reference(image) => image.width,
            Self::Converted(samples) => samples.width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Reference(image) => image.height,
            Self::Converted(samples) => samples.height,
        }
    }

    /// Stage the pixels beside the destination and return the unpublished
    /// transaction. Metadata, container attributes, and atomic publication
    /// stay the writer's single implementation.
    pub fn prepare_observed(
        &self,
        path: &Path,
        options: SaveOptions<'_>,
        metadata: Option<&ImageMetadata>,
        rendered_output: bool,
        context: &ObservationContext,
    ) -> Result<PreparedImageOutput, ImageIoError> {
        let observation = context.enabled().then_some(context);
        match self {
            Self::Reference(image) => prepare_inner(
                path,
                image,
                options,
                metadata,
                rendered_output,
                observation,
            ),
            Self::Converted(samples) => prepare_samples_inner(
                path,
                samples,
                options,
                metadata,
                rendered_output,
                observation,
            ),
        }
    }
}

/// One saving request shared by the host and retained conversion routes.
fn saving_request(
    source_space: &spektrafilm_math::colorspace::RgbColorSpace,
    source_encoded: bool,
    destination_space: &spektrafilm_math::colorspace::RgbColorSpace,
    destination_encoded: bool,
    kind: OutputSampleKind,
) -> OutputSampleGpuParams {
    use spektrafilm_math::colorspace::conversion_matrix;
    // Pinned Python skips the transform entirely when the saving space and
    // encoding already match the output layer (controller.py save guard).
    // Sample conversion still runs: clipping and quantization are the
    // writer's, not the colour transform's.
    let same = source_space.name == destination_space.name && source_encoded == destination_encoded;
    let matrix = if same {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    } else {
        conversion_matrix(source_space, destination_space)
    };
    OutputSampleGpuParams {
        src_cctf: spektrafilm_backend::cctf_kind(source_space.cctf),
        decode: source_encoded && !same,
        dst_cctf: spektrafilm_backend::cctf_kind(destination_space.cctf),
        encode: destination_encoded && !same,
        matrix: std::array::from_fn(|i| std::array::from_fn(|j| matrix[i][j] as f32)),
        kind,
    }
}

fn checked_samples(
    samples: OutputSampleData,
    width: u32,
    height: u32,
    kind: OutputSampleKind,
) -> Result<OutputSampleData, ImageIoError> {
    let count = width as usize * height as usize * 3;
    if samples.width != width
        || samples.height != height
        || samples.kind != kind
        || sample_values_len(&samples.values) != count
    {
        return Err(ImageIoError::Convert(
            "saving conversion returned samples that do not match the requested image".into(),
        ));
    }
    Ok(samples)
}

/// Saving color and sample conversion for host output, executed on the
/// backend when it provides the device path.
///
/// The reference pixels are used only by a backend that reports itself as CPU:
/// once a GPU operation is selected, a missing device conversion or a device
/// failure fails the save instead of silently demoting it.
pub fn convert_samples_with_backend(
    image: &ImageBuf,
    source: &str,
    source_encoded: bool,
    destination: &str,
    destination_encoded: bool,
    kind: OutputSampleKind,
    backend: &dyn ComputeBackend,
) -> Result<SavingPixels, ImageIoError> {
    use spektrafilm_math::colorspace::resolve;
    let source_space = resolve(source).map_err(ImageIoError::InvalidColorSpace)?;
    let destination_space = resolve(destination).map_err(ImageIoError::InvalidColorSpace)?;
    let params = saving_request(
        source_space,
        source_encoded,
        destination_space,
        destination_encoded,
        kind,
    );
    match backend.convert_image_samples(image, &params) {
        Ok(Some(samples)) => Ok(SavingPixels::Converted(checked_samples(
            samples,
            image.width,
            image.height,
            kind,
        )?)),
        Ok(None) if backend.is_gpu() => Err(ImageIoError::Convert(
            "the selected GPU backend has no saving sample conversion".into(),
        )),
        Ok(None) => Ok(SavingPixels::Reference(convert_image(
            image,
            source,
            source_encoded,
            destination,
            destination_encoded,
        )?)),
        Err(error) => Err(ImageIoError::Convert(format!("saving conversion: {error}"))),
    }
}

/// Saving color and sample conversion for a retained device output.
///
/// A retained carrier is converted in place on the device: the samples are
/// never downloaded and re-uploaded to cross the saving boundary. A host
/// carrier keeps the reference conversion above.
pub fn convert_retained_samples_with_backend(
    image: &DeviceImage,
    destination: &str,
    destination_encoded: bool,
    kind: OutputSampleKind,
    backend: &dyn ComputeBackend,
) -> Result<SavingPixels, ImageIoError> {
    use spektrafilm_math::colorspace::resolve;
    let destination_space = resolve(destination).map_err(ImageIoError::InvalidColorSpace)?;
    let source_space = image.space();
    let source_encoded = match image.domain() {
        SampleDomain::Encoded => true,
        SampleDomain::Linear => false,
        SampleDomain::Density | SampleDomain::LogExposure => {
            return Err(ImageIoError::Convert(
                "retained output is a diagnostic exposure/density domain, not RGB samples".into(),
            ));
        }
    };
    if !image.is_retained() {
        let host = image.to_host(backend).map_err(ImageIoError::Convert)?;
        return convert_samples_with_backend(
            &host,
            source_space.name,
            source_encoded,
            destination,
            destination_encoded,
            kind,
            backend,
        );
    }
    let params = saving_request(
        source_space,
        source_encoded,
        destination_space,
        destination_encoded,
        kind,
    );
    match backend.convert_retained_samples(image, &params) {
        Ok(Some(samples)) => Ok(SavingPixels::Converted(checked_samples(
            samples,
            image.width(),
            image.height(),
            kind,
        )?)),
        Ok(None) if backend.is_gpu() => Err(ImageIoError::Convert(
            "the selected GPU backend has no saving sample conversion".into(),
        )),
        Ok(None) => {
            let host = image.to_host(backend).map_err(ImageIoError::Convert)?;
            Ok(SavingPixels::Reference(convert_image(
                &host,
                source_space.name,
                source_encoded,
                destination,
                destination_encoded,
            )?))
        }
        Err(error) => Err(ImageIoError::Convert(format!("saving conversion: {error}"))),
    }
}

/// Prepare a save from already converted writer samples.
pub fn prepare_save_samples_observed(
    path: &Path,
    samples: &OutputSampleData,
    options: SaveOptions<'_>,
    metadata: Option<&ImageMetadata>,
    context: &ObservationContext,
) -> Result<PreparedImageOutput, ImageIoError> {
    prepare_samples_inner(
        path,
        samples,
        options,
        metadata,
        false,
        context.enabled().then_some(context),
    )
}

/// The pinned upstream mapping intentionally has no linear P3 profile.
pub fn icc_profile(space: &str, encoded: bool) -> Option<&'static [u8]> {
    macro_rules! profile {
        ($path:literal) => {
            Some(include_bytes!(concat!("../../../data/icc/", $path)).as_slice())
        };
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A disabled context keeps the preparation path identical to the
    /// observed one while exercising the shared boundary.
    fn context() -> ObservationContext {
        ObservationContext::new(spektrafilm_backend::telemetry::CollectionMode::Off)
    }

    fn options() -> SaveOptions<'static> {
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        }
    }

    /// A retained CMYK result must survive the Save boundary as the same
    /// three-channel RGB pixels: the saving-space conversion must not
    /// re-encode, re-run the appearance, or drop signed headroom.
    #[test]
    fn saving_space_conversion_preserves_the_retained_rgb_result() {
        // Same space and same transfer state is the pinned save guard:
        // the retained result is returned unchanged, not re-encoded.
        let image =
            ImageBuf::from_data(2, 1, [-0.2, 0.4, 1.5, 0.9, 0.1, 0.3].map(from_f64).to_vec());
        let same = convert_image(&image, "sRGB", true, "sRGB", true).unwrap();
        assert_eq!(same.data, image.data);
        assert_eq!(same.width, image.width);
        assert_eq!(same.height, image.height);
        // A different saving space still returns three-channel RGB with the
        // headroom intact, so the writer — not this boundary — owns clipping.
        let other = convert_image(&image, "sRGB", true, "ACES2065-1", false).unwrap();
        assert_eq!(other.data.len(), image.data.len());
        assert!(other.data.iter().all(|v| to_f64(*v).is_finite()));
        assert!(other.data.iter().any(|v| to_f64(*v) > 1.0));
    }

    /// The Save path prepares a complete file without publishing it: a
    /// failed or cancelled write must leave the destination untouched, and
    /// a no-clobber publish must refuse to replace an existing file.
    #[test]
    fn prepared_save_never_touches_the_destination_until_publication() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("retained.png");
        let image = ImageBuf::from_data(1, 1, [0.25, 0.5, 0.75].map(from_f64).to_vec());
        let context = context();

        let prepared = prepare_save_observed(&destination, &image, options(), None, &context)
            .expect("prepare a complete file");
        // Preparing is not publishing: nothing reaches the destination yet.
        assert!(!destination.exists());
        // Dropping the prepared output removes only its temporary file and
        // still leaves the destination absent.
        drop(prepared);
        assert!(!destination.exists());

        // Publishing with replace=false creates the file, and a second
        // no-clobber publish over an existing destination is refused, so a
        // retained result can never be silently overwritten.
        let prepared =
            prepare_save_observed(&destination, &image, options(), None, &context).unwrap();
        prepared.publish(false).unwrap();
        assert!(destination.exists());
        let before = std::fs::read(&destination).unwrap();
        let second =
            prepare_save_observed(&destination, &image, options(), None, &context).unwrap();
        assert!(
            second.publish(false).is_err(),
            "a no-clobber publish must refuse an existing destination"
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            before,
            "a refused publication must leave the retained file untouched"
        );
    }

    /// The retained-result boundary must reject a writer-contract violation
    /// before any file is created, so an invalid save never leaves a
    /// partial artifact behind.
    #[test]
    fn invalid_save_options_are_rejected_before_any_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("retained.png");
        let image = ImageBuf::from_data(1, 1, [0.25, 0.5, 0.75].map(from_f64).to_vec());
        let context = context();
        // PNG cannot carry linear samples.
        let linear = SaveOptions {
            cctf_encoding: false,
            ..options()
        };
        assert!(
            prepare_save_observed(&destination, &image, linear, None, &context).is_err(),
            "a linear PNG must be rejected at the writer boundary"
        );
        assert!(!destination.exists());
    }
}
