use super::packet::{Invocation as PreparedInvocation, Prepared};
use std::{
    ffi::{CStr, c_char, c_void},
    sync::atomic::{AtomicBool, Ordering},
};
#[repr(C)]
struct Invocation {
    configuration: *const f32,
    configuration_count: usize,
    exposure: *const f32,
    film: *const f32,
    paper: *const f32,
    lut_float_count: usize,
    lut_dimension: i32,
    feature_mask: i32,
    seed: u32,
    spatial_support: i32,
}
#[repr(C)]
struct Component {
    head: Invocation,
    stencils: *const f32,
    stencil_count: usize,
}
unsafe extern "C" {
    pub(crate) fn sf_fotufilm_expf(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_exp2f(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_logf(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_log1pf(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_log2f(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_log10f(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_powf(x: f32, y: f32) -> f32;
    pub(crate) fn sf_fotufilm_tanf(x: f32) -> f32;
    pub(crate) fn sf_fotufilm_film_tile_build(
        texels: i32,
        supersample: i32,
        mark_shape: i32,
        terms: *const f32,
        term_count: i32,
        sublayers: *const f32,
        fractions: *const f32,
        levels: i32,
        seed: u32,
        record: i32,
        light: *mut f32,
    ) -> i32;
    fn sf_fotufilm_process(
        base: *const Invocation,
        tail: *const Invocation,
        components: *const Component,
        count: usize,
        records: i32,
        tiles: *const f32,
        tile_count: usize,
        input_r: *const f32,
        input_g: *const f32,
        input_b: *const f32,
        output_r: *mut f32,
        output_g: *mut f32,
        output_b: *mut f32,
        width: i32,
        height: i32,
        cancel: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
        context: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn sf_fotufilm_configuration_count() -> usize;
    fn sf_fotufilm_lut_float_count() -> usize;
    fn sf_fotufilm_tile_float_count() -> usize;
    fn sf_fotufilm_source_revision() -> *const c_char;
}
pub(crate) fn counts() -> (usize, usize, usize) {
    // Counts are compile-time constants exported by the same pinned graph we execute.
    unsafe {
        (
            sf_fotufilm_configuration_count(),
            sf_fotufilm_lut_float_count(),
            sf_fotufilm_tile_float_count(),
        )
    }
}
fn invocation(current: &PreparedInvocation, base: &PreparedInvocation) -> Invocation {
    let exposure = if current.exposure.is_empty() {
        &base.exposure
    } else {
        &current.exposure
    };
    let film = if current.film.is_empty() {
        &base.film
    } else {
        &current.film
    };
    let paper = current
        .paper
        .as_ref()
        .or(base.paper.as_ref())
        .unwrap_or(film);
    Invocation {
        configuration: current.configuration.as_ptr(),
        configuration_count: current.configuration.len(),
        exposure: exposure.as_ptr(),
        film: film.as_ptr(),
        paper: paper.as_ptr(),
        lut_float_count: exposure.len(),
        lut_dimension: current.dimension,
        feature_mask: current.mask,
        seed: current.seed,
        spatial_support: current.support,
    }
}
unsafe extern "C" fn cancelled(context: *mut c_void) -> i32 {
    // Rust retains the borrowed cancellation token until this synchronous native call returns.
    let cancellation = unsafe { &*context.cast::<AtomicBool>() };
    i32::from(cancellation.load(Ordering::Relaxed))
}
pub(crate) fn render(
    prepared: &Prepared,
    input: &[f32],
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<f32>, String> {
    let width =
        i32::try_from(prepared.width).map_err(|_| "Fotufilm width exceeds native limits")?;
    let height =
        i32::try_from(prepared.height).map_err(|_| "Fotufilm height exceeds native limits")?;
    let pixels = (prepared.width as usize)
        .checked_mul(prepared.height as usize)
        .ok_or("Fotufilm geometry overflow")?;
    if input.len()
        != pixels
            .checked_mul(3)
            .ok_or("Fotufilm sample count overflow")?
    {
        return Err("Fotufilm native source geometry mismatch".into());
    }
    // Native source identity is checked before binding caller-owned arrays.
    let revision = unsafe { CStr::from_ptr(sf_fotufilm_source_revision()) }
        .to_str()
        .map_err(|e| e.to_string())?;
    if revision != super::SOURCE_REVISION {
        return Err("Fotufilm producer and native graph revisions differ".into());
    }
    let base = invocation(&prepared.base, &prepared.base);
    let tail = prepared
        .tail
        .as_ref()
        .map(|tail| invocation(tail, &prepared.base));
    let components: Vec<_> = prepared
        .components
        .iter()
        .map(|component| Component {
            head: invocation(&component.head, &prepared.base),
            stencils: component.stencils.as_ptr(),
            stencil_count: component.stencils.len(),
        })
        .collect();
    let mut output = vec![0.0; input.len()];
    let mut error = [0 as c_char; 2048];
    let (callback, context) = match cancellation {
        Some(cancellation) => (
            Some(cancelled as unsafe extern "C" fn(*mut c_void) -> i32),
            (cancellation as *const AtomicBool).cast_mut().cast(),
        ),
        None => (None, std::ptr::null_mut()),
    };
    // Every source/configuration/output allocation remains owned here for the complete call.
    // The bridge validates counts, retains registered banks until completion, and never stores pointers.
    let status = unsafe {
        sf_fotufilm_process(
            &base,
            tail.as_ref().map_or(std::ptr::null(), |tail| tail),
            components.as_ptr(),
            components.len(),
            prepared.records,
            prepared.tiles.as_ptr(),
            prepared.tiles.len(),
            input.as_ptr(),
            input.as_ptr().add(pixels),
            input.as_ptr().add(pixels * 2),
            output.as_mut_ptr(),
            output.as_mut_ptr().add(pixels),
            output.as_mut_ptr().add(pixels * 2),
            width,
            height,
            callback,
            context,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status == 1 {
        return Err("Fotufilm operation cancelled".into());
    }
    if status != 0 {
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
        return Err(format!(
            "Fotufilm native render failed ({status}): {message}"
        ));
    }
    if output.iter().any(|v| !v.is_finite()) {
        return Err("Fotufilm native render produced nonfinite samples".into());
    }
    Ok(output)
}
