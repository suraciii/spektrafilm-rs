//! Field metadata derived from the runtime parameter model itself: types come
//! from a serialization traversal of `RuntimeParams`, vocabularies and bounds
//! from `params::validation`, and defaults from the serialized default value.
use serde::Serializer;
use serde::ser::{Impossible, SerializeSeq, SerializeStruct, SerializeTuple};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

type RecError = serde::de::value::Error;

/// Rust types per dotted serialized path, captured by serializing
/// `RuntimeParams::default()` through a recording serializer. serde
/// distinguishes f32/f64/u32/bool/str at the serializer boundary, so the
/// recorded spellings are the actual declaration types.
fn types() -> &'static BTreeMap<String, String> {
    static TYPES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TYPES.get_or_init(|| {
        let mut out = BTreeMap::new();
        serde::Serialize::serialize(
            &super::super::RuntimeParams::default(),
            &mut Recorder {
                path: String::new(),
                out: &mut out,
            },
        )
        .expect("type recording serialization cannot fail");
        out
    })
}

/// `Option` inner types for fields whose default is `None` (the recorder only
/// sees the null). Any new nullable field must be listed here.
fn nullable_inner(path: &str) -> Option<&'static str> {
    Some(match path {
        "film_render.development_time" | "print_render.development_time" => "f64",
        "film_render.grain.v2_size"
        | "film_render.grain.v2_amount"
        | "film_render.grain.v2_shadows"
        | "film_render.grain.v2_midtones"
        | "film_render.grain.v2_highlights"
        | "film_render.grain.v2_chroma"
        | "film_render.grain.v2_resolution_factor" => "f32",
        "taps.inject" | "taps.collect" => "String",
        "neutral_print_filters_protected" => "[bool; 3]",
        "camera.capture_illuminant" => "String",
        _ => return None,
    })
}

struct Recorder<'a> {
    path: String,
    out: &'a mut BTreeMap<String, String>,
}
impl Recorder<'_> {
    fn record(&mut self, ty: impl Into<String>) -> Result<(), RecError> {
        self.out.insert(self.path.clone(), ty.into());
        Ok(())
    }
    fn descend<T: serde::Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<(), RecError> {
        let next = if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        };
        let saved = std::mem::replace(&mut self.path, next);
        let result = value.serialize(&mut *self);
        self.path = saved;
        result
    }
}

struct SeqRec<'a, 'b> {
    rec: &'a mut Recorder<'b>,
    len: Option<usize>,
    recorded: bool,
}
macro_rules! seq_impl {
    ($trait:ident, $element:ident) => {
        impl<'a, 'b> $trait for SeqRec<'a, 'b> {
            type Ok = ();
            type Error = RecError;
            fn $element<T: serde::Serialize + ?Sized>(
                &mut self,
                value: &T,
            ) -> Result<(), RecError> {
                if !self.recorded {
                    self.recorded = true;
                    let mut tmp = BTreeMap::new();
                    let path = self.rec.path.clone();
                    value.serialize(&mut Recorder {
                        path: path.clone(),
                        out: &mut tmp,
                    })?;
                    let inner = tmp.get(&path).cloned().unwrap_or_else(|| "?".into());
                    let len = self.len.map(|n| n.to_string()).unwrap_or_default();
                    self.rec.record(format!("[{inner}; {len}]"))?;
                }
                Ok(())
            }
            fn end(self) -> Result<(), RecError> {
                Ok(())
            }
        }
    };
}
seq_impl!(SerializeSeq, serialize_element);
seq_impl!(SerializeTuple, serialize_element);

struct StructRec<'a, 'b> {
    rec: &'a mut Recorder<'b>,
}
impl<'a, 'b> SerializeStruct for StructRec<'a, 'b> {
    type Ok = ();
    type Error = RecError;
    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), RecError> {
        self.rec.descend(key, value)
    }
    fn end(self) -> Result<(), RecError> {
        Ok(())
    }
}

impl<'a, 'b> Serializer for &'a mut Recorder<'b> {
    type Ok = ();
    type Error = RecError;
    type SerializeSeq = SeqRec<'a, 'b>;
    type SerializeTuple = SeqRec<'a, 'b>;
    type SerializeTupleStruct = Impossible<(), RecError>;
    type SerializeTupleVariant = Impossible<(), RecError>;
    type SerializeMap = Impossible<(), RecError>;
    type SerializeStruct = StructRec<'a, 'b>;
    type SerializeStructVariant = Impossible<(), RecError>;

    fn serialize_bool(self, _: bool) -> Result<(), RecError> {
        self.record("bool")
    }
    fn serialize_i8(self, _: i8) -> Result<(), RecError> {
        self.record("i8")
    }
    fn serialize_i16(self, _: i16) -> Result<(), RecError> {
        self.record("i16")
    }
    fn serialize_i32(self, _: i32) -> Result<(), RecError> {
        self.record("i32")
    }
    fn serialize_i64(self, _: i64) -> Result<(), RecError> {
        self.record("i64")
    }
    fn serialize_u8(self, _: u8) -> Result<(), RecError> {
        self.record("u8")
    }
    fn serialize_u16(self, _: u16) -> Result<(), RecError> {
        self.record("u16")
    }
    fn serialize_u32(self, _: u32) -> Result<(), RecError> {
        self.record("u32")
    }
    fn serialize_u64(self, _: u64) -> Result<(), RecError> {
        self.record("u64")
    }
    fn serialize_f32(self, _: f32) -> Result<(), RecError> {
        self.record("f32")
    }
    fn serialize_f64(self, _: f64) -> Result<(), RecError> {
        self.record("f64")
    }
    fn serialize_char(self, _: char) -> Result<(), RecError> {
        self.record("char")
    }
    fn serialize_str(self, _: &str) -> Result<(), RecError> {
        self.record("String")
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), RecError> {
        self.record("bytes")
    }
    fn serialize_none(self) -> Result<(), RecError> {
        let inner = nullable_inner(&self.path).ok_or_else(|| {
            serde::ser::Error::custom(format!("unregistered nullable parameter: {}", self.path))
        })?;
        self.record(format!("Option<{inner}>"))
    }
    fn serialize_some<T: serde::Serialize + ?Sized>(self, value: &T) -> Result<(), RecError> {
        value.serialize(&mut *self)?;
        let inner = self
            .out
            .get(&self.path)
            .cloned()
            .unwrap_or_else(|| "?".into());
        self.record(format!("Option<{inner}>"))
    }
    fn serialize_unit(self) -> Result<(), RecError> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), RecError> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), RecError> {
        self.record(name)
    }
    fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), RecError> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), RecError> {
        Err(serde::ser::Error::custom(
            "newtype variants are not parameter leaves",
        ))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, RecError> {
        Ok(SeqRec {
            rec: self,
            len,
            recorded: false,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, RecError> {
        Ok(SeqRec {
            rec: self,
            len: Some(len),
            recorded: false,
        })
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, RecError> {
        Err(serde::ser::Error::custom(
            "tuple structs are not parameter groups",
        ))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, RecError> {
        Err(serde::ser::Error::custom(
            "tuple variants are not parameter leaves",
        ))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, RecError> {
        Err(serde::ser::Error::custom("maps are not parameter groups"))
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, RecError> {
        Ok(StructRec { rec: self })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, RecError> {
        Err(serde::ser::Error::custom(
            "struct variants are not parameter leaves",
        ))
    }
}

pub(super) fn rust_type(path: &str) -> String {
    types()
        .get(path)
        .expect("serialized field has recorded type")
        .clone()
}
pub(super) fn nullable(path: &str) -> bool {
    types()
        .get(path)
        .is_some_and(|ty| ty.starts_with("Option<"))
}

pub(super) fn metadata(path: &str, default: &Value) -> Value {
    let bounds = super::super::validation::leaf_numeric_bounds(path);
    serde_json::json!({
        "path": path, "type": rust_type(path), "nullable": nullable(path), "default": default,
        "unit": unit(path), "enum_values": super::super::validation::enum_values(path),
        "minimum": bounds.map(|b| b.0), "maximum": bounds.and_then(|b| b.1),
        "component_domains": super::super::validation::gamut_component_domains(path),
        "conditions": conditions(path), "default_source": default_source(path),
        "description": description(path)
    })
}

fn unit(path: &str) -> Option<&'static str> {
    if path == "enlarger.preflash_exposure" {
        return Some("linear exposure multiplier");
    }
    if matches!(
        path,
        "enlarger.preflash_y_filter_shift" | "enlarger.preflash_m_filter_shift"
    ) {
        return Some("Kodak CC");
    }
    if path == "output_mix.amount" || path.ends_with(".developer_exhaustion") {
        return Some("dimensionless");
    }
    if path.starts_with("film_render.grain.fotufilm.") && !path.ends_with(".material") {
        Some(if path.ends_with(".clump_radius_um") {
            "µm"
        } else if path.ends_with(".rms_density") || path.ends_with(".fog_density") {
            "optical density"
        } else {
            "dimensionless"
        })
    } else if path.ends_with("_ev") {
        Some("EV")
    } else if path.ends_with("_um2") {
        Some("µm²")
    } else if path.ends_with("_um") {
        Some("µm")
    } else if path.ends_with("_mm") {
        Some("mm")
    } else if path.ends_with("development_time") {
        Some("minutes")
    } else if matches!(
        path,
        "settings.spectral_shape" | "settings.spectral_gaussian_blur"
    ) {
        Some("nm")
    } else if path == "scanner.lens_blur" {
        Some("px (sigma)")
    } else if path == "scanner.unsharp_mask" {
        Some("[px (sigma), dimensionless (amount)]")
    } else {
        None
    }
}
fn conditions(path: &str) -> Vec<String> {
    let mut out: Vec<String> = super::effects::conditions(path)
        .into_iter()
        .map(|condition| condition.description().into())
        .collect();
    if path == "camera.film_format_mm" {
        out.push("finite and strictly positive; exposed full-source long edge, not roll width or film-stock designation".into());
    }
    if path == "scanner.receiver_profile" {
        out.push("film scans only; legacy_colorimetric preserves CIE spectral-to-output acquisition; lab_scan_approx is an approximate uncalibrated Fotufilm narrow-band receiver (630/545/465 nm, sigma 8.5/14/9.5 nm), version fotufilm_lab_scan_approx_v1".into());
    } else if path == "scanner.negative_conversion_method" {
        out.push("density reads record densities and acquires no receiver capture, so its output is receiver-independent; separable and coupled interpret a receiver capture and apply to a real negative capture or the exact-cell scan path; density behaves as separable where the incoming image is a capture".into());
        out.push("digital_reference pairs its modeled receiver on input > film > scan only; Reference Exposure requires auto_setup off, identity points and default reconstruction controls; fixed Kodak Endura receiver and Portra400 dye basis, equal-energy light and fixed 18% response".into());
    } else if path == "scanner.scan_output" {
        out.push("negative_to_positive requires a negative film profile and direct input > film > scan or input > convert-film > scan route; scanner white/black correction and active Grain Procedural are unsupported".into());
    }
    if path.starts_with("scanner.reference_") {
        out.push("output-relative stops; neutral 0; exposure multiplies all channels by 2^EV and balance multiplies independent linear-sRGB channels by 2^EV after fixed reference tone, before output-space conversion".into());
    }
    if matches!(path, "scanner.black_point" | "scanner.white_point") {
        out.push("each channel requires 0 <= black_point < white_point <= 1; points remap without clipping".into());
    }
    if path.starts_with("film_render.grain.fotufilm.") {
        out.push("authored effective-appearance control; not measured stock calibration".into());
        if super::super::validation::fotufilm_minimum_exclusive(path) {
            out.push("finite and strictly positive (each array component)".into());
        }
        if path.ends_with(".sublayer_shares") {
            out.push(
                "four positive shares must sum to 1 within 1e-6; never automatically normalized"
                    .into(),
            );
        }
    }
    out
}

fn description(path: &str) -> Option<&'static str> {
    match path {
        "output_mix.amount" => Some(
            "Overall effect amount: blends the prepared source with the complete result in linear output RGB; zero retains a negative capture as negative, one retains the full effect.",
        ),
        "enlarger.preflash_exposure" => Some(
            "Independent uniform material-plane exposure on admitted print-development routes; zero disables flash and retains its filter settings. Photographic-paper highlights may change, with possible fog, loss of paper white and color shifts; no fixed white or midgray is promised.",
        ),
        "enlarger.preflash_y_filter_shift" | "enlarger.preflash_m_filter_shift" => Some(
            "Independent preflash filtration in Kodak CC units around the calibrated neutral pack, separate from main M/Y shifts; zero shift is the neutral-pack setting.",
        ),
        path if path.ends_with(".developer_exhaustion") => Some(
            "Model-specific material development control; requires enabled chemistry, actual development and a supported fitted density-curve model. Does not provide universal highlight compression.",
        ),
        "scanner.lens_blur" => Some(
            "Optical softness in linear scanner readout before interpretation, corrections, gamut processing and digital sharpening.",
        ),
        "scanner.unsharp_mask" => {
            Some("Digital scan sharpening after optical readout and output processing.")
        }
        path if path.starts_with("print_render.glare.") => Some(
            "Print viewing/readout stray light after optical blur; does not expose the paper or form material density.",
        ),
        "film_render.grain.fotufilm.material" => Some(
            "Effective material; auto selects dye_reversal for positive films, silver for negative monochrome, and dye_negative for negative color. Override for chromogenic monochrome.",
        ),
        "film_render.grain.fotufilm.amount"
        | "film_render.grain.fotufilm.size"
        | "film_render.grain.fotufilm.softness"
        | "film_render.grain.fotufilm.color"
        | "film_render.grain.fotufilm.layers" => Some(
            "Fotufilm Film sampling-only appearance control; does not change the fitted population.",
        ),
        path if path.starts_with("film_render.grain.fotufilm.") => Some(
            "Fotufilm Film effective-appearance population control; authored metadata, not a measured stock claim.",
        ),
        "io.input_color_space" => Some(
            "Color primaries of source raster samples before simulation; not scanner output or saving conversion.",
        ),
        "io.input_cctf_decoding" => Some(
            "Decode the source raster transfer function when true; false declares linear samples. This does not encode scanner or saving output.",
        ),
        "io.output_color_space" => Some(
            "Scanner output color space; independent of source declaration and final saving color conversion.",
        ),
        "io.output_cctf_encoding" => Some(
            "Encode scanner output samples; independent of source transfer decoding and final saving encoding.",
        ),
        "camera.exposure_compensation_ev" => {
            Some("Camera exposure compensation in EV, distinct from enlarger print exposure.")
        }
        "enlarger.print_exposure" => {
            Some("Print exposure multiplier, distinct from camera exposure compensation in EV.")
        }
        "camera.film_format_mm" => Some(
            "Exposed full-source long-edge dimension in millimeters; not roll width or film-stock designation.",
        ),
        "camera.capture_illuminant" => Some(
            "Scene illuminant the filming front pass projects through; unset follows the film profile reference illuminant, and an explicit named catalog source or BB<kelvin> blackbody overrides it. Distinct from the film reference (sensitivity calibration), scan and enlarger illuminants, and the output white point; never inferred from EXIF or RAW metadata and never rewrites RAW white balance.",
        ),
        "film_render.halation.source_compression" => Some(
            "Experimental local source-response compression for halation only. Zero preserves the linear source response; positive values lift sub-anchor sources toward the brightest source before halation blur while leaving sources above the fixed anchor unchanged. This does not change the base film raw exposure and is not a measured stock threshold.",
        ),
        "io.crop_size" => Some(
            "Both components are fractions of the uncropped source long edge, not independent width and height fractions.",
        ),
        "film_render.grain.v2_size" => {
            Some("Grain Procedural Size; used only with engine procedural and profile custom.")
        }
        "film_render.grain.v2_amount" => Some(
            "Grain Procedural Amount in percent; used with named and custom Procedural profiles, distinct from RMS granularity.",
        ),
        "film_render.grain.rms_granularity" => Some(
            "RMS granularity is distinct from Procedural Amount; Dye uses it to scale per-channel event area; upstream Spektra preserves it as a no-op.",
        ),
        _ => None,
    }
}
fn default_source(path: &str) -> &'static str {
    if matches!(
        path,
        "film_render.grain.rms_granularity"
            | "film_render.grain.density_min"
            | "film_render.grain.uniformity"
            | "film_render.grain.particle_scale_sublayers"
            | "film_render.dir_couplers.gamma_samelayer_rgb"
            | "film_render.dir_couplers.gamma_interlayer_r_to_gb"
            | "film_render.dir_couplers.gamma_interlayer_g_to_rb"
            | "film_render.dir_couplers.gamma_interlayer_b_to_rg"
            | "film_render.halation.halation_first_sigma_um"
            | "film_render.halation.halation_strength"
    ) || path.ends_with("development_time")
        || path.ends_with("_filter_neutral")
    {
        "static; may be derived from stock profile or database before explicit edits"
    } else {
        "static"
    }
}
