//! Public LUT bundle data model and wire encodings.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::lut_transport::{self, ColorSpaceEntry};
use crate::params::{InputGamutCompressParams, OutputGamutCompressParams, RuntimeParams, Tap};

pub fn normalize_stock(stock: &str) -> String {
    let lowered = stock.to_lowercase();
    let mut parts = lowered.split('_');
    let first = parts.next().unwrap_or_default();
    if [
        "kodak",
        "fujifilm",
        "fuji",
        "ilford",
        "agfa",
        "cinestill",
        "polaroid",
        "ferrania",
        "lomography",
    ]
    .contains(&first)
    {
        parts.take(2).collect()
    } else {
        std::iter::once(first).chain(parts).take(2).collect()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Topology {
    #[default]
    #[serde(rename = "1lut")]
    One,
    #[serde(rename = "2lut")]
    Two,
    #[serde(rename = "3lut")]
    Three,
    #[serde(rename = "4lut")]
    Four,
}
impl Topology {
    pub fn name(self) -> &'static str {
        match self {
            Self::One => "1lut",
            Self::Two => "2lut",
            Self::Three => "3lut",
            Self::Four => "4lut",
        }
    }
    pub fn taps(self) -> &'static [Tap] {
        match self {
            Self::One => &[Tap::RgbIn, Tap::RgbOut],
            Self::Two => &[Tap::RgbIn, Tap::CmyFilm, Tap::RgbOut],
            Self::Three => &[Tap::RgbIn, Tap::LogEFilm, Tap::CmyFilm, Tap::RgbOut],
            Self::Four => &[
                Tap::RgbIn,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::LogEPrint,
                Tap::RgbOut,
            ],
        }
    }
}

fn resolution_default() -> usize {
    33
}
fn container_default() -> String {
    "directory".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSpec {
    pub film_profile: String,
    pub print_profiles: Vec<String>,
    pub input_color_space: String,
    pub output_color_space: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub topology: Topology,
    #[serde(default = "resolution_default")]
    pub resolution: usize,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default = "container_default")]
    pub container: String,
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub qa_print_index: Option<usize>,
    #[serde(default)]
    pub input_gamut_compress: InputGamutCompressParams,
    #[serde(default, deserialize_with = "deserialize_output_gamut")]
    pub output_gamut_compress: OutputGamutCompressParams,
    /// Effective upstream exposure stops; omitted/`"auto"` resolves from input.
    #[serde(default, deserialize_with = "deserialize_stops")]
    pub stops_above_midgray: Option<f64>,
    /// True only for explicit `native`/`null`; numeric zero remains a real stop value.
    #[serde(skip)]
    native_input_gain: bool,
    /// Legacy deliberate exposure adjustment, retained for compatibility.
    #[serde(default)]
    pub exposure_ev: f64,
    #[serde(default)]
    pub ocio_config: bool,
    #[serde(default)]
    pub include_combinations: bool,
}
fn deserialize_stops<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Number(f64),
        Text(String),
        Null(()),
    }
    match Value::deserialize(d)? {
        Value::Number(value) => Ok(Some(value)),
        Value::Text(value) if value == "auto" => Ok(None),
        Value::Text(value) if value == "native" => Ok(Some(f64::NAN)),
        Value::Text(value) => Err(serde::de::Error::custom(format!(
            "invalid stops_above_midgray {value:?}"
        ))),
        Value::Null(()) => Ok(Some(f64::NAN)),
    }
}
fn deserialize_output_gamut<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<OutputGamutCompressParams, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Name(String),
        Spec(OutputGamutCompressParams),
    }
    Ok(match Value::deserialize(d)? {
        Value::Name(algorithm) => OutputGamutCompressParams {
            algorithm,
            ..Default::default()
        },
        Value::Spec(spec) => spec,
    })
}
impl BundleSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.clone().normalize()
    }
    /// Resolve a raw spec's input stops, applying legacy EV and retaining its metadata.
    /// BundleBuilder::build owns this mutation; callers should validate raw specs instead.
    pub fn normalize(&mut self) -> Result<(), String> {
        let input = lut_transport::resolve(&self.input_color_space)?;
        let output = lut_transport::resolve(&self.output_color_space)?;
        if !input.input {
            return Err(format!(
                "{} is not registered as an input color space",
                input.name
            ));
        }
        if !output.output {
            return Err(format!(
                "{} is not registered as an output color space",
                output.name
            ));
        }
        self.input_color_space = input.name.into();
        self.output_color_space = output.name.into();
        let requested_native =
            self.native_input_gain || self.stops_above_midgray.is_some_and(|value| value.is_nan());
        let mut stops = if requested_native {
            0.0
        } else {
            self.stops_above_midgray
                .unwrap_or_else(|| input.auto_stops_above_midgray())
        };
        if self.exposure_ev != 0.0 {
            stops += self.exposure_ev;
        }
        if !stops.is_finite() {
            return Err("stops_above_midgray must be finite".into());
        }
        self.native_input_gain = requested_native && self.exposure_ev == 0.0;
        self.stops_above_midgray = Some(stops);
        if self.print_profiles.is_empty() {
            return Err("print_profiles must contain at least one entry".into());
        }
        if self.resolution < 2 {
            return Err("resolution must be >= 2".into());
        }
        let count = self
            .resolution
            .checked_pow(3)
            .and_then(|v| v.checked_mul(3));
        if count.is_none()
            || self
                .resolution
                .checked_mul(self.resolution)
                .is_none_or(|v| v > u32::MAX as usize)
            || self.resolution > u32::MAX as usize
        {
            return Err("resolution exceeds the image lattice size limit".into());
        }
        if self.container != "directory" && self.container != "zip" {
            return Err("container must be directory or zip".into());
        }
        if self
            .qa_print_index
            .is_some_and(|i| i >= self.print_profiles.len())
        {
            return Err("qa_print_index is outside print_profiles".into());
        }
        if !self.exposure_ev.is_finite() {
            return Err("exposure_ev must be finite".into());
        }
        if self.name.is_empty() {
            let print = if self.print_profiles.len() == 1 {
                normalize_stock(&self.print_profiles[0])
            } else {
                format!("{}printpack", self.print_profiles.len())
            };
            self.name = format!(
                "spektrafilm_v034_{}_{}_{}_{}_{}",
                normalize_stock(&self.film_profile),
                print,
                self.topology.name(),
                input.short_tag,
                output.short_tag
            );
        }
        Ok(())
    }
    pub(crate) fn uses_native_input_gain(&self) -> bool {
        self.native_input_gain
    }
}
pub(super) fn effective_input_gain(spec: &BundleSpec, input: &ColorSpaceEntry) -> f64 {
    if spec.native_input_gain {
        input.native_input_gain()
    } else {
        input.input_gain_for_stops(spec.stops_above_midgray.expect("normalized BundleSpec"))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LogEWire {
    pub min: f64,
    pub max: f64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DensityWire {
    pub d_max: [f64; 3],
    pub d_min: [f64; 3],
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoundaryWires {
    pub log_e_film: Option<LogEWire>,
    pub cmy_film: Option<DensityWire>,
    pub log_e_print: Option<LogEWire>,
    pub cmy_print: Option<DensityWire>,
}
impl BoundaryWires {
    pub fn decode(&self, tap: Tap, rgb: [f64; 3]) -> Result<[f64; 3], String> {
        match tap {
            Tap::CmyFilm => {
                let w = self.cmy_film.ok_or("missing cmy_film boundary wire")?;
                Ok(std::array::from_fn(|c| {
                    w.d_min[c] + rgb[c] * (w.d_max[c] - w.d_min[c])
                }))
            }
            Tap::LogEFilm | Tap::LogEPrint => {
                let w = if tap == Tap::LogEFilm {
                    self.log_e_film
                } else {
                    self.log_e_print
                }
                .ok_or("missing log exposure boundary wire")?;
                Ok(rgb.map(|v| w.min + v * (w.max - w.min)))
            }
            _ => Err(format!(
                "{} is not an intermediate boundary wire",
                tap.name()
            )),
        }
    }
    pub fn encode(&self, tap: Tap, rgb: [f64; 3]) -> Result<[f64; 3], String> {
        match tap {
            Tap::CmyFilm => {
                let w = self.cmy_film.ok_or("missing cmy_film boundary wire")?;
                Ok(std::array::from_fn(|c| {
                    ((rgb[c] - w.d_min[c]) / (w.d_max[c] - w.d_min[c])).clamp(0.0, 1.0)
                }))
            }
            Tap::LogEFilm | Tap::LogEPrint => {
                let w = if tap == Tap::LogEFilm {
                    self.log_e_film
                } else {
                    self.log_e_print
                }
                .ok_or("missing log exposure boundary wire")?;
                Ok(rgb.map(|v| (v - w.min) / (w.max - w.min)))
            }
            _ => Err(format!(
                "{} is not an intermediate boundary wire",
                tap.name()
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lut {
    pub resolution: usize,
    /// Index `(r * resolution + g) * resolution + b`; blue varies fastest.
    pub table: Vec<[f64; 3]>,
    pub title: String,
}
impl Lut {
    pub fn at(&self, r: usize, g: usize, b: usize) -> [f64; 3] {
        self.table[(r * self.resolution + g) * self.resolution + b]
    }
    /// Trilinear interpolation in normalized code space, clamped at the lattice edges.
    pub fn sample(&self, rgb: [f64; 3]) -> [f64; 3] {
        let n = self.resolution;
        let p = rgb.map(|v| v.clamp(0.0, 1.0) * (n - 1) as f64);
        let lo = p.map(|v| (v.floor() as usize).min(n - 2));
        let f: [f64; 3] = std::array::from_fn(|i| p[i] - lo[i] as f64);
        let mut out = [0.0; 3];
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    let weight = (if r == 0 { 1.0 - f[0] } else { f[0] })
                        * (if g == 0 { 1.0 - f[1] } else { f[1] })
                        * (if b == 0 { 1.0 - f[2] } else { f[2] });
                    let value = self.at(lo[0] + r, lo[1] + g, lo[2] + b);
                    for c in 0..3 {
                        out[c] += value[c] * weight;
                    }
                }
            }
        }
        out
    }
    /// CUBE serialization is red-fast; internal tables remain blue-fast.
    pub fn write_cube(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut writer = std::io::BufWriter::new(file);
        writeln!(writer, "# Created by spektrafilm-rs; reference 0.3.4\nTITLE \"{}\"\nLUT_3D_SIZE {}\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1", self.title, self.resolution).map_err(|e| e.to_string())?;
        for b in 0..self.resolution {
            for g in 0..self.resolution {
                for r in 0..self.resolution {
                    let p = self.at(r, g, b);
                    writeln!(writer, "{:.10} {:.10} {:.10}", p[0], p[1], p[2])
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        writer.flush().map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LutFileMeta {
    pub role: String,
    pub path: String,
    pub domain: String,
    pub range: String,
    pub print_profile: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSpaceMeta {
    pub name: String,
    pub cctf: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StocksMeta {
    pub film: String,
    pub prints: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputExposureMeta {
    pub stops_above_midgray: f64,
    pub exposure_ev: f64,
    pub gain: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMeta {
    pub schema_version: u32,
    pub name: String,
    pub topology: Topology,
    pub resolution: usize,
    pub target: Option<String>,
    pub provenance: BTreeMap<String, String>,
    pub stocks: StocksMeta,
    pub color_spaces: BTreeMap<String, ColorSpaceMeta>,
    pub wires: BoundaryWires,
    pub luts: Vec<LutFileMeta>,
    pub input_exposure: Option<InputExposureMeta>,
    pub params_snapshot: BTreeMap<String, serde_json::Value>,
    /// SHA-256 of each complete serialized snapshot, including digest disclosure.
    #[serde(default)]
    pub params_digest: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub spec: BundleSpec,
    pub luts: Vec<(String, Lut)>,
    pub meta: BundleMeta,
    #[serde(default)]
    pub baked_params: BTreeMap<String, RuntimeParams>,
}
