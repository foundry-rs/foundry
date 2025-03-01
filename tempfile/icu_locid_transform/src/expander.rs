// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::{provider::*, LocaleTransformError};

use icu_locid::subtags::{Language, Region, Script};
use icu_locid::LanguageIdentifier;
use icu_provider::prelude::*;

use crate::TransformResult;

/// Implements the *Add Likely Subtags* and *Remove Likely Subtags*
/// algorithms as defined in *[UTS #35: Likely Subtags]*.
///
/// # Examples
///
/// Add likely subtags:
///
/// ```
/// use icu::locid::locale;
/// use icu::locid_transform::{LocaleExpander, TransformResult};
///
/// let lc = LocaleExpander::new();
///
/// let mut locale = locale!("zh-CN");
/// assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
/// assert_eq!(locale, locale!("zh-Hans-CN"));
///
/// let mut locale = locale!("zh-Hant-TW");
/// assert_eq!(lc.maximize(&mut locale), TransformResult::Unmodified);
/// assert_eq!(locale, locale!("zh-Hant-TW"));
/// ```
///
/// Remove likely subtags:
///
/// ```
/// use icu::locid::locale;
/// use icu::locid_transform::{LocaleExpander, TransformResult};
///
/// let lc = LocaleExpander::new();
///
/// let mut locale = locale!("zh-Hans-CN");
/// assert_eq!(lc.minimize(&mut locale), TransformResult::Modified);
/// assert_eq!(locale, locale!("zh"));
///
/// let mut locale = locale!("zh");
/// assert_eq!(lc.minimize(&mut locale), TransformResult::Unmodified);
/// assert_eq!(locale, locale!("zh"));
/// ```
///
/// Normally, only CLDR locales with Basic or higher coverage are included. To include more
/// locales for maximization, use [`try_new_extended`](Self::try_new_extended_unstable):
///
/// ```
/// use icu::locid::locale;
/// use icu::locid_transform::{LocaleExpander, TransformResult};
///
/// let lc = LocaleExpander::new_extended();
///
/// let mut locale = locale!("atj");
/// assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
/// assert_eq!(locale, locale!("atj-Latn-CA"));
/// ```
///
/// [UTS #35: Likely Subtags]: https://www.unicode.org/reports/tr35/#Likely_Subtags
#[derive(Debug, Clone)]
pub struct LocaleExpander {
    likely_subtags_l: DataPayload<LikelySubtagsForLanguageV1Marker>,
    likely_subtags_sr: DataPayload<LikelySubtagsForScriptRegionV1Marker>,
    likely_subtags_ext: Option<DataPayload<LikelySubtagsExtendedV1Marker>>,
}

struct LocaleExpanderBorrowed<'a> {
    likely_subtags_l: &'a LikelySubtagsForLanguageV1<'a>,
    likely_subtags_sr: &'a LikelySubtagsForScriptRegionV1<'a>,
    likely_subtags_ext: Option<&'a LikelySubtagsExtendedV1<'a>>,
}

impl LocaleExpanderBorrowed<'_> {
    fn get_l(&self, l: Language) -> Option<(Script, Region)> {
        let key = &l.into_tinystr().to_unvalidated();
        self.likely_subtags_l.language.get_copied(key).or_else(|| {
            self.likely_subtags_ext
                .and_then(|ext| ext.language.get_copied(key))
        })
    }

    fn get_ls(&self, l: Language, s: Script) -> Option<Region> {
        let key = &(
            l.into_tinystr().to_unvalidated(),
            s.into_tinystr().to_unvalidated(),
        );
        self.likely_subtags_l
            .language_script
            .get_copied(key)
            .or_else(|| {
                self.likely_subtags_ext
                    .and_then(|ext| ext.language_script.get_copied(key))
            })
    }

    fn get_lr(&self, l: Language, r: Region) -> Option<Script> {
        let key = &(
            l.into_tinystr().to_unvalidated(),
            r.into_tinystr().to_unvalidated(),
        );
        self.likely_subtags_l
            .language_region
            .get_copied(key)
            .or_else(|| {
                self.likely_subtags_ext
                    .and_then(|ext| ext.language_region.get_copied(key))
            })
    }

    fn get_s(&self, s: Script) -> Option<(Language, Region)> {
        let key = &s.into_tinystr().to_unvalidated();
        self.likely_subtags_sr.script.get_copied(key).or_else(|| {
            self.likely_subtags_ext
                .and_then(|ext| ext.script.get_copied(key))
        })
    }

    fn get_sr(&self, s: Script, r: Region) -> Option<Language> {
        let key = &(
            s.into_tinystr().to_unvalidated(),
            r.into_tinystr().to_unvalidated(),
        );
        self.likely_subtags_sr
            .script_region
            .get_copied(key)
            .or_else(|| {
                self.likely_subtags_ext
                    .and_then(|ext| ext.script_region.get_copied(key))
            })
    }

    fn get_r(&self, r: Region) -> Option<(Language, Script)> {
        let key = &r.into_tinystr().to_unvalidated();
        self.likely_subtags_sr.region.get_copied(key).or_else(|| {
            self.likely_subtags_ext
                .and_then(|ext| ext.region.get_copied(key))
        })
    }

    fn get_und(&self) -> (Language, Script, Region) {
        self.likely_subtags_l.und
    }
}

#[inline]
fn update_langid(
    language: Language,
    script: Option<Script>,
    region: Option<Region>,
    langid: &mut LanguageIdentifier,
) -> TransformResult {
    let mut modified = false;

    if langid.language.is_empty() && !language.is_empty() {
        langid.language = language;
        modified = true;
    }

    if langid.script.is_none() && script.is_some() {
        langid.script = script;
        modified = true;
    }

    if langid.region.is_none() && region.is_some() {
        langid.region = region;
        modified = true;
    }

    if modified {
        TransformResult::Modified
    } else {
        TransformResult::Unmodified
    }
}

#[inline]
fn update_langid_minimize(
    language: Language,
    script: Option<Script>,
    region: Option<Region>,
    langid: &mut LanguageIdentifier,
) -> TransformResult {
    let mut modified = false;

    if langid.language != language {
        langid.language = language;
        modified = true;
    }

    if langid.script != script {
        langid.script = script;
        modified = true;
    }

    if langid.region != region {
        langid.region = region;
        modified = true;
    }

    if modified {
        TransformResult::Modified
    } else {
        TransformResult::Unmodified
    }
}

#[cfg(feature = "compiled_data")]
impl Default for LocaleExpander {
    fn default() -> Self {
        Self::new()
    }
}

impl LocaleExpander {
    /// Creates a [`LocaleExpander`] with compiled data for commonly-used locales
    /// (locales with *Basic* or higher [CLDR coverage]).
    ///
    /// Use this constructor if you want limited likely subtags for data-oriented use cases.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// [CLDR coverage]: https://www.unicode.org/reports/tr35/tr35-info.html#Coverage_Levels
    #[cfg(feature = "compiled_data")]
    pub const fn new() -> Self {
        LocaleExpander {
            likely_subtags_l: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_L_V1,
            ),
            likely_subtags_sr: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_SR_V1,
            ),
            likely_subtags_ext: None,
        }
    }

    /// Creates a [`LocaleExpander`] with compiled data for all locales.
    ///
    /// Use this constructor if you want to include data for all locales, including ones
    /// that may not have data for other services (i.e. [CLDR coverage] below *Basic*).
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// [CLDR coverage]: https://www.unicode.org/reports/tr35/tr35-info.html#Coverage_Levels
    #[cfg(feature = "compiled_data")]
    pub const fn new_extended() -> Self {
        LocaleExpander {
            likely_subtags_l: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_L_V1,
            ),
            likely_subtags_sr: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_SR_V1,
            ),
            likely_subtags_ext: Some(DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_EXT_V1,
            )),
        }
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new_extended)]
    pub fn try_new_extended_unstable<P>(
        provider: &P,
    ) -> Result<LocaleExpander, LocaleTransformError>
    where
        P: DataProvider<LikelySubtagsForLanguageV1Marker>
            + DataProvider<LikelySubtagsForScriptRegionV1Marker>
            + DataProvider<LikelySubtagsExtendedV1Marker>
            + ?Sized,
    {
        let likely_subtags_l = provider.load(Default::default())?.take_payload()?;
        let likely_subtags_sr = provider.load(Default::default())?.take_payload()?;
        let likely_subtags_ext = Some(provider.load(Default::default())?.take_payload()?);

        Ok(LocaleExpander {
            likely_subtags_l,
            likely_subtags_sr,
            likely_subtags_ext,
        })
    }

    icu_provider::gen_any_buffer_data_constructors!(locale: skip, options: skip, error: LocaleTransformError,
        #[cfg(skip)]
        functions: [
        new_extended,
        try_new_extended_with_any_provider,
        try_new_extended_with_buffer_provider,
        try_new_extended_unstable,
        Self
    ]);

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(ANY, Self::new)]
    pub fn try_new_with_any_provider(
        provider: &(impl AnyProvider + ?Sized),
    ) -> Result<LocaleExpander, LocaleTransformError> {
        Self::try_new_compat(&provider.as_downcasting())
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(BUFFER, Self::new)]
    #[cfg(feature = "serde")]
    pub fn try_new_with_buffer_provider(
        provider: &(impl BufferProvider + ?Sized),
    ) -> Result<LocaleExpander, LocaleTransformError> {
        Self::try_new_compat(&provider.as_deserializing())
    }

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<P>(provider: &P) -> Result<LocaleExpander, LocaleTransformError>
    where
        P: DataProvider<LikelySubtagsForLanguageV1Marker>
            + DataProvider<LikelySubtagsForScriptRegionV1Marker>
            + ?Sized,
    {
        let likely_subtags_l = provider.load(Default::default())?.take_payload()?;
        let likely_subtags_sr = provider.load(Default::default())?.take_payload()?;

        Ok(LocaleExpander {
            likely_subtags_l,
            likely_subtags_sr,
            likely_subtags_ext: None,
        })
    }

    fn try_new_compat<P>(provider: &P) -> Result<LocaleExpander, LocaleTransformError>
    where
        P: DataProvider<LikelySubtagsForLanguageV1Marker>
            + DataProvider<LikelySubtagsForScriptRegionV1Marker>
            + DataProvider<LikelySubtagsExtendedV1Marker>
            + DataProvider<LikelySubtagsV1Marker>
            + ?Sized,
    {
        let payload_l = provider
            .load(Default::default())
            .and_then(DataResponse::take_payload);
        let payload_sr = provider
            .load(Default::default())
            .and_then(DataResponse::take_payload);
        let payload_ext = provider
            .load(Default::default())
            .and_then(DataResponse::take_payload);

        let (likely_subtags_l, likely_subtags_sr, likely_subtags_ext) =
            match (payload_l, payload_sr, payload_ext) {
                (Ok(l), Ok(sr), Err(_)) => (l, sr, None),
                (Ok(l), Ok(sr), Ok(ext)) => (l, sr, Some(ext)),
                _ => {
                    let result: DataPayload<LikelySubtagsV1Marker> =
                        provider.load(Default::default())?.take_payload()?;
                    (
                        result.map_project_cloned(|st, _| {
                            LikelySubtagsForLanguageV1::clone_from_borrowed(st)
                        }),
                        result.map_project(|st, _| st.into()),
                        None,
                    )
                }
            };

        Ok(LocaleExpander {
            likely_subtags_l,
            likely_subtags_sr,
            likely_subtags_ext,
        })
    }

    fn as_borrowed(&self) -> LocaleExpanderBorrowed {
        LocaleExpanderBorrowed {
            likely_subtags_l: self.likely_subtags_l.get(),
            likely_subtags_sr: self.likely_subtags_sr.get(),
            likely_subtags_ext: self.likely_subtags_ext.as_ref().map(|p| p.get()),
        }
    }

    /// The maximize method potentially updates a passed in locale in place
    /// depending up the results of running the 'Add Likely Subtags' algorithm
    /// from <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
    ///
    /// If the result of running the algorithm would result in a new locale, the
    /// locale argument is updated in place to match the result, and the method
    /// returns [`TransformResult::Modified`]. Otherwise, the method
    /// returns [`TransformResult::Unmodified`] and the locale argument is
    /// unchanged.
    ///
    /// This function does not guarantee that any particular set of subtags
    /// will be present in the resulting locale.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::locid_transform::{LocaleExpander, TransformResult};
    ///
    /// let lc = LocaleExpander::new();
    ///
    /// let mut locale = locale!("zh-CN");
    /// assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
    /// assert_eq!(locale, locale!("zh-Hans-CN"));
    ///
    /// let mut locale = locale!("zh-Hant-TW");
    /// assert_eq!(lc.maximize(&mut locale), TransformResult::Unmodified);
    /// assert_eq!(locale, locale!("zh-Hant-TW"));
    /// ```
    ///
    /// If there is no data for a particular language, the result is not
    /// modified. Note that [`LocaleExpander::new_extended`] supports
    /// more languages.
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::locid_transform::{LocaleExpander, TransformResult};
    ///
    /// let lc = LocaleExpander::new();
    ///
    /// // No subtags data for ccp in the default set:
    /// let mut locale = locale!("ccp");
    /// assert_eq!(lc.maximize(&mut locale), TransformResult::Unmodified);
    /// assert_eq!(locale, locale!("ccp"));
    ///
    /// // The extended set supports it:
    /// let lc = LocaleExpander::new_extended();
    /// let mut locale = locale!("ccp");
    /// assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
    /// assert_eq!(locale, locale!("ccp-Cakm-BD"));
    ///
    /// // But even the extended set does not support all language subtags:
    /// let mut locale = locale!("mul");
    /// assert_eq!(lc.maximize(&mut locale), TransformResult::Unmodified);
    /// assert_eq!(locale, locale!("mul"));
    /// ```
    pub fn maximize<T: AsMut<LanguageIdentifier>>(&self, mut langid: T) -> TransformResult {
        let langid = langid.as_mut();
        let data = self.as_borrowed();

        if !langid.language.is_empty() && langid.script.is_some() && langid.region.is_some() {
            return TransformResult::Unmodified;
        }

        if !langid.language.is_empty() {
            if let Some(region) = langid.region {
                if let Some(script) = data.get_lr(langid.language, region) {
                    return update_langid(Language::UND, Some(script), None, langid);
                }
            }
            if let Some(script) = langid.script {
                if let Some(region) = data.get_ls(langid.language, script) {
                    return update_langid(Language::UND, None, Some(region), langid);
                }
            }
            if let Some((script, region)) = data.get_l(langid.language) {
                return update_langid(Language::UND, Some(script), Some(region), langid);
            }
            // Language not found: return unmodified.
            return TransformResult::Unmodified;
        }
        if let Some(script) = langid.script {
            if let Some(region) = langid.region {
                if let Some(language) = data.get_sr(script, region) {
                    return update_langid(language, None, None, langid);
                }
            }
            if let Some((language, region)) = data.get_s(script) {
                return update_langid(language, None, Some(region), langid);
            }
        }
        if let Some(region) = langid.region {
            if let Some((language, script)) = data.get_r(region) {
                return update_langid(language, Some(script), None, langid);
            }
        }

        // We failed to find anything in the und-SR, und-S, or und-R tables,
        // to fall back to bare "und"
        debug_assert!(langid.language.is_empty());
        update_langid(
            data.get_und().0,
            Some(data.get_und().1),
            Some(data.get_und().2),
            langid,
        )
    }

    /// This returns a new Locale that is the result of running the
    /// 'Remove Likely Subtags' algorithm from
    /// <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
    ///
    /// If the result of running the algorithm would result in a new locale, the
    /// locale argument is updated in place to match the result, and the method
    /// returns [`TransformResult::Modified`]. Otherwise, the method
    /// returns [`TransformResult::Unmodified`] and the locale argument is
    /// unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::locid_transform::{LocaleExpander, TransformResult};
    ///
    /// let lc = LocaleExpander::new();
    ///
    /// let mut locale = locale!("zh-Hans-CN");
    /// assert_eq!(lc.minimize(&mut locale), TransformResult::Modified);
    /// assert_eq!(locale, locale!("zh"));
    ///
    /// let mut locale = locale!("zh");
    /// assert_eq!(lc.minimize(&mut locale), TransformResult::Unmodified);
    /// assert_eq!(locale, locale!("zh"));
    /// ```
    pub fn minimize<T: AsMut<LanguageIdentifier>>(&self, langid: T) -> TransformResult {
        self.minimize_impl(langid, true)
    }

    /// This returns a new Locale that is the result of running the
    /// 'Remove Likely Subtags, favoring script' algorithm from
    /// <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
    ///
    /// If the result of running the algorithm would result in a new locale, the
    /// locale argument is updated in place to match the result, and the method
    /// returns [`TransformResult::Modified`]. Otherwise, the method
    /// returns [`TransformResult::Unmodified`] and the locale argument is
    /// unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::locid_transform::{LocaleExpander, TransformResult};
    ///
    /// let lc = LocaleExpander::new();
    ///
    /// let mut locale = locale!("zh_TW");
    /// assert_eq!(
    ///     lc.minimize_favor_script(&mut locale),
    ///     TransformResult::Modified
    /// );
    /// assert_eq!(locale, locale!("zh_Hant"));
    /// ```
    pub fn minimize_favor_script<T: AsMut<LanguageIdentifier>>(
        &self,
        langid: T,
    ) -> TransformResult {
        self.minimize_impl(langid, false)
    }

    fn minimize_impl<T: AsMut<LanguageIdentifier>>(
        &self,
        mut langid: T,
        favor_region: bool,
    ) -> TransformResult {
        let langid = langid.as_mut();

        let mut max = langid.clone();
        self.maximize(&mut max);

        let mut trial = max.clone();

        trial.script = None;
        trial.region = None;
        self.maximize(&mut trial);
        if trial == max {
            return update_langid_minimize(max.language, None, None, langid);
        }

        if favor_region {
            trial.script = None;
            trial.region = max.region;
            self.maximize(&mut trial);

            if trial == max {
                return update_langid_minimize(max.language, None, max.region, langid);
            }

            trial.script = max.script;
            trial.region = None;
            self.maximize(&mut trial);
            if trial == max {
                return update_langid_minimize(max.language, max.script, None, langid);
            }
        } else {
            trial.script = max.script;
            trial.region = None;
            self.maximize(&mut trial);
            if trial == max {
                return update_langid_minimize(max.language, max.script, None, langid);
            }

            trial.script = None;
            trial.region = max.region;
            self.maximize(&mut trial);

            if trial == max {
                return update_langid_minimize(max.language, None, max.region, langid);
            }
        }

        update_langid_minimize(max.language, max.script, max.region, langid)
    }

    // TODO(3492): consider turning this and a future get_likely_region/get_likely_language public
    #[inline]
    pub(crate) fn get_likely_script<T: AsRef<LanguageIdentifier>>(
        &self,
        langid: T,
    ) -> Option<Script> {
        let langid = langid.as_ref();
        langid
            .script
            .or_else(|| self.infer_likely_script(langid.language, langid.region))
    }

    fn infer_likely_script(&self, language: Language, region: Option<Region>) -> Option<Script> {
        let data = self.as_borrowed();

        // proceed through _all possible cases_ in order of specificity
        // (borrowed from LocaleExpander::maximize):
        // 1. language + region
        // 2. language
        // 3. region
        // we need to check all cases, because e.g. for "en-US" the default script is associated
        // with "en" but not "en-US"
        if language != Language::UND {
            if let Some(region) = region {
                // 1. we know both language and region
                if let Some(script) = data.get_lr(language, region) {
                    return Some(script);
                }
            }
            // 2. we know language, but we either do not know region or knowing region did not help
            if let Some((script, _)) = data.get_l(language) {
                return Some(script);
            }
        }
        if let Some(region) = region {
            // 3. we know region, but we either do not know language or knowing language did not help
            if let Some((_, script)) = data.get_r(region) {
                return Some(script);
            }
        }
        // we could not figure out the script from the given locale
        None
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
            if self.keys.contains(&key) {
                return Err(DataErrorKind::MissingDataKey.with_str_context("rejected"));
            }

            let l = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_L_V1;
            let ext = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_EXT_V1;
            let sr = crate::provider::Baked::SINGLETON_LOCID_TRANSFORM_LIKELYSUBTAGS_SR_V1;

            let payload = if key.hashed() == LikelySubtagsV1Marker::KEY.hashed() {
                DataPayload::<LikelySubtagsV1Marker>::from_owned(LikelySubtagsV1 {
                    language_script: l
                        .language_script
                        .iter_copied()
                        .chain(ext.language_script.iter_copied())
                        .collect(),
                    language_region: l
                        .language_region
                        .iter_copied()
                        .chain(ext.language_region.iter_copied())
                        .collect(),
                    language: l
                        .language
                        .iter_copied()
                        .chain(ext.language.iter_copied())
                        .collect(),
                    script_region: ext.script_region.clone(),
                    script: ext.script.clone(),
                    region: ext.region.clone(),
                    und: l.und,
                })
                .wrap_into_any_payload()
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
            keys: vec![
                LikelySubtagsForLanguageV1Marker::KEY,
                LikelySubtagsForScriptRegionV1Marker::KEY,
                LikelySubtagsExtendedV1Marker::KEY,
            ],
        };
        let lc = LocaleExpander::try_new_with_any_provider(&provider)
            .expect("should create with old keys");
        let mut locale = locale!("zh-CN");
        assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("zh-Hans-CN"));
    }

    #[test]
    fn test_new_keys() {
        let provider = RejectByKeyProvider {
            keys: vec![LikelySubtagsV1Marker::KEY],
        };
        let lc = LocaleExpander::try_new_with_any_provider(&provider)
            .expect("should create with new keys");
        let mut locale = locale!("zh-CN");
        assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("zh-Hans-CN"));
    }

    #[test]
    fn test_mixed_keys() {
        // Include the old key and one of the new keys but not both new keys.
        // Not sure if this is a useful test.
        let provider = RejectByKeyProvider {
            keys: vec![LikelySubtagsForScriptRegionV1Marker::KEY],
        };
        let lc = LocaleExpander::try_new_with_any_provider(&provider)
            .expect("should create with mixed keys");
        let mut locale = locale!("zh-CN");
        assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("zh-Hans-CN"));
    }

    #[test]
    fn test_no_keys() {
        let provider = RejectByKeyProvider {
            keys: vec![
                LikelySubtagsForLanguageV1Marker::KEY,
                LikelySubtagsForScriptRegionV1Marker::KEY,
                LikelySubtagsV1Marker::KEY,
            ],
        };
        if LocaleExpander::try_new_with_any_provider(&provider).is_ok() {
            panic!("should not create: no data present")
        };
    }

    #[test]
    fn test_new_small_keys() {
        // Include the new small keys but not the extended key
        let provider = RejectByKeyProvider {
            keys: vec![
                LikelySubtagsExtendedV1Marker::KEY,
                LikelySubtagsV1Marker::KEY,
            ],
        };
        let lc = LocaleExpander::try_new_with_any_provider(&provider)
            .expect("should create with mixed keys");
        let mut locale = locale!("zh-CN");
        assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("zh-Hans-CN"));
    }

    #[test]
    fn test_minimize_favor_script() {
        let lc = LocaleExpander::new();
        let mut locale = locale!("yue-Hans");
        assert_eq!(
            lc.minimize_favor_script(&mut locale),
            TransformResult::Unmodified
        );
        assert_eq!(locale, locale!("yue-Hans"));
    }

    #[test]
    fn test_minimize_favor_region() {
        let lc = LocaleExpander::new();
        let mut locale = locale!("yue-Hans");
        assert_eq!(lc.minimize(&mut locale), TransformResult::Modified);
        assert_eq!(locale, locale!("yue-CN"));
    }
}
