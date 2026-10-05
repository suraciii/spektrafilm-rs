//! Calibrated deterministic LUT bundles matching spektrafilm 0.3.4.
//! Tables have canonical `[r][g][b][channel]` order (blue varies fastest).
//! Transport encodings belong to this module; runtime stages receive linear RGB
//! or physical tap values. Format writers alone reorder lattice vertices.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::lut_transport::{self, ColorSpaceEntry};
use crate::neutral_filters::NeutralFilters;
use crate::params::{InputGamutCompressParams, OutputGamutCompressParams, RuntimeParams, Tap};
use crate::params_builder::digest_params;
use crate::pipeline::Pipeline;
use crate::profile;

pub const REFERENCE_COMMIT: &str = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc";

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
        match self { Self::One => "1lut", Self::Two => "2lut", Self::Three => "3lut", Self::Four => "4lut" }
    }
    pub fn taps(self) -> &'static [Tap] {
        match self {
            Self::One => &[Tap::RgbIn, Tap::RgbOut],
            Self::Two => &[Tap::RgbIn, Tap::CmyFilm, Tap::RgbOut],
            Self::Three => &[Tap::RgbIn, Tap::LogEFilm, Tap::CmyFilm, Tap::RgbOut],
            Self::Four => &[Tap::RgbIn, Tap::LogEFilm, Tap::CmyFilm, Tap::LogEPrint, Tap::RgbOut],
        }
    }
}

/// `auto` resolves native log/HDR headroom or four stops for encoded SDR;
/// `Native(())` preserves the transport's native linear scale.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Headroom { Auto(String), Stops(f64), Native(()) }
impl Default for Headroom { fn default() -> Self { Self::Auto("auto".into()) } }
fn resolution_default() -> usize { 33 }
fn container_default() -> String { "directory".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSpec {
    pub film_profile: String,
    pub print_profiles: Vec<String>,
    pub input_color_space: String,
    pub output_color_space: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub topology: Topology,
    #[serde(default = "resolution_default")] pub resolution: usize,
    #[serde(default)] pub target: Option<String>,
    #[serde(default = "container_default")] pub container: String,
    #[serde(default)] pub qa: bool,
    #[serde(default)] pub qa_print_index: Option<usize>,
    #[serde(default)] pub input_gamut_compress: InputGamutCompressParams,
    #[serde(default, deserialize_with = "deserialize_output_gamut")] pub output_gamut_compress: OutputGamutCompressParams,
    #[serde(default)] pub stops_above_midgray: Headroom,
    #[serde(default)] pub ocio_config: bool,
    #[serde(default)] pub include_combinations: bool,
}
fn deserialize_output_gamut<'de, D: serde::Deserializer<'de>>(d: D) -> Result<OutputGamutCompressParams, D::Error> {
    #[derive(Deserialize)] #[serde(untagged)] enum Value { Name(String), Spec(OutputGamutCompressParams) }
    Ok(match Value::deserialize(d)? {
        Value::Name(algorithm) => OutputGamutCompressParams { algorithm, ..Default::default() },
        Value::Spec(spec) => spec,
    })
}
impl BundleSpec {
    pub fn validate(&self) -> Result<(), String> { self.clone().normalize() }
    pub fn normalize(&mut self) -> Result<(), String> {
        let input = lut_transport::resolve(&self.input_color_space)?;
        let output = lut_transport::resolve(&self.output_color_space)?;
        if !input.input { return Err(format!("{} is not registered as an input color space", input.name)); }
        if !output.output { return Err(format!("{} is not registered as an output color space", output.name)); }
        self.input_color_space = input.name.into();
        self.output_color_space = output.name.into();
        if self.print_profiles.is_empty() { return Err("print_profiles must contain at least one entry".into()); }
        if self.resolution < 2 { return Err("resolution must be >= 2".into()); }
        let count = self.resolution.checked_pow(3).and_then(|v| v.checked_mul(3));
        if count.is_none() || self.resolution.checked_mul(self.resolution).is_none_or(|v| v > u32::MAX as usize) || self.resolution > u32::MAX as usize {
            return Err("resolution exceeds the image lattice size limit".into());
        }
        if self.container != "directory" && self.container != "zip" { return Err("container must be directory or zip".into()); }
        if self.qa_print_index.is_some_and(|i| i >= self.print_profiles.len()) { return Err("qa_print_index is outside print_profiles".into()); }
        self.stops_above_midgray = match &self.stops_above_midgray {
            Headroom::Auto(s) if s == "auto" => Headroom::Stops(input.native_stops()),
            Headroom::Auto(s) => return Err(format!("stops_above_midgray must be a number, null, or auto; got {s:?}")),
            Headroom::Stops(n) if !n.is_finite() => return Err("stops_above_midgray must be finite".into()),
            other => other.clone(),
        };
        if self.name.is_empty() {
            let print = if self.print_profiles.len() == 1 { normalize_stock(&self.print_profiles[0]) } else { format!("{}printpack", self.print_profiles.len()) };
            self.name = format!("spektrafilm_v034_{}_{}_{}_{}_{}", normalize_stock(&self.film_profile), print, self.topology.name(), input.short_tag, output.short_tag);
        }
        Ok(())
    }
    pub fn stops(&self) -> Option<f64> {
        match self.stops_above_midgray { Headroom::Stops(n) => Some(n), _ => None }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LogEWire { pub min: f64, pub max: f64 }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DensityWire { pub d_max: [f64; 3], pub d_min: [f64; 3] }
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
                Ok(std::array::from_fn(|c| w.d_min[c] + rgb[c] * (w.d_max[c] - w.d_min[c])))
            }
            Tap::LogEFilm | Tap::LogEPrint => {
                let w = if tap == Tap::LogEFilm { self.log_e_film } else { self.log_e_print }.ok_or("missing log exposure boundary wire")?;
                Ok(rgb.map(|v| w.min + v * (w.max - w.min)))
            }
            _ => Err(format!("{} is not an intermediate boundary wire", tap.name())),
        }
    }
    pub fn encode(&self, tap: Tap, rgb: [f64; 3]) -> Result<[f64; 3], String> {
        match tap {
            Tap::CmyFilm => {
                let w = self.cmy_film.ok_or("missing cmy_film boundary wire")?;
                Ok(std::array::from_fn(|c| ((rgb[c] - w.d_min[c]) / (w.d_max[c] - w.d_min[c])).clamp(0.0, 1.0)))
            }
            Tap::LogEFilm | Tap::LogEPrint => {
                let w = if tap == Tap::LogEFilm { self.log_e_film } else { self.log_e_print }.ok_or("missing log exposure boundary wire")?;
                Ok(rgb.map(|v| (v - w.min) / (w.max - w.min)))
            }
            _ => Err(format!("{} is not an intermediate boundary wire", tap.name())),
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
    pub fn at(&self, r: usize, g: usize, b: usize) -> [f64; 3] { self.table[(r * self.resolution + g) * self.resolution + b] }
    /// Trilinear interpolation in normalized code space, clamped at the lattice edges.
    pub fn sample(&self, rgb: [f64; 3]) -> [f64; 3] {
        let n = self.resolution;
        let p = rgb.map(|v| v.clamp(0.0, 1.0) * (n - 1) as f64);
        let lo = p.map(|v| (v.floor() as usize).min(n - 2));
        let f: [f64; 3] = std::array::from_fn(|i| p[i] - lo[i] as f64);
        let mut out = [0.0; 3];
        for r in 0..2 { for g in 0..2 { for b in 0..2 {
            let weight = (if r == 0 { 1.0-f[0] } else { f[0] }) * (if g == 0 { 1.0-f[1] } else { f[1] }) * (if b == 0 { 1.0-f[2] } else { f[2] });
            let value = self.at(lo[0]+r, lo[1]+g, lo[2]+b);
            for c in 0..3 { out[c] += value[c] * weight; }
        } } }
        out
    }
    /// CUBE serialization is red-fast; internal tables remain blue-fast.
    pub fn write_cube(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut writer = std::io::BufWriter::new(file);
        writeln!(writer, "# Created by spektrafilm-rs; reference 0.3.4\nTITLE \"{}\"\nLUT_3D_SIZE {}\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1", self.title, self.resolution).map_err(|e| e.to_string())?;
        for b in 0..self.resolution { for g in 0..self.resolution { for r in 0..self.resolution {
            let p = self.at(r,g,b);
            writeln!(writer, "{:.10} {:.10} {:.10}", p[0], p[1], p[2]).map_err(|e| e.to_string())?;
        } } }
        writer.flush().map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LutFileMeta { pub role: String, pub path: String, pub domain: String, pub range: String, pub print_profile: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSpaceMeta { pub name: String, pub cctf: bool }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StocksMeta { pub film: String, pub prints: Vec<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputExposureMeta { pub stops_above_midgray: f64, pub gain: f64 }
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle { pub spec: BundleSpec, pub luts: Vec<(String, Lut)>, pub meta: BundleMeta }

pub struct BundleBuilder { pub spec: BundleSpec }
impl BundleBuilder {
    pub fn new(spec: BundleSpec) -> Self { Self { spec } }
    /// Construct once per print and evaluate each lattice as one ImageBuf.
    /// No spatial effects or stochastic grain; non-spatial DIR remains active.
    pub fn build(&self, data_dir: &Path, backend: &dyn ComputeBackend) -> Result<Bundle, String> {
        let mut spec = self.spec.clone();
        spec.normalize()?;
        let input = lut_transport::resolve(&spec.input_color_space)?;
        let output = lut_transport::resolve(&spec.output_color_space)?;
        let neutral = NeutralFilters::load(data_dir)?;
        let first = make_pipeline(&spec, &spec.print_profiles[0], input, output, data_dir, &neutral)?;
        let wires = measure_wires(&first, &spec, input, backend)?;
        let mut luts = Vec::new();
        let mut metas = Vec::new();
        let recipes = recipes(spec.topology, spec.include_combinations);
        for recipe in recipes.iter().filter(|r| r.shared) {
            let (path, lut, meta) = bake_recipe(recipe, None, &first, &spec, &wires, input, output, backend)?;
            luts.push((path,lut)); metas.push(meta);
        }
        for (i, print) in spec.print_profiles.iter().enumerate() {
            let extra;
            let pipeline = if i == 0 { &first } else { extra = make_pipeline(&spec, print, input, output, data_dir, &neutral)?; &extra };
            for recipe in recipes.iter().filter(|r| !r.shared) {
                let (path,lut,meta) = bake_recipe(recipe, Some(print), pipeline, &spec, &wires, input, output, backend)?;
                luts.push((path,lut)); metas.push(meta);
            }
        }
        let gain = input.input_gain(spec.stops());
        let mut snapshots = BTreeMap::new();
        for print in &spec.print_profiles {
            snapshots.insert(print.clone(), serde_json::json!({
                "film_profile":spec.film_profile,"print_profile":print,
                "input_color_space":input.primaries,"output_color_space":output.primaries,
                "input_cctf":input.cctf,"output_cctf":output.cctf,
                "input_cctf_decoding":false,"output_cctf_encoding":false,"lut_mode":true,
                "input_gamut_compress":spec.input_gamut_compress,"output_gamut_compress":spec.output_gamut_compress,
                "stops_above_midgray":spec.stops(),"input_exposure_gain":gain,"resolution":spec.resolution,"topology":spec.topology
            }));
        }
        let meta = BundleMeta {
            schema_version:1,name:spec.name.clone(),topology:spec.topology,resolution:spec.resolution,target:spec.target.clone(),
            provenance:BTreeMap::from([
                ("spektrafilm_version".into(),"0.3.4".into()), ("lut_creator_version".into(),"0.3.4".into()),
                ("reference_commit".into(),REFERENCE_COMMIT.into()),
                ("project_url".into(),"https://github.com/andreavolpato/spektrafilm".into()),
                ("license".into(),"spektrafilm LUT by Andrea Volpato, licensed under CC BY-SA 4.0.".into()),
                ("citation".into(),"Please cite the spektrafilm project and CITATION.cff.".into()),
            ]),
            stocks:StocksMeta {film:spec.film_profile.clone(),prints:spec.print_profiles.clone()},
            color_spaces:BTreeMap::from([("input".into(),ColorSpaceMeta{name:input.name.into(),cctf:input.cctf.is_some()}),("output".into(),ColorSpaceMeta{name:output.name.into(),cctf:output.cctf.is_some()})]),
            wires,luts:metas,input_exposure:spec.stops().map(|stops| InputExposureMeta{stops_above_midgray:stops,gain}),params_snapshot:snapshots,
            artifacts: Vec::new(),
        };
        Ok(Bundle{spec,luts,meta})
    }
}

fn make_pipeline(spec: &BundleSpec, print_stock: &str, input: &ColorSpaceEntry, output: &ColorSpaceEntry, data_dir: &Path, neutral: &NeutralFilters) -> Result<Pipeline,String> {
    let film = profile::load_profile_by_name(data_dir,&spec.film_profile).map_err(|e| e.to_string())?;
    let print = profile::load_profile_by_name(data_dir,print_stock).map_err(|e| e.to_string())?;
    let mut params = RuntimeParams::default();
    params.debug.lut_mode = true;
    params.io.input_color_space = input.primaries.into();
    params.io.output_color_space = output.primaries.into();
    params.io.input_cctf_decoding = false;
    params.io.output_cctf_encoding = false;
    params.io.input_gamut_compress = spec.input_gamut_compress.clone();
    params.io.output_gamut_compress = spec.output_gamut_compress.clone();
    params.validate()?;
    let params = digest_params(params,&film,&print,Some(neutral),true);
    Pipeline::new_with_spectral(film,print,params,data_dir)
}

/// Uniform code cube packed `[r][g][b]` into a (n*n)-wide, n-high image.
fn lattice_image(n: usize, inject: Tap, spec: &BundleSpec, wires: &BoundaryWires, input: &ColorSpaceEntry) -> Result<ImageBuf,String> {
    let mut data = Vec::with_capacity(n*n*n*3);
    let gain = input.input_gain(spec.stops());
    for r in 0..n { for g in 0..n { for b in 0..n {
        let code = [r,g,b].map(|i| i as f64/(n-1) as f64);
        let physical = if inject == Tap::RgbIn { input.decode_rgb(code).map(|v|v*gain) } else { wires.decode(inject,code)? };
        // Python explicitly casts every injected lattice to float32, including
        // when the scientific stages subsequently evaluate in float64.
        data.extend(physical.map(|v|from_f64(v as f32 as f64)));
    } } }
    Ok(ImageBuf::from_data((n*n) as u32,n as u32,data))
}
fn measure_wires(pipeline: &Pipeline,spec: &BundleSpec,input: &ColorSpaceEntry,backend: &dyn ComputeBackend) -> Result<BoundaryWires,String> {
    let mut wires = BoundaryWires::default();
    for &tap in spec.topology.taps().iter().skip(1).filter(|&&t|t != Tap::RgbOut) {
        let image = lattice_image(9,Tap::RgbIn,spec,&wires,input)?;
        let raw = pipeline.process_with_taps(image,backend,Some(Tap::RgbIn),Some(tap))?;
        if raw.data.iter().any(|v| !to_f64(*v).is_finite()) { return Err(format!("nonfinite {} probe",tap.name())); }
        if tap == Tap::CmyFilm {
            let mut max = [f64::NEG_INFINITY;3];
            for p in raw.data.chunks_exact(3) { for c in 0..3 { max[c] = max[c].max(to_f64(p[c])); } }
            wires.cmy_film = Some(DensityWire{d_min:[-0.2;3],d_max:max.map(|v|(v*1.05*10000.0).ceil()/10000.0)});
        } else {
            let lo = raw.data.iter().map(|v|to_f64(*v)).fold(f64::INFINITY,f64::min);
            let hi = raw.data.iter().map(|v|to_f64(*v)).fold(f64::NEG_INFINITY,f64::max);
            let wire = Some(LogEWire{min:((lo-0.1)*10000.0).floor()/10000.0,max:((hi+0.1)*10000.0).ceil()/10000.0});
            if tap == Tap::LogEFilm { wires.log_e_film = wire; } else { wires.log_e_print = wire; }
        }
    }
    Ok(wires)
}
struct Recipe { inject:Tap, collect:Tap, role:String, suffix:Option<String>, shared:bool, combination:bool }
fn recipes(topology:Topology,combinations:bool)->Vec<Recipe> {
    let taps = topology.taps();
    let mut result = Vec::new();
    for i in 0..taps.len()-1 {
        let (role,suffix) = match topology {
            Topology::One => ("combined",None),
            Topology::Two => if i==0 {("film",Some("film".into()))} else {("print",Some("print".into()))},
            Topology::Three | Topology::Four => {
                let role = match i {0=>"filming_expose",1=>"filming_develop",2 if topology==Topology::Three=>"printing_combined",2=>"printing_expose",_=>"printing_develop_scan"};
                (role,Some(format!("l{}",i+1)))
            }
        };
        result.push(Recipe{inject:taps[i],collect:taps[i+1],role:role.into(),suffix,shared:matches!(taps[i+1],Tap::LogEFilm|Tap::CmyFilm),combination:false});
    }
    if combinations {
        // Increasing span length matches pinned ordering: l12,l23,l34,l123,...
        for span in 2..taps.len() { for i in 0..taps.len()-span {
            let ids = (i+1..=i+span).map(|id|id.to_string()).collect::<String>();
            result.push(Recipe{inject:taps[i],collect:taps[i+span],role:format!("subchain_{ids}"),suffix:Some(format!("l{ids}")),shared:matches!(taps[i+span],Tap::LogEFilm|Tap::CmyFilm),combination:true});
        } }
    }
    result
}
#[allow(clippy::too_many_arguments)]
fn bake_recipe(recipe:&Recipe,print:Option<&str>,pipeline:&Pipeline,spec:&BundleSpec,wires:&BoundaryWires,input:&ColorSpaceEntry,output:&ColorSpaceEntry,backend:&dyn ComputeBackend)->Result<(String,Lut,LutFileMeta),String> {
    let image = lattice_image(spec.resolution,recipe.inject,spec,wires,input)?;
    let raw = pipeline.process_with_taps(image,backend,Some(recipe.inject),Some(recipe.collect))?;
    let mut table = Vec::with_capacity(spec.resolution.pow(3));
    for pixel in raw.data.chunks_exact(3) {
        let physical = [to_f64(pixel[0]),to_f64(pixel[1]),to_f64(pixel[2])];
        let code = if recipe.collect == Tap::RgbOut { output.encode_rgb(physical.map(|v|v*output.output_gain())) } else { wires.encode(recipe.collect,physical)? };
        if code.iter().any(|v|!v.is_finite()) { return Err(format!("nonfinite {} LUT sample",recipe.role)); }
        table.push(code.map(|v|v.clamp(0.0,1.0)));
    }
    let mut title = format!("v034_{}",normalize_stock(&spec.film_profile));
    if let Some(print) = print { title.push('_'); title.push_str(&normalize_stock(print)); }
    if let Some(suffix) = &recipe.suffix { title.push('_'); title.push_str(suffix); }
    let path = format!("{}lut_{}.cube",if recipe.combination {"combinations/"} else {""},title);
    let domain = if recipe.inject==Tap::RgbIn {"input_rgb"} else {recipe.inject.name()};
    let range = if recipe.collect==Tap::RgbOut {"output_rgb"} else {recipe.collect.name()};
    let meta = LutFileMeta{role:recipe.role.clone(),path:path.clone(),domain:domain.into(),range:range.into(),print_profile:print.map(str::to_owned)};
    Ok((path,Lut{resolution:spec.resolution,table,title},meta))
}
pub fn normalize_stock(stock:&str)->String {
    let lowered = stock.to_lowercase();
    let mut parts = lowered.split('_');
    let first = parts.next().unwrap_or_default();
    if ["kodak","fujifilm","fuji","ilford","agfa","cinestill","polaroid","ferrania","lomography"].contains(&first) { parts.take(2).collect() } else { std::iter::once(first).chain(parts).take(2).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(input: &str, output: &str) -> BundleSpec {
        serde_json::from_value(serde_json::json!({
            "film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"],
            "input_color_space":input, "output_color_space":output
        })).unwrap()
    }

    #[test]
    fn inactive_roles_fail_before_baking_and_slug_roles_match() {
        assert!(spec("ACEScg", "srgb").normalize().unwrap_err().contains("input color space"));
        assert!(spec("srgb", "vlog").normalize().unwrap_err().contains("output color space"));
        let mut s = spec("vlog", "rec2100pq");
        s.normalize().unwrap();
        assert_eq!(s.input_color_space,"Panasonic V-Log");
        assert_eq!(s.output_color_space,"Rec.2100 PQ");
        let input = lut_transport::resolve(&s.input_color_space).unwrap();
        assert!((input.input_gain(s.stops())-1.0).abs()<1e-12);
        assert_eq!(lut_transport::resolve("rec2100pq").unwrap().output_gain(),100.0/0.18);
        let mut sdr = spec("srgb", "srgb");
        sdr.normalize().unwrap();
        assert_eq!(sdr.stops(),Some(4.0));
        assert!((lut_transport::resolve("srgb").unwrap().input_gain(sdr.stops())-2.88).abs()<1e-12);
    }

    #[test]
    fn interpolation_preserves_channel_axis_and_clamps_domain_edges() {
        let mut table = Vec::new();
        for r in 0..3 { for g in 0..3 { for b in 0..3 {
            table.push([r as f64/2.0, g as f64/2.0, b as f64/2.0]);
        } } }
        let lut = Lut{resolution:3,table,title:"identity".into()};
        let p = lut.sample([0.13,0.71,0.37]);
        for (actual, expected) in p.into_iter().zip([0.13,0.71,0.37]) { assert!((actual-expected).abs()<1e-14); }
        assert_eq!(lut.sample([-1.0,1.0,2.0]),[0.0,1.0,1.0]);
    }

    #[test]
    fn density_wire_preserves_negative_fog_and_channel_specific_ranges() {
        let wires = BoundaryWires{cmy_film:Some(DensityWire{d_min:[-0.2;3],d_max:[1.0,2.0,3.0]}),..Default::default()};
        let physical = [-0.1,1.1,2.9];
        let code = wires.encode(Tap::CmyFilm,physical).unwrap();
        let decoded = wires.decode(Tap::CmyFilm,code).unwrap();
        for (actual, expected) in decoded.into_iter().zip(physical) { assert!((actual-expected).abs()<1e-14); }
        assert_eq!(wires.encode(Tap::CmyFilm,[-1.0,3.0,4.0]).unwrap(),[0.0,1.0,1.0]);
    }
}
