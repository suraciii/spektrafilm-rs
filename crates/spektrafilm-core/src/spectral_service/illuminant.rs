//! Exact illuminant catalog sampling and pinned reference spectra.
use super::{SpectralShape, default_spectral_shape};
use spektrafilm_math::spectral::{self, N_WAVELENGTHS};
use std::borrow::Cow;
use std::sync::LazyLock;
#[derive(serde::Deserialize)]
struct IlluminantCatalog {
    spectral_shape: [f64; 3],
    sources: std::collections::BTreeMap<String, Vec<f64>>,
}

static ILLUMINANT_CATALOG: LazyLock<IlluminantCatalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../data/illuminants/catalog.json"))
        .expect("bundled exact illuminant catalog")
});

fn illuminant_catalog() -> &'static IlluminantCatalog {
    &ILLUMINANT_CATALOG
}

/// Sample an exact catalog source, or evaluate a Planck blackbody, on a grid.
/// Catalog spectra cannot be extrapolated beyond their measured/baked bounds.
pub fn select_illuminant_on_grid_f64(name: &str, shape: SpectralShape) -> Result<Vec<f64>, String> {
    let mut values = if let Some(temperature) = parse_blackbody_temperature(name) {
        (0..shape.samples)
            .map(|i| {
                let wavelength_m = shape.wavelength(i) * 1e-9;
                3.741_771_852e-16
                    / (wavelength_m.powi(5)
                        * (1.438_776_877e-2 / (wavelength_m * temperature)).exp_m1())
            })
            .collect::<Vec<_>>()
    } else {
        let catalog = illuminant_catalog();
        let source = catalog
            .sources
            .get(name)
            .ok_or_else(|| format!("unsupported illuminant {name:?}"))?;
        let source_shape = SpectralShape::new(catalog.spectral_shape)?;
        if source.len() != source_shape.samples {
            return Err(format!(
                "catalog source {name:?} does not match its spectral grid"
            ));
        }
        if shape.bounds[0] < source_shape.bounds[0] || shape.bounds[1] > source_shape.bounds[1] {
            return Err(format!(
                "illuminant {name:?} supports {:?}, requested {:?}",
                source_shape.bounds, shape.bounds
            ));
        }
        (0..shape.samples)
            .map(|i| {
                let x = (shape.wavelength(i) - source_shape.bounds[0]) / source_shape.bounds[2];
                let lo = (x.floor() as usize).min(source.len() - 1);
                let hi = (lo + 1).min(source.len() - 1);
                source[lo] + (source[hi] - source[lo]) * (x - lo as f64)
            })
            .collect::<Vec<_>>()
    };
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return Err(format!(
            "illuminant {name:?} has invalid spectral normalization"
        ));
    }
    for value in &mut values {
        *value /= mean;
    }
    Ok(values)
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
    0.09456970290598739,
    0.1358223126707035,
    0.18067928085342475,
    0.2259785007417468,
    0.2677351393678483,
    0.30492220983936275,
    0.3404115534894003,
    0.3725452187996161,
    0.40207481582863175,
    0.43144620857294336,
    0.46257922241084165,
    0.49512511929127734,
    0.5289219800537478,
    0.5626679850892985,
    0.5958260010917935,
    0.6299593886971075,
    0.6644097398593504,
    0.700554052742682,
    0.7408873075661121,
    0.7818238319378953,
    0.8222884814131736,
    0.8625802756777197,
    0.9030662159717104,
    0.9436349150547809,
    0.9830168583481824,
    1.0205556844981798,
    1.0564491305809889,
    1.0903341528774033,
    1.1219168901687964,
    1.1552754643901004,
    1.18957077142463,
    1.224385079368606,
    1.2608110664687997,
    1.3017320113172588,
    1.3454684726941972,
    1.3859275667105297,
    1.4213335202694428,
    1.4514983451100734,
    1.4793862774441284,
    1.506950815002019,
    1.5335196656859136,
    1.554810667188734,
    1.5759275295354451,
    1.594335896239912,
    1.6071424189513246,
    1.6178330856868541,
    1.6264112869522358,
    1.6318859951354185,
    1.6324648760560658,
    1.6295535378804413,
    1.6210239391887935,
    1.612877388213371,
    1.5996095071678285,
    1.5806584699460842,
    1.5560501956617274,
    1.5336590858122428,
    1.5058658931227298,
    1.4662994867298649,
    1.428579312658189,
    1.390812091273809,
    1.3452831165124508,
    1.2992635156028105,
    1.2522330269883137,
    1.2009204344497504,
    1.148659982040946,
    1.0954695988974383,
    1.03926723560586,
    0.9844711396581393,
    0.9236646594179271,
    0.8705698517734068,
    0.8139325853073219,
    0.7456552858076373,
    0.6831551422932013,
    0.630258990076302,
    0.5900955284329665,
    0.5422665143829449,
    0.5029095765733292,
    0.4567257035827689,
    0.4130914600721322,
    0.37567089330813325,
    0.3419938635287332,
];

pub const ILLUMINANT_TH_KG3_L: [f32; N_WAVELENGTHS] = narrow_illuminant(&ILLUMINANT_TH_KG3_L_F64);

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
    let mut names: Vec<_> = illuminant_catalog()
        .sources
        .keys()
        .map(String::as_str)
        .collect();
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
    (temperature.is_finite() && (1667.0..=25000.0).contains(&temperature)).then_some(temperature)
}

fn generated_illuminant(name: &str) -> Option<(Cow<'static, [f32]>, Cow<'static, [f64]>)> {
    let values =
        select_illuminant_on_grid_f64(name, SpectralShape::new(default_spectral_shape()).ok()?)
            .ok()?;
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
