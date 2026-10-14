use super::err;
use crate::common::Result;

unsafe extern "C" {
    fn spektrafilm_flip(
        a: *const f32,
        b: *const f32,
        n: usize,
        w: usize,
        h: usize,
        ppd: f32,
        out: *mut f32,
    ) -> i32;
}
pub fn official_flip(
    a: &[[f64; 3]],
    b: &[[f64; 3]],
    width: usize,
    height: usize,
    ppd: f64,
) -> Result<Vec<f64>> {
    if a.len() != width * height || b.len() != a.len() {
        return Err(err("invalid_image_layout", "FLIP dimensions mismatch"));
    }
    if !ppd.is_finite() || ppd <= 0. {
        return Err(err(
            "invalid_protocol",
            "FLIP pixels_per_degree must be positive",
        ));
    }
    let aa: Vec<f32> = a
        .iter()
        .flat_map(|p| p.map(|x| x.clamp(0., 1.) as f32))
        .collect();
    let bb: Vec<f32> = b
        .iter()
        .flat_map(|p| p.map(|x| x.clamp(0., 1.) as f32))
        .collect();
    let mut o = vec![0f32; a.len()];
    let rc = unsafe {
        spektrafilm_flip(
            aa.as_ptr(),
            bb.as_ptr(),
            a.len(),
            width,
            height,
            ppd as f32,
            o.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(err(
            "computation_failure",
            format!("Official FLIP bridge failed ({rc})"),
        ));
    }
    if o.iter().any(|x| !x.is_finite()) {
        return Err(err(
            "computation_failure",
            "Official FLIP returned invalid map",
        ));
    }
    Ok(o.into_iter().map(|x| x as f64).collect())
}
