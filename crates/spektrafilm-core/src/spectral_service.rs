/// Descriptor-driven spectral upsampling and exact illuminant sampling.
use std::borrow::Cow;
use std::path::Path;
use std::sync::{LazyLock, Mutex, OnceLock};
use rayon::prelude::*;
use spektrafilm_math::npy;
use spektrafilm_math::spectral::{self, N_WAVELENGTHS, TcLut};

pub const fn default_spectral_shape() -> [f64; 3] {
    [380.0, 780.0, 5.0]
}

/// A regular wavelength grid. Samples include both endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralShape {
    pub bounds: [f64; 3],
    pub samples: usize,
}

impl SpectralShape {
    pub fn new(bounds: [f64; 3]) -> Result<Self, String> {
        let [start, end, step] = bounds;
        if !bounds.iter().all(|v| v.is_finite()) || start <= 0.0 || end <= start || step <= 0.0 {
            return Err("expected finite positive wavelengths, end > start and step > 0".into());
        }
        let intervals = (end - start) / step;
        if intervals > 100_000.0 || (intervals - intervals.round()).abs() > 1e-8 {
            return Err("grid must contain an integral number of intervals and at most 100001 samples".into());
        }
        Ok(Self { bounds, samples: intervals.round() as usize + 1 })
    }

    pub fn wavelength(self, index: usize) -> f64 {
        self.bounds[0] + self.bounds[2] * index as f64
    }
}

#[derive(serde::Deserialize)]
struct IlluminantCatalog {
    spectral_shape: [f64; 3],
    sources: std::collections::BTreeMap<String, Vec<f64>>,
}

static ILLUMINANT_CATALOG: LazyLock<IlluminantCatalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../data/illuminants/catalog.json"))
        .expect("bundled exact illuminant catalog")
});

fn illuminant_catalog() -> &'static IlluminantCatalog {
    &ILLUMINANT_CATALOG
}

/// Sample an exact catalog source, or evaluate a Planck blackbody, on a grid.
/// Catalog spectra cannot be extrapolated beyond their measured/baked bounds.
pub fn select_illuminant_on_grid_f64(name: &str, shape: SpectralShape) -> Result<Vec<f64>, String> {
    let mut values = if let Some(temperature) = parse_blackbody_temperature(name) {
        (0..shape.samples).map(|i| {
            let wavelength_m = shape.wavelength(i) * 1e-9;
            3.741_771_852e-16 / (wavelength_m.powi(5)
                * (1.438_776_877e-2 / (wavelength_m * temperature)).exp_m1())
        }).collect::<Vec<_>>()
    } else {
        let catalog = illuminant_catalog();
        let source = catalog.sources.get(name)
            .ok_or_else(|| format!("unsupported illuminant {name:?}"))?;
        let source_shape = SpectralShape::new(catalog.spectral_shape)?;
        if source.len() != source_shape.samples {
            return Err(format!("catalog source {name:?} does not match its spectral grid"));
        }
        if shape.bounds[0] < source_shape.bounds[0] || shape.bounds[1] > source_shape.bounds[1] {
            return Err(format!("illuminant {name:?} supports {:?}, requested {:?}", source_shape.bounds, shape.bounds));
        }
        (0..shape.samples).map(|i| {
            let x = (shape.wavelength(i) - source_shape.bounds[0]) / source_shape.bounds[2];
            let lo = (x.floor() as usize).min(source.len() - 1);
            let hi = (lo + 1).min(source.len() - 1);
            source[lo] + (source[hi] - source[lo]) * (x - lo as f64)
        }).collect::<Vec<_>>()
    };
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return Err(format!("illuminant {name:?} has invalid spectral normalization"));
    }
    for value in &mut values { *value /= mean; }
    Ok(values)
}
/// Camera taking filters shipped with the experimental runtime.
pub const NO_COLOR_FILTER: &str = "none";
const COLOR_FILTERS: &[&str] = &["none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1"];

pub fn available_color_filters() -> Vec<&'static str> {
    COLOR_FILTERS.to_vec()
}

pub fn is_supported_color_filter(name: &str) -> bool {
    COLOR_FILTERS.contains(&name)
}

/// Load a measured camera-filter curve and interpolate it onto the engine's
/// 380–780 nm, 5 nm spectral grid. Unknown names are rejected before any
/// filesystem access so configuration errors are deterministic.
pub fn load_color_filter_transmittance(
    data_dir: &Path,
    name: &str,
) -> Result<Option<Vec<f64>>, String> {
    if name == NO_COLOR_FILTER {
        return Ok(None);
    }
    if !is_supported_color_filter(name) {
        return Err(format!(
            "unsupported camera color filter {name:?}; supported: {}",
            COLOR_FILTERS.join(", ")
        ));
    }
    let stem = name.strip_prefix("hoya_").unwrap();
    let path = data_dir.join("filters/colored/hoya").join(format!("{stem}.csv"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading camera color filter {}: {e}", path.display()))?;
    let mut points = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        let mut fields = raw.split(',');
        let wavelength = fields.next().and_then(|v| v.trim().parse::<f64>().ok())
            .ok_or_else(|| format!("invalid wavelength at {}:{}", path.display(), line + 1))?;
        let transmission = fields.next().and_then(|v| v.trim().parse::<f64>().ok())
            .ok_or_else(|| format!("invalid transmittance at {}:{}", path.display(), line + 1))?;
        if !wavelength.is_finite() || !transmission.is_finite() {
            return Err(format!("non-finite camera filter sample at {}:{}", path.display(), line + 1));
        }
        points.push((wavelength, transmission.clamp(0.0, 1.0)));
    }
    if points.len() < 2 {
        return Err(format!("camera color filter {} has fewer than two samples", path.display()));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(N_WAVELENGTHS);
    for index in 0..N_WAVELENGTHS {
        let wavelength = 380.0 + 5.0 * index as f64;
        let value = if wavelength <= points[0].0 {
            points[0].1
        } else if wavelength >= points[points.len() - 1].0 {
            points[points.len() - 1].1
        } else {
            let upper = points.partition_point(|(x, _)| *x < wavelength);
            let (x0, y0) = points[upper - 1];
            let (x1, y1) = points[upper];
            y0 + (y1 - y0) * (wavelength - x0) / (x1 - x0)
        };
        out.push(value);
    }
    Ok(Some(out))
}
/// Descriptor for one shipped spectral upsampler.  The sidecar format is
/// deliberately small; keeping this representation local avoids making the
/// runtime depend on a TOML parser just to select a LUT.
#[derive(Debug, Clone, PartialEq)]
pub struct LutDescriptor {
    pub identifier: String,
    pub file: String,
    pub kind: String,
    pub title: String,
    pub lut_size: usize,
    pub bands: usize,
    /// Wavelength grid encoded by the asset, as [start_nm, end_nm, step_nm].
    pub spectral_shape: [f64; 3],
    pub scene_illuminant: Option<String>,
    pub midgray: Option<f64>,
}

/// Pinned Python `standard_illuminant("T")`, normalized to mean one.
pub const ILLUMINANT_T_F64: [f64; N_WAVELENGTHS] = [
    0.04429086048707871,
    0.051585750595832355,
    0.06069536577621067,
    0.08052635711823716,
    0.10060221565179168,
    0.11943272418540037,
    0.13728858562967097,
    0.15467112077302433,
    0.16866507309404713,
    0.1829082842522153,
    0.19976734369038093,
    0.2173077944063887,
    0.2326798663933696,
    0.2510444390730186,
    0.2697795471874511,
    0.2905205523565897,
    0.31179466156241625,
    0.3337220581980729,
    0.35451979503226494,
    0.37858377973780444,
    0.39958340688637045,
    0.422120260822385,
    0.44670745384066046,
    0.47049908200856416,
    0.49632114404384897,
    0.522977799611895,
    0.5479190721125559,
    0.5755373919150063,
    0.6023864883284424,
    0.6305289345911937,
    0.6582191651370278,
    0.686881056389086,
    0.7173724325758171,
    0.747604209548834,
    0.7778256884073104,
    0.809161816636445,
    0.8419707046347984,
    0.8743153620820648,
    0.9059410302689376,
    0.937950444880361,
    0.9697294044529698,
    1.0022160761344994,
    1.0343503119459732,
    1.0666486964784918,
    1.0992623234923373,
    1.1310901666113176,
    1.1632143989406825,
    1.1951538308384908,
    1.2256472448160873,
    1.2570867339479983,
    1.2868689599150938,
    1.3175466878101896,
    1.3486075872861383,
    1.3785499410728028,
    1.4081914316826425,
    1.4385009133027542,
    1.4684603699913443,
    1.4978645194556324,
    1.5304444840631708,
    1.5643993702428256,
    1.5988494848691726,
    1.6278929583694202,
    1.6581353118655981,
    1.6838037962950616,
    1.7081754309383532,
    1.732923209247628,
    1.7641983427548216,
    1.7869933057174028,
    1.8125966948064482,
    1.8416205709436502,
    1.8635213785546003,
    1.8791814272048726,
    1.9067867621494494,
    1.9368180338409047,
    1.9561762040234099,
    1.973705424857322,
    2.0029397142179697,
    1.9368180338409047,
    1.9561762040234099,
    1.973705424857322,
    2.0029397142179697,
];

pub const ILLUMINANT_T: [f32; N_WAVELENGTHS] = narrow_illuminant(&ILLUMINANT_T_F64);

/// Pinned Python `standard_illuminant("TH-KG3")`, normalized to mean one.
pub const ILLUMINANT_TH_KG3_F64: [f64; N_WAVELENGTHS] = [
    0.27115487737678473,
    0.2933630241983439,
    0.31540886133985474,
    0.3369808836611168,
    0.3586779925678246,
    0.38052016290102403,
    0.40301341542370506,
    0.4267870797493813,
    0.4516515921147848,
    0.4780646983713809,
    0.5066708689059379,
    0.5366576010691867,
    0.5680769778513239,
    0.5999220381342626,
    0.630515347854697,
    0.6612217385546929,
    0.6920909115305464,
    0.7236647580300142,
    0.7576009324911563,
    0.7930688015230838,
    0.829084542867803,
    0.8654853588024557,
    0.902212749209677,
    0.938990050475476,
    0.9747004155955517,
    1.0087417855358256,
    1.040916738775546,
    1.0715558331394295,
    1.1009111192408305,
    1.131926197237691,
    1.1638985698017286,
    1.19636460651255,
    1.2303390279092097,
    1.2682954247580434,
    1.3068524359316893,
    1.3428629981362672,
    1.375710370915656,
    1.4046281460277452,
    1.4308952617117419,
    1.4554028101360024,
    1.4771790396751103,
    1.4964115098687572,
    1.5136660039295053,
    1.5280688440333003,
    1.5381464692108247,
    1.5459434102593674,
    1.5515901051035574,
    1.5540261727761033,
    1.5535666436238242,
    1.5500616564793381,
    1.5408266875226315,
    1.5320737503736817,
    1.5194886088637505,
    1.5030832634966458,
    1.479985561016079,
    1.4599852627082766,
    1.4367535752409983,
    1.4032296202488896,
    1.3718429999284323,
    1.3404558220815574,
    1.3019482391882462,
    1.2649087261004042,
    1.2261391348332888,
    1.1802123657454517,
    1.136867763608742,
    1.0926534663168934,
    1.0452427815027683,
    1.0004794004746733,
    0.9480577282133905,
    0.9023807920757143,
    0.8546651971962678,
    0.7922263441280082,
    0.7345212240830614,
    0.6892285469718005,
    0.6522498893006442,
    0.607482301998138,
    0.5717547839358964,
    0.526940374973515,
    0.4866761911161827,
    0.449539809526992,
    0.41455292589926274,
];

pub const ILLUMINANT_TH_KG3: [f32; N_WAVELENGTHS] = narrow_illuminant(&ILLUMINANT_TH_KG3_F64);
/// Pinned Python `standard_illuminant("TH-KG3-L")`, normalized to mean one.
/// This is the 3400 K tungsten-halogen source with Schott KG3 and the
/// Canon 24-70 mm f/2.8 lens transmission curve.
pub const ILLUMINANT_TH_KG3_L_F64: [f64; N_WAVELENGTHS] = [
    0.09456970290598739, 0.1358223126707035, 0.18067928085342475,
    0.2259785007417468, 0.2677351393678483, 0.30492220983936275,
    0.3404115534894003, 0.3725452187996161, 0.40207481582863175,
    0.43144620857294336, 0.46257922241084165, 0.49512511929127734,
    0.5289219800537478, 0.5626679850892985, 0.5958260010917935,
    0.6299593886971075, 0.6644097398593504, 0.700554052742682,
    0.7408873075661121, 0.7818238319378953, 0.8222884814131736,
    0.8625802756777197, 0.9030662159717104, 0.9436349150547809,
    0.9830168583481824, 1.0205556844981798, 1.0564491305809889,
    1.0903341528774033, 1.1219168901687964, 1.1552754643901004,
    1.18957077142463, 1.224385079368606, 1.2608110664687997,
    1.3017320113172588, 1.3454684726941972, 1.3859275667105297,
    1.4213335202694428, 1.4514983451100734, 1.4793862774441284,
    1.506950815002019, 1.5335196656859136, 1.554810667188734,
    1.5759275295354451, 1.594335896239912, 1.6071424189513246,
    1.6178330856868541, 1.6264112869522358, 1.6318859951354185,
    1.6324648760560658, 1.6295535378804413, 1.6210239391887935,
    1.612877388213371, 1.5996095071678285, 1.5806584699460842,
    1.5560501956617274, 1.5336590858122428, 1.5058658931227298,
    1.4662994867298649, 1.428579312658189, 1.390812091273809,
    1.3452831165124508, 1.2992635156028105, 1.2522330269883137,
    1.2009204344497504, 1.148659982040946, 1.0954695988974383,
    1.03926723560586, 0.9844711396581393, 0.9236646594179271,
    0.8705698517734068, 0.8139325853073219, 0.7456552858076373,
    0.6831551422932013, 0.630258990076302, 0.5900955284329665,
    0.5422665143829449, 0.5029095765733292, 0.4567257035827689,
    0.4130914600721322, 0.37567089330813325, 0.3419938635287332,
];

pub const ILLUMINANT_TH_KG3_L: [f32; N_WAVELENGTHS] =
    narrow_illuminant(&ILLUMINANT_TH_KG3_L_F64);


/// Pinned Python `standard_illuminant("K75P")`, normalized to mean one.
pub const ILLUMINANT_K75P_F64: [f64; N_WAVELENGTHS] = [
    0.2003553567372573,
    0.3026585381824508,
    0.41255329657700374,
    0.5589481864082568,
    0.6999985273583207,
    0.8502194502648629,
    0.996796445387135,
    1.1030169402986139,
    1.1483838412127785,
    1.1664516194437204,
    1.217982861892341,
    1.3062873481650925,
    1.4153622228081846,
    1.4761248097090862,
    1.5818290511517283,
    1.6374597123576529,
    1.7417974808811936,
    1.9191353300817453,
    1.9968724257801131,
    1.8180163340682451,
    1.7032933713595342,
    1.7146197920913813,
    1.6657609151815502,
    1.6792130211863516,
    1.5991377081270122,
    1.568923443216373,
    1.550551943604612,
    1.5371605089974187,
    1.531421322737193,
    1.520914694658612,
    1.5209147857568066,
    1.510681361163851,
    1.5102865415884255,
    1.5016929755965862,
    1.4861759468146336,
    1.4739991244361321,
    1.4651626906574713,
    1.4620045895450389,
    1.478948762643701,
    1.4839592544452673,
    1.4791309590329145,
    1.4717217606691577,
    1.4357682184000984,
    1.3853605810837575,
    1.3387790679409006,
    1.322563589300898,
    1.322927982079325,
    1.3416637833713183,
    1.3486783443560384,
    1.326784441951613,
    1.2759819850598466,
    1.2031641918702434,
    1.1247589820126298,
    1.0824591751144852,
    1.0677315121888,
    1.0412827000540363,
    1.049633398259054,
    1.0495423000644473,
    0.9857431370427188,
    0.8214324207691277,
    0.6165525810985313,
    0.4288598734116133,
    0.26509574630566285,
    0.15523132360991393,
    0.08982281988226239,
    0.062493361500235295,
    0.04571307405367065,
    0.03847076758243348,
    0.02714726199281358,
    0.025762569434790876,
    0.0235033342085433,
    0.021385301183936205,
    0.020952584759554107,
    0.02241015587326222,
    0.021790688149936274,
    0.018028332712677207,
    0.01590574477833977,
    0.02681019867276858,
    0.0273294583820271,
    0.03086406833276927,
    0.02568969087910547,
];

pub const ILLUMINANT_K75P: [f32; N_WAVELENGTHS] = narrow_illuminant(&ILLUMINANT_K75P_F64);

const fn narrow_illuminant(values: &[f64; N_WAVELENGTHS]) -> [f32; N_WAVELENGTHS] {
    let mut result = [0.0; N_WAVELENGTHS];
    let mut i = 0;
    while i < N_WAVELENGTHS {
        result[i] = values[i] as f32;
        i += 1;
    }
    result
}

/// Exact upstream catalog names plus bundled printer illuminants and the
/// analytic blackbody token.
pub fn available_illuminants() -> Vec<&'static str> {
    let mut names: Vec<_> = illuminant_catalog().sources.keys().map(String::as_str).collect();
    for name in ["T", "TH-KG3", "TH-KG3-L", "K75P"] {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.push("BB<temperature>");
    names
}

pub fn is_supported_illuminant(name: &str) -> bool {
    matches!(name, "T" | "TH-KG3" | "TH-KG3-L" | "K75P")
        || illuminant_catalog().sources.contains_key(name)
        || parse_blackbody_temperature(name).is_some()
}


fn parse_blackbody_temperature(name: &str) -> Option<f64> {
    let temperature = name.strip_prefix("BB")?.parse::<f64>().ok()?;
    (temperature.is_finite() && (1667.0..=25000.0).contains(&temperature))
        .then_some(temperature)
}

fn generated_illuminant(name: &str) -> Option<(Cow<'static, [f32]>, Cow<'static, [f64]>)> {
    let values = select_illuminant_on_grid_f64(name, SpectralShape::new(default_spectral_shape()).ok()?).ok()?;
    let f32_values = values.iter().map(|value| *value as f32).collect();
    Some((Cow::Owned(f32_values), Cow::Owned(values)))
}

/// Select the reference or viewing illuminant used by bundled profiles.
pub fn select_illuminant(name: &str) -> Cow<'static, [f32]> {
    match name {
        "D50" => Cow::Borrowed(&spectral::ILLUMINANT_D50),
        "D55" => Cow::Borrowed(&spectral::ILLUMINANT_D55),
        "D65" => Cow::Borrowed(&spectral::ILLUMINANT_D65),
        "T" => Cow::Borrowed(&ILLUMINANT_T),
        "TH-KG3" => Cow::Borrowed(&ILLUMINANT_TH_KG3),
        "TH-KG3-L" => Cow::Borrowed(&ILLUMINANT_TH_KG3_L),
        "K75P" => Cow::Borrowed(&ILLUMINANT_K75P),
        _ => generated_illuminant(name)
            .map(|(f32_values, _)| f32_values)
            .unwrap_or_else(|| panic!("unsupported profile illuminant {name:?}")),
    }
}

pub fn select_illuminant_f64(name: &str) -> Cow<'static, [f64]> {
    match name {
        "D50" => Cow::Borrowed(&spectral::ILLUMINANT_D50_F64),
        "D55" => Cow::Borrowed(&spectral::ILLUMINANT_D55_F64),
        "D65" => Cow::Borrowed(&spectral::ILLUMINANT_D65_F64),
        "T" => Cow::Borrowed(&ILLUMINANT_T_F64),
        "TH-KG3" => Cow::Borrowed(&ILLUMINANT_TH_KG3_F64),
        "TH-KG3-L" => Cow::Borrowed(&ILLUMINANT_TH_KG3_L_F64),
        "K75P" => Cow::Borrowed(&ILLUMINANT_K75P_F64),
        _ => generated_illuminant(name)
            .map(|(_, f64_values)| f64_values)
            .unwrap_or_else(|| panic!("unsupported profile illuminant {name:?}")),
    }
}

// Pull in BLAS for the spectra→tc_lut contraction (matches Python's
// opt_einsum + numpy summation pattern bit-for-bit on macOS Accelerate).
#[cfg(not(target_os = "windows"))]
#[allow(unused_imports)]
use blas_src as _;

fn dgemm_no_transpose(a: &[f64], b: &[f64], c: &mut [f64], m: usize, n: usize, k: usize) {
    assert_eq!(a.len(), m * k, "dgemm_no_transpose: a.len() != m*k");
    assert_eq!(b.len(), k * n, "dgemm_no_transpose: b.len() != k*n");
    assert_eq!(c.len(), m * n, "dgemm_no_transpose: c.len() != m*n");

    #[cfg(not(target_os = "windows"))]
    unsafe {
        cblas::dgemm(
            cblas::Layout::RowMajor,
            cblas::Transpose::None,
            cblas::Transpose::None,
            m as i32,
            n as i32,
            k as i32,
            1.0,
            a,
            k as i32,
            b,
            n as i32,
            0.0,
            c,
            n as i32,
        );
    }

    #[cfg(target_os = "windows")]
    {
        for row in 0..m {
            for col in 0..n {
                let mut sum = 0.0f64;
                for kk in 0..k {
                    sum += a[row * k + kk] * b[kk * n + col];
                }
                c[row * n + col] = sum;
            }
        }
    }
}

/// Load the spectra LUT from the .npy file.
/// Shape: (size, size, 81) — maps tc coordinates → 81-wavelength spectra.
pub fn load_spectra_lut(data_dir: &Path) -> Result<SpectraLut, String> {
    let path = data_dir
        .join("luts")
        .join("spectral_upsampling")
        .join("irradiance_xy_tc.npy");

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("opening spectra LUT {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let (shape, data) =
        npy::load_npy_f32(reader).map_err(|e| format!("loading spectra LUT: {e}"))?;

    if shape.len() != 3 || shape[2] != N_WAVELENGTHS {
        return Err(format!(
            "spectra LUT shape mismatch: expected (N, N, {N_WAVELENGTHS}), got {shape:?}"
        ));
    }
    if shape[0] != shape[1] {
        return Err(format!(
            "spectra LUT must be square, got {}x{}",
            shape[0], shape[1]
        ));
    }

    Ok(SpectraLut {
        size: shape[0],
        n_wavelengths: shape[2],
        data,
    })
}

/// Load the arctic2026alpha02 effective-reflectance LUT.
///
/// Shape: (size, size, 81). The table is a D65-recovered unit-bright
/// reflectance surface in triangular coordinates. At render time we relight it
/// by the film reference illuminant before integrating against film
/// sensitivity.
pub fn load_arctic2026alpha02_lut(data_dir: &Path) -> Result<SpectraLut, String> {
    let path = data_dir
        .join("luts")
        .join("spectral_upsampling")
        .join("arctic2026alpha02")
        .join("reflectance_xy_tc.npy");

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("opening arctic2026alpha02 LUT {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let (shape, data) =
        npy::load_npy_f32(reader).map_err(|e| format!("loading arctic2026alpha02 LUT: {e}"))?;

    if shape.len() != 3 || shape[2] != N_WAVELENGTHS {
        return Err(format!(
            "arctic2026alpha02 LUT shape mismatch: expected (N, N, {N_WAVELENGTHS}), got {shape:?}"
        ));
    }
    if shape[0] != shape[1] {
        return Err(format!(
            "arctic2026alpha02 LUT must be square, got {}x{}",
            shape[0], shape[1]
        ));
    }

    Ok(SpectraLut {
        size: shape[0],
        n_wavelengths: shape[2],
        data,
    })
}

/// Parse the shipped sidecar descriptor into the fields consumed at runtime.
pub fn parse_lut_descriptor(text: &str) -> Result<LutDescriptor, String> {
    let mut values = std::collections::HashMap::<String, String>::new();
    let mut section = "";
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let name = match (section, key.trim()) {
            ("array", "lut_size") => "array.lut_size",
            ("array", "bands") => "array.bands",
            ("array", "spectral_shape") => "array.spectral_shape",
            ("reflectance", "scene_illuminant") => "reflectance.scene_illuminant",
            ("reflectance", "midgray") => "reflectance.midgray",
            _ => key.trim(),
        };
        values.insert(name.to_string(), value.trim().trim_matches('"').to_string());
    }
    let required = |key: &str| values.get(key).cloned().ok_or_else(|| format!("descriptor missing {key}"));
    let shape_text = required("array.spectral_shape")?;
    let shape_text = shape_text.trim().trim_start_matches('[').trim_end_matches(']');
    let shape_values = shape_text
        .split(',')
        .map(|value| value.trim().parse::<f64>().map_err(|_| "invalid spectral_shape".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let spectral_shape = <[f64; 3]>::try_from(shape_values)
        .map_err(|_| "spectral_shape must contain [start, end, step]".to_string())?;
    if !spectral_shape.iter().all(|value| value.is_finite()) || spectral_shape[2] <= 0.0 || spectral_shape[1] < spectral_shape[0] {
        return Err("spectral_shape must contain finite ascending bounds and positive step".into());
    }
    Ok(LutDescriptor {
        identifier: required("identifier")?,
        file: required("file")?,
        kind: required("kind")?,
        title: required("title")?,
        lut_size: required("array.lut_size")?.parse().map_err(|_| "invalid lut_size".to_string())?,
        bands: required("array.bands")?.parse().map_err(|_| "invalid bands".to_string())?,
        spectral_shape,
        scene_illuminant: values.get("reflectance.scene_illuminant").cloned(),
        midgray: values.get("reflectance.midgray").and_then(|v| v.parse().ok()),
    })
}

pub fn available_lut_identifiers(data_dir: &Path) -> Result<Vec<String>, String> {
    let root = data_dir.join("luts").join("spectral_upsampling");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| format!("reading {}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) == Some("toml") {
            ids.push(parse_lut_descriptor(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?.identifier);
        }
    }
    ids.sort();
    Ok(ids)
}

/// Stable public name for callers that do not care about LUT internals.
pub fn available_spectral_methods(data_dir: &Path) -> Result<Vec<String>, String> {
    available_lut_identifiers(data_dir)
}

pub fn lut_descriptor(data_dir: &Path, identifier: &str) -> Result<LutDescriptor, String> {
    let root = data_dir.join("luts").join("spectral_upsampling");
    for entry in std::fs::read_dir(&root).map_err(|e| format!("reading {}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("toml") { continue; }
        let desc = parse_lut_descriptor(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?;
        if desc.identifier == identifier { return Ok(desc); }
    }
    Err(format!("unknown spectral LUT method {identifier:?}; available: {:?}", available_lut_identifiers(data_dir).unwrap_or_default()))
}

static SPECTRA_CACHE: OnceLock<Mutex<std::collections::HashMap<String, SpectraLut>>> = OnceLock::new();

fn spectral_cache_key(identifier: &str, shape: [f64; 3]) -> String {
    format!("{identifier}|{:.9}:{:.9}:{:.9}", shape[0], shape[1], shape[2])
}

/// Load a registered LUT and validate it against the requested working grid.
/// Resampling is intentionally not implicit: the spectral integration arrays
/// and profile sensitivities must be sampled on the same grid.
pub fn load_lut_on_grid(
    data_dir: &Path,
    identifier: &str,
    requested: SpectralShape,
) -> Result<SpectraLut, String> {
    let desc = lut_descriptor(data_dir, identifier)?;
    let cache = SPECTRA_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = spectral_cache_key(identifier, requested.bounds);
    if let Some(lut) = cache.lock().map_err(|_| "spectral cache poisoned".to_string())?.get(&key).cloned() {
        return Ok(lut);
    }
    let path = data_dir.join("luts").join("spectral_upsampling").join(&desc.file);
    let (shape, data) = npy::load_npy_f32(std::io::BufReader::new(std::fs::File::open(&path)
        .map_err(|e| format!("opening spectral LUT {}: {e}", path.display()))?))
        .map_err(|e| format!("loading spectral LUT {}: {e}", path.display()))?;
    let expected_bands = ((desc.spectral_shape[1] - desc.spectral_shape[0]) / desc.spectral_shape[2]).round() as usize + 1;
    if shape != vec![desc.lut_size, desc.lut_size, desc.bands] || desc.bands != expected_bands {
        return Err(format!("{identifier} descriptor/data shape mismatch: descriptor {:?}, bands {}, data shape {:?}", desc.spectral_shape, desc.bands, shape));
    }
    if desc.spectral_shape != requested.bounds || desc.bands != requested.samples {
        return Err(format!(
            "{identifier} spectral grid mismatch: asset {:?} ({} bands), requested {:?} ({} samples); \
             resampling LUTs is unsupported because profile sensitivities and CMFs must share this grid",
            desc.spectral_shape, desc.bands, requested.bounds, requested.samples
        ));
    }
    let lut = SpectraLut { size: shape[0], n_wavelengths: shape[2], data };
    cache.lock().map_err(|_| "spectral cache poisoned".to_string())?.insert(key, lut.clone());
    Ok(lut)
}

pub fn load_lut(data_dir: &Path, identifier: &str) -> Result<SpectraLut, String> {
    load_lut_on_grid(data_dir, identifier, SpectralShape::new(default_spectral_shape()).expect("fixed spectral grid"))
}

#[derive(Clone)]
pub struct SpectraLut {
    pub size: usize,
    pub n_wavelengths: usize,
    /// Flat: [size * size * n_wavelengths]
    pub data: Vec<f32>,
}

impl SpectraLut {
    /// Get spectrum at grid position (i, j). Returns slice of n_wavelengths.
    pub fn spectrum(&self, i: usize, j: usize) -> &[f32] {
        let start = (i * self.size + j) * self.n_wavelengths;
        &self.data[start..start + self.n_wavelengths]
    }

    /// Convert the stored f32 LUT to the f64 working cube.
    ///
    /// f32→f64 is exact, so this matches Python's `np.double` promotion of
    /// the float16 table bit-for-bit.
    pub fn to_f64_cube(&self) -> SpectraCube {
        SpectraCube {
            size: self.size,
            n_wavelengths: self.n_wavelengths,
            data: self.data.iter().map(|&v| v as f64).collect(),
        }
    }
}

/// f64 working copy of the spectra LUT — mirrors Python's
/// `np.double(np.load('irradiance_xy_tc.npy'))`, which promotes the stored
/// float16 table to float64 once at load and keeps every downstream step
/// (spectral Gaussian blur, sensitivity contraction) in f64.
#[derive(Clone)]
pub struct SpectraCube {
    pub size: usize,
    pub n_wavelengths: usize,
    /// Flat: [size * size * n_wavelengths]
    pub data: Vec<f64>,
}

impl SpectraCube {
    /// Get the spectrum at grid position (i, j). Returns n_wavelengths samples.
    pub fn spectrum(&self, i: usize, j: usize) -> &[f64] {
        let start = (i * self.size + j) * self.n_wavelengths;
        &self.data[start..start + self.n_wavelengths]
    }
}

/// Spectral Gaussian blur of the spectra cube along the wavelength axis.
///
/// Port of the `spectral_gaussian_blur` step of Python
/// `compute_hanatos2025_tc_lut`:
/// `scipy.ndimage.gaussian_filter(spectra_lut, (0, 0, sigma))`.
///
/// scipy semantics replicated exactly:
/// - 1-D correlation along the wavelength (last) axis only — the tc-grid
///   axes get sigma 0 and are untouched.
/// - `truncate = 4.0` → kernel radius `lw = int(4.0 * sigma + 0.5)`.
/// - The kernel is normalized over its full truncated support
///   (`phi_x / phi_x.sum()`), not renormalized per sample window.
/// - `mode='reflect'` boundary: the edge sample repeats,
///   `(d c b a | a b c d | d c b a)`, i.e. index `-1 → 0`, `n → n-1`.
pub fn gaussian_blur_wavelength(cube: &SpectraCube, sigma: f64) -> SpectraCube {
    let lw = (4.0 * sigma + 0.5) as i64;
    let n = cube.n_wavelengths;
    if sigma <= 0.0 || lw == 0 || n == 0 {
        return cube.clone();
    }

    // `scipy.ndimage._gaussian_kernel1d` (order 0):
    //   phi_x = exp(-0.5 / sigma^2 * x^2); phi_x /= phi_x.sum()
    let sigma2 = sigma * sigma;
    let mut kernel = vec![0.0f64; (2 * lw + 1) as usize];
    for (k, w) in kernel.iter_mut().enumerate() {
        let x = k as f64 - lw as f64;
        *w = (-0.5 / sigma2 * x * x).exp();
    }
    let kernel_sum = pairwise_sum_f64(&kernel);
    for w in kernel.iter_mut() {
        *w /= kernel_sum;
    }

    let n_i64 = n as i64;
    let mut data = vec![0.0f64; cube.data.len()];
    let src = cube.data.as_slice();
    // Each tc cell's spectrum is an independent 81-sample row; cells are
    // independent, so blur them in parallel.
    data.par_chunks_exact_mut(n)
        .enumerate()
        .for_each(|(cell, out)| {
            let row = &src[cell * n..cell * n + n];
            for i in 0..n {
                let mut acc = 0.0f64;
                for k in -lw..=lw {
                    // scipy's symmetric ('reflect') extension — it keeps
                    // reflecting for radii larger than the line: index -1 → 0,
                    // n → n-1, ..., period 2n.
                    let j = (i as i64 + k).rem_euclid(2 * n_i64);
                    let j = if j >= n_i64 { 2 * n_i64 - 1 - j } else { j };
                    acc += kernel[(k + lw) as usize] * row[j as usize];
                }
                out[i] = acc;
            }
        });

    SpectraCube {
        size: cube.size,
        n_wavelengths: cube.n_wavelengths,
        data,
    }
}

/// Compute the TC LUT for a given film stock.
///
/// Port of Python `compute_hanatos2025_tc_lut` (no-adaptation path):
///   tc_lut[i][j][c] = sum_wl( spectra_lut[i][j][wl] * sensitivity[wl][c] )
///
/// The result maps tc coordinates → per-channel film raw exposure,
/// normalized so that the reference illuminant midgray produces balanced
/// exposure on the green channel (matching Python's `raw / raw_midgray[1]`).
pub fn compute_tc_lut(spectra_cube: &SpectraCube, sensitivity: &[[f64; 3]]) -> TcLut {
    let size = spectra_cube.size;
    let n_wl = spectra_cube.n_wavelengths.min(sensitivity.len());
    let channels = 3;

    let mut data = vec![0.0f64; size * size * channels];

    for i in 0..size {
        for j in 0..size {
            let spectrum = spectra_cube.spectrum(i, j);
            let mut raw = [0.0f64; 3];
            for wl in 0..n_wl {
                for c in 0..3 {
                    raw[c] += spectrum[wl] * sensitivity[wl][c];
                }
            }
            let base = (i * size + j) * channels;
            data[base] = raw[0];
            data[base + 1] = raw[1];
            data[base + 2] = raw[2];
        }
    }

    TcLut {
        size,
        channels,
        data,
    }
}

/// Build a TC→film-raw LUT from an effective-reflectance surface by relighting
/// each reflectance with the film/reference illuminant first:
///
/// `raw[i,j,c] = sum_wl reflectance[i,j,wl] * illuminant[wl] * sensitivity[wl,c]`
pub fn compute_reflectance_tc_lut(
    reflectance_lut: &SpectraLut,
    sensitivity: &[[f64; 3]],
    illuminant: &[f64],
) -> TcLut {
    let size = reflectance_lut.size;
    let n_wl = reflectance_lut
        .n_wavelengths
        .min(sensitivity.len())
        .min(illuminant.len());
    let channels = 3;
    let mut data = vec![0.0f64; size * size * channels];

    for i in 0..size {
        for j in 0..size {
            let spectrum = reflectance_lut.spectrum(i, j);
            let mut raw = [0.0f64; 3];
            for wl in 0..n_wl {
                let lit = spectrum[wl] as f64 * illuminant[wl];
                for c in 0..3 {
                    raw[c] += lit * sensitivity[wl][c];
                }
            }
            let base = (i * size + j) * channels;
            data[base] = raw[0];
            data[base + 1] = raw[1];
            data[base + 2] = raw[2];
        }
    }

    TcLut {
        size,
        channels,
        data,
    }
}
/// Descriptor-driven dispatcher used by the pipeline. Reflectance LUTs are
/// relit under the film reference illuminant and normalized at the descriptor's
/// scene white on every channel; irradiance LUTs use the Hanatos path.
pub fn compute_registered_tc_lut(
    data_dir: &Path,
    identifier: &str,
    sensitivity: &[[f64; 3]],
    reference_illuminant: &[f64],
    adaptation: Option<&Hanatos2025Adaptation<'_>>,
) -> Result<TcLut, String> {
    let descriptor = lut_descriptor(data_dir, identifier)?;
    let spectra = load_lut(data_dir, identifier)?;
    match descriptor.kind.as_str() {
        "reflectance" => {
            let mut lut = compute_reflectance_tc_lut(&spectra, sensitivity, reference_illuminant);
            let scene = descriptor.scene_illuminant.as_deref().ok_or_else(|| "reflectance descriptor missing scene_illuminant".to_string())?;
            let scene_xy = spectral::illuminant_to_xy(&select_illuminant(scene));
            let (tx, ty) = spectral::xy_to_tc(scene_xy.0, scene_xy.1);
            let neutral = spektrafilm_math::lut::bicubic_2d(
                &spectra.data,
                spectra.size,
                spectra.size,
                spectra.n_wavelengths,
                tx as f32 * (spectra.size - 1) as f32,
                ty as f32 * (spectra.size - 1) as f32,
            );
            let n_wl = neutral.len().min(sensitivity.len()).min(reference_illuminant.len());
            let mut response = [0.0f64; 3];
            for wl in 0..n_wl {
                for c in 0..3 {
                    response[c] += neutral[wl] as f64
                        * reference_illuminant[wl]
                        * sensitivity[wl][c];
                }
            }
            for c in 0..lut.channels {
                let n = response[c];
                if n.abs() > 1e-15 {
                    for cell in lut.data.chunks_exact_mut(lut.channels) { cell[c] /= n; }
                }
            }
            Ok(lut)
        }
        "irradiance" => {
            let adaptation = adaptation.ok_or_else(|| "hanatos2025 requires sensitivity adaptation".to_string())?;
            compute_hanatos2025_tc_lut(&spectra, sensitivity, adaptation)
        }
        other => Err(format!("unsupported spectral LUT kind {other:?}")),
    }
}


/// Contract the spectra cube against the sensitivity through the erf4 spectral
/// bandpass window, normalizing against the reference illuminant.
fn contract_with_window(
    spectra_cube: &SpectraCube,
    sensitivity: &[[f64; 3]],
    window_params: &[f64],
    illuminant: &[f64],
) -> TcLut {
    let n_wl = spectra_cube
        .n_wavelengths
        .min(sensitivity.len())
        .min(N_WAVELENGTHS);
    let window = eval_erf4_bandpass(window_params);
    let mut num_per_wl = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    let mut den_per_wl = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    for wl in 0..n_wl {
        for c in 0..3 {
            let si = sensitivity[wl][c] * illuminant[wl];
            num_per_wl[c].push(si * window[wl][c]);
            den_per_wl[c].push(si);
        }
    }
    let mut window_normalized = window.clone();
    for c in 0..3 {
        let num_c = pairwise_sum_f64(&num_per_wl[c]);
        let den_c = pairwise_sum_f64(&den_per_wl[c]);
        if num_c > 1e-10 && den_c > 1e-10 {
            let normalization_c = num_c / den_c;
            for wl in 0..n_wl {
                window_normalized[wl][c] = window[wl][c] / normalization_c;
            }
        }
    }

    // Compute raw LUT via BLAS dgemm to match Python's
    // `opt_einsum.contract('ijl,lm->ijm', spectra, sens*window)`.
    // numpy/opt_einsum routes this through GEMM with pairwise
    // accumulation, so a hand-rolled left-to-right loop is off by
    // 1-2 ULP per cell. Going through dgemm matches bit-for-bit.
    let size = spectra_cube.size;
    let channels = 3;
    let n_pix = size * size;
    let mut spec_f64 = vec![0.0f64; n_pix * n_wl];
    for i in 0..n_pix {
        let src = &spectra_cube.data
            [i * spectra_cube.n_wavelengths..i * spectra_cube.n_wavelengths + n_wl];
        let dst = &mut spec_f64[i * n_wl..(i + 1) * n_wl];
        dst.copy_from_slice(src);
    }
    // Build sens*window as (n_wl × 3) row-major.
    let mut sw_flat = vec![0.0f64; n_wl * 3];
    for wl in 0..n_wl {
        for c in 0..3 {
            sw_flat[wl * 3 + c] = sensitivity[wl][c] * window_normalized[wl][c];
        }
    }
    let mut data = vec![0.0f64; n_pix * channels];
    dgemm_no_transpose(&spec_f64, &sw_flat, &mut data, n_pix, channels, n_wl);

    TcLut {
        size,
        channels,
        data,
    }
}

/// Hanatos2025 sensitivity-adaptation controls that shape the filming TC LUT.
///
/// Mirrors the LUT-relevant fields of Python's
/// `Hanatos2025SensitivityAdaptation` (profiles/io.py): the erf4 bandpass
/// window parameters, the poly4 log-exposure-correction surface parameters,
/// the spectral Gaussian blur sigma and the reference illuminant (SPD for
/// normalization, xy for the surface center).
pub struct Hanatos2025Adaptation<'a> {
    /// (c_uv, sigma_uv, c_ir, sigma_ir) — erf4 bandpass parameters.
    pub window_params: &'a [f64],
    /// 3 channels × 15 polynomial coefficients (c0 unused) — poly4 surface.
    pub surface_params: &'a [Vec<f64>],
    /// Gaussian blur sigma in nm applied to the spectra LUT (0 = off).
    pub spectral_gaussian_blur: f64,
    /// Reference illuminant SPD (f64) for the window normalization.
    pub reference_illuminant: &'a [f64],
    /// Reference illuminant xy chromaticity (surface evaluation center).
    pub reference_illuminant_xy: (f64, f64),
    pub apply_window: bool,
    pub apply_surface: bool,
}

/// Compute the filming TC LUT with Hanatos2025 sensitivity adaptation.
///
/// Port of Python `compute_hanatos2025_tc_lut(sensitivity,
/// hanatos2025_adaptation)` (utils/spectral_upsampling.py), minus the
/// input-gamut bake which the pipeline applies afterwards as before:
///
/// 1. `spectral_gaussian_blur > 0` → Gaussian-blur the spectra cube along
///    the wavelength axis (`scipy.ndimage.gaussian_filter` semantics).
/// 2. `apply_window` → contract through the erf4 bandpass window,
///    normalized against the reference illuminant so white balance is
///    preserved; otherwise contract the sensitivity directly.
/// 3. `apply_surface` → multiply by `2 ** surface`, the poly4
///    log-exposure-correction surface centered on the illuminant.
///
/// Both window and surface preserve white balance by construction.
pub fn compute_hanatos2025_tc_lut(
    spectra_lut: &SpectraLut,
    sensitivity: &[[f64; 3]],
    adaptation: &Hanatos2025Adaptation,
) -> Result<TcLut, String> {
    let apply_window = adaptation.apply_window;
    if apply_window && adaptation.window_params.len() != 4 {
        return Err(format!(
            "hanatos2025 adaptation window requires exactly 4 erf4 parameters \
             (c_uv, sigma_uv, c_ir, sigma_ir), got {}",
            adaptation.window_params.len()
        ));
    }
    if adaptation.apply_surface {
        let rows = adaptation.surface_params.len();
        if rows != 3 || adaptation.surface_params.iter().any(|r| r.len() != 15) {
            return Err(format!(
                "hanatos2025 adaptation surface requires 3 channels × 15 poly4 \
                 coefficients, got {rows} rows of lengths {:?}",
                adaptation
                    .surface_params
                    .iter()
                    .map(|r| r.len())
                    .collect::<Vec<_>>()
            ));
        }
    }

    let mut cube = spectra_lut.to_f64_cube();
    if adaptation.spectral_gaussian_blur > 0.0 {
        cube = gaussian_blur_wavelength(&cube, adaptation.spectral_gaussian_blur);
    }

    let mut tc_lut = if apply_window {
        contract_with_window(
            &cube,
            sensitivity,
            adaptation.window_params,
            adaptation.reference_illuminant,
        )
    } else {
        compute_tc_lut(&cube, sensitivity)
    };

    if adaptation.apply_surface {
        apply_log_exposure_surface(
            &mut tc_lut,
            adaptation.surface_params,
            adaptation.reference_illuminant_xy,
        );
    }

    Ok(tc_lut)
}

/// Evaluate the poly4 log-exposure-correction surface on the tc grid.
///
/// Port of Python `eval_poly4_log_exposure_surface` (the 0.3.4 default
/// surface model): a degree-4 polynomial per channel evaluated on the
/// `linspace(0, 1, size)` tc grid, centered (in tc coordinates) on the
/// reference illuminant chromaticity, passed through the bounded
/// Jakob & Hanika 2019 algebraic sigmoid (±`_HANATOS2025_MAX_CORRECTION_STOPS`
/// = 2 stops).
///
/// Returns a flat `[size * size * 3]` grid aligned with the TC LUT layout.
pub fn eval_poly4_log_exposure_surface(
    surface_params: &[Vec<f64>],
    illuminant_xy: (f64, f64),
    surface_size: usize,
) -> Vec<f64> {
    let (cx, cy) = spektrafilm_math::spectral::xy_to_tc(illuminant_xy.0, illuminant_xy.1);
    // np.linspace(0, 1, surface_size): start + i * step, step = 1/(n-1).
    let step = 1.0 / (surface_size as f64 - 1.0);
    let mut data = vec![0.0f64; surface_size * surface_size * 3];
    for i in 0..surface_size {
        let tx = i as f64 * step;
        for j in 0..surface_size {
            let ty = j as f64 * step;
            let base = (i * surface_size + j) * 3;
            for ch in 0..3 {
                let raw = poly2d_deg4(tx, ty, &surface_params[ch], (cx, cy));
                data[base + ch] = hanika_sigmoid(raw, HANATOS2025_MAX_CORRECTION_STOPS);
            }
        }
    }
    data
}

/// Multiply a TC LUT by `2 ** surface` — the `apply_surface` step of Python
/// `compute_hanatos2025_tc_lut` (`raw_lut *= 2**surface`).
pub fn apply_log_exposure_surface(
    tc_lut: &mut TcLut,
    surface_params: &[Vec<f64>],
    illuminant_xy: (f64, f64),
) {
    let surface = eval_poly4_log_exposure_surface(surface_params, illuminant_xy, tc_lut.size);
    for (cell, s) in tc_lut.data.iter_mut().zip(surface) {
        *cell *= 2.0f64.powf(s);
    }
}

/// Bounded correction of the hanatos2025 surface fit. Port of Python
/// `hanika_sigmoid`: `z / sqrt(1 + (z / max_val)^2)`, bounded to
/// ±`max_val` stops.
#[inline]
fn hanika_sigmoid(z: f64, max_val: f64) -> f64 {
    z / (1.0 + (z / max_val) * (z / max_val)).sqrt()
}

/// Degree-4 bivariate polynomial. Port of Python `poly2d_deg4`: the c0
/// term is intentionally dropped so `center_tc` maps to zero correction.
#[inline]
fn poly2d_deg4(tx: f64, ty: f64, params: &[f64], center_tc: (f64, f64)) -> f64 {
    // Python unpacks `_, c1, ..., c14 = params` (15 coefficients).
    let c1 = params[1];
    let c2 = params[2];
    let c3 = params[3];
    let c4 = params[4];
    let c5 = params[5];
    let c6 = params[6];
    let c7 = params[7];
    let c8 = params[8];
    let c9 = params[9];
    let c10 = params[10];
    let c11 = params[11];
    let c12 = params[12];
    let c13 = params[13];
    let c14 = params[14];
    let x = tx - center_tc.0;
    let y = ty - center_tc.1;
    let x2 = x * x;
    let y2 = y * y;
    let xy = x * y;
    let x3 = x2 * x;
    let y3 = y2 * y;
    c1 * x
        + c2 * y
        + c3 * x2
        + c4 * y2
        + c5 * xy
        + c6 * x3
        + c7 * y3
        + c8 * (x2 * y)
        + c9 * (x * y2)
        + c10 * (x2 * x2)
        + c11 * (y2 * y2)
        + c12 * (x3 * y)
        + c13 * (x2 * y2)
        + c14 * (x * y3)
}

/// Maximum hanatos2025 surface correction in stops — Python
/// `_HANATOS2025_MAX_CORRECTION_STOPS` (matches the fit).
const HANATOS2025_MAX_CORRECTION_STOPS: f64 = 2.0;

/// Camera UV/IR band-pass filter. Port of Python `compute_band_pass_filter`
/// (model/color_filters.py): two erf edges whose amplitudes are clipped to
/// [0, 1], multiplied together.
///
/// `filter_uv`/`filter_ir`: (amplitude, cutoff_wavelength_nm, edge_width_nm).
/// The IR edge uses a negated width, flipping the erf slope.
pub fn compute_band_pass_filter(filter_uv: [f64; 3], filter_ir: [f64; 3]) -> [f64; N_WAVELENGTHS] {
    let amp_uv = filter_uv[0].clamp(0.0, 1.0);
    let amp_ir = filter_ir[0].clamp(0.0, 1.0);
    let mut band_pass = [0.0f64; N_WAVELENGTHS];
    for i in 0..N_WAVELENGTHS {
        let wl = spectral::WAVELENGTH_MIN as f64 + (i as f64) * spectral::WAVELENGTH_STEP as f64;
        let edge_uv = 1.0 - amp_uv + amp_uv * sigmoid_erf(wl, filter_uv[1], filter_uv[2]);
        let edge_ir = 1.0 - amp_ir + amp_ir * sigmoid_erf(wl, filter_ir[1], -filter_ir[2]);
        band_pass[i] = edge_uv * edge_ir;
    }
    band_pass
}

/// `scipy.special.erf((x - center) / width) * 0.5 + 0.5` — Python
/// `sigmoid_erf` (model/color_filters.py).
#[inline]
fn sigmoid_erf(x: f64, center: f64, width: f64) -> f64 {
    erf((x - center) / width) * 0.5 + 0.5
}

/// Apply the camera UV/IR band-pass filter to the film sensitivity.
///
/// Port of the filtering in Python `FilmingStage._rgb_to_film_raw`
/// (runtime/stages/filming.py), active when either filter amplitude is
/// positive:
///
/// ```python
/// band_pass_filter = compute_band_pass_filter(filter_uv, filter_ir)
/// normalization = (np.sum(sensitivity * band_pass_filter * illuminant[:, None], axis=0)
///                  / np.sum(sensitivity * illuminant[:, None], axis=0))
/// sensitivity *= band_pass_filter[:, None] / normalization
/// ```
///
/// The reference-illuminant normalization preserves white balance: the
/// filtered sensitivity integrates to the same channel ratios under the
/// film's reference illuminant. Python applies this before BOTH upsampler
/// branches (hanatos2025 tc LUT and mallett2019 matrix), so callers must
/// filter the sensitivity before building either front-end.
pub fn apply_camera_uv_ir_band_pass(
    sensitivity: &mut [[f64; 3]],
    filter_uv: [f64; 3],
    filter_ir: [f64; 3],
    illuminant: &[f64],
) {
    let band_pass = compute_band_pass_filter(filter_uv, filter_ir);
    let n_wl = sensitivity.len().min(illuminant.len()).min(N_WAVELENGTHS);

    // numpy reduces the 81-wavelength axis with pairwise summation; the
    // numerator multiplies left-to-right: `(sens * band_pass) * illuminant`.
    let mut num = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    let mut den = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    for wl in 0..n_wl {
        for c in 0..3 {
            let si = (sensitivity[wl][c] * band_pass[wl]) * illuminant[wl];
            num[c].push(si);
            den[c].push(sensitivity[wl][c] * illuminant[wl]);
        }
    }
    let mut normalization = [1.0f64; 3];
    for c in 0..3 {
        normalization[c] = pairwise_sum_f64(&num[c]) / pairwise_sum_f64(&den[c]);
    }

    // Python: `sensitivity *= band_pass_filter[:, None] / normalization`
    for wl in 0..n_wl {
        for c in 0..3 {
            sensitivity[wl][c] *= band_pass[wl] / normalization[c];
        }
    }
}

/// Compute the midgray normalization factor for Hanatos2025.
///
/// Port of Python: `raw_midgray = einsum('k,km->m', illuminant * 0.184, sensitivity)`
/// then normalize by `raw_midgray[1]` (green channel).
pub fn compute_midgray_normalization(sensitivity: &[[f64; 3]], illuminant: &[f32]) -> f64 {
    let n_wl = sensitivity.len().min(illuminant.len());
    let mut raw_midgray = [0.0f64; 3];
    for wl in 0..n_wl {
        for c in 0..3 {
            raw_midgray[c] += illuminant[wl] as f64 * 0.184 * sensitivity[wl][c];
        }
    }
    if raw_midgray[1] > 1e-10 {
        1.0 / raw_midgray[1]
    } else {
        1.0
    }
}

/// Public re-export: pairwise sum for f64 spectra/wavelength reductions.
/// See `pairwise_sum_f64` doc below.
pub fn pairwise_sum_f64_pub(xs: &[f64]) -> f64 {
    pairwise_sum_f64(xs)
}

/// Pairwise (recursive-halving) f64 summation. Matches numpy's
/// `np.add.reduce` reduction pattern, which is what `np.sum` uses for
/// 1-D arrays. For 81 elements this is recursive halving; the result
/// differs from a naive left-to-right sum by 1-2 ULPs but matches numpy
/// bit-for-bit.
fn pairwise_sum_f64(xs: &[f64]) -> f64 {
    match xs.len() {
        0 => 0.0,
        1 => xs[0],
        2 => xs[0] + xs[1],
        n => {
            let mid = n / 2;
            pairwise_sum_f64(&xs[..mid]) + pairwise_sum_f64(&xs[mid..])
        }
    }
}

/// Evaluate the erf4 spectral bandpass window.
/// params: (c_uv, sigma_uv, c_ir, sigma_ir)
/// Returns [81][3] window values (same for all 3 channels in erf4 model).
fn eval_erf4_bandpass(params: &[f64]) -> Vec<[f64; 3]> {
    let sqrt2 = std::f64::consts::SQRT_2;
    let c_uv = params[0];
    let sigma_uv = params[1];
    let c_ir = params[2];
    let sigma_ir = params[3];

    let mut window = vec![[0.0f64; 3]; N_WAVELENGTHS];
    for i in 0..N_WAVELENGTHS {
        let wl = spectral::WAVELENGTH_MIN as f64 + (i as f64) * spectral::WAVELENGTH_STEP as f64;
        let edge_uv = 0.5 * (1.0 + erf((wl - c_uv) / (sigma_uv * sqrt2)));
        let edge_ir = 0.5 * (1.0 - erf((wl - c_ir) / (sigma_ir * sqrt2)));
        let w = edge_uv * edge_ir;
        window[i] = [w, w, w];
    }
    window
}

/// f64 erf via libm — matches scipy.special.erf at full f64 precision.
#[inline]
fn erf(x: f64) -> f64 {
    libm::erf(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn data_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    fn portra_sensitivity() -> Vec<[f64; 3]> {
        let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        film.log_sensitivity_f64()
            .iter()
            .map(|row| {
                let mut out = [0.0f64; 3];
                for c in 0..3 {
                    let v = 10.0f64.powf(row[c]);
                    out[c] = if v.is_nan() { 0.0 } else { v };
                }
                out
            })
            .collect()
    }

    fn assert_close(got: f64, want: f64, ctx: &str) {
        let tol = 1e-12_f64.max(want.abs() * 1e-12);
        assert!(
            (got - want).abs() <= tol,
            "{ctx}: got {got:.17}, want {want:.17} (diff {:.3e})",
            (got - want).abs()
        );
    }

    /// Python `compute_band_pass_filter` parity (model/color_filters.py) —
    /// reference values generated by replicating the pinned 0.3.4 code with
    /// numpy + math.erf for filter_uv=(0.6, 415, 9), filter_ir=(0.8, 670, 18)
    /// against the kodak_portra_400 wavelengths (380–780 nm, 5 nm step).
    #[test]
    fn camera_band_pass_filter_parity() {
        let bpf = compute_band_pass_filter([0.6, 415.0, 9.0], [0.8, 670.0, 18.0]);
        for (i, want) in &[
            (0usize, 0.40000001141188285_f64),
            (20, 1.0),
            (40, 0.99999999999938505),
            (60, 0.37282335245675713),
            (80, 0.19999999999999996),
        ] {
            assert_close(bpf[*i], *want, &format!("bandpass[{i}]"));
        }

        // Amplitudes clip to [0, 1] before the edge mix (Python np.clip).
        let clipped = compute_band_pass_filter([1.7, 415.0, 9.0], [0.0, 670.0, 18.0]);
        assert_close(
            clipped[40],
            1.0,
            "clamped uv amplitude passes full strength",
        );
        let zero_amp = compute_band_pass_filter([0.0, 415.0, 9.0], [0.0, 670.0, 18.0]);
        assert!(
            zero_amp.iter().all(|&v| v == 1.0),
            "zero amplitudes = identity"
        );
    }

    /// Python `FilmingStage._rgb_to_film_raw` camera-filter parity: the
    /// band-pass must multiply the sensitivity with reference-illuminant
    /// normalization (numerator `(sens * bpf) * illuminant`, denominator
    /// `sens * illuminant`, pairwise sums), preserving white balance.
    #[test]
    fn camera_uv_ir_filter_parity() {
        let mut sensitivity = portra_sensitivity();
        // Synthetic illuminant shared with the numpy reference generator.
        let illuminant: Vec<f64> = (0..N_WAVELENGTHS)
            .map(|i| {
                let wl =
                    spectral::WAVELENGTH_MIN as f64 + (i as f64) * spectral::WAVELENGTH_STEP as f64;
                1.0 + 0.1 * (2.0 * std::f64::consts::PI * (wl - 380.0) / 200.0).sin()
            })
            .collect();
        let before = sensitivity.clone();
        apply_camera_uv_ir_band_pass(
            &mut sensitivity,
            [0.6, 415.0, 9.0],
            [0.8, 670.0, 18.0],
            &illuminant,
        );

        // The reference normalization (per channel) is
        // sum((sens*bpf)*illu) / sum(sens*illu); it is verified through the
        // filtered-sensitivity reference values and the white-balance
        // invariant below.

        // Filtered sensitivity reference values (numpy replication of the
        // pinned Python code on kodak_portra_400).
        let cases = [
            (0usize, 0usize, 8.9267062837658333e-05_f64),
            (10, 1, 0.028524682793428562),
            (40, 2, 5.2281691314240252e-05),
            (60, 0, 0.0019794061694498643),
            (80, 2, 1.3560445696968211e-07),
        ];
        for (wl, c, want) in cases {
            assert_close(
                sensitivity[wl][c],
                want,
                &format!("filtered sensitivity[{wl}][{c}]"),
            );
        }

        // White balance is preserved: the filtered sensitivity integrates to
        // the same per-channel response under the reference illuminant.
        let mut resp_before = [0.0f64; 3];
        let mut resp_after = [0.0f64; 3];
        for wl in 0..N_WAVELENGTHS {
            for c in 0..3 {
                resp_before[c] += before[wl][c] * illuminant[wl];
                resp_after[c] += sensitivity[wl][c] * illuminant[wl];
            }
        }
        let ratio_before = resp_before[0] / resp_before[1];
        let ratio_after = resp_after[0] / resp_after[1];
        assert!(
            (ratio_before - ratio_after).abs() < 1e-12,
            "white balance shifted: {ratio_before} → {ratio_after}"
        );
    }

    /// `gaussian_blur_wavelength` parity with the `spectral_gaussian_blur`
    /// step of Python `compute_hanatos2025_tc_lut`
    /// (`scipy.ndimage.gaussian_filter(spectra_lut, (0, 0, sigma))`):
    /// truncate-4.0 radius, full-support kernel normalization, and the
    /// reflect boundary that repeats the edge sample. Reference values from
    /// an independent numpy replication of the scipy semantics.
    #[test]
    fn gaussian_blur_wavelength_parity() {
        let row: Vec<f64> = (0..81)
            .map(|i| 0.3 + 0.2 * (0.15 * i as f64 + 0.7).cos())
            .collect();
        let mut row = row;
        row[37] = 5.0;
        let cube = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: row.clone(),
        };
        let blurred = gaussian_blur_wavelength(&cube, 7.5);
        for (i, want) in &[
            (0usize, 0.31560871760374259_f64),
            (1, 0.31340073670466867),
            (37, 0.64554715171155164),
            (40, 0.61810654447618485),
            (80, 0.42851042941668716),
        ] {
            assert_close(blurred.data[*i], *want, &format!("blur[{i}]"));
        }

        // scipy 'reflect' keeps a constant signal constant, and a zero sigma
        // is a no-op (Python guards `spectral_gaussian_blur > 0`).
        let flat = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: vec![0.37; 81],
        };
        let flat_blurred = gaussian_blur_wavelength(&flat, 12.0);
        assert!(
            flat_blurred.data.iter().all(|&v| (v - 0.37).abs() < 1e-15),
            "constant spectrum must be preserved by the reflect boundary"
        );
        let unblurred = gaussian_blur_wavelength(&cube, 0.0);
        assert_eq!(unblurred.data, cube.data, "sigma 0 must be a no-op");

        // Full-width kernel normalization: radius int(4*7.5+0.5) = 30, and
        // the interior of a delta impulse reproduces the kernel weights.
        let mut impulse = vec![0.0f64; 81];
        impulse[40] = 1.0;
        let impulse_cube = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: impulse,
        };
        let response = gaussian_blur_wavelength(&impulse_cube, 7.5).data;
        // Interior samples reproduce the (normalized) kernel weights:
        // out[40 + d] / out[40] == exp(-0.5 d²/σ²) for |d| small enough to
        // stay clear of both reflect boundaries.
        let sigma = 7.5f64;
        let peak = response[40];
        for d in -10i64..=10 {
            let want = (-0.5 / (sigma * sigma) * (d * d) as f64).exp();
            let got = response[(40 + d) as usize] / peak;
            assert!(
                (got - want).abs() < 1e-12,
                "kernel shape mismatch at offset {d}: {got} vs {want}"
            );
        }
    }

    /// `eval_poly4_log_exposure_surface` parity with Python
    /// `eval_poly4_log_exposure_surface` on the kodak_portra_400 surface
    /// parameters and D55 reference illuminant.
    #[test]
    fn poly4_surface_parity() {
        let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let surface_params = &film.data.hanatos2025_adaptation_surface_params;
        assert_eq!(surface_params.len(), 3);
        assert!(surface_params.iter().all(|r| r.len() == 15));

        // D55 xy from the f64 constants (matches Python `_illuminant_to_xy`).
        let xy = spektrafilm_math::spectral::illuminant_to_xy(&spectral::ILLUMINANT_D55);
        assert_close(xy.0, 0.33243163892154942, "D55 x");
        assert_close(xy.1, 0.34744439893064599, "D55 y");
        let (cx, cy) = spektrafilm_math::spectral::xy_to_tc(xy.0, xy.1);
        assert_close(cx, 0.44564751671296865, "surface center tx");
        assert_close(cy, 0.52046265100004552, "surface center ty");

        let size = 192;
        let surface = eval_poly4_log_exposure_surface(surface_params, xy, size);
        let at = |i: usize, j: usize, ch: usize| surface[(i * size + j) * 3 + ch];
        // (log2-correction, 2**correction) reference pairs.
        let cases = [
            (
                0usize,
                0usize,
                0usize,
                1.9108227429072688_f64,
                3.7602347794518272_f64,
            ),
            (96, 96, 1, -0.011682565105707881, 0.99193496128764136),
            (191, 191, 2, 0.34181696988254229, 1.267351726964274),
            (50, 130, 0, 0.084473213471850842, 1.0603005119142745),
        ];
        for (i, j, ch, want_s, want_pow) in cases {
            let got = at(i, j, ch);
            assert_close(got, want_s, &format!("surface[{i}][{j}][{ch}]"));
            // `raw_lut *= 2**surface` step.
            let mut lut = TcLut {
                size,
                channels: 3,
                data: vec![1.0; size * size * 3],
            };
            apply_log_exposure_surface(&mut lut, surface_params, xy);
            assert_close(
                lut.data[(i * size + j) * 3 + ch],
                want_pow,
                &format!("2**surface[{i}][{j}][{ch}]"),
            );
        }

        // The correction is bounded to ±2 stops (hanatos fit bound).
        assert!(
            surface.iter().all(|&v| v.abs() <= 2.0 + 1e-12),
            "surface exceeds the ±2-stop bound"
        );
    }

    /// `compute_hanatos2025_tc_lut` adaptation semantics: blur, window and
    /// surface each change the LUT, and malformed parameters fail with
    /// actionable errors instead of silently producing artifacts.
    #[test]
    fn compute_hanatos2025_tc_lut_adaptation() {
        let dir = data_dir();
        let spectra_lut = load_spectra_lut(&dir).unwrap();
        let sensitivity = portra_sensitivity();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let window_params = film.data.hanatos2025_adaptation_window_params.clone();
        let surface_params = &film.data.hanatos2025_adaptation_surface_params;
        let xy = spektrafilm_math::spectral::illuminant_to_xy(&spectral::ILLUMINANT_D55);
        let illu = spectral::ILLUMINANT_D55_F64;

        let adaptation = |blur: f64, win: bool, surf: bool| Hanatos2025Adaptation {
            window_params: &window_params,
            surface_params,
            spectral_gaussian_blur: blur,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: win,
            apply_surface: surf,
        };

        let base =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, true, false))
                .unwrap();
        let blurred =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(10.0, true, false))
                .unwrap();
        let surfaced =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, true, true))
                .unwrap();
        let windowless =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, false, false))
                .unwrap();

        assert_eq!(base.size, spectra_lut.size);
        assert!(base.data.iter().zip(&blurred.data).any(|(a, b)| a != b));
        assert!(base.data.iter().zip(&surfaced.data).any(|(a, b)| a != b));
        assert!(base.data.iter().zip(&windowless.data).any(|(a, b)| a != b));

        // Malformed window (Python unpacks exactly 4 erf4 parameters).
        let short_window = [1.0, 2.0, 3.0];
        let bad = Hanatos2025Adaptation {
            window_params: &short_window,
            surface_params,
            spectral_gaussian_blur: 0.0,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: true,
            apply_surface: false,
        };
        let err = compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &bad).err().expect("invalid window must fail");
        assert!(err.contains("4 erf4"), "window error not actionable: {err}");

        // Malformed surface (Python unpacks exactly 3 × 15 coefficients).
        let bad_surface = vec![vec![0.0; 15], vec![0.0; 15]];
        let bad = Hanatos2025Adaptation {
            window_params: &window_params,
            surface_params: &bad_surface,
            spectral_gaussian_blur: 0.0,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: true,
            apply_surface: true,
        };
        let err = compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &bad).err().expect("invalid surface must fail");
        assert!(
            err.contains("3 channels"),
            "surface error not actionable: {err}"
        );
    }
    #[test]
    fn descriptor_parser_and_registry_shape_contract() {
        let text = r#"
            identifier = "example"
            file = "example.npy"
            kind = "reflectance"
            title = "Example"
            [array]
            lut_size = 2
            bands = 81
            spectral_shape = [380.0, 780.0, 5.0]
            [reflectance]
            scene_illuminant = "D65"
            midgray = 0.184
        "#;
        let d = parse_lut_descriptor(text).unwrap();
        assert_eq!(d.identifier, "example");
        assert_eq!(d.lut_size * d.lut_size * d.bands, 324);
        assert_eq!(d.scene_illuminant.as_deref(), Some("D65"));
    }

    #[test]
    fn reflectance_contract_has_three_channels() {
        let spectra = SpectraLut {
            size: 2,
            n_wavelengths: N_WAVELENGTHS,
            data: vec![1.0; 4 * N_WAVELENGTHS],
        };
        let sensitivity = vec![[1.0; 3]; N_WAVELENGTHS];
        let lut = compute_reflectance_tc_lut(
            &spectra,
            &sensitivity,
            &spectral::ILLUMINANT_D65_F64,
        );
        assert_eq!(lut.channels, 3);
        assert_eq!(lut.data.len(), 2 * 2 * 3);
        assert!(lut.data.iter().all(|v| v.is_finite() && *v > 0.0));
    }

    #[test]
    fn bundled_reflectance_registry_methods_produce_finite_luts() {
        let data = data_dir();
        let sensitivity = portra_sensitivity();
        let ids = available_lut_identifiers(&data).unwrap();
        assert!(ids.len() >= 6, "expected bundled spectral registry entries: {ids:?}");
        for id in ids {
            let descriptor = lut_descriptor(&data, &id).unwrap();
            if descriptor.kind != "reflectance" {
                continue;
            }
            let lut = compute_registered_tc_lut(
                &data,
                &id,
                &sensitivity,
                &spectral::ILLUMINANT_D55_F64,
                None,
            )
            .unwrap_or_else(|error| panic!("{id} failed: {error}"));
            assert_eq!(lut.channels, 3);
            assert!(lut.data.iter().all(|value| value.is_finite()), "{id} contains non-finite values");
        }
    }

    #[test]
    fn generated_illuminants_are_supported_and_normalized() {
        for name in ["A", "D60", "D75", "E", "BB3400", "BB5000", "TH-KG3-L"] {
            assert!(is_supported_illuminant(name), "{name} should be supported");
            let values = select_illuminant_f64(name);
            assert_eq!(values.len(), N_WAVELENGTHS);
            assert!(values.iter().all(|value| value.is_finite() && *value > 0.0));
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            assert!((mean - 1.0).abs() < 1e-12, "{name} mean={mean}");
        }
        assert!(!is_supported_illuminant("BB1000"));
        assert!(!is_supported_illuminant("not-an-illuminant"));
    }
}
