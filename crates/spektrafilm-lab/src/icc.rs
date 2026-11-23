use crate::common::{Result, err};
use crate::image_data::ImageData;
use sha2::{Digest, Sha256};
use std::{ffi::c_void, fs, path::Path};

struct Cms {
    _library: libloading::Library,
    open: unsafe extern "C" fn(*const c_void, u32) -> *mut c_void,
    srgb: unsafe extern "C" fn() -> *mut c_void,
    create: unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32, u32, u32) -> *mut c_void,
    apply: unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void, u32),
    delete: unsafe extern "C" fn(*mut c_void),
    close: unsafe extern "C" fn(*mut c_void) -> i32,
    version: unsafe extern "C" fn() -> u32,
}
impl Cms {
    fn load() -> Result<Self> {
        unsafe {
            let executable = std::env::current_exe().ok();
            let names = if cfg!(target_os = "windows") {
                vec!["liblcms2-2.dll", "lcms2.dll"]
            } else if cfg!(target_os = "macos") {
                vec!["liblcms2.2.dylib", "liblcms2.dylib"]
            } else {
                vec!["liblcms2.so.2", "liblcms2.so"]
            };
            let mut candidates = Vec::new();
            if let Some(directory) = executable.as_ref().and_then(|p| p.parent()) {
                for name in &names {
                    for relative in [
                        "lib",
                        "lib/spektrafilm",
                        "../lib/spektrafilm",
                        "../Frameworks",
                    ] {
                        candidates.push(directory.join(relative).join(name));
                    }
                    candidates.push(directory.join(name));
                }
            }
            for name in names {
                candidates.push(std::path::PathBuf::from(name));
            }
            let library = candidates
                .into_iter()
                .find_map(|p| libloading::Library::new(p).ok())
                .ok_or_else(|| {
                    err(
                        "missing_dependency",
                        "LittleCMS 2 shared library is required for ICC transforms",
                    )
                })?;
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| err("missing_dependency", e.to_string()))?
                };
            }
            Ok(Self {
                open: symbol!("cmsOpenProfileFromMem"),
                srgb: symbol!("cmsCreate_sRGBProfile"),
                create: symbol!("cmsCreateTransform"),
                apply: symbol!("cmsDoTransform"),
                delete: symbol!("cmsDeleteTransform"),
                close: symbol!("cmsCloseProfile"),
                version: symbol!("cmsGetEncodedCMMversion"),
                _library: library,
            })
        }
    }
}
pub fn icc_version() -> Result<u32> {
    let cms = Cms::load()?;
    Ok(unsafe { (cms.version)() })
}
const RGB_DBL: u32 = (1 << 22) | (4 << 16) | (3 << 3);

fn embedded(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        if let Ok(reader) = decoder.read_info()
            && let Some(profile) = reader.info().icc_profile.as_ref()
        {
            return Some(profile.to_vec());
        }
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut i = 2usize;
        let mut parts = std::collections::BTreeMap::new();
        let mut total = 0;
        while i + 4 <= bytes.len() {
            if bytes[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            i += 2;
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            if i + 2 > bytes.len() {
                break;
            }
            let n = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
            if n < 2 || i + n > bytes.len() {
                break;
            }
            if marker == 0xe2 && n >= 16 && &bytes[i + 2..i + 14] == b"ICC_PROFILE\0" {
                let seq = bytes[i + 14];
                let count = bytes[i + 15];
                if count == 0
                    || seq == 0
                    || seq > count
                    || parts.contains_key(&seq)
                    || (total != 0 && total != count)
                {
                    return None;
                }
                total = count;
                parts.insert(seq, bytes[i + 16..i + n].to_vec());
            }
            i += n;
        }
        if total != 0 && parts.len() == total as usize {
            return Some(parts.into_values().flatten().collect());
        }
    }
    if bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
        || bytes.starts_with(b"II+\0")
        || bytes.starts_with(b"MM\0+")
    {
        let mut decoder = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).ok()?;
        return decoder.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675)).ok();
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let mut i = 12;
        while i + 8 <= bytes.len() {
            let n = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().ok()?) as usize;
            if i + 8 + n > bytes.len() {
                return None;
            }
            if &bytes[i..i + 4] == b"ICCP" {
                return Some(bytes[i + 8..i + 8 + n].to_vec());
            }
            i += 8 + n + (n % 2)
        }
    }
    None
}
fn profile_bytes(image: &ImageData) -> Result<Vec<u8>> {
    let icc = image
        .metadata
        .get("icc")
        .or_else(|| image.metadata.get("color").and_then(|x| x.get("icc")))
        .and_then(|x| x.as_object())
        .ok_or_else(|| {
            err(
                "missing_color_declaration",
                "ICC transform requires a hashed profile declaration",
            )
        })?;
    let expected = icc
        .get("sha256")
        .and_then(|x| x.as_str())
        .ok_or_else(|| err("missing_color_declaration", "ICC profile hash is required"))?;
    let bytes = if let Some(p) = icc.get("path").and_then(|x| x.as_str()) {
        fs::read(p)?
    } else {
        embedded(image.path.as_deref().ok_or_else(|| {
            err(
                "invalid_color_profile",
                "ICC transform has no profile source",
            )
        })?)
        .ok_or_else(|| {
            err(
                "invalid_color_profile",
                "Declared embedded ICC profile is absent",
            )
        })?
    };
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(err("invalid_color_profile", "ICC profile hash differs"));
    }
    Ok(bytes)
}
pub fn transform_rgb(image: &ImageData) -> Result<Vec<[f64; 3]>> {
    let bytes = profile_bytes(image)?;
    let cms = Cms::load()?;
    if image
        .samples
        .iter()
        .any(|p| p.iter().any(|x| !x.is_finite()))
    {
        return Err(err(
            "invalid_color_profile",
            "ICC conversion requires finite H x W x 3 samples",
        ));
    }
    unsafe {
        let src = (cms.open)(bytes.as_ptr() as *const c_void, bytes.len() as u32);
        let dst = (cms.srgb)();
        if src.is_null() || dst.is_null() {
            if !src.is_null() {
                (cms.close)(src);
            }
            if !dst.is_null() {
                (cms.close)(dst);
            }
            return Err(err(
                "unsupported_color_profile",
                "ICC profile could not be opened as an RGB transform",
            ));
        }
        let t = (cms.create)(src, RGB_DBL, dst, RGB_DBL, 1, 0);
        if t.is_null() {
            (cms.close)(src);
            (cms.close)(dst);
            return Err(err(
                "unsupported_color_profile",
                "LittleCMS could not build the RGB ICC transform",
            ));
        }
        let mut out = vec![[0.0_f64; 3]; image.samples.len()];
        (cms.apply)(
            t,
            image.samples.as_ptr() as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            image.samples.len() as u32,
        );
        (cms.delete)(t);
        (cms.close)(src);
        (cms.close)(dst);
        if out.iter().any(|p| p.iter().any(|x| !x.is_finite())) {
            return Err(err(
                "unsupported_color_profile",
                "ICC transform produced nonfinite values",
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiff_embedded_icc_tag_preserves_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.tif");
        let mut bytes = b"II*\0".to_vec();
        bytes.extend_from_slice(&8u32.to_le_bytes());
        let tags = [
            (256u16, 4u16, 1u32, 1u32),
            (257, 4, 1, 1),
            (258, 3, 1, 8),
            (259, 3, 1, 1),
            (262, 3, 1, 1),
            (273, 4, 1, 123),
            (277, 3, 1, 1),
            (278, 4, 1, 1),
            (279, 4, 1, 1),
            (34675, 7, 3, 0x030201),
        ];
        bytes.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in tags {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.push(0);
        fs::write(&path, bytes).unwrap();
        assert_eq!(embedded(&path), Some(vec![1, 2, 3]));
    }
    #[test]
    fn webp_embedded_icc_preserves_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.webp");
        let mut bytes = b"RIFF\0\0\0\0WEBPICCP".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 0]);
        fs::write(&path, bytes).unwrap();
        assert_eq!(embedded(&path), Some(vec![1, 2, 3]));
    }
    #[test]
    fn multipart_jpeg_profiles_are_ordered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.jpg");
        let mut bytes = vec![255, 216];
        for (seq, data) in [(2u8, vec![3u8, 4]), (1, vec![1, 2])] {
            bytes.extend_from_slice(&[255, 226]);
            bytes.extend_from_slice(&((16 + data.len()) as u16).to_be_bytes());
            bytes.extend_from_slice(b"ICC_PROFILE\0");
            bytes.extend_from_slice(&[seq, 2]);
            bytes.extend_from_slice(&data)
        }
        bytes.extend_from_slice(&[255, 217]);
        fs::write(&path, bytes).unwrap();
        assert_eq!(embedded(&path), Some(vec![1, 2, 3, 4]));
    }
    #[test]
    fn profile_hash_checked_before_native_transform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.icc");
        fs::write(&path, b"profile").unwrap();
        let image = ImageData {
            samples: vec![[0.; 3]],
            width: 1,
            height: 1,
            color_space: String::new(),
            transfer: String::new(),
            metadata: serde_json::json!({"icc":{"path":path,"sha256":"bad"}}),
            path: None,
        };
        assert_eq!(
            transform_rgb(&image).unwrap_err().code,
            "invalid_color_profile"
        );
    }
}
