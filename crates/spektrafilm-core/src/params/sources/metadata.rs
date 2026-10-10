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
        "io.output_gamut_compress.lightness_compression" => "[f32; 3]",
        "taps.inject" | "taps.collect" => "String",
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
        "conditions": conditions(path), "default_source": default_source(path)
    })
}

fn unit(path: &str) -> Option<&'static str> {
    if path.ends_with("_ev") {
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
    let mut out = Vec::new();
    if matches!(
        path,
        "film_render.grain.rms_granularity" | "film_render.grain.micro_sublayers"
    ) || path.starts_with("film_render.glare.")
    {
        out.push("preserved upstream no-op".into());
        return out;
    }
    if path.starts_with("film_render.grain.") {
        if !path.ends_with(".active") && !path.ends_with(".engine") {
            out.push("film_render.grain.active = true; stochastic effects enabled".into());
        }
        if path.contains(".v2_") {
            out.push("film_render.grain.engine = v2".into());
            // resolved_grain_v2 applies v2_amount/v2_timer/v2_resolution_type
            // for named profiles; all other v2 controls are custom-only.
            if !["v2_profile", "v2_amount", "v2_timer", "v2_resolution_type"]
                .iter()
                .any(|s| path.ends_with(s))
            {
                out.push("film_render.grain.v2_profile = custom".into());
            }
            if path.ends_with("v2_timer") {
                out.push(
                    "0 selects seed-derived phase; otherwise phase is strictly below 1".into(),
                );
            }
        } else if path.contains(".v3_") {
            out.push("film_render.grain.engine = v3".into());
            out.push("film grain V3 selected under its supported stock, format and route".into());
        } else if !path.ends_with(".active") && !path.ends_with(".engine") {
            out.push("film_render.grain.engine = v1".into());
            if path.ends_with(".n_sub_layers") {
                out.push("film_render.grain.sublayers_active = false".into());
            } else if [
                "particle_scale_sublayers",
                "blur_dye_clouds_um",
                "micro_structure",
                "mult_usm_sigma",
                "mult_usm_amount",
            ]
            .iter()
            .any(|field| path.ends_with(field))
            {
                out.push("film_render.grain.sublayers_active = true; film profile supplies sublayer density curves".into());
            }
        }
    } else if path.contains("diffusion_filter.") {
        out.push(
            "diffusion_filter.active; strength and spatial_scale positive; spatial effects enabled"
                .into(),
        );
    } else if matches!(
        path,
        "film_render.halation.boost_ev"
            | "film_render.halation.boost_range"
            | "film_render.halation.protect_ev"
    ) {
        out.push("film_render.halation.boost_ev != 0; LUT mode disabled".into());
    } else if path.contains(".halation.") {
        out.push("film_render.halation.active; spatial effects enabled".into());
    } else if path.starts_with("print_render.glare.") {
        out.push("io.scan_film = false; print_render.glare.active; print_render.glare.percent > 0; stochastic effects enabled".into());
    } else if path.contains(".dir_couplers.") {
        out.push("film_render.dir_couplers.active".into());
    } else if path == "camera.auto_exposure_method" {
        out.push("camera.auto_exposure = true".into());
    } else if path.starts_with("io.crop_") {
        out.push("io.crop = true".into());
    } else if path == "debug.print_timings" {
        out.push("declared upstream no-op".into());
    } else if path == "scanner.scan_output" {
        out.push("positive_scan requires a negative film and workflow.route = input > film > scan; scanner white/black correction and active Grain V2 are unsupported".into());
    } else if path.starts_with("magazine_print_color.") {
        out.push("workflow.route = input > film > scan > magazine".into());
        if path.ends_with(".strength") {
            out.push("magazine_print_color.active = true".into());
        } else {
            out.push("magazine_print_color.strength > 0".into());
        }
    }
    out
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
