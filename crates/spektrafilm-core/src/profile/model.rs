//! Profile data model and JSON wire semantics.

use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub metadata: ProfileMetadata,
    pub info: ProfileInfo,
    pub data: ProfileData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMetadata {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub copyright: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub citation: String,
    #[serde(default)]
    pub datasource: String,
}
fn ser_f64<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value.is_finite() {
        serializer.serialize_f64(*value)
    } else {
        serializer.serialize_none()
    }
}

fn ser_f64_vec<S>(values: &Vec<f64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    values
        .iter()
        .map(|value| value.is_finite().then_some(*value))
        .collect::<Vec<_>>()
        .serialize(serializer)
}

fn ser_f64_matrix<S>(values: &Vec<Vec<f64>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    values
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| value.is_finite().then_some(*value))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .serialize(serializer)
}

fn ser_f64_tensor<S>(values: &Vec<Vec<Vec<f64>>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    values
        .iter()
        .map(|matrix| {
            matrix
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|value| value.is_finite().then_some(*value))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .serialize(serializer)
}

fn ser_base_density<S>(values: &Vec<Vec<f64>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let rows = values
        .iter()
        .map(|row| {
            if row.len() <= 1 {
                row.first()
                    .and_then(|value| value.is_finite().then_some(*value))
                    .map_or(serde_json::Value::Null, serde_json::Value::from)
            } else {
                serde_json::Value::Array(
                    row.iter()
                        .map(|value| {
                            value
                                .is_finite()
                                .then_some(*value)
                                .map_or(serde_json::Value::Null, serde_json::Value::from)
                        })
                        .collect(),
                )
            }
        })
        .collect::<Vec<_>>();
    rows.serialize(serializer)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub stock: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "type", default = "default_negative")]
    pub film_type: String,
    #[serde(default = "default_film")]
    pub support: String,
    #[serde(default = "default_filming")]
    pub stage: String,
    #[serde(rename = "use", default = "default_still")]
    pub usage: String,
    #[serde(default = "default_weak")]
    pub antihalation: String,
    pub target_print: Option<String>,
    #[serde(default = "default_color")]
    pub channel_model: String,
    #[serde(default = "default_status_m")]
    pub densitometer: String,
    #[serde(default, serialize_with = "ser_f64")]
    pub log_sensitivity_density_over_min: f64,
    #[serde(default = "default_d55")]
    pub reference_illuminant: String,
    #[serde(default = "default_d50")]
    pub viewing_illuminant: String,
}

fn default_negative() -> String {
    "negative".into()
}
fn default_film() -> String {
    "film".into()
}
fn default_filming() -> String {
    "filming".into()
}
fn default_still() -> String {
    "still".into()
}
fn default_weak() -> String {
    "weak".into()
}
fn default_color() -> String {
    "color".into()
}
fn default_status_m() -> String {
    "status_M".into()
}
fn default_d55() -> String {
    "D55".into()
}
fn default_d50() -> String {
    "D50".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    #[serde(default, serialize_with = "ser_f64_vec")]
    pub wavelengths: Vec<f64>,
    #[serde(
        default,
        deserialize_with = "deser_zero_matrix",
        serialize_with = "ser_f64_matrix"
    )]
    pub log_sensitivity: Vec<Vec<f64>>,
    #[serde(
        default,
        deserialize_with = "deser_zero_vec",
        serialize_with = "ser_f64_vec"
    )]
    pub hanatos2025_adaptation_window_params: Vec<f64>,
    #[serde(
        default,
        deserialize_with = "deser_zero_matrix",
        serialize_with = "ser_f64_matrix"
    )]
    pub hanatos2025_adaptation_surface_params: Vec<Vec<f64>>,
    /// NaN-preserving: null values mean "no data at this wavelength"
    #[serde(
        default,
        deserialize_with = "deser_nullable_matrix",
        serialize_with = "ser_f64_matrix"
    )]
    pub channel_density: Vec<Vec<f64>>,
    /// NaN-preserving base+fog as stored in the JSON: one row per wavelength,
    /// with 1 column for colour profiles or N columns (one per development
    /// time) for B&W families. `base_density` below holds the resolved column.
    #[serde(
        rename = "base_density",
        default,
        deserialize_with = "deser_base_density_rows",
        serialize_with = "ser_base_density"
    )]
    pub base_density_rows: Vec<Vec<f64>>,
    /// Resolved base+fog spectrum (the selected development-time column of
    /// `base_density_rows`). `load_profile` selects the default column;
    /// `resolve_for_render` re-selects it from the chosen development time.
    #[serde(skip)]
    pub base_density: Vec<f64>,
    /// NaN-preserving
    #[serde(
        default,
        deserialize_with = "deser_nullable_vec",
        serialize_with = "ser_f64_vec"
    )]
    pub midscale_neutral_density: Vec<f64>,
    #[serde(default, serialize_with = "ser_f64_vec")]
    pub log_exposure: Vec<f64>,
    #[serde(
        default,
        deserialize_with = "deser_zero_matrix",
        serialize_with = "ser_f64_matrix"
    )]
    pub density_curves: Vec<Vec<f64>>,
    #[serde(
        default,
        deserialize_with = "deser_zero_tensor",
        serialize_with = "ser_f64_tensor"
    )]
    pub density_curves_layers: Vec<Vec<Vec<f64>>>,
    /// Parametric fit of the density curves (sum-of-CDFs per channel), used by
    /// the print-curve morph. Absent on older profiles. On B&W profiles the
    /// first axis indexes development time instead of channels.
    #[serde(default)]
    pub density_curves_model: Option<DensityCurvesModel>,
    /// Development-time family axis (minutes), one entry per density-curves
    /// column on B&W profiles. Empty for colour profiles.
    #[serde(
        default,
        deserialize_with = "deser_zero_vec",
        serialize_with = "ser_f64_vec"
    )]
    pub development_time: Vec<f64>,
}

/// Sum-of-CDFs parametric model of a profile's density curves.
///
/// `centers`, `amplitudes`, `sigmas` are each `[n_channels][n_layers]`. Each
/// channel's density is `sum_i amplitudes[i] * Phi((x - centers[i]) / sigmas[i])`,
/// where `Phi` is the (sign-flipped for positive stocks) standard-normal CDF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DensityCurvesModel {
    #[serde(default)]
    pub model_type: String,
    #[serde(default, serialize_with = "ser_f64_matrix")]
    pub centers: Vec<Vec<f64>>,
    #[serde(default, serialize_with = "ser_f64_matrix")]
    pub amplitudes: Vec<Vec<f64>>,
    #[serde(default, serialize_with = "ser_f64_matrix")]
    pub sigmas: Vec<Vec<f64>>,
    /// Per-layer median-preserving skew for `sept_norm_cdfs`.
    #[serde(default)]
    pub alphas: Option<Vec<Vec<f64>>>,
}

impl DensityCurvesModel {
    pub fn n_channels(&self) -> usize {
        self.centers.len()
    }
    pub fn n_layers(&self) -> usize {
        self.centers.first().map_or(0, Vec::len)
    }
}

/// Deserialize with null → 0.0 (for data that must be finite: sensitivity, density curves).
/// Whole-field `null` (B&W profiles null out unused fields) reads as empty.
fn deser_zero_vec<'de, D>(deserializer: D) -> Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<Vec<Option<f64>>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|x| x.unwrap_or(0.0))
        .collect())
}
fn deser_zero_matrix<'de, D>(deserializer: D) -> Result<Vec<Vec<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<Vec<Vec<Option<f64>>>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|row| row.into_iter().map(|x| x.unwrap_or(0.0)).collect())
        .collect())
}
fn deser_zero_tensor<'de, D>(deserializer: D) -> Result<Vec<Vec<Vec<f64>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<Vec<Vec<Vec<Option<f64>>>>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|m| {
            m.into_iter()
                .map(|r| r.into_iter().map(|x| x.unwrap_or(0.0)).collect())
                .collect()
        })
        .collect())
}

/// Deserialize with null → NaN (for spectral data where null means "no measurement").
/// Whole-field `null` reads as empty.
fn deser_nullable_vec<'de, D>(deserializer: D) -> Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<Vec<Option<f64>>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|x| x.unwrap_or(f64::NAN))
        .collect())
}

/// Deserialize a Vec<Vec<f64>> where elements may be null.
/// For channel_density, null means "no data at this wavelength" — we use NaN
/// to propagate this correctly through spectral calculations (matching Python).
fn deser_nullable_matrix<'de, D>(deserializer: D) -> Result<Vec<Vec<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<Vec<Vec<Option<f64>>>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|row| row.into_iter().map(|x| x.unwrap_or(f64::NAN)).collect())
        .collect())
}

/// Base+fog density: accepts both the colour shape (1-D, one value per
/// wavelength) and the B&W development-time family shape (n_wl × N rows).
/// Normalized to one row per wavelength; nulls → NaN.
fn deser_base_density_rows<'de, D>(deserializer: D) -> Result<Vec<Vec<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Row {
        One(Option<f64>),
        Many(Vec<Option<f64>>),
    }
    let v: Option<Vec<Row>> = Deserialize::deserialize(deserializer)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|r| match r {
            Row::One(x) => vec![x.unwrap_or(f64::NAN)],
            Row::Many(xs) => xs.into_iter().map(|x| x.unwrap_or(f64::NAN)).collect(),
        })
        .collect())
}

impl Profile {
    pub fn is_negative(&self) -> bool {
        self.info.film_type == "negative"
    }
    pub fn is_positive(&self) -> bool {
        self.info.film_type == "positive"
    }
    pub fn is_film(&self) -> bool {
        self.info.support == "film"
    }
    pub fn is_paper(&self) -> bool {
        self.info.support == "paper"
    }
    pub fn is_color(&self) -> bool {
        self.info.channel_model == "color"
    }
    pub fn is_bw(&self) -> bool {
        self.info.channel_model == "bw"
    }
    pub fn is_filming(&self) -> bool {
        self.info.stage == "filming"
    }
    pub fn is_printing(&self) -> bool {
        self.info.stage == "printing"
    }
    /// A stock that can serve as the print stage: a paper or a print film
    /// (`support` paper or film) recorded at the printing stage.
    pub fn is_print_stock(&self) -> bool {
        (self.is_paper() || self.is_film()) && self.is_printing()
    }
}
