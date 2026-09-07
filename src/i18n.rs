use serde::{Deserialize, Serialize};

/// Persisted UI language. `System` resolves to a shipped locale.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiLang {
    #[default]
    System,
    English,
    Chinese,
}

impl UiLang {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Chinese];

    pub fn locale(self) -> &'static str {
        match self.resolved() {
            Self::System => unreachable!("system language always resolves to a shipped locale"),
            Self::English => "en",
            Self::Chinese => "zh-CN",
        }
    }

    pub fn resolved(self) -> Self {
        match self {
            Self::System => Self::from_system(),
            explicit => explicit,
        }
    }

    /// Autonym for the picker option. Independent of the current locale.
    pub fn picker_label(self) -> String {
        match self {
            Self::System => t("language.system"),
            Self::English => "English".into(),
            Self::Chinese => "简体中文".into(),
        }
    }

    fn from_system() -> Self {
        Self::from_locale_id(&system_locale())
    }

    fn from_locale_id(locale: &str) -> Self {
        let locale = locale
            .split(['.', '@'])
            .next()
            .unwrap_or(locale)
            .replace('_', "-")
            .to_ascii_lowercase();
        if locale == "zh-cn" || locale == "zh-sg" || locale.starts_with("zh-hans") || locale == "zh"
        {
            Self::Chinese
        } else {
            Self::English
        }
    }
}

pub fn set_language(language: UiLang) {
    rust_i18n::set_locale(language.locale());
}

pub fn t(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

#[cfg(target_os = "windows")]
fn system_locale() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        "en".into()
    }
}

#[cfg(not(target_os = "windows"))]
fn system_locale() -> String {
    first_locale(["LC_ALL", "LC_MESSAGES", "LANG"].map(|key| std::env::var(key).ok()))
}

#[cfg(not(target_os = "windows"))]
fn first_locale(values: [Option<String>; 3]) -> String {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "en".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_are_shipped() {
        let locales = rust_i18n::available_locales!();
        assert!(locales.iter().any(|locale| locale.as_ref() == "en"));
        assert!(locales.iter().any(|locale| locale.as_ref() == "zh-CN"));
    }

    #[test]
    fn language_locale_ids() {
        assert_eq!(UiLang::English.locale(), "en");
        assert_eq!(UiLang::Chinese.locale(), "zh-CN");
        assert!(matches!(UiLang::System.locale(), "en" | "zh-CN"));
    }

    #[test]
    fn picker_autonyms_ignore_current_locale() {
        assert_eq!(UiLang::English.picker_label(), "English");
        assert_eq!(UiLang::Chinese.picker_label(), "简体中文");
    }

    #[test]
    fn system_is_default_and_resolves() {
        assert_eq!(UiLang::default(), UiLang::System);
        assert_eq!(
            serde_json::to_string(&UiLang::System).unwrap(),
            r#""system""#
        );
        assert!(matches!(
            UiLang::System.resolved(),
            UiLang::English | UiLang::Chinese
        ));
    }

    #[test]
    fn simplified_chinese_system_locales() {
        assert_eq!(UiLang::from_locale_id("zh-CN"), UiLang::Chinese);
        assert_eq!(UiLang::from_locale_id("zh_SG"), UiLang::Chinese);
        assert_eq!(UiLang::from_locale_id("zh-Hans-CN"), UiLang::Chinese);
        assert_eq!(UiLang::from_locale_id("zh-Hant-TW"), UiLang::English);
        assert_eq!(UiLang::from_locale_id("en-US"), UiLang::English);
    }

    #[test]
    fn appearance_copy_round_trips() {
        assert_eq!(
            &*rust_i18n::t!("settings.appearance", locale = "en"),
            "Appearance"
        );
        assert_eq!(
            &*rust_i18n::t!("settings.appearance", locale = "zh-CN"),
            "外观"
        );
        assert_eq!(
            &*rust_i18n::t!("settings.language", locale = "zh-CN"),
            "语言"
        );
    }

    #[test]
    fn catalogs_have_matching_keys_and_parameters() {
        let catalog = include_str!("../locales/app.yml");
        let placeholders = regex::Regex::new(r"%\{([^}]+)\}").unwrap();
        for key in catalog.lines().filter_map(|line| {
            (!line.starts_with(char::is_whitespace) && !line.starts_with('_'))
                .then(|| line.strip_suffix(':'))
                .flatten()
        }) {
            let en = crate::_rust_i18n_backend()
                .translate("en", key)
                .unwrap_or_else(|| panic!("missing en: {key}"));
            let zh = crate::_rust_i18n_backend()
                .translate("zh-CN", key)
                .unwrap_or_else(|| panic!("missing zh: {key}"));
            assert!(!en.trim().is_empty(), "empty en: {key}");
            assert!(!zh.trim().is_empty(), "empty zh: {key}");
            let params = |text: &str| {
                placeholders
                    .captures_iter(text)
                    .map(|capture| capture[1].to_owned())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            assert_eq!(params(&en), params(&zh), "placeholder mismatch: {key}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn empty_locale_overrides_fall_through() {
        assert_eq!(
            first_locale([
                Some("".into()),
                Some("zh_CN.UTF-8".into()),
                Some("en_US".into())
            ]),
            "zh_CN.UTF-8"
        );
        assert_eq!(
            first_locale([Some("C".into()), None, Some("zh_CN".into())]),
            "C"
        );
        assert_eq!(first_locale([None, Some("".into()), None]), "en");
        assert_eq!(
            UiLang::from_locale_id("zh_CN.UTF-8@variant"),
            UiLang::Chinese
        );
    }
}
