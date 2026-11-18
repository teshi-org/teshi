//! UI locale preference and resolution for the TUI.
//!
//! The persisted preference is `system` / `en` / `zh-CN`. Resolution produces an
//! effective locale that both the TUI catalogs and Feature-directory selection
//! share. Gherkin dialect keywords inside `.feature` files are unrelated.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Environment override for the UI locale (`system` / `en` / `zh-CN` and aliases).
pub const TESHI_LOCALE_ENV: &str = "TESHI_LOCALE";

/// Environment override for Feature-directory language only (`zh-CN`, `en-US`, …).
pub const TESHI_FEATURE_LANGUAGE_ENV: &str = "TESHI_FEATURE_LANGUAGE";

/// Persisted UI locale preference stored in [`crate::AppSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiLocalePreference {
    /// Follow the operating-system UI language.
    #[default]
    #[serde(alias = "System")]
    System,
    /// English UI and `features/en-US` when present.
    #[serde(alias = "en-US", alias = "en_US", alias = "English")]
    En,
    /// Simplified Chinese UI and `features/zh-CN` when present.
    #[serde(rename = "zh-CN", alias = "zh", alias = "zh_CN", alias = "zh-Hans")]
    ZhCn,
}

impl UiLocalePreference {
    /// Stable settings / display tokens, in cycle order.
    pub const ALL: [Self; 3] = [Self::System, Self::En, Self::ZhCn];

    /// Wire value written to `settings.json` and accepted by `TESHI_LOCALE`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parse a user or environment token; unknown values yield [`None`].
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().replace('_', "-");
        let folded = normalized.to_ascii_lowercase();
        match folded.as_str() {
            "system" => Some(Self::System),
            "en" | "en-us" | "english" => Some(Self::En),
            "zh" | "zh-cn" | "zh-hans" | "zh-sg" | "zh-chs" => Some(Self::ZhCn),
            _ if folded.starts_with("en-") || folded.starts_with("english-") => Some(Self::En),
            _ if folded.starts_with("zh-cn")
                || folded.starts_with("zh-hans")
                || folded.starts_with("zh-sg")
                || folded.starts_with("zh-chs") =>
            {
                Some(Self::ZhCn)
            }
            _ => None,
        }
    }

    /// Next preference in the Settings picker cycle.
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::En,
            Self::En => Self::ZhCn,
            Self::ZhCn => Self::System,
        }
    }
}

impl fmt::Display for UiLocalePreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Locale after resolving preference, environment, and the host OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveUiLocale {
    /// English catalogs and `features/en-US`.
    En,
    /// Simplified Chinese catalogs and `features/zh-CN`.
    ZhCn,
}

impl EffectiveUiLocale {
    /// BCP 47 tag used for Feature language directories.
    pub fn feature_language_tag(self) -> &'static str {
        match self {
            Self::En => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Catalog / settings token (`en` or `zh-CN`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }
}

/// Map a system or free-form locale name onto an effective UI locale.
///
/// Unsupported languages fall back to English, matching iCode's quiet fallback.
pub fn effective_from_system_locale(raw: Option<&str>) -> EffectiveUiLocale {
    match raw.and_then(UiLocalePreference::parse) {
        Some(UiLocalePreference::ZhCn) => EffectiveUiLocale::ZhCn,
        Some(UiLocalePreference::En) | Some(UiLocalePreference::System) | None => {
            EffectiveUiLocale::En
        }
    }
}

/// Resolve the effective UI locale.
///
/// Order: non-empty `TESHI_LOCALE` → `preference` → `system_locale` (via `system`).
pub fn resolve_effective_ui_locale(
    preference: UiLocalePreference,
    system_locale: Option<&str>,
) -> EffectiveUiLocale {
    if let Ok(value) = std::env::var(TESHI_LOCALE_ENV) {
        if let Some(parsed) = UiLocalePreference::parse(&value) {
            return match parsed {
                UiLocalePreference::System => effective_from_system_locale(system_locale),
                UiLocalePreference::En => EffectiveUiLocale::En,
                UiLocalePreference::ZhCn => EffectiveUiLocale::ZhCn,
            };
        }
    }
    match preference {
        UiLocalePreference::System => effective_from_system_locale(system_locale),
        UiLocalePreference::En => EffectiveUiLocale::En,
        UiLocalePreference::ZhCn => EffectiveUiLocale::ZhCn,
    }
}

/// Locale tag for Feature-directory selection.
///
/// Order: non-empty `TESHI_FEATURE_LANGUAGE` → effective UI locale's feature tag.
pub fn resolve_feature_discovery_locale(
    preference: UiLocalePreference,
    system_locale: Option<&str>,
) -> String {
    if let Ok(value) = std::env::var(TESHI_FEATURE_LANGUAGE_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.replace('_', "-");
        }
    }
    resolve_effective_ui_locale(preference, system_locale)
        .feature_language_tag()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_parse_accepts_common_aliases() {
        assert_eq!(
            UiLocalePreference::parse("system"),
            Some(UiLocalePreference::System)
        );
        assert_eq!(
            UiLocalePreference::parse("en"),
            Some(UiLocalePreference::En)
        );
        assert_eq!(
            UiLocalePreference::parse("en_US"),
            Some(UiLocalePreference::En)
        );
        assert_eq!(
            UiLocalePreference::parse("zh-CN"),
            Some(UiLocalePreference::ZhCn)
        );
        assert_eq!(
            UiLocalePreference::parse("zh_Hans"),
            Some(UiLocalePreference::ZhCn)
        );
        assert_eq!(UiLocalePreference::parse("fr-FR"), None);
    }

    #[test]
    fn preference_serde_uses_stable_tokens() {
        assert_eq!(
            serde_json::to_string(&UiLocalePreference::ZhCn).unwrap(),
            "\"zh-CN\""
        );
        assert_eq!(
            serde_json::from_str::<UiLocalePreference>("\"zh-CN\"").unwrap(),
            UiLocalePreference::ZhCn
        );
        assert_eq!(
            serde_json::from_str::<UiLocalePreference>("\"system\"").unwrap(),
            UiLocalePreference::System
        );
    }

    #[test]
    fn system_chinese_maps_to_zh_cn_features() {
        assert_eq!(
            effective_from_system_locale(Some("zh-CN")),
            EffectiveUiLocale::ZhCn
        );
        assert_eq!(
            effective_from_system_locale(Some("zh_CN.UTF-8")),
            EffectiveUiLocale::ZhCn
        );
        assert_eq!(
            effective_from_system_locale(Some("fr-FR")),
            EffectiveUiLocale::En
        );
    }

    #[test]
    fn feature_tag_follows_effective_locale() {
        assert_eq!(EffectiveUiLocale::En.feature_language_tag(), "en-US");
        assert_eq!(EffectiveUiLocale::ZhCn.feature_language_tag(), "zh-CN");
    }

    fn with_locale_envs<T>(pairs: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = crate::app_data::test_env_lock();
        let previous: Vec<_> = pairs
            .iter()
            .map(|(key, value)| {
                let prior = std::env::var_os(key);
                match value {
                    Some(v) => unsafe { std::env::set_var(key, v) },
                    None => unsafe { std::env::remove_var(key) },
                }
                (*key, prior)
            })
            .collect();
        let result = f();
        for (key, prior) in previous {
            match prior {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        result
    }

    #[test]
    fn teshi_locale_env_overrides_saved_preference() {
        with_locale_envs(&[(TESHI_LOCALE_ENV, Some("zh-CN"))], || {
            assert_eq!(
                resolve_effective_ui_locale(UiLocalePreference::En, Some("en-US")),
                EffectiveUiLocale::ZhCn
            );
        });
        with_locale_envs(&[(TESHI_LOCALE_ENV, Some("en"))], || {
            assert_eq!(
                resolve_effective_ui_locale(UiLocalePreference::ZhCn, Some("zh-CN")),
                EffectiveUiLocale::En
            );
        });
        // Unknown tokens fall through to the saved preference.
        with_locale_envs(&[(TESHI_LOCALE_ENV, Some("fr-FR"))], || {
            assert_eq!(
                resolve_effective_ui_locale(UiLocalePreference::ZhCn, Some("en-US")),
                EffectiveUiLocale::ZhCn
            );
        });
    }

    #[test]
    fn feature_language_env_overrides_ui_locale_tag() {
        with_locale_envs(&[(TESHI_FEATURE_LANGUAGE_ENV, Some("ja-JP"))], || {
            assert_eq!(
                resolve_feature_discovery_locale(UiLocalePreference::En, Some("en-US")),
                "ja-JP"
            );
        });
        with_locale_envs(
            &[
                (TESHI_FEATURE_LANGUAGE_ENV, None),
                (TESHI_LOCALE_ENV, Some("zh-CN")),
            ],
            || {
                assert_eq!(
                    resolve_feature_discovery_locale(UiLocalePreference::En, Some("en-US")),
                    "zh-CN"
                );
            },
        );
    }
}
