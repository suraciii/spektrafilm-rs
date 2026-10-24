//! Lensfun metadata and modifier preparation; no pixel storage crosses this ABI.
use crate::{RawError, c_string};
use spektrafilm_backend::RawLensParams;
use std::{
    ffi::{CStr, CString, c_char},
    path::Path,
};

unsafe extern "C" {
    fn sf_raw_prepare_lens(
        path: *const c_char,
        width: u32,
        height: u32,
        result: *mut RawLensParams,
        found: *mut i32,
        summary: *mut c_char,
        summary_length: usize,
        identity: *mut *mut c_char,
        error: *mut c_char,
        error_length: usize,
    ) -> i32;
}

pub(crate) fn prepare(
    path: &Path,
    width: u32,
    height: u32,
) -> Result<(Option<RawLensParams>, String, String), RawError> {
    let path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| RawError::Native("Path contains NUL".into()))?;
    let mut result = std::mem::MaybeUninit::<RawLensParams>::uninit();
    let mut found = 0;
    let mut summary = [0 as c_char; 1024];
    let mut error = [0 as c_char; 1024];
    let mut identity = std::ptr::null_mut();
    let status = unsafe {
        sf_raw_prepare_lens(
            path.as_ptr(),
            width,
            height,
            result.as_mut_ptr(),
            &mut found,
            summary.as_mut_ptr(),
            summary.len(),
            &mut identity,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        return Err(RawError::Native(c_string(&error)));
    }
    let model = unsafe { CStr::from_ptr(identity).to_string_lossy().into_owned() };
    unsafe { crate::sf_raw_free_lens_identity(identity) };
    Ok((
        if found != 0 {
            Some(unsafe { result.assume_init() })
        } else {
            None
        },
        c_string(&summary),
        model,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_metadata_needs_no_database_or_image_plane() {
        let (plan, summary, identity) = prepare(
            Path::new("/nonexistent-spektrafilm-lens-metadata"),
            6000,
            4000,
        )
        .unwrap();
        assert!(plan.is_none());
        assert!(summary.is_empty());
        assert_eq!(
            identity,
            crate::lens_model_identity(Path::new("/nonexistent-spektrafilm-lens-metadata"))
                .unwrap()
        );
        assert!(identity.ends_with("\nno-matching-lens-correction"));
    }

    #[test]
    fn native_preparation_rejects_invalid_image_extents_before_metadata() {
        for (width, height) in [(0, 1), (1, 0), (u32::MAX, 1), (1, u32::MAX)] {
            let error = prepare(
                Path::new("/nonexistent-spektrafilm-lens-metadata"),
                width,
                height,
            )
            .unwrap_err();
            assert!(error.to_string().contains("native dimensions"));
        }
    }

    #[test]
    fn bridge_rejects_nul_paths_and_has_fixed_coefficient_extent() {
        assert!(
            prepare(Path::new("a\0b"), 1, 1)
                .unwrap_err()
                .to_string()
                .contains("NUL")
        );
        assert_eq!(std::mem::size_of::<RawLensParams>(), 144);
        assert_eq!(std::mem::align_of::<RawLensParams>(), 4);
    }
}
