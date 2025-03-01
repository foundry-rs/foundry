// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! The collection of code for locale canonicalization.

use crate::provider::*;
use crate::LocaleTransformError;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::LocaleExpander;
use crate::TransformResult;
use icu_locid::extensions::Extensions;
use icu_locid::subtags::{Language, Region, Script};
use icu_locid::{
    extensions::unicode::key,
    subtags::{language, Variant, Variants},
    LanguageIdentifier, Locale,
};
use icu_provider::prelude::*;
use tinystr::TinyAsciiStr;

/// Implements the algorithm defined in *[UTS #35: Annex C, LocaleId Canonicalization]*.
///
/// # Examples
///
/// ```
/// use icu::locid::Locale;
/// use icu::locid_transform::{LocaleCanonicalizer, TransformResult};
///
/// let lc = LocaleCanonicalizer::new();
///
/// let mut locale: Locale = "ja-Latn-fonipa-hepburn-heploc".parse().unwrap();
/// assert_eq!(lc.canonicalize(&mut locale), TransformResult::Modified);
/// assert_eq!(locale, "ja-Latn-alalc97-fonipa".parse().unwrap());
/// ```
///
/// [UTS #35: Annex C, LocaleId Canonicalization]: http://unicode.org/reports/tr35/#LocaleId_Canonicalization
#[derive(Debug)]
pub struct LocaleCanonicalizer {
    /// Data to support canonicalization.
    aliases: DataPayload<AliasesV2Marker>,
    /// Likely subtags implementation for delegation.
    expander: LocaleExpander,
}

fn uts35_rule_matches<'a, I>(
    source: &LanguageIdentifier,
    language: Language,
    script: Option<Script>,
    region: Option<Region>,
    raw_variants: I,
) -> bool
where
    I: Iterator<Item = &'a str>,
{
    (language.is_empty() || language == source.language)
        && (script.is_none() || script == source.script)
        && (region.is_none() || region == source.region)
        && {
            // Checks if variants are a subset of source variants.
            // As both iterators are sorted, this can be done linearly.
            let mut source_variants = source.variants.iter();
            'outer: for raw_variant in raw_variants {
                for source_variant in source_variants.by_ref() {
                    match source_variant.strict_cmp(raw_variant.as_bytes()) {
                        Ordering::Equal => {
                            // The source_variant is equal, move to next raw_variant
                            continue 'outer;
                        }
                        Ordering::Less => {
                            // The source_variant is smaller, take the next source_variant
                        }
                        Ordering::Greater => {
                            // The source_variant is greater,
                            // raw_variants is not a subset of source_variants
                            return false;
                        }
                    }
                }
                // There are raw_variants left after we exhausted source_variants
                return false;
            }
            true
        }
}

fn uts35_replacement<'a, I>(
    source: &mut LanguageIdentifier,
    ruletype_has_language: bool,
    ruletype_has_script: bool,
    ruletype_has_region: bool,
    ruletype_variants: Option<I>,
    replacement: &LanguageIdentifier,
) where
    I: Iterator<Item = &'a str>,
{
    if ruletype_has_language || (source.language.is_empty() && !replacement.language.is_empty()) {
        source.language = replacement.language;
    }
    if ruletype_has_script || (source.script.is_none() && replacement.script.is_some()) {
        source.script = replacement.script;
    }
    if ruletype_has_region || (source.region.is_none() && replacement.region.is_some()) {
        source.region = replacement.region;
    }
    if let Some(skips) = ruletype_variants {
        // The rule matches if the ruletype variants are a subset of the source variants.
        // This means ja-Latn-fonipa-hepburn-heploc matches against the rule for
        // hepburn-heploc and is canonicalized to ja-Latn-alalc97-fonipa

        // We're merging three sorted deduped iterators into a new sequence:
        // sources - skips + replacements

        let mut sources = source.variants.iter().peekable();
        let mut replacements = replacement.variants.iter().peekable();
        let mut skips = skips.peekable();

        let mut variants: Vec<Variant> = Vec::new();

        loop {
            match (sources.peek(), skips.peek(), replacements.peek()) {
                (Some(&source), Some(skip), _)
                    if source.strict_cmp(skip.as_bytes()) == Ordering::Greater =>
                {
                    skips.next();
                }
                (Some(&source), Some(skip), _)
                    if source.strict_cmp(skip.as_bytes()) == Ordering::Equal =>
                {
                    skips.next();
                    sources.next();
                }
                (Some(&source), _, Some(&replacement))
                    if replacement.cmp(source) == Ordering::Less =>
                {
                    variants.push(*replacement);
                    replacements.next();
                }
                (Some(&source), _, Some(&replacement))
                    if replacement.cmp(source) == Ordering::Equal =>
                {
                    variants.push(*source);
                    sources.next();
                    replacements.next();
                }
                (Some(&source), _, _) => {
                    variants.push(*source);
                    sources.next();
                }
                (None, _, Some(&replacement)) => {
                    variants.push(*replacement);
                    replacements.next();
                }
                (None, _, None) => {
                    break;
                }
            }
        }
        source.variants = Variants::from_vec_unchecked(variants);
    }
}

#[inline]
fn uts35_check_language_rules(
    langid: &mut LanguageIdentifier,
    alias_data: &DataPayload<AliasesV2Marker>,
) -> TransformResult {
    if !langid.language.is_empty() {
        let lang: TinyAsciiStr<3> = langid.language.into();
        let replacement = if lang.len() == 2 {
            alias_data
                .get()
                .language_len2
                .get(&lang.resize().to_unvalidated())
        } else {
            alias_data.get().language_len3.get(&lang.to_unvalidated())
        };

        if let Some(replacement) = replacement {
            if let Ok(new_langid) = replacement.parse() {
                uts35_replacement::<core::iter::Empty<&str>>(
                    langid,
                    true,
                    false,
                    false,
                    None,
                    &new_langid,
                );
                return TransformResult::Modified;
            }
        }
    }

    TransformResult::Unmodified
}

#[cfg(feature = "compiled_data")]
impl Default for LocaleCanonicalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl LocaleCanonicalizer {
    /// A constructor which creates a [`LocaleCanonicalizer`] from compiled data.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    pub const fn new() -> Self {
        Self::new_with_expander(LocaleExpander::new_extended())
    }

    // Note: This is a custom impl because the bounds on LocaleExpander::try_new_unstable changed
    #[doc = icu_provider::gen_any_buffer_unstable_docs!(ANY, Self::new)]
    pub fn try_new_with_any_provider(
        provider: &(impl AnyProvider + ?Sized),
    ) -> Result<Self, LocaleTransformError> {
        let expander = LocaleExpander::try_new_with_any_provider(provider)?;
        Self::try_new_with_expander_compat(&provider.as_downcasting(), expander)
    }

    // Note: This is a custom impl because the bounds on LocaleExpander::try_new_unstable changed
    #[doc = icu_provider::gen_any_buffer_unstable_docs!(BUFFER, Self::new)]
    #[cfg(feature = "serde")]
    pub fn try_new_with_buffer_provider(
        provider: &(impl BufferProvider + ?Sized),
    ) -> Result<Self, LocaleTransformError> {
        let expander = LocaleExpander::try_new_with_buffer_provider(provider)?;
        Self::try_new_with_expander_compat(&provider.as_deserializing(), expander)
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<P>(provider: &P) -> Result<Self, LocaleTransformError>
    where
        P: DataProvider<AliasesV2Marker>
            + DataProvider<LikelySubtagsForLanguageV1Marker>
            + DataProvider<LikelySubtagsForScriptRegionV1Marker>
            + ?Sized,
    {
        let expander = LocaleExpander::try_new_unstable(provider)?;
        Self::try_new_with_expander_unstable(provider, expander)
    }

    /// Creates a [`LocaleCanonicalizer`] with a custom [`LocaleExpander`] and compiled data.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    pub const fn new_with_expander(expander: LocaleExpander) -> Self {
        Self {
            aliases: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_ALIASES_V2,
            ),
            expander,
        }
    }

    fn try_new_with_expander_compat<P>(
        provider: &P,
        expander: LocaleExpander,
    ) -> Result<Self, LocaleTransformError>
    where
        P: DataProvider<AliasesV2Marker> + DataProvider<AliasesV1Marker> + ?Sized,
    {
        let payload_v2: Result<DataPayload<AliasesV2Marker>, _> = provider
            .load(Default::default())
            .and_then(DataResponse::take_payload);
        let aliases = if let Ok(payload) = payload_v2 {
            payload
        } else {
            let payload_v1: DataPayload<AliasesV1Marker> = provider
                .load(Default::default())
                .and_then(DataResponse::take_payload)?;
            payload_v1.try_map_project(|st, _| st.try_into())?
        };

        Ok(Self { aliases, expander })
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new_with_expander)]
    pub fn try_new_with_expander_unstable<P>(
        provider: &P,
        expander: LocaleExpander,
    ) -> Result<Self, LocaleTransformError>
    where
        P: DataProvider<AliasesV2Marker> + ?Sized,
    {
        let aliases: DataPayload<AliasesV2Marker> =
            provider.load(Default::default())?.take_payload()?;

        Ok(Self { aliases, expander })
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(ANY, Self::new_with_expander)]
    pub fn try_new_with_expander_with_any_provider(
        provider: &(impl AnyProvider + ?Sized),
        options: LocaleExpander,
    ) -> Result<Self, LocaleTransformError> {
        Self::try_new_with_expander_compat(&provider.as_downcasting(), options)
    }

    #[cfg(feature = "serde")]
    #[doc = icu_provider::gen_any_buffer_unstable_docs!(BUFFER,Self::new_with_expander)]
    pub fn try_new_with_expander_with_buffer_provider(
        provider: &(impl BufferProvider + ?Sized),
        options: LocaleExpander,
    ) -> Result<Self, LocaleTransformError> {
        Self::try_new_with_expander_compat(&provider.as_deserializing(), options)
    }

    /// The canonicalize method potentially updates a passed in locale in place
    /// depending up the results of running the canonicalization algorithm
    /// from <http://unicode.org/reports/tr35/#LocaleId_Canonicalization>.
    ///
    /// Some BCP47 canonicalization data is not part of the CLDR json package. Because
    /// of this, some canonicalizations are not performed, e.g. the canonicalization of
    /// `und-u-ca-islamicc` to `und-u-ca-islamic-civil`. This will be fixed in a future
    /// release once the missing data has been added to the CLDR json data. See:
    /// <https://github.com/unicode-org/icu4x/issues/746>
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::Locale;
    /// use icu::locid_transform::{LocaleCanonicalizer, TransformResult};
    ///
    /// let lc = LocaleCanonicalizer::new();
    ///
    /// let mut locale: Locale = "ja-Latn-fonipa-hepburn-heploc".parse().unwrap();
    /// assert_eq!(lc.canonicalize(&mut locale), TransformResult::Modified);
    /// assert_eq!(locale, "ja-Latn-alalc97-fonipa".parse().unwrap());
    /// ```
    pub fn canonicalize(&self, locale: &mut Locale) -> TransformResult {
        let mut result = TransformResult::Unmodified;

        // This loops until we get a 'fixed point', where applying the rules do not
        // result in any more changes.
        loop {
            // These are linear searches due to the ordering imposed by the canonicalization
            // rules, where rules with more variants should be considered first. With the
            // current data in CLDR, we will only do this for locales which have variants,
            // or new rules which we haven't special-cased yet (of which there are fewer
            // than 20).
            let modified = if locale.id.variants.is_empty() {
                self.canonicalize_absolute_language_fallbacks(&mut locale.id)
            } else {
                self.canonicalize_language_variant_fallbacks(&mut locale.id)
            };
            if modified {
                result = TransformResult::Modified;
                continue;
            }

            if !locale.id.language.is_empty() {
                // If the region is specified, check sgn-region rules first
                if let Some(region) = locale.id.region {
                    if locale.id.language == language!("sgn") {
                        if let Some(&sgn_lang) = self
                            .aliases
                            .get()
                            .sgn_region
                            .get(&region.into_tinystr().to_unvalidated())
                        {
                            uts35_replacement::<core::iter::Empty<&str>>(
                                &mut locale.id,
                                true,
                                false,
                                true,
                                None,
                                &sgn_lang.into(),
                            );
                            result = TransformResult::Modified;
                            continue;
                        }
                    }
                }

                if uts35_check_language_rules(&mut locale.id, &self.aliases)
                    == TransformResult::Modified
                {
                    result = TransformResult::Modified;
                    continue;
                }
            }

            if let Some(script) = locale.id.script {
                if let Some(&replacement) = self
                    .aliases
                    .get()
                    .script
                    .get(&script.into_tinystr().to_unvalidated())
                {
                    locale.id.script = Some(replacement);
                    result = TransformResult::Modified;
                    continue;
                }
            }

            if let Some(region) = locale.id.region {
                let replacement = if region.is_alphabetic() {
                    self.aliases
                        .get()
                        .region_alpha
                        .get(&region.into_tinystr().resize().to_unvalidated())
                } else {
                    self.aliases
                        .get()
                        .region_num
                        .get(&region.into_tinystr().to_unvalidated())
                };
                if let Some(&replacement) = replacement {
                    locale.id.region = Some(replacement);
                    result = TransformResult::Modified;
                    continue;
                }

                if let Some(regions) = self
                    .aliases
                    .get()
                    .complex_region
                    .get(&region.into_tinystr().to_unvalidated())
                {
                    // Skip if regions are empty
                    if let Some(default_region) = regions.get(0) {
                        let mut maximized = LanguageIdentifier {
                            language: locale.id.language,
                            script: locale.id.script,
                            region: None,
                            variants: Variants::default(),
                        };

                        locale.id.region = Some(
                            match (self.expander.maximize(&mut maximized), maximized.region) {
                                (TransformResult::Modified, Some(candidate))
                                    if regions.iter().any(|x| x == candidate) =>
                                {
                                    candidate
                                }
                                _ => default_region,
                            },
                        );
                        result = TransformResult::Modified;
                        continue;
                    }
                }
            }

            if !locale.id.variants.is_empty() {
                let mut modified = Vec::with_capacity(0);
                for (idx, &variant) in locale.id.variants.iter().enumerate() {
                    if let Some(&updated) = self
                        .aliases
                        .get()
                        .variant
                        .get(&variant.into_tinystr().to_unvalidated())
                    {
                        if modified.is_empty() {
                            modified = locale.id.variants.to_vec();
                        }
                        #[allow(clippy::indexing_slicing)]
                        let _ = core::mem::replace(&mut modified[idx], updated);
                    }
                }

                if !modified.is_empty() {
                    modified.sort();
                    modified.dedup();
                    locale.id.variants = Variants::from_vec_unchecked(modified);
                    result = TransformResult::Modified;
                    continue;
                }
            }

            // Nothing matched in this iteration, we're done.
            break;
        }

        if !locale.extensions.transform.is_empty() || !locale.extensions.unicode.is_empty() {
            self.canonicalize_extensions(&mut locale.extensions, &mut result);
        }
        result
    }

    fn canonicalize_extensions(&self, extensions: &mut Extensions, result: &mut TransformResult) {
        // Handle Locale extensions in their own loops, because these rules do not interact
        // with each other.
        if let Some(ref mut lang) = extensions.transform.lang {
            while uts35_check_language_rules(lang, &self.aliases) == TransformResult::Modified {
                *result = TransformResult::Modified;
            }
        }

        if !extensions.unicode.keywords.is_empty() {
            for key in [key!("rg"), key!("sd")] {
                if let Some(value) = extensions.unicode.keywords.get_mut(&key) {
                    if let &[only_value] = value.as_tinystr_slice() {
                        if let Some(modified_value) = self
                            .aliases
                            .get()
                            .subdivision
                            .get(&only_value.resize().to_unvalidated())
                        {
                            if let Ok(modified_value) = modified_value.parse() {
                                *value = modified_value;
                                *result = TransformResult::Modified;
                            }
                        }
                    }
                }
            }
        }
    }

    fn canonicalize_language_variant_fallbacks(&self, lid: &mut LanguageIdentifier) -> bool {
        // These language/variant comibnations have around 20 rules
        for LanguageStrStrPair(lang, raw_variants, raw_to) in self
            .aliases
            .get()
            .language_variants
            .iter()
            .map(zerofrom::ZeroFrom::zero_from)
        {
            let raw_variants = raw_variants.split('-');
            // if is_iter_sorted(raw_variants.clone()) { // can we sort at construction?
            if uts35_rule_matches(lid, lang, None, None, raw_variants.clone()) {
                if let Ok(to) = raw_to.parse() {
                    uts35_replacement(lid, !lang.is_empty(), false, false, Some(raw_variants), &to);
                    return true;
                }
            }
        }
        false
    }

    fn canonicalize_absolute_language_fallbacks(&self, lid: &mut LanguageIdentifier) -> bool {
        for StrStrPair(raw_from, raw_to) in self
            .aliases
            .get()
            .language
            .iter()
            .map(zerofrom::ZeroFrom::zero_from)
        {
            if let Ok(from) = raw_from.parse::<LanguageIdentifier>() {
                if uts35_rule_matches(
                    lid,
                    from.language,
                    from.script,
                    from.region,
                    from.variants.iter().map(Variant::as_str),
                ) {
                    if let Ok(to) = raw_to.parse() {
                        uts35_replacement(
                            lid,
                            !from.language.is_empty(),
                            from.script.is_some(),
                            from.region.is_some(),
                            Some(from.variants.iter().map(Variant::as_str)),
                            &to,
                        );
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_uts35_rule_matches() {
        for (source, rule, result) in [
            ("ja", "und", true),
            ("und-heploc-hepburn", "und-hepburn", true),
            ("ja-heploc-hepburn", "und-hepburn", true),
            ("ja-hepburn", "und-hepburn-heploc", false),
        ] {
            let source = source.parse().unwrap();
            let rule = rule.parse::<LanguageIdentifier>().unwrap();
            assert_eq!(
                uts35_rule_matches(
                    &source,
                    rule.language,
                    rule.script,
                    rule.region,
                    rule.variants.iter().map(Variant::as_str),
                ),
                result,
                "{}",
                source
            );
        }
    }

    #[test]
    fn test_uts35_replacement() {
        for (locale, rule_0, rule_1, result) in [
            (
                "ja-Latn-fonipa-hepburn-heploc",
                "und-hepburn-heploc",
                "und-alalc97",
                "ja-Latn-alalc97-fonipa",
            ),
            ("sgn-DD", "und-DD", "und-DE", "sgn-DE"),
            ("sgn-DE", "sgn-DE", "gsg", "gsg"),
        ] {
            let mut locale: Locale = locale.parse().unwrap();
            let rule_0 = rule_0.parse::<LanguageIdentifier>().unwrap();
            let rule_1 = rule_1.parse().unwrap();
            let result = result.parse::<Locale>().unwrap();
            uts35_replacement(
                &mut locale.id,
                !rule_0.language.is_empty(),
                rule_0.script.is_some(),
                rule_0.region.is_some(),
                Some(rule_0.variants.iter().map(Variant::as_str)),
                &rule_1,
            );
            assert_eq!(result, locale);
        }
    }
}

#[cfg(feature = "serde")]
#[cfg(test)]
mod tests {
    use super::*;
    use icu_locid::locale;

    struct RejectByKeyProvider {
        keys: Vec<DataKey>,
    }

    impl AnyProvider for RejectByKeyProvider {
        fn load_any(&self, key: DataKey, _: DataRequest) -> Result<AnyResponse, DataError> {
            use alloc::borrow::Cow;

            println!("{:#?}", key);
            if self.keys.contains(&key) {
                return Err(DataErrorKind::MissingDataKey.with_str_context("rejected"));
            }

            let aliases_v2 = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_ALIASES_V2;
            let l = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_L_V1;
            let ext = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_EXT_V1;
            let sr = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_SR_V1;

            let payload = if key.hashed() == AliasesV1Marker::KEY.hashed() {
                let aliases_v1 = AliasesV1 {
                    language_variants: zerovec::VarZeroVec::from(&[StrStrPair(
                        Cow::Borrowed("aa-saaho"),
                        Cow::Borrowed("ssy"),
                    )]),
                    ..Default::default()
                };
                DataPayload::<AliasesV1Marker>::from_owned(aliases_v1).wrap_into_any_payload()
            } else if key.hashed() == AliasesV2Marker::KEY.hashed() {
                DataPayload::<AliasesV2Marker>::from_static_ref(aliases_v2).wrap_into_any_payload()
            } else if key.hashed() == LikelySubtagsForLanguageV1Marker::KEY.hashed() {
                DataPayload::<LikelySubtagsForLanguageV1Marker>::from_static_ref(l)
                    .wrap_into_any_payload()
            } else if key.hashed() == LikelySubtagsExtendedV1Marker::KEY.hashed() {
                DataPayload::<LikelySubtagsExtendedV1Marker>::from_static_ref(ext)
                    .wrap_into_any_payload()
            } else if key.hashed() == LikelySubtagsForScriptRegionV1Marker::KEY.hashed() {
                DataPayload::<LikelySubtagsForScriptRegionV1Marker>::from_static_ref(sr)
                    .wrap_into_any_payload()
            } else {
                return Err(DataErrorKind::MissingDataKey.into_error());
            };

            Ok(AnyResponse {
                payload: Some(payload),
                metadata: Default::default(),
            })
        }
    }

    #[test]
    fn test_old_keys() {
        let provider = RejectByKeyProvider {
            keys: vec![AliasesV2Marker::KEY],
        };
        let lc = LocaleCanonicalizer::try_new_with_any_provider(&provider)
            .expect("should create with old keys");
        let mut locale = locale!("aa-saaho");
        assert_eq!(lc.canonicalize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("ssy"));
    }

    #[test]
    fn test_new_keys() {
        let provider = RejectByKeyProvider {
            keys: vec![AliasesV1Marker::KEY],
        };
        let lc = LocaleCanonicalizer::try_new_with_any_provider(&provider)
            .expect("should create with old keys");
        let mut locale = locale!("aa-saaho");
        assert_eq!(lc.canonicalize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("ssy"));
    }

    #[test]
    fn test_no_keys() {
        let provider = RejectByKeyProvider {
            keys: vec![AliasesV1Marker::KEY, AliasesV2Marker::KEY],
        };
        if LocaleCanonicalizer::try_new_with_any_provider(&provider).is_ok() {
            panic!("should not create: no data present")
        };
    }
}
