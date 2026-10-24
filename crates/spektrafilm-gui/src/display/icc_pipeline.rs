//! Authored ICC data extraction. Numerical curve inversion and optimizer tables
//! belong to the compute device, not profile discovery.
use lcms2::{ColorSpaceSignature, Profile, Tag, TagSignature};
use std::ffi::c_void;

pub(crate) struct ResolvedIccPipeline {
    words: Vec<u32>,
    values: Vec<f64>,
}
impl ResolvedIccPipeline {
    pub(crate) fn params(&self) -> spektrafilm_backend::ViewerIccGpuParams<'_> {
        spektrafilm_backend::ViewerIccGpuParams {
            words: &self.words,
            values: &self.values,
        }
    }
}
#[repr(C)]
struct Segment {
    x0: f32,
    x1: f32,
    kind: i32,
    params: [f64; 10],
    count: u32,
    samples: *const f32,
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
#[repr(C)]
struct Interpolation {
    context: *const c_void,
    flags: u32,
    inputs: u32,
    outputs: u32,
    samples: [u32; 15],
    domain: [u32; 15],
    strides: [u32; 15],
    table: *const c_void,
    function: *const c_void,
}
#[repr(C)]
struct Clut {
    table: *const c_void,
    interpolation: *const Interpolation,
    entries: u32,
    floats: i32,
}
unsafe extern "C" {
    fn cmsGetToneCurveSegment(index: i32, curve: *const c_void) -> *const Segment;
    fn cmsGetToneCurveEstimatedTableEntries(curve: *const c_void) -> u32;
    fn cmsGetToneCurveEstimatedTable(curve: *const c_void) -> *const u16;
    fn cmsPipelineGetPtrToFirstStage(pipeline: *const c_void) -> *const c_void;
    fn cmsStageNext(stage: *const c_void) -> *const c_void;
    fn cmsStageType(stage: *const c_void) -> u32;
    fn cmsStageInputChannels(stage: *const c_void) -> u32;
    fn cmsStageOutputChannels(stage: *const c_void) -> u32;
    fn cmsStageData(stage: *const c_void) -> *const c_void;
}
struct Builder {
    words: Vec<u32>,
    values: Vec<f64>,
    stages: Vec<[u32; 4]>,
}
impl Builder {
    fn new() -> Self {
        Self {
            words: Vec::new(),
            values: Vec::new(),
            stages: Vec::new(),
        }
    }
    fn data(&mut self, values: &[f64]) -> u32 {
        let at = self.values.len() as u32;
        self.values.extend_from_slice(values);
        at
    }
    fn payload(&mut self, words: &[u32]) -> u32 {
        let at = self.words.len() as u32;
        self.words.extend_from_slice(words);
        at
    }
    fn stage(&mut self, kind: u32, inputs: u32, outputs: u32, payload: u32) {
        self.stages.push([kind, inputs, outputs, payload]);
    }
    fn matrix(&mut self, m: &[f64], inputs: u32, outputs: u32, offset: Option<&[f64]>) {
        let values = self.data(m);
        let offset = offset.map(|v| self.data(v)).unwrap_or(u32::MAX);
        let p = self.payload(&[values, offset]);
        self.stage(2, inputs, outputs, p);
    }
    // LCMS curve references are transparent native borrowed handles. We copy only
    // native segment coefficients and authored samples, never its shadow table
    // when a parametric/segmented representation exists.
    unsafe fn curve(&mut self, curve: *const c_void, inverse: bool) -> u32 {
        let mut segments = Vec::new();
        let mut i = 0;
        loop {
            let p = unsafe { cmsGetToneCurveSegment(i, curve) };
            if p.is_null() {
                break;
            }
            // Native sampled segments leave Params uninitialized; parametric
            // segments may leave count/samples and unused Params uninitialized.
            // Project raw fields without ever forming a Rust Segment reference.
            let kind = unsafe { std::ptr::addr_of!((*p).kind).read() };
            let x0 = self.data(&[f64::from(unsafe { std::ptr::addr_of!((*p).x0).read() })]);
            let x1 = self.data(&[f64::from(unsafe { std::ptr::addr_of!((*p).x1).read() })]);
            let parameter_count = match kind.unsigned_abs() {
                0 => 0,
                1 | 108 | 109 => 1,
                2 => 3,
                3 | 6 => 4,
                4 | 7 | 8 => 5,
                5 => 7,
                _ => panic!("ICC native curve type {kind} is not a built-in LCMS curve"),
            };
            let mut coefficients = [0.0; 10];
            let native_params = unsafe { std::ptr::addr_of!((*p).params).cast::<f64>() };
            for (index, value) in coefficients.iter_mut().take(parameter_count).enumerate() {
                *value = unsafe { native_params.add(index).read() };
            }
            let params = self.data(&coefficients);
            let (samples, count) = if kind == 0 {
                let count = unsafe { std::ptr::addr_of!((*p).count).read() };
                let samples = if count == 0 {
                    0
                } else {
                    let native_samples = unsafe { std::ptr::addr_of!((*p).samples).read() };
                    self.data(
                        &unsafe { std::slice::from_raw_parts(native_samples, count as usize) }
                            .iter()
                            .map(|&v| f64::from(v))
                            .collect::<Vec<_>>(),
                    )
                };
                (samples, count)
            } else {
                (0, 0)
            };
            segments.push([kind as u32, x0, x1, params, samples, count]);
            i += 1;
        }
        let (table, count) = if segments.is_empty() {
            let count = unsafe { cmsGetToneCurveEstimatedTableEntries(curve) };
            let p = unsafe { cmsGetToneCurveEstimatedTable(curve) };
            (
                self.data(
                    &unsafe { std::slice::from_raw_parts(p, count as usize) }
                        .iter()
                        .map(|&v| f64::from(v))
                        .collect::<Vec<_>>(),
                ),
                count,
            )
        } else {
            (0, 0)
        };
        let first = self.words.len() as u32;
        for s in &segments {
            self.words.extend_from_slice(s);
        }
        self.payload(&[
            u32::from(inverse),
            segments.len() as u32,
            table,
            count,
            first,
        ])
    }
    fn curves(&mut self, curves: [&lcms2::ToneCurveRef; 3], inverse: bool) {
        let c = curves.map(|curve| unsafe {
            self.curve((curve as *const lcms2::ToneCurveRef).cast(), inverse)
        });
        let p = self.payload(&c);
        self.stage(1, 3, 3, p);
    }
    fn authored_pipeline(
        &mut self,
        pipeline: &lcms2::PipelineRef,
        trilinear: bool,
    ) -> Result<(), String> {
        let mut stage = unsafe {
            cmsPipelineGetPtrToFirstStage((pipeline as *const lcms2::PipelineRef).cast())
        };
        while !stage.is_null() {
            let (kind, inputs, outputs, data) = unsafe {
                (
                    cmsStageType(stage),
                    cmsStageInputChannels(stage),
                    cmsStageOutputChannels(stage),
                    cmsStageData(stage),
                )
            };
            match kind {
                0x63767374 => {
                    let d = unsafe { &*data.cast::<CurveSet>() };
                    let curves = unsafe { std::slice::from_raw_parts(d.curves, d.count as usize) };
                    let offsets: Vec<_> = curves
                        .iter()
                        .map(|&c| unsafe { self.curve(c, false) })
                        .collect();
                    let p = self.payload(&offsets);
                    self.stage(1, inputs, outputs, p);
                }
                0x6d617466 => {
                    let d = unsafe { &*data.cast::<Matrix>() };
                    self.matrix(
                        unsafe {
                            std::slice::from_raw_parts(d.values, (inputs * outputs) as usize)
                        },
                        inputs,
                        outputs,
                        if d.offset.is_null() {
                            None
                        } else {
                            Some(unsafe { std::slice::from_raw_parts(d.offset, outputs as usize) })
                        },
                    );
                }
                0x636c7574 => {
                    let d = unsafe { &*data.cast::<Clut>() };
                    let interpolation = unsafe { &*d.interpolation };
                    let values = if d.floats != 0 {
                        unsafe {
                            std::slice::from_raw_parts(d.table.cast::<f32>(), d.entries as usize)
                        }
                        .iter()
                        .map(|&v| f64::from(v))
                        .collect::<Vec<_>>()
                    } else {
                        unsafe {
                            std::slice::from_raw_parts(d.table.cast::<u16>(), d.entries as usize)
                        }
                        .iter()
                        .map(|&v| f64::from(v))
                        .collect::<Vec<_>>()
                    };
                    let offset = self.data(&values);
                    let mut payload = vec![
                        offset,
                        interpolation.flags | if trilinear { 256 } else { 0 },
                    ];
                    payload.extend_from_slice(&interpolation.samples[..inputs as usize]);
                    let p = self.payload(&payload);
                    self.stage(3, inputs, outputs, p);
                }
                0x69646e20 | 0x62414353 | 0x65414353 => self.stage(0, inputs, outputs, 0),
                0x6c327820 => self.stage(4, inputs, outputs, 0),
                0x78326c20 => self.stage(5, inputs, outputs, 0),
                0x636c7020 => self.stage(8, inputs, outputs, 0),
                _ => {
                    return Err(format!(
                        "ICC native stage {kind:08x} has no authored representation"
                    ));
                }
            }
            stage = unsafe { cmsStageNext(stage) };
        }
        Ok(())
    }
    fn profile_input(&mut self, profile: &Profile) -> Result<(), String> {
        let tags = profile.tag_signatures();
        let float = tags.contains(&TagSignature::DToB1Tag);
        let tag = if float {
            Some(TagSignature::DToB1Tag)
        } else if tags.contains(&TagSignature::AToB1Tag) {
            Some(TagSignature::AToB1Tag)
        } else if tags.contains(&TagSignature::AToB0Tag) {
            Some(TagSignature::AToB0Tag)
        } else {
            None
        };
        if let Some(tag) = tag {
            let Tag::Pipeline(pipeline) = profile.read_tag(tag) else {
                return Err("ICC relative input pipeline could not be parsed".into());
            };
            self.authored_pipeline(pipeline, false)?;
            if float {
                if profile.pcs() == ColorSpaceSignature::LabData {
                    self.matrix(
                        &[0.01, 0., 0., 0., 1. / 255., 0., 0., 0., 1. / 255.],
                        3,
                        3,
                        Some(&[0., 128. / 255., 128. / 255.]),
                    );
                } else {
                    self.matrix(
                        &[
                            32768. / 65535.,
                            0.,
                            0.,
                            0.,
                            32768. / 65535.,
                            0.,
                            0.,
                            0.,
                            32768. / 65535.,
                        ],
                        3,
                        3,
                        None,
                    );
                }
            } else if profile.pcs() == ColorSpaceSignature::LabData
                && raw_type(profile, tag) == Some(0x6d667432)
            {
                self.stage(6, 3, 3, 0);
            }
            if profile.pcs() == ColorSpaceSignature::LabData {
                self.stage(5, 3, 3, 0);
            }
        } else {
            let (curves, matrix) = shaper(profile)?;
            self.curves(curves, false);
            self.matrix(
                &matrix.map(|r| r.map(|v| v * (32768. / 65535.))).concat(),
                3,
                3,
                None,
            );
        }
        Ok(())
    }
    fn preoptimize(&mut self) {
        loop {
            let mut changed = false;
            let mut i = 0;
            while i < self.stages.len() {
                if self.stages[i][0] == 0 {
                    self.stages.remove(i);
                    changed = true;
                    continue;
                }
                if i + 1 >= self.stages.len() {
                    break;
                }
                let a = self.stages[i];
                let b = self.stages[i + 1];
                if matches!((a[0], b[0]), (4, 5) | (5, 4) | (6, 7) | (7, 6)) {
                    self.stages.drain(i..i + 2);
                    changed = true;
                    continue;
                }
                if a[0] == 2 && b[0] == 2 && a[1] == 3 && a[2] == 3 && b[1] == 3 && b[2] == 3 {
                    let pa = a[3] as usize;
                    let pb = b[3] as usize;
                    if self.words[pa + 1] == u32::MAX && self.words[pb + 1] == u32::MAX {
                        let av = self.words[pa] as usize;
                        let bv = self.words[pb] as usize;
                        let m: [f64; 9] = std::array::from_fn(|n| {
                            (0..3)
                                .map(|k| {
                                    self.values[bv + (n / 3) * 3 + k]
                                        * self.values[av + k * 3 + n % 3]
                                })
                                .sum()
                        });
                        self.stages.drain(i..i + 2);
                        if !(0..9).all(|n| {
                            (m[n] - if n / 3 == n % 3 { 1.0 } else { 0.0 }).abs() < 0.00001
                        }) {
                            let v = self.data(&m);
                            let p = self.payload(&[v, u32::MAX]);
                            self.stages.insert(i, [2, 3, 3, p]);
                        }
                        changed = true;
                        continue;
                    }
                }
                i += 1;
            }
            if !changed {
                break;
            }
        }
    }
    fn finish(mut self, main_count: usize) -> ResolvedIccPipeline {
        // Relocate word offsets after the stage directory has its final size.
        let directory = 2 + self.stages.len() * 4;
        for stage in &mut self.stages {
            if matches!(stage[0], 1 | 2 | 3 | 9) {
                stage[3] += directory as u32;
            }
        }
        // Payload offsets point into the same word arena. Curve descriptors and
        // segment directories are relocated, while values offsets stay intact.
        let mut at = 0;
        while at < self.stages.len() {
            let s = self.stages[at];
            let p = s[3].saturating_sub(directory as u32) as usize;
            if s[0] == 1 {
                for c in 0..s[2] as usize {
                    let curve = self.words[p + c] as usize;
                    self.words[curve + 4] += directory as u32;
                    self.words[p + c] += directory as u32;
                }
            }
            at += 1;
        }
        let mut words = vec![main_count as u32, 2];
        for s in self.stages {
            words.extend_from_slice(&s);
        }
        words.extend(self.words);
        ResolvedIccPipeline {
            words,
            values: self.values,
        }
    }
}
fn shaper(profile: &Profile) -> Result<([&lcms2::ToneCurveRef; 3], [[f64; 3]; 3]), String> {
    fn curve(tag: Tag<'_>) -> Result<&lcms2::ToneCurveRef, String> {
        if let Tag::ToneCurve(c) = tag {
            Ok(c)
        } else {
            Err("ICC profile has no RGB tone curve".into())
        }
    }
    fn xyz(tag: Tag<'_>) -> Result<lcms2::CIEXYZ, String> {
        if let Tag::CIEXYZ(c) = tag {
            Ok(*c)
        } else {
            Err("ICC profile has no RGB colorant".into())
        }
    }
    let curves = [
        curve(profile.read_tag(TagSignature::RedTRCTag))?,
        curve(profile.read_tag(TagSignature::GreenTRCTag))?,
        curve(profile.read_tag(TagSignature::BlueTRCTag))?,
    ];
    let r = xyz(profile.read_tag(TagSignature::RedColorantTag))?;
    let g = xyz(profile.read_tag(TagSignature::GreenColorantTag))?;
    let b = xyz(profile.read_tag(TagSignature::BlueColorantTag))?;
    Ok((curves, [[r.X, g.X, b.X], [r.Y, g.Y, b.Y], [r.Z, g.Z, b.Z]]))
}
fn inverse(m: [[f64; 3]; 3]) -> Result<[[f64; 3]; 3], String> {
    let c0 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
    let c1 = -m[1][0] * m[2][2] + m[1][2] * m[2][0];
    let c2 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
    let d = m[0][0] * c0 + m[0][1] * c1 + m[0][2] * c2;
    if d.abs() < 0.0001 {
        return Err("ICC colorant matrix is singular".into());
    }
    Ok([
        [
            c0 / d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d,
        ],
        [
            c1 / d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d,
        ],
        [
            c2 / d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d,
        ],
    ])
}
fn raw_type(profile: &Profile, tag: TagSignature) -> Option<u32> {
    let bytes = profile.icc().ok()?;
    let be = |i: usize| {
        bytes
            .get(i..i + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let n = be(128)? as usize;
    for i in 0..n {
        let p = 132 + i * 12;
        if be(p)? == tag as u32 {
            return be(be(p + 4)? as usize);
        }
    }
    None
}
pub(super) fn resolve(
    source: &Profile,
    destination: &Profile,
) -> Result<ResolvedIccPipeline, String> {
    let mut b = Builder::new();
    let (curves, matrix) = shaper(source)?;
    b.curves(curves, false);
    let source_matrix = matrix.map(|r| r.map(|v| v * (32768.0 / 65535.0)));
    b.matrix(&source_matrix.concat(), 3, 3, None);
    if matches!(
        destination.device_class(),
        lcms2::ProfileClassSignature::LinkClass | lcms2::ProfileClassSignature::AbstractClass
    ) {
        if destination.pcs() != ColorSpaceSignature::RgbData {
            return Err("ICC link output is not RGB".into());
        }
        let input = destination.color_space();
        if input == ColorSpaceSignature::LabData {
            b.stage(4, 3, 3, 0);
        } else if input != ColorSpaceSignature::XYZData {
            return Err("ICC link input does not connect to sRGB PCS".into());
        }
        let float = destination
            .tag_signatures()
            .contains(&TagSignature::DToB0Tag);
        let tag = if float {
            TagSignature::DToB0Tag
        } else {
            TagSignature::AToB0Tag
        };
        if float {
            if input == ColorSpaceSignature::LabData {
                b.matrix(
                    &[100., 0., 0., 0., 255., 0., 0., 0., 255.],
                    3,
                    3,
                    Some(&[0., -128., -128.]),
                );
            } else {
                b.matrix(
                    &[
                        65535. / 32768.,
                        0.,
                        0.,
                        0.,
                        65535. / 32768.,
                        0.,
                        0.,
                        0.,
                        65535. / 32768.,
                    ],
                    3,
                    3,
                    None,
                );
            }
        } else if input == ColorSpaceSignature::LabData
            && raw_type(destination, tag) == Some(0x6d667432)
        {
            b.stage(7, 3, 3, 0);
        }
        let Tag::Pipeline(pipeline) = destination.read_tag(tag) else {
            return Err("ICC link pipeline could not be parsed".into());
        };
        b.authored_pipeline(pipeline, false)?;
        b.preoptimize();
        let count = b.stages.len();
        return Ok(b.finish(count));
    }
    if destination.color_space() != ColorSpaceSignature::RgbData {
        return Err("ICC output format is not RGB".into());
    }
    let tags = destination.tag_signatures();
    let float_tag = tags.contains(&TagSignature::BToD0Tag);
    let lut_tag = if float_tag {
        Some(TagSignature::BToD0Tag)
    } else if tags.contains(&TagSignature::BToA0Tag) {
        Some(TagSignature::BToA0Tag)
    } else {
        None
    };
    // A v4 perceptual connection always enables LCMS BPC. Its generated
    // darkest-colorant evaluation is represented explicitly for device prep.
    let bpc = destination.encoded_icc_version() >= 0x04000000
        && !matches!(
            destination.device_class(),
            lcms2::ProfileClassSignature::LinkClass
                | lcms2::ProfileClassSignature::AbstractClass
                | lcms2::ProfileClassSignature::NamedColorClass
        );
    let bpc_payload = if bpc {
        let p = b.payload(&[u32::from(destination.is_matrix_shaper()), 0, 0]);
        b.stage(9, 3, 3, p);
        Some(p)
    } else {
        None
    };
    if destination.pcs() == ColorSpaceSignature::LabData {
        b.stage(4, 3, 3, 0);
    }
    if let Some(tag) = lut_tag {
        if float_tag {
            if destination.pcs() == ColorSpaceSignature::LabData {
                b.matrix(
                    &[100., 0., 0., 0., 255., 0., 0., 0., 255.],
                    3,
                    3,
                    Some(&[0., -128., -128.]),
                );
            } else {
                b.matrix(
                    &[
                        65535. / 32768.,
                        0.,
                        0.,
                        0.,
                        65535. / 32768.,
                        0.,
                        0.,
                        0.,
                        65535. / 32768.,
                    ],
                    3,
                    3,
                    None,
                );
            }
        } else if destination.pcs() == ColorSpaceSignature::LabData
            && raw_type(destination, tag) == Some(0x6d667432)
        {
            b.stage(7, 3, 3, 0);
        }
        let Tag::Pipeline(pipeline) = destination.read_tag(tag) else {
            return Err("ICC authored output pipeline could not be parsed".into());
        };
        b.authored_pipeline(pipeline, destination.pcs() == ColorSpaceSignature::LabData)?;
    } else {
        if destination.pcs() == ColorSpaceSignature::LabData {
            b.stage(5, 3, 3, 0);
        }
        let (curves, matrix) = shaper(destination)?;
        let matrix = inverse(matrix)?.map(|r| r.map(|v| v * (65535.0 / 32768.0)));
        b.matrix(&matrix.concat(), 3, 3, None);
        b.curves(curves, true);
    }
    b.preoptimize();
    let main_count = b.stages.len();
    if let Some(p) = bpc_payload
        && destination.is_matrix_shaper()
    {
        let start = b.stages.len();
        b.profile_input(destination)?;
        b.words[p as usize + 1] = start as u32;
        b.words[p as usize + 2] = (b.stages.len() - start) as u32;
    }
    Ok(b.finish(main_count))
}
