// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use icu_locid::extensions::unicode::{key, Key};
use icu_locid::subtags::Language;
use icu_locid::LanguageIdentifier;
use icu_provider::FallbackPriority;

use super::*;

const SUBDIVISION_KEY: Key = key!("sd");

impl<'a> LocaleFallbackerWithConfig<'a> {
    pub(crate) fn normalize(&self, locale: &mut DataLocale) {
        let language = locale.language();
        // 1. Populate the region (required for region fallback only)
        if self.config.priority == FallbackPriority::Region && locale.region().is_none() {
            // 1a. First look for region based on language+script
            if let Some(script) = locale.script() {
                locale.set_region(
                    self.likely_subtags
                        .ls2r
                        .get_2d(
                            &language.into_tinystr().to_unvalidated(),
                            &script.into_tinystr().to_unvalidated(),
                        )
                        .copied(),
                );
            }
            // 1b. If that fails, try language only
            if locale.region().is_none() {
                locale.set_region(
                    self.likely_subtags
                        .l2r
                        .get(&language.into_tinystr().to_unvalidated())
                        .copied(),
                );
            }
        }
        // 2. Remove the script if it is implied by the other subtags
        if let Some(script) = locale.script() {
            let default_script = self
                .likely_subtags
                .l2s
                .get_copied(&language.into_tinystr().to_unvalidated())
                .unwrap_or(DEFAULT_SCRIPT);
            if let Some(region) = locale.region() {
                if script
                    == self
                        .likely_subtags
                        .lr2s
                        .get_copied_2d(
                            &language.into_tinystr().to_unvalidated(),
                            &region.into_tinystr().to_unvalidated(),
                        )
                        .unwrap_or(default_script)
                {
                    locale.set_script(None);
                }
            } else if script == default_script {
                locale.set_script(None);
            }
        }
        // 3. Remove irrelevant extension subtags
        locale.retain_unicode_ext(|key| {
            match *key {
                // Always retain -u-sd
                SUBDIVISION_KEY => true,
                // Retain the query-specific keyword
                _ if Some(*key) == self.config.extension_key => true,
                // Drop all others
                _ => false,
            }
        });
        // 4. If there is an invalid "sd" subtag, drop it
        // For now, ignore it, and let fallback do it for us
    }
}

impl<'a> LocaleFallbackIteratorInner<'a> {
    pub fn step(&mut self, locale: &mut DataLocale) {
        match self.config.priority {
            FallbackPriority::Language => self.step_language(locale),
            FallbackPriority::Region => self.step_region(locale),
            // TODO(#1964): Change the collation fallback rules to be different
            // from the language fallback fules.
            FallbackPriority::Collation => self.step_language(locale),
            // This case should not normally happen, but `FallbackPriority` is non_exhaustive.
            // Make it go directly to `und`.
            _ => {
                debug_assert!(
                    false,
                    "Unknown FallbackPriority: {:?}",
                    self.config.priority
                );
                *locale = Default::default()
            }
        }
    }

    fn step_language(&mut self, locale: &mut DataLocale) {
        // 1. Remove the extension fallback keyword
        if let Some(extension_key) = self.config.extension_key {
            if let Some(value) = locale.remove_unicode_ext(&extension_key) {
                self.backup_extension = Some(value);
                return;
            }
        }
        // 2. Remove the subdivision keyword
        if let Some(value) = locale.remove_unicode_ext(&SUBDIVISION_KEY) {
            self.backup_subdivision = Some(value);
            return;
        }
        // 3. Assert that the locale is a language identifier
        debug_assert!(!locale.has_unicode_ext());
        // 4. Remove variants
        if locale.has_variants() {
            self.backup_variants = Some(locale.clear_variants());
            return;
        }
        // 5. Check for parent override
        if let Some(parent) = self.get_explicit_parent(locale) {
            locale.set_langid(parent);
            self.restore_extensions_variants(locale);
            return;
        }
        // 6. Add the script subtag if necessary
        if locale.script().is_none() {
            if let Some(region) = locale.region() {
                let language = locale.language();
                if let Some(script) = self.likely_subtags.lr2s.get_copied_2d(
                    &language.into_tinystr().to_unvalidated(),
                    &region.into_tinystr().to_unvalidated(),
                ) {
                    locale.set_script(Some(script));
                    self.restore_extensions_variants(locale);
                    return;
                }
            }
        }
        // 7. Remove region
        if locale.region().is_some() {
            locale.set_region(None);
            self.restore_extensions_variants(locale);
            return;
        }
        // 8. Remove language+script
        debug_assert!(!locale.language().is_empty()); // don't call .step() on und
        locale.set_script(None);
        locale.set_language(Language::UND);
    }

    fn step_region(&mut self, locale: &mut DataLocale) {
        // 1. Remove the extension fallback keyword
        if let Some(extension_key) = self.config.extension_key {
            if let Some(value) = locale.remove_unicode_ext(&extension_key) {
                self.backup_extension = Some(value);
                return;
            }
        }
        // 2. Remove the subdivision keyword
        if let Some(value) = locale.remove_unicode_ext(&SUBDIVISION_KEY) {
            self.backup_subdivision = Some(value);
            return;
        }
        // 3. Assert that the locale is a language identifier
        debug_assert!(!locale.has_unicode_ext());
        // 4. Remove variants
        if locale.has_variants() {
            self.backup_variants = Some(locale.clear_variants());
            return;
        }
        // 5. Remove language+script
        if !locale.language().is_empty() || locale.script().is_some() {
            locale.set_script(None);
            locale.set_language(Language::UND);
            self.restore_extensions_variants(locale);
            return;
        }
        // 6. Remove region
        debug_assert!(locale.region().is_some()); // don't call .step() on und
        locale.set_region(None);
    }

    fn restore_extensions_variants(&mut self, locale: &mut DataLocale) {
        if let Some(value) = self.backup_extension.take() {
            #[allow(clippy::unwrap_used)] // not reachable unless extension_key is present
            locale.set_unicode_ext(self.config.extension_key.unwrap(), value);
        }
        if let Some(value) = self.backup_subdivision.take() {
            locale.set_unicode_ext(SUBDIVISION_KEY, value);
        }
        if let Some(variants) = self.backup_variants.take() {
            locale.set_variants(variants);
        }
    }

    fn get_explicit_parent(&self, locale: &DataLocale) -> Option<LanguageIdentifier> {
        self.supplement
            .and_then(|supplement| {
                supplement
                    .parents
                    .get_copied_by(|uvstr| locale.strict_cmp(uvstr).reverse())
            })
            .or_else(|| {
                self.parents
                    .parents
                    .get_copied_by(|uvstr| locale.strict_cmp(uvstr).reverse())
            })
            .map(LanguageIdentifier::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writeable::Writeable;

    /// Unicode extension keywords take part in fallback, but [auxiliary keys] are not modified.
    ///
    /// [auxiliary keys]: icu_provider::AuxiliaryKeys
    #[test]
    fn test_aux_key_fallback() {
        use super::LocaleFallbacker;

        let fallbacker = LocaleFallbacker::new();
        let mut fallback_iterator = fallbacker
            .for_config(Default::default())
            .fallback_for("en-US-u-sd-usca-x-aux".parse().unwrap());

        assert_eq!(fallback_iterator.get().to_string(), "en-US-u-sd-usca-x-aux");
        fallback_iterator.step();
        assert_eq!(fallback_iterator.get().to_string(), "en-US-x-aux");
        fallback_iterator.step();
        assert_eq!(fallback_iterator.get().to_string(), "en-u-sd-usca-x-aux");
        fallback_iterator.step();
        assert_eq!(fallback_iterator.get().to_string(), "en-x-aux");
        fallback_iterator.step();
        assert_eq!(fallback_iterator.get().to_string(), "und-x-aux");
        assert!(fallback_iterator.get().is_und());
    }

    struct TestCase {
        input: &'static str,
        requires_data: bool,
        extension_key: Option<Key>,
        fallback_supplement: Option<LocaleFallbackSupplement>,
        // Note: The first entry in the chain is the normalized locale
        expected_language_chain: &'static [&'static str],
        expected_region_chain: &'static [&'static str],
    }

    // TODO: Consider loading these from a JSON file
    const TEST_CASES: &[TestCase] = &[
        TestCase {
            input: "en-u-hc-h12-sd-usca",
            requires_data: false,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en-u-sd-usca", "en"],
            expected_region_chain: &["en-u-sd-usca", "en", "und-u-sd-usca"],
        },
        TestCase {
            input: "en-US-u-hc-h12-sd-usca",
            requires_data: false,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en-US-u-sd-usca", "en-US", "en-u-sd-usca", "en"],
            expected_region_chain: &["en-US-u-sd-usca", "en-US", "und-US-u-sd-usca", "und-US"],
        },
        TestCase {
            input: "en-US-fonipa-u-hc-h12-sd-usca",
            requires_data: false,
            extension_key: Some(key!("hc")),
            fallback_supplement: None,
            expected_language_chain: &[
                "en-US-fonipa-u-hc-h12-sd-usca",
                "en-US-fonipa-u-sd-usca",
                "en-US-fonipa",
                "en-US",
                "en-fonipa-u-hc-h12-sd-usca",
                "en-fonipa-u-sd-usca",
                "en-fonipa",
                "en",
            ],
            expected_region_chain: &[
                "en-US-fonipa-u-hc-h12-sd-usca",
                "en-US-fonipa-u-sd-usca",
                "en-US-fonipa",
                "en-US",
                "und-US-fonipa-u-hc-h12-sd-usca",
                "und-US-fonipa-u-sd-usca",
                "und-US-fonipa",
                "und-US",
            ],
        },
        TestCase {
            input: "en-u-hc-h12-sd-usca",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en-u-sd-usca", "en"],
            expected_region_chain: &["en-US-u-sd-usca", "en-US", "und-US-u-sd-usca", "und-US"],
        },
        TestCase {
            input: "en-Latn-u-sd-usca",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en-u-sd-usca", "en"],
            expected_region_chain: &["en-US-u-sd-usca", "en-US", "und-US-u-sd-usca", "und-US"],
        },
        TestCase {
            input: "en-Latn-US-u-sd-usca",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en-US-u-sd-usca", "en-US", "en-u-sd-usca", "en"],
            expected_region_chain: &["en-US-u-sd-usca", "en-US", "und-US-u-sd-usca", "und-US"],
        },
        TestCase {
            // TODO(#4413): -u-rg is not yet supported; when it is, this test should be updated
            input: "en-u-rg-gbxxxx",
            requires_data: false,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["en"],
            expected_region_chain: &["en"],
        },
        TestCase {
            input: "sr-ME",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["sr-ME", "sr-Latn-ME", "sr-Latn"],
            expected_region_chain: &["sr-ME", "und-ME"],
        },
        TestCase {
            input: "sr-Latn-ME",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["sr-ME", "sr-Latn-ME", "sr-Latn"],
            expected_region_chain: &["sr-ME", "und-ME"],
        },
        TestCase {
            input: "sr-ME-fonipa",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &[
                "sr-ME-fonipa",
                "sr-ME",
                "sr-Latn-ME-fonipa",
                "sr-Latn-ME",
                "sr-Latn-fonipa",
                "sr-Latn",
            ],
            expected_region_chain: &["sr-ME-fonipa", "sr-ME", "und-ME-fonipa", "und-ME"],
        },
        TestCase {
            input: "sr-RS",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["sr-RS", "sr"],
            expected_region_chain: &["sr-RS", "und-RS"],
        },
        TestCase {
            input: "sr-Cyrl-RS",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["sr-RS", "sr"],
            expected_region_chain: &["sr-RS", "und-RS"],
        },
        TestCase {
            input: "sr-Latn-RS",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["sr-Latn-RS", "sr-Latn"],
            expected_region_chain: &["sr-Latn-RS", "und-RS"],
        },
        TestCase {
            input: "de-Latn-LI",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["de-LI", "de"],
            expected_region_chain: &["de-LI", "und-LI"],
        },
        TestCase {
            input: "ca-ES-valencia",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["ca-ES-valencia", "ca-ES", "ca-valencia", "ca"],
            expected_region_chain: &["ca-ES-valencia", "ca-ES", "und-ES-valencia", "und-ES"],
        },
        TestCase {
            input: "es-AR",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["es-AR", "es-419", "es"],
            expected_region_chain: &["es-AR", "und-AR"],
        },
        TestCase {
            input: "hi-IN",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["hi-IN", "hi"],
            expected_region_chain: &["hi-IN", "und-IN"],
        },
        TestCase {
            input: "hi-Latn-IN",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["hi-Latn-IN", "hi-Latn", "en-IN", "en-001", "en"],
            expected_region_chain: &["hi-Latn-IN", "und-IN"],
        },
        TestCase {
            input: "zh-CN",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            // Note: "zh-Hans" is not reachable because it is the default script for "zh".
            // The fallback algorithm does not visit the language-script bundle when the
            // script is the default for the language
            expected_language_chain: &["zh-CN", "zh"],
            expected_region_chain: &["zh-CN", "und-CN"],
        },
        TestCase {
            input: "zh-TW",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["zh-TW", "zh-Hant-TW", "zh-Hant"],
            expected_region_chain: &["zh-TW", "und-TW"],
        },
        TestCase {
            input: "yue-HK",
            requires_data: true,
            extension_key: None,
            fallback_supplement: None,
            expected_language_chain: &["yue-HK", "yue"],
            expected_region_chain: &["yue-HK", "und-HK"],
        },
        TestCase {
            input: "yue-HK",
            requires_data: true,
            extension_key: None,
            fallback_supplement: Some(LocaleFallbackSupplement::Collation),
            expected_language_chain: &["yue-HK", "yue", "zh-Hant", "zh"],
            expected_region_chain: &["yue-HK", "und-HK"],
        },
    ];

    #[test]
    fn test_fallback() {
        let fallbacker_no_data = LocaleFallbacker::new_without_data();
        let fallbacker_no_data = fallbacker_no_data.as_borrowed();
        let fallbacker_with_data = LocaleFallbacker::new();
        for cas in TEST_CASES {
            for (priority, expected_chain) in [
                (
                    LocaleFallbackPriority::Language,
                    cas.expected_language_chain,
                ),
                (LocaleFallbackPriority::Region, cas.expected_region_chain),
            ] {
                let mut config = LocaleFallbackConfig::default();
                config.priority = priority;
                config.extension_key = cas.extension_key;
                config.fallback_supplement = cas.fallback_supplement;
                let fallbacker = if cas.requires_data {
                    fallbacker_with_data
                } else {
                    fallbacker_no_data
                };
                let mut it = fallbacker
                    .for_config(config)
                    .fallback_for(cas.input.parse().unwrap());
                for &expected in expected_chain {
                    assert_eq!(
                        expected,
                        &*it.get().write_to_string(),
                        "{:?} ({:?})",
                        cas.input,
                        priority
                    );
                    it.step();
                }
                assert_eq!(
                    "und",
                    &*it.get().write_to_string(),
                    "{:?} ({:?})",
                    cas.input,
                    priority
                );
            }
        }
    }
}
