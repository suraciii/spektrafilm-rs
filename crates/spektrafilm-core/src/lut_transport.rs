//! Pinned LUT color-space transport, matching Python color_spaces.py and colour defaults.
//! HLG's display transform couples RGB channels; use the RGB methods for color data.

#[derive(Debug)]
pub struct ColorSpaceEntry {
    pub name: &'static str,
    pub short_tag: &'static str,
    pub primaries: &'static str,
    pub input: bool,
    pub output: bool,
    pub midgray_linear: f64,
    pub kind: &'static str,
    pub cctf: Option<&'static str>,
    pub scene_referred_input: bool,
    pub ocio_alias: &'static str,
    curve: Curve,
}

#[derive(Clone, Copy, Debug)]
enum Curve {
    Linear, Srgb, Bt1886, Gamma(f64), Acescc, Acescct, Logc3, Logc4,
    Slog3, Vlog, Flog, Flog2, CanonLog3, Log3g10, DaVinci, Apple,
    Blackmagic, Pq, Hlg, Dlog, ProPhoto, Nlog,
}

pub fn registry() -> &'static [ColorSpaceEntry] { REGISTRY }

/// Resolve canonical names and exact short tags. Role eligibility is checked by callers.
pub fn resolve(name: &str) -> Result<&'static ColorSpaceEntry, String> {
    REGISTRY.iter().find(|entry| entry.name == name || entry.short_tag == name)
        .ok_or_else(|| format!("Unknown color space {name:?}"))
}

impl ColorSpaceEntry {
    /// Scalar HLG treats the sample as a neutral RGB triplet, as colour does.
    pub fn decode(&self, value: f64) -> f64 {
        if matches!(self.curve, Curve::Hlg) { self.decode_rgb([value; 3])[0] }
        else { self.curve.decode(value) }
    }

    pub fn encode(&self, value: f64) -> f64 {
        if matches!(self.curve, Curve::Hlg) { self.encode_rgb([value; 3])[0] }
        else { self.curve.encode(value) }
    }

    pub fn decode_rgb(&self, rgb: [f64; 3]) -> [f64; 3] {
        if matches!(self.curve, Curve::Hlg) {
            let scene = rgb.map(|x| hlg_inverse_oetf(x.clamp(0.0, 1.0)));
            let gain = 1000.0 * hlg_luminance(scene).abs().powf(1.2 - 1.0);
            scene.map(|x| x * gain)
        } else { rgb.map(|x| self.curve.decode(x)) }
    }

    pub fn encode_rgb(&self, rgb: [f64; 3]) -> [f64; 3] {
        if matches!(self.curve, Curve::Hlg) {
            let display = rgb.map(nonnegative);
            let luminance = hlg_luminance(display);
            if luminance == 0.0 { return [0.0; 3]; }
            let gain = (luminance / 1000.0).abs().powf((1.0 - 1.2) / 1.2);
            display.map(|x| hlg_oetf(gain * x / 1000.0))
        } else { rgb.map(|x| self.curve.encode(x)) }
    }

    pub fn native_stops(&self) -> f64 {
        if self.scene_referred_input { 6.0 }
        else if self.kind == "encoded_sdr" { 4.0 }
        else { (self.decode(1.0) / self.midgray_linear).log2() }
    }

    pub fn input_gain(&self, stops: Option<f64>) -> f64 {
        stops.map_or(1.0, |stops| 0.18 * stops.exp2() / self.decode(1.0))
    }

    pub fn output_gain(&self) -> f64 { self.midgray_linear / 0.18 }
}

// Preserve NaNs like numpy.maximum / numpy.clip, unlike f64::max.
fn nonnegative(x: f64) -> f64 { if x < 0.0 { 0.0 } else { x } }
fn signed_pow(x: f64, exponent: f64) -> f64 { x.abs().powf(exponent).copysign(x) }
fn pow10(x: f64) -> f64 { 10.0_f64.powf(x) }
fn hlg_luminance(rgb: [f64; 3]) -> f64 {
    0.2627 * rgb[0] + 0.6780 * rgb[1] + 0.0593 * rgb[2]
}
fn hlg_inverse_oetf(x: f64) -> f64 {
    if x <= 0.5 { (x / 0.5).powi(2) / 12.0 }
    else { (((x - 0.559910729529562) / 0.17883277).exp() + 0.28466892) / 12.0 }
}
fn hlg_oetf(x: f64) -> f64 {
    let x = x * 12.0;
    if x <= 1.0 { 0.5 * signed_pow(x, 0.5) }
    else { 0.17883277 * (x - 0.28466892).ln() + 0.559910729529562 }
}

impl Curve {
    fn decode(self, x: f64) -> f64 {
        match self {
            Self::Linear => x,
            Self::Srgb => if x <= 12.92 * 0.0031308 { x / 12.92 } else { signed_pow((x + 0.055) / 1.055, 2.4) },
            Self::Bt1886 => nonnegative(x).powf(2.4),
            Self::Gamma(gamma) => x.powf(gamma),
            Self::Acescc => {
                if x >= (65504.0_f64.log2() + 9.72) / 17.52 { 65504.0 }
                else if x < (9.72 - 15.0) / 17.52 { ((x * 17.52 - 9.72).exp2() - 2.0_f64.powi(-16)) * 2.0 }
                else { (x * 17.52 - 9.72).exp2() }
            }
            Self::Acescct => if x > 0.155251141552511 { (x * 17.52 - 9.72).exp2() } else { (x - 0.0729055341958355) / 10.5402377416545 },
            Self::Logc3 => if x > 5.367655 * 0.010591 + 0.092809 { (pow10((x - 0.385537) / 0.24719) - 0.052272) / 5.555556 } else { (x - 0.092809) / 5.367655 },
            Self::Logc4 => if x >= 0.0 { ((14.0 * ((x - 0.09286412512218964) / 0.9071358748778103) + 6.0).exp2() - 64.0) / 2231.8263090676883 } else { x * 0.1135972086105891 - 0.01805699611991131 },
            Self::Slog3 => if x >= 171.2102946929 / 1023.0 { pow10((x * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01 } else { (x * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0) },
            Self::Vlog => if x < 0.181 { (x - 0.125) / 5.6 } else { pow10((x - 0.598206) / 0.241514) - 0.00873 },
            Self::Flog => if x < 0.100537775223865 { (x - 0.092864) / 8.735631 } else { pow10((x - 0.790453) / 0.344676) / 0.555556 - 0.009468 / 0.555556 },
            Self::Flog2 => if x < 0.100686685370811 { (x - 0.092864) / 8.799461 } else { pow10((x - 0.384316) / 0.245281) / 5.555556 - 0.064829 / 5.555556 },
            Self::CanonLog3 => canon_decode(x) * 0.9,
            Self::Log3g10 => if x < 0.0 { x / 15.1927 - 0.01 } else { (pow10(x / 0.224282) - 1.0) / 155.975327 - 0.01 },
            Self::DaVinci => if x <= 0.02740668 { x / 10.44426855 } else { (x / 0.07329248 - 7.0).exp2() - 0.0075 },
            Self::Apple => {
                if x >= 47.28711236 * (0.01_f64 + 0.05641088).powi(2) { ((x - 0.69336945) / 0.08550479).exp2() - 0.00964052 }
                else if x >= 0.0 { (x / 47.28711236).sqrt() - 0.05641088 }
                else if x < 0.0 { -0.05641088 } else { 0.0 }
            }
            Self::Blackmagic => if x < 8.283605932402494 * 0.005 + 0.09246575342465753 { (x - 0.09246575342465753) / 8.283605932402494 } else { ((x - 0.5300133392291939) / 0.08692876065491224).exp() - 0.005494072432257808 },
            // Python explicitly uses BT.2100 EOTF here, not its scene OETF/CCTF.
            Self::Pq => {
                let p = x.clamp(0.0, 1.0).powf(1.0 / (2523.0 / 32.0));
                10000.0 * (nonnegative(p - 3424.0 / 4096.0) / (2413.0 / 128.0 - (2392.0 / 128.0) * p)).powf(1.0 / (2610.0 / 16384.0))
            }
            Self::Hlg => unreachable!("HLG requires the RGB transform"),
            Self::Dlog => if x <= 0.14 { (x - 0.0929) / 6.025 } else { (pow10(3.89616 * x - 2.27752) - 0.0108) / 0.9892 },
            Self::ProPhoto => if x < 16.0 * 16.0_f64.powf(1.8 / (1.0 - 1.8)) { x / 16.0 } else { signed_pow(x, 1.8) },
            Self::Nlog => if x < 0.4418377321603128 { signed_pow(x / 0.635386119257087, 3.0) - 0.0075 } else { ((x - 0.6050830889540567) / 0.1466275659824047).exp() },
        }
    }

    fn encode(self, x: f64) -> f64 {
        match self {
            Self::Linear => x,
            Self::Srgb => if x <= 0.0031308 { x * 12.92 } else { 1.055 * signed_pow(x, 1.0 / 2.4) - 0.055 },
            Self::Bt1886 => signed_pow(x, 1.0 / 2.4),
            Self::Gamma(gamma) => x.powf(1.0 / gamma),
            Self::Acescc => if x < 0.0 { (-16.0 + 9.72) / 17.52 } else if x < 2.0_f64.powi(-15) { ((2.0_f64.powi(-16) + x * 0.5).log2() + 9.72) / 17.52 } else { (x.log2() + 9.72) / 17.52 },
            Self::Acescct => if x <= 0.0078125 { 10.5402377416545 * x + 0.0729055341958355 } else { (x.log2() + 9.72) / 17.52 },
            Self::Logc3 => if x > 0.010591 { 0.24719 * (5.555556 * x + 0.052272).log10() + 0.385537 } else { 5.367655 * x + 0.092809 },
            Self::Logc4 => if x >= -0.01805699611991131 { ((2231.8263090676883 * x + 64.0).log2() - 6.0) / 14.0 * 0.9071358748778103 + 0.09286412512218964 } else { (x + 0.01805699611991131) / 0.1135972086105891 },
            Self::Slog3 => if x >= 0.01125 { (420.0 + ((x + 0.01) / 0.19).log10() * 261.5) / 1023.0 } else { (x * (171.2102946929 - 95.0) / 0.01125 + 95.0) / 1023.0 },
            Self::Vlog => if x < 0.01 { 5.6 * x + 0.125 } else { 0.241514 * (x + 0.00873).log10() + 0.598206 },
            Self::Flog => if x < 0.00089 { 8.735631 * x + 0.092864 } else { 0.344676 * (0.555556 * x + 0.009468).log10() + 0.790453 },
            Self::Flog2 => if x < 0.000889 { 8.799461 * x + 0.092864 } else { 0.245281 * (5.555556 * x + 0.064829).log10() + 0.384316 },
            Self::CanonLog3 => {
                let x = x / 0.9;
                if x < canon_decode(0.097465473) { -0.36726845 * (-x * 14.98325 + 1.0).log10() + 0.12783901 }
                else if x <= canon_decode(0.15277891) { 1.9754798 * x + 0.12512219 }
                else if x > canon_decode(0.15277891) { 0.36726845 * (x * 14.98325 + 1.0).log10() + 0.12240537 } else { 0.0 }
            }
            Self::Log3g10 => { let x = x + 0.01; if x < 0.0 { x * 15.1927 } else { 0.224282 * (x * 155.975327 + 1.0).log10() } },
            Self::DaVinci => if x <= 0.00262409 { x * 10.44426855 } else { 0.07329248 * ((x + 0.0075).log2() + 7.0) },
            Self::Apple => if x >= 0.01 { 0.08550479 * (x + 0.00964052).log2() + 0.69336945 } else if x >= -0.05641088 { 47.28711236 * (x + 0.05641088).powi(2) } else { 0.0 },
            Self::Blackmagic => if x < 0.005 { 8.283605932402494 * x + 0.09246575342465753 } else { 0.08692876065491224 * (x + 0.005494072432257808).ln() + 0.5300133392291939 },
            Self::Pq => {
                let p = (nonnegative(x) / 10000.0).powf(2610.0 / 16384.0);
                ((3424.0 / 4096.0 + (2413.0 / 128.0) * p) / (1.0 + (2392.0 / 128.0) * p)).powf(2523.0 / 32.0)
            }
            Self::Hlg => unreachable!("HLG requires the RGB transform"),
            Self::Dlog => if x <= 0.0078 { 6.025 * x + 0.0929 } else { (x * 0.9892 + 0.0108).log10() * 0.256663 + 0.584555 },
            Self::ProPhoto => if x < 16.0_f64.powf(1.8 / (1.0 - 1.8)) { x * 16.0 } else { signed_pow(x, 1.0 / 1.8) },
            Self::Nlog => if x < 0.328 { 0.635386119257087 * signed_pow(x + 0.0075, 1.0 / 3.0) } else { 0.1466275659824047 * x.ln() + 0.6050830889540567 },
        }
    }
}

fn canon_decode(x: f64) -> f64 {
    if x < 0.097465473 { -(pow10((0.12783901 - x) / 0.36726845) - 1.0) / 14.98325 }
    else if x <= 0.15277891 { (x - 0.12512219) / 1.9754798 }
    else if x > 0.15277891 { (pow10((x - 0.12240537) / 0.36726845) - 1.0) / 14.98325 } else { 0.0 }
}

static REGISTRY: &[ColorSpaceEntry] = &[
    ColorSpaceEntry {
        name: "ACES2065-1",
        short_tag: "aces20651",
        primaries: "ACES2065-1",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "ACES - ACES2065-1",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "ACEScg",
        short_tag: "acescg",
        primaries: "ACEScg",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "ACES - ACEScg",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "Rec.709 Linear",
        short_tag: "rec709lin",
        primaries: "ITU-R BT.709",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "Rec.2020 Linear",
        short_tag: "rec2020lin",
        primaries: "ITU-R BT.2020",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "ProPhoto Linear",
        short_tag: "prophotolin",
        primaries: "ProPhoto RGB",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "sRGB Linear",
        short_tag: "srgblin",
        primaries: "sRGB",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "sRGB",
        short_tag: "srgb",
        primaries: "sRGB",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "sRGB - Display",
        cctf: Some("sRGB"),
        curve: Curve::Srgb,
    },
    ColorSpaceEntry {
        name: "Rec.709",
        short_tag: "rec709",
        primaries: "ITU-R BT.709",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "Rec.1886 Rec.709 - Display",
        cctf: Some("ITU-R BT.1886"),
        curve: Curve::Bt1886,
    },
    ColorSpaceEntry {
        name: "Display P3",
        short_tag: "displayp3",
        primaries: "Display P3",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "Display P3 - Display",
        cctf: Some("sRGB"),
        curve: Curve::Srgb,
    },
    ColorSpaceEntry {
        name: "Rec.2020",
        short_tag: "rec2020",
        primaries: "ITU-R BT.2020",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "Rec.1886 Rec.2020 - Display",
        cctf: Some("ITU-R BT.1886"),
        curve: Curve::Bt1886,
    },
    ColorSpaceEntry {
        name: "DCI-P3",
        short_tag: "dcip3",
        primaries: "DCI-P3",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "G2.6-P3-DCI - Display",
        cctf: Some("Gamma 2.6"),
        curve: Curve::Gamma(2.6),
    },
    ColorSpaceEntry {
        name: "Adobe RGB",
        short_tag: "adobergb",
        primaries: "Adobe RGB (1998)",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("Gamma 2.2"),
        curve: Curve::Gamma(2.2),
    },
    ColorSpaceEntry {
        name: "ACEScct",
        short_tag: "acescct",
        primaries: "ACEScct",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "ACEScct",
        cctf: Some("ACEScct"),
        curve: Curve::Acescct,
    },
    ColorSpaceEntry {
        name: "ARRI LogC3 (EI800)",
        short_tag: "logc3",
        primaries: "ARRI Wide Gamut 3",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "ARRI LogC3 (EI800) - AWG3",
        cctf: Some("ARRI LogC3"),
        curve: Curve::Logc3,
    },
    ColorSpaceEntry {
        name: "ARRI LogC4",
        short_tag: "logc4",
        primaries: "ARRI Wide Gamut 4",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "ARRI LogC4 - AWG4",
        cctf: Some("ARRI LogC4"),
        curve: Curve::Logc4,
    },
    ColorSpaceEntry {
        name: "Sony S-Log3",
        short_tag: "slog3",
        primaries: "S-Gamut3",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Sony S-Log3 - S-Gamut3",
        cctf: Some("S-Log3"),
        curve: Curve::Slog3,
    },
    ColorSpaceEntry {
        name: "Sony S-Log3 (S-Gamut3.Cine)",
        short_tag: "slog3cine",
        primaries: "S-Gamut3.Cine",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Sony S-Log3 - S-Gamut3.Cine",
        cctf: Some("S-Log3"),
        curve: Curve::Slog3,
    },
    ColorSpaceEntry {
        name: "Panasonic V-Log",
        short_tag: "vlog",
        primaries: "V-Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Panasonic V-Log - V-Gamut",
        cctf: Some("V-Log"),
        curve: Curve::Vlog,
    },
    ColorSpaceEntry {
        name: "Fujifilm F-Log",
        short_tag: "flog",
        primaries: "F-Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("F-Log"),
        curve: Curve::Flog,
    },
    ColorSpaceEntry {
        name: "Fujifilm F-Log2",
        short_tag: "flog2",
        primaries: "F-Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("F-Log2"),
        curve: Curve::Flog2,
    },
    ColorSpaceEntry {
        name: "Canon Log 3",
        short_tag: "canonlog3",
        primaries: "Cinema Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Canon Log 3 - Cinema Gamut",
        cctf: Some("Canon Log 3"),
        curve: Curve::CanonLog3,
    },
    ColorSpaceEntry {
        name: "RED Log3G10",
        short_tag: "redlog3g10",
        primaries: "REDWideGamutRGB",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "RED Log3G10 - REDWideGamutRGB",
        cctf: Some("Log3G10"),
        curve: Curve::Log3g10,
    },
    ColorSpaceEntry {
        name: "DaVinci Intermediate",
        short_tag: "davinciintermediate",
        primaries: "DaVinci Wide Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("DaVinci Intermediate"),
        curve: Curve::DaVinci,
    },
    ColorSpaceEntry {
        name: "Apple Log",
        short_tag: "applelog",
        primaries: "ITU-R BT.2020",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Apple Log",
        cctf: Some("Apple Log Profile"),
        curve: Curve::Apple,
    },
    ColorSpaceEntry {
        name: "Blackmagic Film Gen 5",
        short_tag: "bmfilmgen5",
        primaries: "Blackmagic Wide Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("Blackmagic Film Generation 5"),
        curve: Curve::Blackmagic,
    },
    ColorSpaceEntry {
        name: "Rec.2100 PQ",
        short_tag: "rec2100pq",
        primaries: "ITU-R BT.2020",
        input: true,
        output: true,
        midgray_linear: 100.0,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Rec.2100-PQ - Display",
        cctf: Some("ITU-R BT.2100 PQ"),
        curve: Curve::Pq,
    },
    ColorSpaceEntry {
        name: "Rec.2100 HLG",
        short_tag: "rec2100hlg",
        primaries: "ITU-R BT.2020",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "Rec.2100-HLG - Display",
        cctf: Some("ITU-R BT.2100 HLG"),
        curve: Curve::Hlg,
    },
    ColorSpaceEntry {
        name: "ACEScc",
        short_tag: "acescc",
        primaries: "ACEScg",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "ACEScc",
        cctf: Some("ACEScc"),
        curve: Curve::Acescc,
    },
    ColorSpaceEntry {
        name: "P3-D65 Linear",
        short_tag: "p3d65lin",
        primaries: "P3-D65",
        input: false,
        output: false,
        midgray_linear: 0.18,
        kind: "linear",
        scene_referred_input: true,
        ocio_alias: "",
        cctf: None,
        curve: Curve::Linear,
    },
    ColorSpaceEntry {
        name: "P3-D65 PQ",
        short_tag: "p3d65pq",
        primaries: "P3-D65",
        input: true,
        output: true,
        midgray_linear: 100.0,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "ST2084 P3-D65 - Display",
        cctf: Some("ITU-R BT.2100 PQ"),
        curve: Curve::Pq,
    },
    ColorSpaceEntry {
        name: "DJI D-Log",
        short_tag: "dlog",
        primaries: "DJI D-Gamut",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("D-Log"),
        curve: Curve::Dlog,
    },
    ColorSpaceEntry {
        name: "ProPhoto RGB",
        short_tag: "prophoto",
        primaries: "ProPhoto RGB",
        input: true,
        output: true,
        midgray_linear: 0.18,
        kind: "encoded_sdr",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("ProPhoto RGB"),
        curve: Curve::ProPhoto,
    },
    ColorSpaceEntry {
        name: "Nikon N-Log",
        short_tag: "nlog",
        primaries: "ITU-R BT.2020",
        input: true,
        output: false,
        midgray_linear: 0.18,
        kind: "log",
        scene_referred_input: false,
        ocio_alias: "",
        cctf: Some("N-Log"),
        curve: Curve::Nlog,
    },
];
