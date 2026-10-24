//! Image-independent LittleCMS preparation for fixed RGB8 device kernels.
use lcms2::{Intent, PixelFormat, Profile, Transform};
use sha2::{Digest, Sha256};
use spektrafilm_backend::{ViewerIccGpuParams, ViewerIccKind};
use std::{ffi::c_void, sync::Arc};

#[path = "icc_pipeline/cache.rs"]
mod cache;

pub(crate) struct ResolvedIccPipeline {
    identity: [u8; 32],
    kind: ViewerIccKind,
    words: Vec<u32>,
}
impl ResolvedIccPipeline {
    pub(crate) fn params(&self) -> ViewerIccGpuParams<'_> {
        ViewerIccGpuParams {
            identity: self.identity,
            kind: self.kind,
            words: &self.words,
        }
    }
}

#[repr(C)]
struct CurveSet {
    count: u32,
    curves: *const *const c_void,
}
#[repr(C)]
struct Matrix {
    values: *const f64,
    offset: *const f64,
}
unsafe extern "C" {
    fn cmsGetTransformPipeline(transform: *const c_void) -> *const c_void;
    fn cmsPipelineGetPtrToFirstStage(pipeline: *const c_void) -> *const c_void;
    fn cmsStageNext(stage: *const c_void) -> *const c_void;
    fn cmsStageType(stage: *const c_void) -> u32;
    fn cmsStageInputChannels(stage: *const c_void) -> u32;
    fn cmsStageOutputChannels(stage: *const c_void) -> u32;
    fn cmsStageData(stage: *const c_void) -> *const c_void;
    fn cmsEvalToneCurveFloat(curve: *const c_void, value: f32) -> f32;
}

fn profile_identity(hash: &mut Sha256, profile: &Profile) -> Result<(), String> {
    let bytes = profile.icc().map_err(|e| e.to_string())?;
    if bytes.len() < 128 {
        return Err("ICC profile header is truncated".into());
    }
    hash.update((bytes.len() as u64).to_le_bytes());
    // A generated sRGB profile has a fresh creation time and profile ID.
    // Neither field changes the conversion; all conversion-bearing bytes do.
    hash.update(&bytes[..24]);
    hash.update(&bytes[36..84]);
    hash.update(&bytes[100..]);
    Ok(())
}

pub(super) fn resolve(
    source: &Profile,
    destination: &Profile,
) -> Result<Arc<ResolvedIccPipeline>, String> {
    let mut hash = Sha256::new();
    hash.update(b"spektrafilm-viewer-icc-2:RGB8:perceptual:flags0:q14-or-exact");
    hash.update(lcms2::version().to_le_bytes());
    profile_identity(&mut hash, source)?;
    profile_identity(&mut hash, destination)?;
    let identity = hash.finalize().into();
    cache::get(identity, || prepare(identity, source, destination))
}

fn prepare(
    identity: [u8; 32],
    source: &Profile,
    destination: &Profile,
) -> Result<ResolvedIccPipeline, String> {
    let transform: Transform<[u8; 3], [u8; 3]> = Transform::new(
        source,
        PixelFormat::RGB_8,
        destination,
        PixelFormat::RGB_8,
        Intent::Perceptual,
    )
    .map_err(|e| format!("ICC transform preparation: {e}"))?;
    // lcms2::Transform is repr(transparent) over its native handle. Borrow it
    // only while the Rust transform owns the pipeline and its stage data.
    let handle = unsafe {
        std::ptr::from_ref(&transform)
            .cast::<*const c_void>()
            .read()
    };
    let pipeline = unsafe { cmsGetTransformPipeline(handle) };
    let (kind, words) = prepare_transform(&transform, pipeline);
    Ok(ResolvedIccPipeline {
        identity,
        kind,
        words,
    })
}

fn prepare_transform(
    transform: &Transform<[u8; 3], [u8; 3]>,
    pipeline: *const c_void,
) -> (ViewerIccKind, Vec<u32>) {
    // Some valid optimized transforms do not expose a stage pipeline. The
    // transform itself still defines every RGB8 result; never infer identity.
    if pipeline.is_null() {
        return (ViewerIccKind::ExactRgb8, exact_rgb8(transform));
    }
    let first = unsafe { cmsPipelineGetPtrToFirstStage(pipeline) };
    if first.is_null() {
        (ViewerIccKind::Identity, vec![0])
    } else if stage_is(first, 0x63767374) && unsafe { cmsStageNext(first) }.is_null() {
        let mut pixels = std::array::from_fn::<_, 256, _>(|i| [i as u8; 3]);
        transform.transform_in_place(&mut pixels);
        let mut words = vec![0; ViewerIccKind::Curves.word_count()];
        for channel in 0..3 {
            for (i, pixel) in pixels.iter().enumerate() {
                put_byte(&mut words, channel * 256 + i, pixel[channel]);
            }
        }
        (ViewerIccKind::Curves, words)
    } else if let Some(words) = matrix_shaper(first, transform) {
        (ViewerIccKind::MatrixShaper, words)
    } else {
        (ViewerIccKind::ExactRgb8, exact_rgb8(transform))
    }
}

fn stage_is(stage: *const c_void, kind: u32) -> bool {
    !stage.is_null()
        && unsafe {
            cmsStageType(stage) == kind
                && cmsStageInputChannels(stage) == 3
                && cmsStageOutputChannels(stage) == 3
        }
}

fn q14(value: f64) -> Option<i32> {
    let value = (value * 16384.0 + 0.5).floor();
    (value.is_finite() && value >= i32::MIN as f64 && value <= i32::MAX as f64)
        .then_some(value as i32)
}

fn matrix_shaper(
    first: *const c_void,
    transform: &Transform<[u8; 3], [u8; 3]>,
) -> Option<Vec<u32>> {
    // This evaluator and its Q14 quantization match the pinned LCMS 2.19
    // cmsopt.c. Other CMM versions retain exactness through exhaustive RGB8.
    // The optimized formatter bit alone does not identify an evaluator, so
    // also require the complete three-stage RGB chain below.
    if lcms2::version() != 2190
        || transform.output_format().0 & (1 << 21) == 0
        || !stage_is(first, 0x63767374)
    {
        return None;
    }
    let matrix = unsafe { cmsStageNext(first) };
    if !stage_is(matrix, 0x6d617466) {
        return None;
    }
    let last = unsafe { cmsStageNext(matrix) };
    if !stage_is(last, 0x63767374) || !unsafe { cmsStageNext(last) }.is_null() {
        return None;
    }
    // These layouts are the documented LCMS plug-in stage-data ABI, not the
    // private MatShaper optimizer allocation. All pointers remain borrowed.
    let input = unsafe { cmsStageData(first).cast::<CurveSet>().as_ref() }?;
    let output = unsafe { cmsStageData(last).cast::<CurveSet>().as_ref() }?;
    let matrix = unsafe { cmsStageData(matrix).cast::<Matrix>().as_ref() }?;
    if input.count != 3
        || output.count != 3
        || input.curves.is_null()
        || output.curves.is_null()
        || matrix.values.is_null()
    {
        return None;
    }
    let mut words = vec![0; ViewerIccKind::MatrixShaper.word_count()];
    for (i, word) in words[..9].iter_mut().enumerate() {
        *word = q14(unsafe { matrix.values.add(i).read() })? as u32;
    }
    for i in 0..3 {
        words[9 + i] = if matrix.offset.is_null() {
            0
        } else {
            q14(unsafe { matrix.offset.add(i).read() })? as u32
        };
    }
    for channel in 0..3 {
        let input_curve = unsafe { input.curves.add(channel).read() };
        let output_curve = unsafe { output.curves.add(channel).read() };
        if input_curve.is_null() || output_curve.is_null() {
            return None;
        }
        for i in 0..256 {
            let value = unsafe { cmsEvalToneCurveFloat(input_curve, (i as f64 / 255.0) as f32) };
            words[12 + channel * 256 + i] = if value >= 131072.0 {
                i32::MAX as u32
            } else {
                q14(f64::from(value))? as u32
            };
        }
        for i in 0..16385 {
            let value = unsafe { cmsEvalToneCurveFloat(output_curve, i as f32 / 16384.0) };
            if !value.is_finite() {
                return None;
            }
            // LCMS FillSecondShaper: saturate to a word, then FROM_16_TO_8.
            let word = (f64::from(value.clamp(0.0, 1.0)) * 65535.0 + 0.5).floor() as u32;
            let byte = (((word * 65281 + 8388608) >> 24) & 255) as u8;
            put_byte(&mut words[780..], channel * 16385 + i, byte);
        }
    }
    Some(words)
}

fn put_byte(words: &mut [u32], index: usize, byte: u8) {
    words[index / 4] |= u32::from(byte) << ((index % 4) * 8);
}

fn exact_rgb8(transform: &Transform<[u8; 3], [u8; 3]>) -> Vec<u32> {
    // The viewer already quantizes its input to RGB8. Enumerating its entire
    // finite input domain is exact, unlike resampling an ICC transform into a
    // coarse LUT. These synthetic coordinates are never source-image pixels.
    // One red plane bounds temporary storage; the packed map is 48 MiB.
    let mut pixels = vec![[0u8; 3]; 256 * 256];
    let mut words = Vec::with_capacity(ViewerIccKind::ExactRgb8.word_count());
    for red in 0..256 {
        for (i, pixel) in pixels.iter_mut().enumerate() {
            *pixel = [red as u8, (i / 256) as u8, i as u8];
        }
        transform.transform_in_place(&mut pixels);
        for group in pixels.as_chunks::<4>().0 {
            words.push(u32::from_le_bytes([
                group[0][0],
                group[0][1],
                group[0][2],
                group[1][0],
            ]));
            words.push(u32::from_le_bytes([
                group[1][1],
                group[1][2],
                group[2][0],
                group[2][1],
            ]));
            words.push(u32::from_le_bytes([
                group[2][2],
                group[3][0],
                group[3][1],
                group[3][2],
            ]));
        }
    }
    words
}

#[cfg(test)]
#[path = "icc_pipeline/tests.rs"]
mod tests;
