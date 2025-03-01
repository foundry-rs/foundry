// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use core::cmp::Ordering;
use core::str::FromStr;

#[allow(deprecated)]
use crate::ordering::SubtagOrderingResult;
use crate::parser::{
    parse_language_identifier, parse_language_identifier_with_single_variant, ParserError,
    ParserMode, SubtagIterator,
};
use crate::subtags;
use alloc::string::String;
use writeable::Writeable;

/// A core struct representing a [`Unicode BCP47 Language Identifier`].
///
/// # Examples
///
/// ```
/// use icu::locid::{
///     langid,
///     subtags::{language, region},
/// };
///
/// let li = langid!("en-US");
///
/// assert_eq!(li.language, language!("en"));
/// assert_eq!(li.script, None);
/// assert_eq!(li.region, Some(region!("US")));
/// assert_eq!(li.variants.len(), 0);
/// ```
///
/// # Parsing
///
/// Unicode recognizes three levels of standard conformance for any language identifier:
///
///  * *well-formed* - syntactically correct
///  * *valid* - well-formed and only uses registered language, region, script and variant subtags...
///  * *canonical* - valid and no deprecated codes or structure.
///
/// At the moment parsing normalizes a well-formed language identifier converting
/// `_` separators to `-` and adjusting casing to conform to the Unicode standard.
///
/// Any bogus subtags will cause the parsing to fail with an error.
/// No subtag validation is performed.
///
/// # Examples
///
/// ```
/// use icu::locid::{
///     langid,
///     subtags::{language, region, script, variant},
/// };
///
/// let li = langid!("eN_latn_Us-Valencia");
///
/// assert_eq!(li.language, language!("en"));
/// assert_eq!(li.script, Some(script!("Latn")));
/// assert_eq!(li.region, Some(region!("US")));
/// assert_eq!(li.variants.get(0), Some(&variant!("valencia")));
/// ```
///
/// [`Unicode BCP47 Language Identifier`]: https://unicode.org/reports/tr35/tr35.html#Unicode_language_identifier
#[derive(Default, PartialEq, Eq, Clone, Hash)]
#[allow(clippy::exhaustive_structs)] // This struct is stable (and invoked by a macro)
pub struct LanguageIdentifier {
    /// Language subtag of the language identifier.
    pub language: subtags::Language,
    /// Script subtag of the language identifier.
    pub script: Option<subtags::Script>,
    /// Region subtag of the language identifier.
    pub region: Option<subtags::Region>,
    /// Variant subtags of the language identifier.
    pub variants: subtags::Variants,
}

impl LanguageIdentifier {
    /// A constructor which takes a utf8 slice, parses it and
    /// produces a well-formed [`LanguageIdentifier`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    ///
    /// LanguageIdentifier::try_from_bytes(b"en-US").expect("Parsing failed");
    /// ```
    pub fn try_from_bytes(v: &[u8]) -> Result<Self, ParserError> {
        parse_language_identifier(v, ParserMode::LanguageIdentifier)
    }

    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    // The return type should be `Result<Self, ParserError>` once the `const_precise_live_drops`
    // is stabilized ([rust-lang#73255](https://github.com/rust-lang/rust/issues/73255)).
    pub const fn try_from_bytes_with_single_variant(
        v: &[u8],
    ) -> Result<
        (
            subtags::Language,
            Option<subtags::Script>,
            Option<subtags::Region>,
            Option<subtags::Variant>,
        ),
        ParserError,
    > {
        parse_language_identifier_with_single_variant(v, ParserMode::LanguageIdentifier)
    }

    /// A constructor which takes a utf8 slice which may contain extension keys,
    /// parses it and produces a well-formed [`LanguageIdentifier`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::{langid, LanguageIdentifier};
    ///
    /// let li = LanguageIdentifier::try_from_locale_bytes(b"en-US-x-posix")
    ///     .expect("Parsing failed.");
    ///
    /// assert_eq!(li, langid!("en-US"));
    /// ```
    ///
    /// This method should be used for input that may be a locale identifier.
    /// All extensions will be lost.
    pub fn try_from_locale_bytes(v: &[u8]) -> Result<Self, ParserError> {
        parse_language_identifier(v, ParserMode::Locale)
    }

    /// The default undefined language "und". Same as [`default()`](Default::default()).
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    ///
    /// assert_eq!(LanguageIdentifier::default(), LanguageIdentifier::UND);
    /// ```
    pub const UND: Self = Self {
        language: subtags::Language::UND,
        script: None,
        region: None,
        variants: subtags::Variants::new(),
    };

    /// This is a best-effort operation that performs all available levels of canonicalization.
    ///
    /// At the moment the operation will normalize casing and the separator, but in the future
    /// it may also validate and update from deprecated subtags to canonical ones.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    ///
    /// assert_eq!(
    ///     LanguageIdentifier::canonicalize("pL_latn_pl").as_deref(),
    ///     Ok("pl-Latn-PL")
    /// );
    /// ```
    pub fn canonicalize<S: AsRef<[u8]>>(input: S) -> Result<String, ParserError> {
        let lang_id = Self::try_from_bytes(input.as_ref())?;
        Ok(lang_id.write_to_string().into_owned())
    }

    /// Compare this [`LanguageIdentifier`] with BCP-47 bytes.
    ///
    /// The return value is equivalent to what would happen if you first converted this
    /// [`LanguageIdentifier`] to a BCP-47 string and then performed a byte comparison.
    ///
    /// This function is case-sensitive and results in a *total order*, so it is appropriate for
    /// binary search. The only argument producing [`Ordering::Equal`] is `self.to_string()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    /// use std::cmp::Ordering;
    ///
    /// let bcp47_strings: &[&str] = &[
    ///     "pl-Latn-PL",
    ///     "und",
    ///     "und-Adlm",
    ///     "und-GB",
    ///     "und-ZA",
    ///     "und-fonipa",
    ///     "zh",
    /// ];
    ///
    /// for ab in bcp47_strings.windows(2) {
    ///     let a = ab[0];
    ///     let b = ab[1];
    ///     assert!(a.cmp(b) == Ordering::Less);
    ///     let a_langid = a.parse::<LanguageIdentifier>().unwrap();
    ///     assert!(a_langid.strict_cmp(a.as_bytes()) == Ordering::Equal);
    ///     assert!(a_langid.strict_cmp(b.as_bytes()) == Ordering::Less);
    /// }
    /// ```
    pub fn strict_cmp(&self, other: &[u8]) -> Ordering {
        self.writeable_cmp_bytes(other)
    }

    pub(crate) fn as_tuple(
        &self,
    ) -> (
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
        &subtags::Variants,
    ) {
        (self.language, self.script, self.region, &self.variants)
    }

    /// Compare this [`LanguageIdentifier`] with another [`LanguageIdentifier`] field-by-field.
    /// The result is a total ordering sufficient for use in a [`BTreeMap`].
    ///
    /// Unlike [`Self::strict_cmp`], this function's ordering may not equal string ordering.
    ///
    /// [`BTreeMap`]: alloc::collections::BTreeMap
    pub fn total_cmp(&self, other: &Self) -> Ordering {
        self.as_tuple().cmp(&other.as_tuple())
    }

    /// Compare this [`LanguageIdentifier`] with an iterator of BCP-47 subtags.
    ///
    /// This function has the same equality semantics as [`LanguageIdentifier::strict_cmp`]. It is intended as
    /// a more modular version that allows multiple subtag iterators to be chained together.
    ///
    /// For an additional example, see [`SubtagOrderingResult`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    /// use std::cmp::Ordering;
    ///
    /// let subtags: &[&[u8]] = &[b"ca", b"ES", b"valencia"];
    ///
    /// let loc = "ca-ES-valencia".parse::<LanguageIdentifier>().unwrap();
    /// assert_eq!(
    ///     Ordering::Equal,
    ///     loc.strict_cmp_iter(subtags.iter().copied()).end()
    /// );
    ///
    /// let loc = "ca-ES".parse::<LanguageIdentifier>().unwrap();
    /// assert_eq!(
    ///     Ordering::Less,
    ///     loc.strict_cmp_iter(subtags.iter().copied()).end()
    /// );
    ///
    /// let loc = "ca-ZA".parse::<LanguageIdentifier>().unwrap();
    /// assert_eq!(
    ///     Ordering::Greater,
    ///     loc.strict_cmp_iter(subtags.iter().copied()).end()
    /// );
    /// ```
    #[deprecated(since = "1.5.0", note = "if you need this, please file an issue")]
    #[allow(deprecated)]
    pub fn strict_cmp_iter<'l, I>(&self, mut subtags: I) -> SubtagOrderingResult<I>
    where
        I: Iterator<Item = &'l [u8]>,
    {
        let r = self.for_each_subtag_str(&mut |subtag| {
            if let Some(other) = subtags.next() {
                match subtag.as_bytes().cmp(other) {
                    Ordering::Equal => Ok(()),
                    not_equal => Err(not_equal),
                }
            } else {
                Err(Ordering::Greater)
            }
        });
        match r {
            Ok(_) => SubtagOrderingResult::Subtags(subtags),
            Err(o) => SubtagOrderingResult::Ordering(o),
        }
    }

    /// Compare this `LanguageIdentifier` with a potentially unnormalized BCP-47 string.
    ///
    /// The return value is equivalent to what would happen if you first parsed the
    /// BCP-47 string to a `LanguageIdentifier` and then performed a structural comparison.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::LanguageIdentifier;
    ///
    /// let bcp47_strings: &[&str] = &[
    ///     "pl-LaTn-pL",
    ///     "uNd",
    ///     "UnD-adlm",
    ///     "uNd-GB",
    ///     "UND-FONIPA",
    ///     "ZH",
    /// ];
    ///
    /// for a in bcp47_strings {
    ///     assert!(a.parse::<LanguageIdentifier>().unwrap().normalizing_eq(a));
    /// }
    /// ```
    pub fn normalizing_eq(&self, other: &str) -> bool {
        macro_rules! subtag_matches {
            ($T:ty, $iter:ident, $expected:expr) => {
                $iter
                    .next()
                    .map(|b| <$T>::try_from_bytes(b) == Ok($expected))
                    .unwrap_or(false)
            };
        }

        let mut iter = SubtagIterator::new(other.as_bytes());
        if !subtag_matches!(subtags::Language, iter, self.language) {
            return false;
        }
        if let Some(ref script) = self.script {
            if !subtag_matches!(subtags::Script, iter, *script) {
                return false;
            }
        }
        if let Some(ref region) = self.region {
            if !subtag_matches!(subtags::Region, iter, *region) {
                return false;
            }
        }
        for variant in self.variants.iter() {
            if !subtag_matches!(subtags::Variant, iter, *variant) {
                return false;
            }
        }
        iter.next().is_none()
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        f(self.language.as_str())?;
        if let Some(ref script) = self.script {
            f(script.as_str())?;
        }
        if let Some(ref region) = self.region {
            f(region.as_str())?;
        }
        for variant in self.variants.iter() {
            f(variant.as_str())?;
        }
        Ok(())
    }

    /// Executes `f` on each subtag string of this `LanguageIdentifier`, with every string in
    /// lowercase ascii form.
    ///
    /// The default canonicalization of language identifiers uses titlecase scripts and uppercase
    /// regions. However, this differs from [RFC6497 (BCP 47 Extension T)], which specifies:
    ///
    /// > _The canonical form for all subtags in the extension is lowercase, with the fields
    /// ordered by the separators, alphabetically._
    ///
    /// Hence, this method is used inside [`Transform Extensions`] to be able to get the correct
    /// canonicalization of the language identifier.
    ///
    /// As an example, the canonical form of locale **EN-LATN-CA-T-EN-LATN-CA** is
    /// **en-Latn-CA-t-en-latn-ca**, with the script and region parts lowercased inside T extensions,
    /// but titlecased and uppercased outside T extensions respectively.
    ///
    /// [RFC6497 (BCP 47 Extension T)]: https://www.ietf.org/rfc/rfc6497.txt
    /// [`Transform extensions`]: crate::extensions::transform
    pub(crate) fn for_each_subtag_str_lowercased<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        f(self.language.as_str())?;
        if let Some(ref script) = self.script {
            f(script.into_tinystr().to_ascii_lowercase().as_str())?;
        }
        if let Some(ref region) = self.region {
            f(region.into_tinystr().to_ascii_lowercase().as_str())?;
        }
        for variant in self.variants.iter() {
            f(variant.as_str())?;
        }
        Ok(())
    }

    /// Writes this `LanguageIdentifier` to a sink, replacing uppercase ascii chars with
    /// lowercase ascii chars.
    ///
    /// The default canonicalization of language identifiers uses titlecase scripts and uppercase
    /// regions. However, this differs from [RFC6497 (BCP 47 Extension T)], which specifies:
    ///
    /// > _The canonical form for all subtags in the extension is lowercase, with the fields
    /// ordered by the separators, alphabetically._
    ///
    /// Hence, this method is used inside [`Transform Extensions`] to be able to get the correct
    /// canonicalization of the language identifier.
    ///
    /// As an example, the canonical form of locale **EN-LATN-CA-T-EN-LATN-CA** is
    /// **en-Latn-CA-t-en-latn-ca**, with the script and region parts lowercased inside T extensions,
    /// but titlecased and uppercased outside T extensions respectively.
    ///
    /// [RFC6497 (BCP 47 Extension T)]: https://www.ietf.org/rfc/rfc6497.txt
    /// [`Transform extensions`]: crate::extensions::transform
    pub(crate) fn write_lowercased_to<W: core::fmt::Write + ?Sized>(
        &self,
        sink: &mut W,
    ) -> core::fmt::Result {
        let mut initial = true;
        self.for_each_subtag_str_lowercased(&mut |subtag| {
            if initial {
                initial = false;
            } else {
                sink.write_char('-')?;
            }
            sink.write_str(subtag)
        })
    }
}

impl AsRef<LanguageIdentifier> for LanguageIdentifier {
    #[inline(always)]
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<LanguageIdentifier> for LanguageIdentifier {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl core::fmt::Debug for LanguageIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        core::fmt::Display::fmt(&self, f)
    }
}

impl FromStr for LanguageIdentifier {
    type Err = ParserError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::try_from_bytes(source.as_bytes())
    }
}

impl_writeable_for_each_subtag_str_no_test!(LanguageIdentifier, selff, selff.script.is_none() && selff.region.is_none() && selff.variants.is_empty() => selff.language.write_to_string());

#[test]
fn test_writeable() {
    use writeable::assert_writeable_eq;
    assert_writeable_eq!(LanguageIdentifier::UND, "und");
    assert_writeable_eq!("und-001".parse::<LanguageIdentifier>().unwrap(), "und-001");
    assert_writeable_eq!(
        "und-Mymr".parse::<LanguageIdentifier>().unwrap(),
        "und-Mymr",
    );
    assert_writeable_eq!(
        "my-Mymr-MM".parse::<LanguageIdentifier>().unwrap(),
        "my-Mymr-MM",
    );
    assert_writeable_eq!(
        "my-Mymr-MM-posix".parse::<LanguageIdentifier>().unwrap(),
        "my-Mymr-MM-posix",
    );
    assert_writeable_eq!(
        "zh-macos-posix".parse::<LanguageIdentifier>().unwrap(),
        "zh-macos-posix",
    );
}

/// # Examples
///
/// ```
/// use icu::locid::{langid, subtags::language, LanguageIdentifier};
///
/// assert_eq!(LanguageIdentifier::from(language!("en")), langid!("en"));
/// ```
impl From<subtags::Language> for LanguageIdentifier {
    fn from(language: subtags::Language) -> Self {
        Self {
            language,
            ..Default::default()
        }
    }
}

/// # Examples
///
/// ```
/// use icu::locid::{langid, subtags::script, LanguageIdentifier};
///
/// assert_eq!(
///     LanguageIdentifier::from(Some(script!("latn"))),
///     langid!("und-Latn")
/// );
/// ```
impl From<Option<subtags::Script>> for LanguageIdentifier {
    fn from(script: Option<subtags::Script>) -> Self {
        Self {
            script,
            ..Default::default()
        }
    }
}

/// # Examples
///
/// ```
/// use icu::locid::{langid, subtags::region, LanguageIdentifier};
///
/// assert_eq!(
///     LanguageIdentifier::from(Some(region!("US"))),
///     langid!("und-US")
/// );
/// ```
impl From<Option<subtags::Region>> for LanguageIdentifier {
    fn from(region: Option<subtags::Region>) -> Self {
        Self {
            region,
            ..Default::default()
        }
    }
}

/// Convert from an LSR tuple to a [`LanguageIdentifier`].
///
/// # Examples
///
/// ```
/// use icu::locid::{
///     langid,
///     subtags::{language, region, script},
///     LanguageIdentifier,
/// };
///
/// let lang = language!("en");
/// let script = script!("Latn");
/// let region = region!("US");
/// assert_eq!(
///     LanguageIdentifier::from((lang, Some(script), Some(region))),
///     langid!("en-Latn-US")
/// );
/// ```
impl
    From<(
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
    )> for LanguageIdentifier
{
    fn from(
        lsr: (
            subtags::Language,
            Option<subtags::Script>,
            Option<subtags::Region>,
        ),
    ) -> Self {
        Self {
            language: lsr.0,
            script: lsr.1,
            region: lsr.2,
            ..Default::default()
        }
    }
}

/// Convert from a [`LanguageIdentifier`] to an LSR tuple.
///
/// # Examples
///
/// ```
/// use icu::locid::{
///     langid,
///     subtags::{language, region, script},
/// };
///
/// let lid = langid!("en-Latn-US");
/// let (lang, script, region) = (&lid).into();
///
/// assert_eq!(lang, language!("en"));
/// assert_eq!(script, Some(script!("Latn")));
/// assert_eq!(region, Some(region!("US")));
/// ```
impl From<&LanguageIdentifier>
    for (
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
    )
{
    fn from(langid: &LanguageIdentifier) -> Self {
        (langid.language, langid.script, langid.region)
    }
}
