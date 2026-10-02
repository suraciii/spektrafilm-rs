//! Standalone OCIO 2.4 configs for delivered LUT bundles (Python 0.3.4 parity).
//! Chains operate on normalized wire codes; physical decode constants are exposed
//! in intermediate descriptions. No inverse film simulation is advertised.
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path};

use crate::lut_baker::{normalize_stock, Bundle, BundleMeta, LutFileMeta, Topology};
use crate::lut_delivery::ArtifactReference;
use crate::lut_transport;

const REFERENCE: &str = "ACES2065-1";
fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(ErrorKind::InvalidInput, message.into()) }
fn quote(value: &str) -> String {
    // JSON quoted scalars are also YAML quoted scalars, including control escapes.
    serde_json::to_string(value).expect("serializing a string cannot fail")
}
fn builtins(name: &str) -> io::Result<Vec<(&'static str, bool)>> {
    let camera = match name {
        "ACES2065-1" => return Ok(vec![]),
        "ACEScg" => "ACEScg_to_ACES2065-1",
        "ACEScct" => "ACEScct_to_ACES2065-1",
        "ACEScc" => "ACEScc_to_ACES2065-1",
        "Panasonic V-Log" => "PANASONIC_VLOG-VGAMUT_to_ACES2065-1",
        "Sony S-Log3" => "SONY_SLOG3-SGAMUT3_to_ACES2065-1",
        "Sony S-Log3 (S-Gamut3.Cine)" => "SONY_SLOG3-SGAMUT3.CINE_to_ACES2065-1",
        "ARRI LogC3 (EI800)" => "ARRI_ALEXA-LOGC-EI800-AWG_to_ACES2065-1",
        "ARRI LogC4" => "ARRI_LOGC4_to_ACES2065-1",
        "Apple Log" => "APPLE_LOG_to_ACES2065-1",
        "Canon Log 3" => "CANON_CLOG3-CGAMUT_to_ACES2065-1",
        "RED Log3G10" => "RED_LOG3G10-RWG_to_ACES2065-1",
        _ => {
            let display = match name {
                "sRGB" => "DISPLAY - CIE-XYZ-D65_to_sRGB",
                "Rec.709" => "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709",
                "Rec.2020" => "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020",
                "Display P3" => "DISPLAY - CIE-XYZ-D65_to_DisplayP3",
                "DCI-P3" => "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-DCI-BFD",
                "Rec.2100 PQ" => "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ",
                "Rec.2100 HLG" => "DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit",
                "P3-D65 PQ" => "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65",
                _ => return Err(invalid(format!("color space {name:?} has no OCIO BuiltinTransform mapping"))),
            };
            return Ok(vec![("UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD", false), (display, false)]);
        }
    };
    Ok(vec![(camera, true)])
}
fn encoding(name: &str) -> io::Result<&'static str> {
    let entry = lut_transport::resolve(name).map_err(invalid)?;
    Ok(match entry.kind { "log" => "log", "encoded_sdr" => "sdr-video", _ => "scene-linear" })
}
fn humanize(stock: &str) -> String {
    stock.split('_').map(|word| {
        let mut chars = word.chars();
        chars.next().map(|first| first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}
fn final_name(film: &str, print: &str) -> String { format!("spektrafilm_{}_{}", normalize_stock(film), normalize_stock(print)) }
fn io_space(out: &mut String, name: &str, family: &str) -> io::Result<()> {
    let entry = lut_transport::resolve(name).map_err(invalid)?;
    writeln!(out, "  - !<ColorSpace>\n    name: {}", quote(name)).unwrap();
    if !entry.ocio_alias.is_empty() { writeln!(out, "    aliases: [{}]", quote(entry.ocio_alias)).unwrap(); }
    writeln!(out, "    family: {family}\n    encoding: {}\n    isdata: false\n    from_scene_reference: !<GroupTransform>\n      children:", encoding(name)?).unwrap();
    for (style, inverse) in builtins(name)? {
        writeln!(out, "        - !<BuiltinTransform> {{style: {}{}}}", quote(style), if inverse { ", direction: inverse" } else { "" }).unwrap();
    }
    Ok(())
}
fn chain_space(out: &mut String, name: &str, family: &str, kind: &str, description: &str, input: &str, chain: &[&LutFileMeta]) {
    writeln!(out, "  - !<ColorSpace>\n    name: {}\n    family: {}\n    encoding: {kind}\n    description: |", quote(name), quote(family)).unwrap();
    for line in description.lines() { writeln!(out, "      {line}").unwrap(); }
    writeln!(out, "    isdata: false\n    from_scene_reference: !<GroupTransform>\n      children:\n        - !<ColorSpaceTransform> {{src: {}, dst: {}}}", quote(REFERENCE), quote(input)).unwrap();
    for lut in chain { writeln!(out, "        - !<FileTransform> {{src: {}, interpolation: tetrahedral}}", quote(&lut.path)).unwrap(); }
}
fn wire_description(meta: &BundleMeta, tap: &str) -> io::Result<String> {
    if tap == "cmy_film" {
        let w = meta.wires.cmy_film.ok_or_else(|| invalid("missing cmy_film wire"))?;
        for c in 0..3 { if !w.d_min[c].is_finite() || !w.d_max[c].is_finite() || w.d_min[c] >= w.d_max[c] { return Err(invalid(format!("malformed cmy_film wire channel {c}: finite d_min < d_max required"))); } }
        let triplet = |values: [f64; 3]| values.map(|v| format!("{v:.4}")).join(", ");
        return Ok(format!("Normalized CMY film density after development (per-channel).\nEncoding: code_c = clip((D_c - d_min_c) / (d_max_c - d_min_c), 0, 1)\n  d_min: ({})\n  d_max: ({})\nDecode: D_c = code_c * (d_max_c - d_min_c) + d_min_c\nAsymmetric: from_scene_reference only. Apply grain or other\ndensity-domain effects here; continue the chain with the\nremaining .cube file(s) directly via FileTransform.", triplet(w.d_min), triplet(w.d_max)));
    }
    let w = if tap == "log_e_film" { meta.wires.log_e_film } else { meta.wires.log_e_print }.ok_or_else(|| invalid(format!("missing {tap} wire")))?;
    if !w.min.is_finite() || !w.max.is_finite() || w.min >= w.max { return Err(invalid(format!("malformed {tap} wire: finite min < max required"))); }
    let stage = if tap == "log_e_film" { "film" } else { "print" };
    let advice = if stage == "film" { "Apply halation, light scattering, or pre-development spatial\neffects in linear-light exposure (after the 10^ decode); continue\nthe chain with the remaining .cube file(s) directly via FileTransform." } else { "Apply enlarger-stage effects (diffusion filters, dodge/burn) here;\ncontinue the chain with the remaining .cube file(s) directly via\nFileTransform." };
    Ok(format!("Normalized log10(exposure) at the {stage} (shared across channels).\nEncoding: code = (log_e - min) / (max - min)\n  min: {:.4}\n  max: {:.4}\nDecode: log10(E) = code * (max - min) + min; E = 10^log10(E).\nAsymmetric: from_scene_reference only. {advice}", w.min, w.max))
}
fn chain_for_print<'a>(root: &Path, meta: &'a BundleMeta, print: &str) -> io::Result<Vec<&'a LutFileMeta>> {
    let stages: &[(&str, &str, &str, bool)] = match meta.topology {
        Topology::One => &[("combined", "input_rgb", "output_rgb", false)],
        Topology::Two => &[("film", "input_rgb", "cmy_film", true), ("print", "cmy_film", "output_rgb", false)],
        Topology::Three => &[("filming_expose", "input_rgb", "log_e_film", true), ("filming_develop", "log_e_film", "cmy_film", true), ("printing_combined", "cmy_film", "output_rgb", false)],
        Topology::Four => &[("filming_expose", "input_rgb", "log_e_film", true), ("filming_develop", "log_e_film", "cmy_film", true), ("printing_expose", "cmy_film", "log_e_print", false), ("printing_develop_scan", "log_e_print", "output_rgb", false)],
    };
    let mut chain = Vec::new();
    for &(role, domain, range, shared) in stages {
        let candidates = meta.luts.iter().filter(|lut| lut.role == role && if shared { lut.print_profile.is_none() } else { lut.print_profile.as_deref() == Some(print) }).filter(|lut| Path::new(&lut.path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cube"))).collect::<Vec<_>>();
        if candidates.len() != 1 { return Err(invalid(format!("print {print:?} requires exactly one delivered .cube reference for {role}; found {}", candidates.len()))); }
        let lut = candidates[0];
        if lut.domain != domain || lut.range != range { return Err(invalid(format!("malformed {role} wire: expected {domain} -> {range}, got {} -> {}", lut.domain, lut.range))); }
        let path = Path::new(&lut.path);
        if lut.path.is_empty() || lut.path.contains('\\') || !path.components().all(|c| matches!(c, Component::Normal(_))) { return Err(invalid(format!("LUT reference must be a portable relative path: {:?}", lut.path))); }
        if !root.join(path).is_file() { return Err(io::Error::new(ErrorKind::NotFound, format!("missing referenced LUT {:?}", lut.path))); }
        chain.push(lut);
    }
    Ok(chain)
}

/// Write a standalone config using actual delivered Cube paths. Call after
/// `write_bundle_files`, append the returned reference, then finalize delivery.
/// Rejects unsupported transports, invalid boundary wires and absent LUTs
/// before writing config.ocio. Multiple delivery formats select the Cube copy.
pub fn emit(root: &Path, bundle: &Bundle, meta: &BundleMeta) -> io::Result<Vec<ArtifactReference>> {
    let text = render(root, bundle, meta)?;
    fs::write(root.join("config.ocio"), text)?;
    Ok(vec![ArtifactReference { path: "config.ocio".into(), kind: "ocio_config".into(), description: "Standalone OCIO 2 config with film/print colorspaces and normalized intermediate taps".into() }])
}
fn render(root: &Path, bundle: &Bundle, meta: &BundleMeta) -> io::Result<String> {
    let spec = &bundle.spec;
    if meta.topology != spec.topology || meta.stocks.film != spec.film_profile || meta.stocks.prints != spec.print_profiles { return Err(invalid("delivered metadata does not match bundle topology/stocks")); }
    if spec.print_profiles.is_empty() { return Err(invalid("OCIO config requires at least one print")); }
    builtins(&spec.input_color_space)?; builtins(&spec.output_color_space)?;
    for (key, expected) in [("input", &spec.input_color_space), ("output", &spec.output_color_space)] {
        let encoded = lut_transport::resolve(expected).map_err(invalid)?.cctf.is_some();
        if meta.color_spaces.get(key).is_none_or(|space| &space.name != expected || space.cctf != encoded) { return Err(invalid(format!("OCIO {key} requires matching color-space transfer metadata"))); }
    }
    let mut out = format!("ocio_profile_version: 2.4\nname: {}\ndescription: {}\nsearch_path: .\nfamily_separator: /\nroles:\n", quote(&meta.name), quote(&format!("Standalone spektrafilm {} config: {} -> {}; reference ACES2065-1 (AP0)", meta.topology.name(), spec.film_profile, spec.print_profiles.join(", "))));
    for role in ["aces_interchange", "color_timing", "compositing_log", "default", "scene_linear"] { writeln!(out, "  {role}: {REFERENCE}").unwrap(); }
    writeln!(out, "displays:\n  {}:", quote(&spec.output_color_space)).unwrap();
    let mut views = Vec::new();
    if meta.topology == Topology::One {
        for print in &spec.print_profiles {
            let view = format!("Spektrafilm {} / {}", humanize(&spec.film_profile), humanize(print));
            writeln!(out, "    - !<View> {{name: {}, colorspace: {}}}", quote(&view), quote(&final_name(&spec.film_profile, print))).unwrap(); views.push(quote(&view));
        }
    }
    views.push(quote("Raw"));
    writeln!(out, "    - !<View> {{name: Raw, colorspace: {}}}\nactive_displays: [{}]\nactive_views: [{}]\ncolorspaces:\n  - !<ColorSpace>\n    name: {}\n    aliases: [\"ACES - ACES2065-1\", lin_ap0]\n    family: ACES\n    encoding: scene-linear\n    isdata: false", quote(&spec.output_color_space), quote(&spec.output_color_space), views.join(", "), quote(REFERENCE)).unwrap();
    if spec.input_color_space != REFERENCE { io_space(&mut out, &spec.input_color_space, "Input")?; }
    if spec.output_color_space != REFERENCE && spec.output_color_space != spec.input_color_space { io_space(&mut out, &spec.output_color_space, "Output")?; }
    let mut names = BTreeSet::new();
    for print in &spec.print_profiles {
        let chain = chain_for_print(root, meta, print)?;
        let film_tag = normalize_stock(&spec.film_profile);
        let print_tag = normalize_stock(print);
        for (i, lut) in chain.iter().enumerate().take(chain.len() - 1) {
            let tap = &lut.range;
            let (name, family) = if tap == "log_e_print" { (format!("{tap}_{film_tag}_{print_tag}"), format!("spektrafilm/intermediates/{film_tag}/{print_tag}")) } else { (format!("{tap}_{film_tag}"), format!("spektrafilm/intermediates/{film_tag}")) };
            if names.insert(name.clone()) { chain_space(&mut out, &name, &family, "log", &wire_description(meta, tap)?, &spec.input_color_space, &chain[..=i]); }
        }
        let name = final_name(&spec.film_profile, print);
        if !names.insert(name.clone()) { return Err(invalid(format!("duplicate normalized film/print colorspace {name:?}"))); }
        chain_space(&mut out, &name, &format!("spektrafilm/{film_tag}/{print_tag}"), encoding(&spec.output_color_space)?, &format!("Spektrafilm film simulation: {} negative\nprinted on {print}, output as {}.\nTopology: {}.\nAsymmetric: from_scene_reference defined,\nto_scene_reference undefined (no inverse LUT in this bundle).", spec.film_profile, spec.output_color_space, meta.topology.name()), &spec.input_color_space, &chain);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Bundle {
        serde_json::from_value(serde_json::json!({
            "spec": {"film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"], "input_color_space":"sRGB", "output_color_space":"sRGB", "topology":"2lut"},
            "luts": [],
            "meta": {"schema_version":1,"name":"boundary","topology":"2lut","resolution":2,"target":null,"provenance":{},"stocks":{"film":"kodak_portra_400","prints":["kodak_portra_endura"]},"color_spaces":{"input":{"name":"sRGB","cctf":true},"output":{"name":"sRGB","cctf":true}},"wires":{"cmy_film":{"d_min":[-0.2,-0.2,-0.2],"d_max":[1,2,3]}},"luts":[{"role":"film","path":"film.cube","domain":"input_rgb","range":"cmy_film","print_profile":null},{"role":"print","path":"print.cube","domain":"cmy_film","range":"output_rgb","print_profile":"kodak_portra_endura"}],"input_exposure":null,"params_snapshot":{}}
        })).unwrap()
    }
    #[test]
    fn rejects_degenerate_and_nonfinite_density_and_exposure_wires() {
        let mut bundle = fixture();
        bundle.meta.wires.cmy_film.as_mut().unwrap().d_max[1] = -0.2;
        assert!(wire_description(&bundle.meta, "cmy_film").unwrap_err().to_string().contains("channel 1"));
        bundle.meta.wires.log_e_film = Some(crate::lut_baker::LogEWire { min: f64::NAN, max: 1.0 });
        assert!(wire_description(&bundle.meta, "log_e_film").is_err());
        bundle.meta.wires.log_e_print = Some(crate::lut_baker::LogEWire { min: 2.0, max: 1.0 });
        assert!(wire_description(&bundle.meta, "log_e_print").is_err());
        bundle.meta.wires.cmy_film = None;
        assert!(wire_description(&bundle.meta, "cmy_film").is_err());
    }
    #[test]
    fn rejects_missing_lut_metadata_and_files_and_wrong_wire_order() {
        let mut bundle = fixture();
        let root = std::env::temp_dir().join(format!("spektrafilm-ocio-missing-{}", std::process::id()));
        assert_eq!(render(&root, &bundle, &bundle.meta).unwrap_err().kind(), ErrorKind::NotFound);
        bundle.meta.luts[0].range = "log_e_film".into();
        assert!(render(&root, &bundle, &bundle.meta).unwrap_err().to_string().contains("expected input_rgb -> cmy_film"));
        bundle.meta.luts.remove(0);
        assert!(render(&root, &bundle, &bundle.meta).unwrap_err().to_string().contains("exactly one delivered .cube reference"));
    }
}
