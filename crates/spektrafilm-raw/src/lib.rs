//! Reference RAW decode: linear 16-bit ACES AP0, followed by Lensfun and WB.
use std::{ffi::{CStr, CString, c_char}, path::Path};
use serde::{Deserialize, Serialize};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteBalance { #[default] AsShot, Daylight, Tungsten, Custom }

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawOptions {
    pub white_balance: WhiteBalance,
    pub temperature: Option<f64>,
    pub tint: Option<f64>,
    pub lens_correction: bool,
}

#[derive(Debug)]
pub struct RawResult {
    /// Linear ACES2065-1 pixels; LibRaw applies camera orientation during decode.
    pub image: ImageBuf,
    pub lens_info: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RawError {
    #[error("RAW input: {0}")] Io(#[from] std::io::Error),
    #[error("RAW decode: {0}")] Native(String),
    #[error("RAW white balance: {0}")] WhiteBalance(String),
}

unsafe extern "C" {
    fn sf_raw_decode(bytes:*const u8,length:usize,camera_wb:i32,pixels:*mut *mut f32,width:*mut u32,height:*mut u32,error:*mut c_char,error_length:usize)->i32;
    fn sf_raw_correct_lens(path:*const c_char,pixels:*mut f32,width:u32,height:u32,summary:*mut c_char,summary_length:usize,error:*mut c_char,error_length:usize)->i32;
    fn sf_raw_free(pixels:*mut f32);
    fn sf_raw_version()->*const c_char;
}

pub fn decoder_version() -> String {
    unsafe { CStr::from_ptr(sf_raw_version()).to_string_lossy().into_owned() }
}

struct NativePixels(*mut f32);
impl Drop for NativePixels { fn drop(&mut self) { unsafe { sf_raw_free(self.0) }; } }

/// Decode through LibRaw, never infer decoder support from the path suffix.
pub fn load(path: &Path, options: &RawOptions) -> Result<RawResult, RawError> {
    let adaptation = options.adaptation()?;
    let bytes = std::fs::read(path)?;
    let mut pixels = std::ptr::null_mut();
    let (mut width, mut height) = (0,0);
    let mut error = [0 as c_char; 1024];
    let status = unsafe { sf_raw_decode(bytes.as_ptr(),bytes.len(),i32::from(options.white_balance == WhiteBalance::AsShot),&mut pixels,&mut width,&mut height,error.as_mut_ptr(),error.len()) };
    if status != 0 { return Err(RawError::Native(c_string(&error))); }
    let owned = NativePixels(pixels);
    let count = (width as usize).checked_mul(height as usize).and_then(|v|v.checked_mul(3)).ok_or_else(||RawError::Native("Image dimensions overflow".into()))?;
    let floats = unsafe { std::slice::from_raw_parts_mut(owned.0,count) };
    let mut lens_info = String::new();
    if options.lens_correction {
        let path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_|RawError::Native("Path contains NUL".into()))?;
        let mut summary = [0 as c_char; 1024];
        let status = unsafe { sf_raw_correct_lens(path.as_ptr(),floats.as_mut_ptr(),width,height,summary.as_mut_ptr(),summary.len(),error.as_mut_ptr(),error.len()) };
        if status != 0 { return Err(RawError::Native(c_string(&error))); }
        lens_info = c_string(&summary);
    }
    if let Some(matrix) = adaptation {
        for rgb in floats.chunks_exact_mut(3) {
            let xyz = mv(ACES_TO_XYZ,[rgb[0] as f64,rgb[1] as f64,rgb[2] as f64]);
            let output = mv(XYZ_TO_ACES,mv(matrix,xyz));
            for c in 0..3 { rgb[c] = output[c] as f32; }
        }
    }
    if matches!(options.white_balance,WhiteBalance::Custom) {
        let tint = options.tint.unwrap_or(1.0);
        // Match numpy.isclose(tint, 1), including its default relative tolerance.
        if (tint-1.0).abs() > 1.001e-5 {
            for rgb in floats.chunks_exact_mut(3) { rgb[1] *= tint as f32; }
        }
    }
    let data = floats.iter().copied().map(from_f32).collect();
    Ok(RawResult { image:ImageBuf::from_data(width,height,data), lens_info })
}
fn c_string(buffer:&[c_char])->String { unsafe { CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned() } }

impl RawOptions {
    pub fn validate(&self)->Result<(),RawError> { self.adaptation().map(|_|()) }
    fn adaptation(&self)->Result<Option<[[f64;3];3]>,RawError> {
        let temperature = match self.white_balance {
            WhiteBalance::AsShot | WhiteBalance::Daylight => return Ok(None),
            WhiteBalance::Tungsten => 2850.0,
            WhiteBalance::Custom => self.temperature.ok_or_else(||RawError::WhiteBalance("A custom raw white balance requires a temperature value.".into()))?,
        };
        if !temperature.is_finite() || !(1667.0..=25000.0).contains(&temperature) {
            return Err(RawError::WhiteBalance("Temperature must be finite and between 1667 and 25000 K (CIE D / Kang 2002 domain).".into()));
        }
        if self.white_balance == WhiteBalance::Custom && self.tint.is_some_and(|v|!v.is_finite()) {
            return Err(RawError::WhiteBalance("Tint must be finite.".into()));
        }
        let source = whitepoint_xyz(temperature);
        let target = whitepoint_xyz(6504.0);
        if source.iter().zip(target).all(|(a,b)|(a-b).abs()<=1e-8+1e-5*b.abs()) { return Ok(None); }
        // colour.chromatic_adaptation(method='Von Kries') defaults to CAT02.
        let s = mv(CAT02,source);
        let t = mv(CAT02,target);
        let mut scaled = CAT02;
        for r in 0..3 { for c in 0..3 { scaled[r][c] *= t[r]/s[r]; } }
        Ok(Some(mm(inverse(CAT02),scaled)))
    }
}

pub fn whitepoint_xyz(t:f64)->[f64;3] {
    let x = if t>=4000.0 {
        if t<=7000.0 { -4.6070e9/t.powi(3)+2.9678e6/t.powi(2)+0.09911e3/t+0.244063 }
        else { -2.0064e9/t.powi(3)+1.9018e6/t.powi(2)+0.24748e3/t+0.237040 }
    } else { -0.2661239e9/t.powi(3)-0.2343589e6/t.powi(2)+0.8776956e3/t+0.179910 };
    let y = if t>=4000.0 { -3.0*x*x+2.87*x-0.275 }
    else if t<=2222.0 { -1.1063814*x.powi(3)-1.34811020*x*x+2.18555832*x-0.20219683 }
    else { -0.9549476*x.powi(3)-1.37418593*x*x+2.09137015*x-0.16748867 };
    [x/y,1.0,(1.0-x-y)/y]
}
const ACES_TO_XYZ:[[f64;3];3]=[[0.9525523959,0.0,0.0000936786],[0.3439664498,0.7281660966,-0.0721325464],[0.0,0.0,1.0088251844]];
const XYZ_TO_ACES:[[f64;3];3]=[[1.0498110175,0.0,-0.0000974845],[-0.4959030231,1.3733130458,0.0982400361],[0.0,0.0,0.9912520182]];
const CAT02:[[f64;3];3]=[[0.7328,0.4296,-0.1624],[-0.7036,1.6975,0.0061],[0.003,0.0136,0.9834]];
fn mv(m:[[f64;3];3],v:[f64;3])->[f64;3] { m.map(|r|r[0]*v[0]+r[1]*v[1]+r[2]*v[2]) }
fn mm(a:[[f64;3];3],b:[[f64;3];3])->[[f64;3];3] { std::array::from_fn(|r|std::array::from_fn(|c|a[r][0]*b[0][c]+a[r][1]*b[1][c]+a[r][2]*b[2][c])) }
fn inverse(m:[[f64;3];3])->[[f64;3];3] {
    let cof = std::array::from_fn::<_,3,_>(|r|std::array::from_fn::<_,3,_>(|c|m[(r+1)%3][(c+1)%3]*m[(r+2)%3][(c+2)%3]-m[(r+1)%3][(c+2)%3]*m[(r+2)%3][(c+1)%3]));
    let determinant = m[0][0]*cof[0][0]+m[0][1]*cof[0][1]+m[0][2]*cof[0][2];
    std::array::from_fn(|r|std::array::from_fn(|c|cof[c][r]/determinant))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tungsten_and_custom_match_reference_adaptation() {
        for (mode, temperature, expected) in [
            (WhiteBalance::Tungsten, None, [0.2995248437,0.4106940329,1.1936421394]),
            (WhiteBalance::Custom, Some(5000.0), [0.2118798941,0.3155113757,0.5267853737]),
        ] {
            let options=RawOptions { white_balance:mode,temperature,..Default::default() };
            let output=mv(XYZ_TO_ACES,mv(options.adaptation().unwrap().unwrap(),mv(ACES_TO_XYZ,[0.2f32 as f64,0.3f32 as f64,0.4f32 as f64])));
            for c in 0..3 { assert!((output[c] as f32 as f64-expected[c]).abs()<6e-8); }
        }
    }
    #[test]
    fn custom_requires_valid_temperature_and_finite_tint() {
        let mut options=RawOptions {white_balance:WhiteBalance::Custom,..Default::default()};
        assert!(options.validate().unwrap_err().to_string().contains("requires a temperature"));
        for temperature in [f64::NAN,0.0,25001.0] { options.temperature=Some(temperature); assert!(options.validate().is_err()); }
        options.temperature=Some(6504.0); options.tint=Some(f64::INFINITY); assert!(options.validate().is_err());
        options.tint=Some(1.0); assert!(options.validate().is_ok());
    }
}
