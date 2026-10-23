//! Named positive-conversion styles for the negative-to-positive
//! interpretation.
//!
//! A style is declared data, never a fitted surface: `neutral` is the
//! frozen reference tone (the historical fitted constants the retired
//! conversion controls once parameterized), and every other name resolves
//! to `data/scan-styles/<name>.json` carrying one tone triple per channel
//! of the same logistic family. Per-channel parameters are what lets a
//! style express a colour tendency on top of the tone.

use std::path::Path;

/// One channel's positive tone parameters — the logistic density curve the
/// record-density reading passes through (see `positive_tone`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelTone {
    /// Logistic midpoint in camera EV.
    pub brightness_ev: f64,
    /// Logistic slope.
    pub contrast: f64,
    /// Maximum density the record can reach.
    pub dmax: f64,
}

/// The frozen reference tone: the historical fitted defaults of the retired
/// `conversion_brightness_ev` / `conversion_contrast` / `conversion_dmax`
/// controls, byte-identical so `conversion_style = neutral` renders exactly
/// as before the style slot existed.
pub(crate) const NEUTRAL: [ChannelTone; 3] = [ChannelTone {
    brightness_ev: 0.8781766618,
    contrast: 0.73551400058216643,
    dmax: 1.5625841465572565,
}; 3];

/// `neutral` needs no data directory; any other style is a JSON file in
/// `scan-styles/` next to the profiles.
pub(crate) fn resolve(name: &str, data_dir: Option<&Path>) -> Result<[ChannelTone; 3], String> {
    if name == "neutral" {
        return Ok(NEUTRAL);
    }
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains(':')
    {
        return Err(format!(
            "scanner.conversion_style {name:?} is not a valid style name"
        ));
    }
    let Some(dir) = data_dir else {
        return Err(
            "scanner.conversion_style requires a pipeline built from a data directory; only \"neutral\" is available here"
                .into(),
        );
    };
    let path = dir.join("scan-styles").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "scanner.conversion_style {:?} cannot be read ({}): {error}; available styles: {}",
            name,
            path.display(),
            available(data_dir).join(", ")
        )
    })?;
    parse(&text, name)
}

/// Style names available under a data directory (`neutral` always, then the
/// `scan-styles/*.json` stems) — for error messages and pickers.
pub(crate) fn available(data_dir: Option<&Path>) -> Vec<String> {
    let mut names = vec!["neutral".to_owned()];
    if let Some(dir) = data_dir
        && let Ok(entries) = std::fs::read_dir(dir.join("scan-styles"))
    {
        let mut extra: Vec<_> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_owned)
                } else {
                    None
                }
            })
            .collect();
        extra.sort();
        names.extend(extra);
    }
    names
}

fn parse(text: &str, name: &str) -> Result<[ChannelTone; 3], String> {
    #[derive(serde::Deserialize)]
    struct StyleFile {
        channels: [ChannelToneFile; 3],
    }
    #[derive(serde::Deserialize)]
    struct ChannelToneFile {
        brightness_ev: f64,
        contrast: f64,
        dmax: f64,
    }
    let parsed: StyleFile = serde_json::from_str(text)
        .map_err(|error| format!("scan style {name:?} is not valid JSON: {error}"))?;
    let tones = parsed.channels.map(|channel| ChannelTone {
        brightness_ev: channel.brightness_ev,
        contrast: channel.contrast,
        dmax: channel.dmax,
    });
    for (index, tone) in tones.iter().enumerate() {
        if !tone.brightness_ev.is_finite()
            || !tone.contrast.is_finite()
            || !tone.dmax.is_finite()
            || tone.contrast <= 0.0
            || tone.dmax <= 0.0
        {
            return Err(format!(
                "scan style {name:?} channel {index} needs finite values with contrast > 0 and dmax > 0"
            ));
        }
    }
    Ok(tones)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_the_frozen_reference_and_needs_no_data_dir() {
        assert_eq!(resolve("neutral", None).unwrap(), NEUTRAL);
        assert_eq!(NEUTRAL[0].brightness_ev, 0.8781766618);
        assert_eq!(NEUTRAL[0].contrast, 0.73551400058216643);
        assert_eq!(NEUTRAL[0].dmax, 1.5625841465572565);
    }

    #[test]
    fn non_neutral_names_reject_traversal_and_missing_directories() {
        assert!(resolve("../etc/passwd", None).is_err());
        assert!(resolve("a/b", None).is_err());
        let error = resolve("missing_style", None).unwrap_err();
        assert!(error.contains("neutral"), "{error}");
    }

    #[test]
    fn user_authored_data_dir_styles_parse_and_validate() {
        let dir = tempfile::tempdir().unwrap();
        let styles = dir.path().join("scan-styles");
        std::fs::create_dir(&styles).unwrap();
        std::fs::write(
            styles.join("warm-probe.json"),
            r#"{"channels":[
                {"brightness_ev":0.9281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},
                {"brightness_ev":0.8781766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},
                {"brightness_ev":0.8281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565}
            ]}"#,
        )
        .unwrap();
        let tones = resolve("warm-probe", Some(dir.path())).unwrap();
        assert_ne!(tones[0], tones[2]);
        assert_eq!(
            available(Some(dir.path())),
            vec!["neutral".to_owned(), "warm-probe".to_owned()]
        );
        assert_eq!(
            available(Some(&crate::test_support::data_dir())),
            vec!["neutral".to_owned()]
        );

        let bad = r#"{"channels":[{"brightness_ev":0,"contrast":0,"dmax":1},{"brightness_ev":0,"contrast":1,"dmax":1},{"brightness_ev":0,"contrast":1,"dmax":1}]}"#;
        assert!(parse(bad, "bad").is_err());
    }
}
