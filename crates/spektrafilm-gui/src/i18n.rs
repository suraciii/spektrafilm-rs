//! GUI-only language, embedded resources and independent application preference.
use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};

use egui::Context;
use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/catalog_diagnostics.rs"));

pub(crate) fn report_catalog_fallbacks() {
    static REPORTED: std::sync::Once = std::sync::Once::new();
    REPORTED.call_once(|| {
        for (locale, key) in MISSING_TRANSLATIONS {
            tracing::warn!(
                locale,
                key,
                "GUI translation missing; English fallback may be used"
            );
        }
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Locale {
    #[default]
    English,
    SimplifiedChinese,
}

impl Locale {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    /// Context data, not rust-i18n's process-global locale, owns presentation.
    pub(crate) fn install(self, ctx: &Context) {
        report_catalog_fallbacks();
        ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(locale_id(), || self) = self);
        let fonts_id = egui::Id::new("spektrafilm.cjk_fonts_installed");
        let installed = ctx.data(|data| data.get_temp::<bool>(fonts_id).unwrap_or(false));
        if !installed {
            let mut fonts = egui::FontDefinitions::default();
            let name = "Noto Sans SC".to_owned();
            fonts.font_data.insert(
                name.clone(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/NotoSansSC-Regular.otf"
                ))),
            );
            // Retain Latin/monospace primaries; cover Chinese in every family.
            for family in fonts.families.values_mut() {
                family.push(name.clone());
            }
            ctx.set_fonts(fonts);
            ctx.data_mut(|data| data.insert_temp(fonts_id, true));
        }
    }
}

fn locale_id() -> egui::Id {
    egui::Id::new("spektrafilm.interface_locale")
}

pub(crate) fn locale(ctx: &Context) -> Locale {
    ctx.data(|data| data.get_temp::<Locale>(locale_id()).unwrap_or_default())
}

pub(crate) fn text(ctx: &Context, key: &str) -> Cow<'static, str> {
    text_for(locale(ctx), key)
}

pub(crate) fn text_for(locale: Locale, key: &str) -> Cow<'static, str> {
    report_catalog_fallbacks();
    // The generated backend returns embedded catalog borrows independently of
    // the lookup key's lifetime. Missing dynamic keys must instead be owned.
    crate::_rust_i18n_try_translate(locale.as_str(), key).unwrap_or_else(|| {
        log::warn!(target: "rust-i18n", "missing translation: {} / {}", locale.as_str(), key);
        Cow::Owned(key.to_owned())
    })
}

/// Named interpolation stays in rust-i18n; every call supplies its context locale.
#[macro_export]
macro_rules! tr {
    ($ctx:expr, $key:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        $crate::i18n::report_catalog_fallbacks();
        rust_i18n::t!($key, locale = $crate::i18n::locale($ctx).as_str() $(, $name = $value)*)
    }};
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LanguagePreference {
    #[default]
    System,
    English,
    SimplifiedChinese,
}

impl LanguagePreference {
    pub(crate) const ALL: [Self; 3] = [Self::System, Self::SimplifiedChinese, Self::English];

    /// These labels deliberately remain recognizable in either presentation.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统 / System",
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
        }
    }

    pub(crate) fn resolve(self) -> Locale {
        match self {
            Self::System => resolve_system_languages(sys_locale::get_locales()),
            Self::English => Locale::English,
            Self::SimplifiedChinese => Locale::SimplifiedChinese,
        }
    }
}

/// Match ordered OS preferences, honoring explicit scripts before regions.
/// Unqualified Chinese deliberately does not imply Simplified Chinese.
pub(crate) fn resolve_system_languages<I, S>(languages: I) -> Locale
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for language in languages {
        let tag = language.as_ref().trim();
        let tag = tag.split(['.', '@']).next().unwrap_or("");
        let mut parts = tag.split(['-', '_']);
        let language = parts.next().unwrap_or("");
        if language.eq_ignore_ascii_case("en") {
            return Locale::English;
        }
        if !language.eq_ignore_ascii_case("zh") {
            continue;
        }
        let mut script = None;
        let mut region = None;
        for part in parts {
            // BCP 47 extensions/private-use cannot supply language/script/region.
            if part.len() == 1 {
                break;
            }
            if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                script = Some(part);
            } else if part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                region = Some(part);
            }
        }
        if let Some(script) = script {
            if script.eq_ignore_ascii_case("Hans") {
                return Locale::SimplifiedChinese;
            }
        } else if region.is_some_and(|region| {
            region.eq_ignore_ascii_case("CN") || region.eq_ignore_ascii_case("SG")
        }) {
            return Locale::SimplifiedChinese;
        }
    }
    Locale::English
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    language: LanguagePreference,
}

#[derive(Clone, Debug)]
pub(crate) struct PreferenceError {
    pub(crate) path: PathBuf,
    pub(crate) detail: String,
    saving: bool,
}

impl PreferenceError {
    pub(crate) fn localized(&self, ctx: &Context) -> String {
        let key = if self.saving {
            "app.language.save_failed"
        } else {
            "app.language.load_failed"
        };
        crate::tr!(ctx, key, path = self.path.display(), detail = &self.detail).into_owned()
    }
}

pub(crate) fn preference_path(directory: &Path) -> PathBuf {
    directory.join("gui_preferences.json")
}

/// Reading never modifies malformed/unreadable data or prevents application startup.
pub(crate) fn load_preference(directory: &Path) -> (LanguagePreference, Option<PreferenceError>) {
    let path = preference_path(directory);
    let result = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes)
            .map(|preferences| preferences.language)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (LanguagePreference::System, None);
        }
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(preference) => (preference, None),
        Err(detail) => (
            LanguagePreference::System,
            Some(PreferenceError {
                path,
                detail,
                saving: false,
            }),
        ),
    }
}

/// Publish a complete sibling temporary file; a failed write keeps the old file.
/// Existing safe writers belong to image/report/preset contracts and are private.
pub(crate) fn save_preference(
    directory: &Path,
    language: LanguagePreference,
) -> Result<(), PreferenceError> {
    let path = preference_path(directory);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(directory)?;
        let bytes = serde_json::to_vec_pretty(&Preferences { language })?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".gui-preferences-")
            .suffix(".tmp")
            .tempfile_in(directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path)?;
        Ok(())
    };
    write().map_err(|error| PreferenceError {
        path,
        detail: error.to_string(),
        saving: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogs() -> [std::collections::BTreeMap<String, String>; 2] {
        [
            serde_json::from_str(include_str!("../locales/en.json")).unwrap(),
            serde_json::from_str(include_str!("../locales/zh-CN.json")).unwrap(),
        ]
    }

    #[test]
    fn shipping_catalogs_have_identical_keys_and_named_arguments() {
        fn placeholders<'a>(key: &str, text: &'a str) -> std::collections::BTreeSet<&'a str> {
            text.split("%{")
                .skip(1)
                .map(|part| {
                    let (name, _) = part
                        .split_once('}')
                        .unwrap_or_else(|| panic!("unclosed placeholder in {key}: {text}"));
                    assert!(!name.is_empty(), "empty placeholder in {key}");
                    name
                })
                .collect()
        }
        let [english, chinese] = catalogs();
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            chinese.keys().collect::<Vec<_>>(),
            "shipping catalogs must not depend on English fallback",
        );
        for (key, source) in &english {
            let translated = &chinese[key];
            assert!(!source.trim().is_empty(), "empty English message: {key}");
            assert!(
                !translated.trim().is_empty(),
                "empty Chinese message: {key}"
            );
            let names = placeholders(key, source);
            assert_eq!(
                names,
                placeholders(key, translated),
                "argument mismatch: {key}"
            );
            let names: Vec<_> = names.into_iter().collect();
            let values = vec!["argument".to_owned(); names.len()];
            for text in [source, translated] {
                let rendered = rust_i18n::replace_patterns(text, &names, &values);
                assert!(!rendered.contains("%{"), "unformatted argument in {key}");
            }
        }
    }

    #[test]
    fn bundled_fonts_cover_shipped_chinese_in_every_text_family() {
        let [_, chinese] = catalogs();
        let characters: std::collections::BTreeSet<_> = chinese
            .values()
            .flat_map(|text| text.chars())
            .chain("中华人民共和国简体中文输入法测试照片胶片印相扫描导出设置通用语言".chars())
            .filter(|character| {
                matches!(*character as u32,
                0x3000..=0x303f | 0x3400..=0x4dbf | 0x4e00..=0x9fff |
                0xf900..=0xfaff | 0xff00..=0xffef | 0x20000..=0x323af)
            })
            .collect();
        assert!(!characters.is_empty());
        let ctx = Context::default();
        Locale::SimplifiedChinese.install(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                for family in fonts.families() {
                    let font = egui::FontId::new(14.0, family);
                    for character in &characters {
                        assert!(
                            fonts.has_glyph(&font, *character),
                            "missing Chinese glyph {character:?} (U+{:04X}) in {:?}",
                            *character as u32,
                            font.family
                        );
                    }
                }
            });
        });
    }

    #[test]
    fn ordered_resolution_respects_scripts_and_falls_through() {
        for (preferences, expected) in [
            (vec!["zh-Hans"], Locale::SimplifiedChinese),
            (vec!["ZH_hAnS_TW"], Locale::SimplifiedChinese),
            (vec!["zh_CN.UTF-8"], Locale::SimplifiedChinese),
            (vec!["zh-SG"], Locale::SimplifiedChinese),
            (vec!["zh-Hant-CN", "en-GB"], Locale::English),
            (vec!["zh-Hant", "zh-Hans-SG"], Locale::SimplifiedChinese),
            (vec!["zh-TW", "fr-FR", "zh-CN"], Locale::SimplifiedChinese),
            (vec!["zh", "en", "zh-CN"], Locale::English),
            (vec!["zh-Latn-CN"], Locale::English),
            (vec!["zh-x-CN"], Locale::English),
            (vec!["zh", "zh-HK", "fr-FR"], Locale::English),
            (vec![], Locale::English),
        ] {
            assert_eq!(resolve_system_languages(preferences), expected);
        }
    }

    #[test]
    fn contexts_do_not_share_language() {
        let english = Context::default();
        let chinese = Context::default();
        Locale::English.install(&english);
        Locale::SimplifiedChinese.install(&chinese);
        assert_eq!(
            text(&english, "app.language.help"),
            text_for(Locale::English, "app.language.help")
        );
        assert_eq!(
            text(&chinese, "app.language.help"),
            text_for(Locale::SimplifiedChinese, "app.language.help")
        );
        assert_ne!(
            text(&english, "app.language.help"),
            text(&chinese, "app.language.help")
        );
    }

    #[test]
    fn missing_invalid_and_unreadable_preferences_use_system_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = preference_path(directory.path());
        assert_eq!(
            load_preference(directory.path()).0,
            LanguagePreference::System
        );
        assert!(!path.exists());
        for bytes in [
            b"not json".as_slice(),
            b"{}",
            b"{\"language\":\"zh\"}",
            b"{\"language\":\"english\",\"extra\":true}",
        ] {
            std::fs::write(&path, bytes).unwrap();
            let (preference, error) = load_preference(directory.path());
            assert_eq!(preference, LanguagePreference::System);
            assert!(error.is_some());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(load_preference(directory.path()).1.is_some());
    }

    #[test]
    fn independent_preference_round_trips_including_system() {
        let directory = tempfile::tempdir().unwrap();
        for preference in LanguagePreference::ALL {
            save_preference(directory.path(), preference).unwrap();
            let (restored, error) = load_preference(directory.path());
            assert_eq!(restored, preference);
            assert!(error.is_none());
        }
    }

    #[test]
    fn publication_failure_retains_destination_and_removes_temporary() {
        let directory = tempfile::tempdir().unwrap();
        let destination = preference_path(directory.path());
        std::fs::create_dir(&destination).unwrap();
        let retained = destination.join("retained");
        std::fs::write(&retained, b"last valid data").unwrap();
        assert!(save_preference(directory.path(), LanguagePreference::English).is_err());
        assert_eq!(std::fs::read(retained).unwrap(), b"last valid data");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
