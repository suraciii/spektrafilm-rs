use serde_json::{Value, json};
use spektrafilm_math::{image::ImageBuf, precision::from_f64};
use spektrafilm_model::grain::fotufilm::{
    Curve, Geometry, Look, Metadata, Population, PopulationInput,
};
use std::{env, error::Error, fs, path::Path};

fn field<'a>(v: &'a Value, path: &str) -> Result<&'a Value, Box<dyn Error>> {
    let mut cur = v;
    for part in path.split('.') {
        cur = cur
            .get(part)
            .ok_or_else(|| format!("missing required fixture field {path}"))?;
    }
    Ok(cur)
}

fn f64_at(v: &Value, path: &str) -> Result<f64, Box<dyn Error>> {
    field(v, path)?
        .as_f64()
        .ok_or_else(|| format!("fixture field {path} is not a number").into())
}

fn array<'a>(v: &'a Value, path: &str) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    field(v, path)?
        .as_array()
        .ok_or_else(|| format!("fixture field {path} is not an array").into())
}

fn f64_array(v: &Value, path: &str) -> Result<Vec<f64>, Box<dyn Error>> {
    array(v, path)?
        .iter()
        .enumerate()
        .map(|(i, x)| {
            x.as_f64()
                .ok_or_else(|| format!("fixture field {path}[{i}] is not a number").into())
        })
        .collect()
}

fn rel_error(actual: f64, expected: f64) -> f64 {
    let scale = expected.abs().max(1e-12);
    (actual - expected).abs() / scale
}

fn comparison(actual: f64, expected: f64) -> Value {
    json!({
        "actual": actual,
        "expected": expected,
        "absoluteError": (actual - expected).abs(),
        "relativeError": rel_error(actual, expected),
    })
}

fn curve(stock: &Value, record: usize) -> Result<Curve, Box<dyn Error>> {
    let records = stock["curves"].as_array().ok_or("curves is not an array")?;
    let record_value = records
        .get(record)
        .ok_or_else(|| format!("missing curve {record}"))?;
    let sampled = record_value
        .get("sampled")
        .ok_or("curve sampled field is missing")?;
    let log_exposure = f64_array(sampled, "logExposure")?;
    let density = f64_array(sampled, "density")?;
    if log_exposure.len() != density.len() || log_exposure.len() < 2 {
        return Err(
            format!("fixture curve {record} has mismatched or insufficient samples").into(),
        );
    }
    Ok(Curve {
        d_min: f64_at(record_value, "dMin")?,
        d_max: *density.last().unwrap(),
        log_exposure,
        density,
    })
}

fn load_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        return Err("usage: fotufilm_reference_probe <fixture-directory>".into());
    }
    let dir = Path::new(&args[1]);
    let stock = load_json(&dir.join("stock-input.json"))?;
    let anchored = load_json(&dir.join("anchored.json"))?;
    let population_oracle = load_json(&dir.join("population.json"))?;
    let pitch_oracle = load_json(&dir.join("pitch.json"))?;
    let residual_oracle = load_json(&dir.join("residual-output.json"))?;

    let curves: [Curve; 3] = [curve(&stock, 0)?, curve(&stock, 1)?, curve(&stock, 2)?];
    let grain_strength = f64_at(&stock, "grainStrength")?;
    let layer_weights = f64_array(&stock, "grainLayerWeights")?;
    if layer_weights.len() != 3 {
        return Err("grainLayerWeights must have three entries".into());
    }
    let crystal = field(&stock, "crystalGrainPopulation")?;
    let sublayer_shares = f64_array(crystal, "sublayerShares")?;
    if sublayer_shares.len() != 4 {
        return Err("crystalGrainPopulation.sublayerShares must have four entries".into());
    }
    let metadata = Metadata {
        sigma48: std::array::from_fn(|r| grain_strength * layer_weights[r]),
        anchor_net_density: [1.0; 3],
        cloud_radius_mm: f64_at(&stock, "grainSizeMM")?,
        layer_size_ratio: [1.0; 3],
        radius_span: f64_at(crystal, "radiusSpan")?,
        sublayer_shares: sublayer_shares.try_into().map_err(|_| "invalid shares")?,
        coating_density_scale: f64_at(crystal, "coatingDensityScale")?,
        fog_density: 0.03,
        reversal: field(&stock, "isReversal")?
            .as_bool()
            .ok_or("isReversal is not bool")?,
        silver: field(&stock, "isMonochrome")?
            .as_bool()
            .ok_or("isMonochrome is not bool")?,
        monochrome: field(&stock, "isMonochrome")?
            .as_bool()
            .ok_or("isMonochrome is not bool")?,
    };
    let look = Look::default();
    let population = Population::prepare(
        PopulationInput {
            curves: curves.clone(),
            reference_curves: Some(curves),
            metadata: metadata.clone(),
            reference_metadata: Some(metadata),
        },
        &|| Ok(()),
    )?;
    let diagnostics = population.diagnostics();
    let bank = population.build_bank(&|| Ok(()))?;

    let mut preparation = Vec::new();
    let anchored_records = array(&anchored, "records")?;
    let population_records = array(&population_oracle, "records")?;
    for r in 0..3 {
        let expected_record = &anchored_records[r];
        let actual_bins = &diagnostics.records[r].bins;
        let expected_bins = array(&population_records[r], "bins")?;
        let mut sublayers = Vec::new();
        for (i, actual) in actual_bins.iter().enumerate() {
            let expected = expected_bins
                .get(i)
                .ok_or_else(|| format!("population record {r} missing bin {i}"))?;
            let anchored_layer = array(expected_record, "sublayers")?
                .get(i)
                .ok_or_else(|| format!("anchored record {r} missing sublayer {i}"))?;
            sublayers.push(json!({
                "index": i,
                "cellMM": comparison(actual.cell_mm, f64_at(anchored_layer, "cellMM")?),
                "coatedPerMM2": comparison(actual.crystals_per_mm2, f64_at(anchored_layer, "coatedPerMM2")?),
                "capacity": comparison(actual.capacity, f64_at(anchored_layer, "capacity")?),
                "dyePerCloud": comparison(actual.dye_per_cloud, f64_at(anchored_layer, "dyePerCloudMM2")?),
                "cloudRadiusMM": comparison(actual.cloud_radius_mm, f64_at(expected, "cloudRadiusMM")?),
            }));
        }
        preparation.push(json!({"record": r, "sublayers": sublayers}));
    }

    let mut pitch = Vec::new();
    for expected in pitch_oracle
        .as_array()
        .ok_or("pitch fixture is not an array")?
    {
        let footprint = f64_at(expected, "footprintTexels")?;
        let actual = bank.pitch_tables([footprint, footprint], &|| Ok(()))?;
        let expected_means = array(expected, "meanDensity")?;
        let expected_corr = array(expected, "blendCorrelation")?;
        let mut records = Vec::new();
        for r in 0..3 {
            let means = expected_means[r]
                .as_array()
                .ok_or("pitch meanDensity record is not an array")?;
            let corr = expected_corr[r]
                .as_array()
                .ok_or("pitch blendCorrelation record is not an array")?;
            records.push(json!({
                "record": r,
                "meanDensity": actual.mean_density[r].iter().enumerate().map(|(k, x)| comparison(*x, means[k].as_f64().unwrap())).collect::<Vec<_>>(),
                "blendCorrelation": actual.blend_correlation[r].iter().enumerate().map(|(k, x)| comparison(*x, corr[k].as_f64().unwrap())).collect::<Vec<_>>(),
            }));
        }
        pitch.push(json!({
            "footprintTexels": footprint,
            "sampleGeometry": {"samplesPerAxis": 128, "sampleCount": 16384, "axes": [footprint, footprint]},
            "records": records,
        }));
    }

    let mut readout = Vec::new();
    for expected in residual_oracle
        .as_array()
        .ok_or("residual fixture is not an array")?
    {
        let width = expected["width"].as_u64().ok_or("missing residual width")? as u32;
        let height = expected["height"]
            .as_u64()
            .ok_or("missing residual height")? as u32;
        let pitch_texels = f64_at(expected, "pitchTexels")?;
        let seed = expected["seed"].as_u64().ok_or("missing residual seed")?;
        let inputs = expected["input"]
            .as_array()
            .ok_or("missing residual input")?;
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for i in 0..(width * height) as usize {
            for channel in inputs.iter().take(3) {
                data.push(from_f64(
                    channel[i].as_f64().ok_or("input sample is not a number")?,
                ));
            }
        }
        let input = ImageBuf::from_data(width, height, data);
        let output = bank.transform(
            &input,
            Geometry {
                source_pitch_mm: [pitch_texels * 0.001; 2],
                origin_mm: [0.0; 2],
                output_pitch_mm: [pitch_texels * 0.001; 2],
                footprint_mm: [pitch_texels * 0.001; 2],
            },
            look,
            seed,
            &|| Ok(()),
        )?;
        let oracle_residual = expected["residual"]
            .as_array()
            .ok_or("missing residual values")?;
        let oracle_corrected = expected["corrected"]
            .as_array()
            .ok_or("missing corrected values")?;
        let mut residual_diffs = Vec::new();
        let mut corrected_diffs = Vec::new();
        let mut residual_statistics = Vec::new();
        let mut corrected_identity_error: f64 = 0.0;
        for (r, (input_channel, (residual_channel, corrected_channel))) in inputs
            .iter()
            .zip(oracle_residual.iter().zip(oracle_corrected.iter()))
            .take(3)
            .enumerate()
        {
            let mut max_residual: f64 = 0.0;
            let mut sum_residual: f64 = 0.0;
            let mut max_corrected: f64 = 0.0;
            let mut sum_corrected: f64 = 0.0;
            let mut actual_residuals = Vec::with_capacity((width * height) as usize);
            let mut oracle_residuals = Vec::with_capacity((width * height) as usize);
            for i in 0..(width * height) as usize {
                let input_value = input_channel[i]
                    .as_f64()
                    .ok_or("input sample is not a number")?;
                let corrected = spektrafilm_math::precision::to_f64(output.data[i * 3 + r]);
                let residual = corrected - input_value;
                let oracle_residual_value = residual_channel[i]
                    .as_f64()
                    .ok_or("residual sample is not a number")?;
                let oracle_corrected_value = corrected_channel[i]
                    .as_f64()
                    .ok_or("corrected sample is not a number")?;
                let dr = (residual - oracle_residual_value).abs();
                let dc = (corrected - oracle_corrected_value).abs();
                max_residual = max_residual.max(dr);
                sum_residual += dr;
                max_corrected = max_corrected.max(dc);
                sum_corrected += dc;
                actual_residuals.push(residual);
                oracle_residuals.push(oracle_residual_value);
                corrected_identity_error = corrected_identity_error
                    .max((oracle_corrected_value - (input_value + oracle_residual_value)).abs());
            }
            let n = (width * height) as f64;
            let actual_mean = actual_residuals.iter().sum::<f64>() / n;
            let oracle_mean = oracle_residuals.iter().sum::<f64>() / n;
            let actual_sigma = (actual_residuals
                .iter()
                .map(|x| (x - actual_mean).powi(2))
                .sum::<f64>()
                / n)
                .sqrt();
            let oracle_sigma = (oracle_residuals
                .iter()
                .map(|x| (x - oracle_mean).powi(2))
                .sum::<f64>()
                / n)
                .sqrt();
            residual_statistics.push(json!({
                "record": r,
                "sigma": comparison(actual_sigma, oracle_sigma),
            }));
            residual_diffs.push(
                json!({"maxAbsoluteDifference": max_residual, "meanAbsoluteDifference": sum_residual / n}),
            );
            corrected_diffs.push(
                json!({"maxAbsoluteDifference": max_corrected, "meanAbsoluteDifference": sum_corrected / n}),
            );
        }
        readout.push(json!({
            "seed": seed,
            "pitchTexels": pitch_texels,
            "footprintMM": [pitch_texels * 0.001, pitch_texels * 0.001],
            "withinSourceCompletenessBound": pitch_texels <= 96.0,
            "residualRealization": {
                "perPixelDifference": residual_diffs,
                "correctedDifference": corrected_diffs,
                "oracleCorrectedEqualsInputPlusResidualMaxAbsError": corrected_identity_error,
                "statisticalBudget": residual_statistics,
            },
        }));
    }

    let anchor_expected: Vec<_> = (0..3)
        .map(|r| f64_at(&array(&anchored, "records")?[r], "measuredSigma48"))
        .collect::<Result<_, _>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "fixture": dir.file_name().and_then(|x| x.to_str()).unwrap_or("fixture"),
            "configuration": {
                "curveSource": "stock-input.json curves[r].sampled.logExposure/density",
                "sigma48": "grainStrength * grainLayerWeights[r]",
                "cloudRadiusMM": "stock grainSizeMM (fixture field is already millimetres)",
                "radiusSpan": "crystalGrainPopulation.radiusSpan",
                "sublayerShares": "crystalGrainPopulation.sublayerShares",
                "coatingDensityScale": "crystalGrainPopulation.coatingDensityScale",
                "layerSizeRatio": [1.0, 1.0, 1.0],
                "anchorNetDensity": [1.0, 1.0, 1.0],
                "fogDensity": 0.03,
                "look": {"amount": 1.0, "size": 1.0, "color": 1.0, "softness": 1.0, "layers": [1.0, 1.0, 1.0]},
                "geometry": "source/output pitch and footprint are pitchTexels * 0.001 mm; origin [0,0]",
                "seeds": [0u64, 42u64, 4294967295u64],
            },
            "preparation": preparation,
            "anchor": {
                "measured": diagnostics.anchor_measured.iter().enumerate().map(|(r, x)| json!({"record":r,"comparison":comparison(*x,anchor_expected[r])})).collect::<Vec<_>>(),
                "logError": diagnostics.anchor_log_error,
            },
            "pitch": pitch,
            "readout": readout,
            "approximations": [
                "Rust uses its complete periodic readout extension; all exported cases are within the source 96 micrometer per-axis completeness bound.",
                "The finite covariance-window 48 micrometer aperture model and authored effective parameterization remain declared design approximations.",
                "Pitch reproduction reports the bounded 128x128 sample geometry used by the Rust API; it does not claim bit identity.",
            ],
        }))?
    );
    Ok(())
}
