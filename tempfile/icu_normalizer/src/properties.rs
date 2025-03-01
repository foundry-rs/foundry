// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Access to the Unicode properties or property-based operations that
//! are required for NFC and NFD.
//!
//! Applications should generally use the full normalizers that are
//! provided at the top level of this crate. However, the APIs in this
//! module are provided for callers such as HarfBuzz that specifically
//! want access to the raw canonical composition operation e.g. for use in a
//! glyph-availability-guided custom normalizer.

use crate::char_from_u16;
use crate::error::NormalizerError;
use crate::in_inclusive_range;
use crate::provider::CanonicalCompositionsV1Marker;
use crate::provider::CanonicalDecompositionDataV1Marker;
use crate::provider::CanonicalDecompositionTablesV1Marker;
use crate::provider::NonRecursiveDecompositionSupplementV1Marker;
use crate::trie_value_has_ccc;
use crate::trie_value_indicates_special_non_starter_decomposition;
use crate::BACKWARD_COMBINING_STARTER_MARKER;
use crate::FDFA_MARKER;
use crate::HANGUL_L_BASE;
use crate::HANGUL_N_COUNT;
use crate::HANGUL_S_BASE;
use crate::HANGUL_S_COUNT;
use crate::HANGUL_T_BASE;
use crate::HANGUL_T_COUNT;
use crate::HANGUL_V_BASE;
use crate::NON_ROUND_TRIP_MARKER;
use crate::SPECIAL_NON_STARTER_DECOMPOSITION_MARKER_U16;
/// want access to the underlying properties e.g. for use in a
/// glyph-availability-guided custom normalizer.
use icu_properties::CanonicalCombiningClass;
use icu_provider::prelude::*;

/// The raw canonical composition operation.
///
/// Callers should generally use `ComposingNormalizer` instead of this API.
/// However, this API is provided for callers such as HarfBuzz that specifically
/// want access to the raw canonical composition operation e.g. for use in a
/// glyph-availability-guided custom normalizer.
#[derive(Debug)]
pub struct CanonicalComposition {
    canonical_compositions: DataPayload<CanonicalCompositionsV1Marker>,
}

#[cfg(feature = "compiled_data")]
impl Default for CanonicalComposition {
    fn default() -> Self {
        Self::new()
    }
}

impl CanonicalComposition {
    /// Performs canonical composition (including Hangul) on a pair of
    /// characters or returns `None` if these characters don't compose.
    /// Composition exclusions are taken into account.
    ///
    /// # Examples
    ///
    /// ```
    /// let comp = icu::normalizer::properties::CanonicalComposition::new();
    ///
    /// assert_eq!(comp.compose('a', 'b'), None); // Just two non-composing starters
    /// assert_eq!(comp.compose('a', '\u{0308}'), Some('ä'));
    /// assert_eq!(comp.compose('ẹ', '\u{0302}'), Some('ệ'));
    /// assert_eq!(comp.compose('𝅗', '𝅥'), None); // Composition exclusion
    /// assert_eq!(comp.compose('ে', 'া'), Some('ো')); // Second is starter
    /// assert_eq!(comp.compose('ᄀ', 'ᅡ'), Some('가')); // Hangul LV
    /// assert_eq!(comp.compose('가', 'ᆨ'), Some('각')); // Hangul LVT
    /// ```
    #[inline(always)]
    pub fn compose(&self, starter: char, second: char) -> Option<char> {
        crate::compose(
            self.canonical_compositions
                .get()
                .canonical_compositions
                .iter(),
            starter,
            second,
        )
    }

    /// Constructs a new `CanonicalComposition` using compiled data.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    pub const fn new() -> Self {
        Self {
            canonical_compositions: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_NORMALIZER_COMP_V1,
            ),
        }
    }

    icu_provider::gen_any_buffer_data_constructors!(locale: skip, options: skip, error: NormalizerError,
        #[cfg(skip)]
        functions: [
            new,
            try_new_with_any_provider,
            try_new_with_buffer_provider,
            try_new_unstable,
            Self,
        ]
    );

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<D>(provider: &D) -> Result<Self, NormalizerError>
    where
        D: DataProvider<CanonicalCompositionsV1Marker> + ?Sized,
    {
        let canonical_compositions: DataPayload<CanonicalCompositionsV1Marker> =
            provider.load(Default::default())?.take_payload()?;
        Ok(CanonicalComposition {
            canonical_compositions,
        })
    }
}

/// The outcome of non-recursive canonical decomposition of a character.
#[allow(clippy::exhaustive_enums)]
#[derive(Debug, PartialEq, Eq)]
pub enum Decomposed {
    /// The character is its own canonical decomposition.
    Default,
    /// The character decomposes to a single different character.
    Singleton(char),
    /// The character decomposes to two characters.
    Expansion(char, char),
}

/// The raw (non-recursive) canonical decomposition operation.
///
/// Callers should generally use `DecomposingNormalizer` instead of this API.
/// However, this API is provided for callers such as HarfBuzz that specifically
/// want access to non-recursive canonical decomposition e.g. for use in a
/// glyph-availability-guided custom normalizer.
#[derive(Debug)]
pub struct CanonicalDecomposition {
    decompositions: DataPayload<CanonicalDecompositionDataV1Marker>,
    tables: DataPayload<CanonicalDecompositionTablesV1Marker>,
    non_recursive: DataPayload<NonRecursiveDecompositionSupplementV1Marker>,
}

#[cfg(feature = "compiled_data")]
impl Default for CanonicalDecomposition {
    fn default() -> Self {
        Self::new()
    }
}

impl CanonicalDecomposition {
    /// Performs non-recursive canonical decomposition (including for Hangul).
    ///
    /// ```
    ///     use icu::normalizer::properties::Decomposed;
    ///     let decomp = icu::normalizer::properties::CanonicalDecomposition::new();
    ///
    ///     assert_eq!(decomp.decompose('e'), Decomposed::Default);
    ///     assert_eq!(
    ///         decomp.decompose('ệ'),
    ///         Decomposed::Expansion('ẹ', '\u{0302}')
    ///     );
    ///     assert_eq!(decomp.decompose('각'), Decomposed::Expansion('가', 'ᆨ'));
    ///     assert_eq!(decomp.decompose('\u{212B}'), Decomposed::Singleton('Å')); // ANGSTROM SIGN
    ///     assert_eq!(decomp.decompose('\u{2126}'), Decomposed::Singleton('Ω')); // OHM SIGN
    ///     assert_eq!(decomp.decompose('\u{1F71}'), Decomposed::Singleton('ά')); // oxia
    /// ```
    #[inline]
    pub fn decompose(&self, c: char) -> Decomposed {
        let lvt = u32::from(c).wrapping_sub(HANGUL_S_BASE);
        if lvt >= HANGUL_S_COUNT {
            return self.decompose_non_hangul(c);
        }
        let t = lvt % HANGUL_T_COUNT;
        if t == 0 {
            let l = lvt / HANGUL_N_COUNT;
            let v = (lvt % HANGUL_N_COUNT) / HANGUL_T_COUNT;
            // Safe because values known to be in range
            return Decomposed::Expansion(
                unsafe { char::from_u32_unchecked(HANGUL_L_BASE + l) },
                unsafe { char::from_u32_unchecked(HANGUL_V_BASE + v) },
            );
        }
        let lv = lvt - t;
        // Safe because values known to be in range
        Decomposed::Expansion(
            unsafe { char::from_u32_unchecked(HANGUL_S_BASE + lv) },
            unsafe { char::from_u32_unchecked(HANGUL_T_BASE + t) },
        )
    }

    /// Performs non-recursive canonical decomposition except Hangul syllables
    /// are reported as `Decomposed::Default`.
    #[inline(always)]
    fn decompose_non_hangul(&self, c: char) -> Decomposed {
        let decomposition = self.decompositions.get().trie.get(c);
        if decomposition <= BACKWARD_COMBINING_STARTER_MARKER {
            return Decomposed::Default;
        }
        // The loop is only broken out of as goto forward
        #[allow(clippy::never_loop)]
        loop {
            let trail_or_complex = (decomposition >> 16) as u16;
            let lead = decomposition as u16;
            if lead > NON_ROUND_TRIP_MARKER && trail_or_complex != 0 {
                // Decomposition into two BMP characters: starter and non-starter
                if in_inclusive_range(c, '\u{1F71}', '\u{1FFB}') {
                    // Look in the other trie due to oxia singleton
                    // mappings to corresponding character with tonos.
                    break;
                }
                return Decomposed::Expansion(char_from_u16(lead), char_from_u16(trail_or_complex));
            }
            if lead > NON_ROUND_TRIP_MARKER {
                // Decomposition into one BMP character or non-starter
                debug_assert_ne!(
                    lead, FDFA_MARKER,
                    "How come we got the U+FDFA NFKD marker here?"
                );
                if lead == SPECIAL_NON_STARTER_DECOMPOSITION_MARKER_U16 {
                    // Non-starter
                    if !in_inclusive_range(c, '\u{0340}', '\u{0F81}') {
                        return Decomposed::Default;
                    }
                    return match c {
                        '\u{0340}' => {
                            // COMBINING GRAVE TONE MARK
                            Decomposed::Singleton('\u{0300}')
                        }
                        '\u{0341}' => {
                            // COMBINING ACUTE TONE MARK
                            Decomposed::Singleton('\u{0301}')
                        }
                        '\u{0343}' => {
                            // COMBINING GREEK KORONIS
                            Decomposed::Singleton('\u{0313}')
                        }
                        '\u{0344}' => {
                            // COMBINING GREEK DIALYTIKA TONOS
                            Decomposed::Expansion('\u{0308}', '\u{0301}')
                        }
                        '\u{0F73}' => {
                            // TIBETAN VOWEL SIGN II
                            Decomposed::Expansion('\u{0F71}', '\u{0F72}')
                        }
                        '\u{0F75}' => {
                            // TIBETAN VOWEL SIGN UU
                            Decomposed::Expansion('\u{0F71}', '\u{0F74}')
                        }
                        '\u{0F81}' => {
                            // TIBETAN VOWEL SIGN REVERSED II
                            Decomposed::Expansion('\u{0F71}', '\u{0F80}')
                        }
                        _ => Decomposed::Default,
                    };
                }
                return Decomposed::Singleton(char_from_u16(lead));
            }
            // The recursive decomposition of ANGSTROM SIGN is in the complex
            // decomposition structure to avoid a branch in `potential_passthrough`
            // for the BMP case.
            if c == '\u{212B}' {
                // ANGSTROM SIGN
                return Decomposed::Singleton('\u{00C5}');
            }
            // Complex decomposition
            // Format for 16-bit value:
            // 15..13: length minus two for 16-bit case and length minus one for
            //         the 32-bit case. Length 8 needs to fit in three bits in
            //         the 16-bit case, and this way the value is future-proofed
            //         up to 9 in the 16-bit case. Zero is unused and length one
            //         in the 16-bit case goes directly into the trie.
            //     12: 1 if all trailing characters are guaranteed non-starters,
            //         0 if no guarantees about non-starterness.
            //         Note: The bit choice is this way around to allow for
            //         dynamically falling back to not having this but instead
            //         having one more bit for length by merely choosing
            //         different masks.
            //  11..0: Start offset in storage. The offset is to the logical
            //         sequence of scalars16, scalars32, supplementary_scalars16,
            //         supplementary_scalars32.
            let offset = usize::from(trail_or_complex & 0xFFF);
            let tables = self.tables.get();
            if offset < tables.scalars16.len() {
                if usize::from(trail_or_complex >> 13) != 0 {
                    // i.e. logical len isn't 2
                    break;
                }
                if let Some(first) = tables.scalars16.get(offset) {
                    if let Some(second) = tables.scalars16.get(offset + 1) {
                        // Two BMP starters
                        return Decomposed::Expansion(char_from_u16(first), char_from_u16(second));
                    }
                }
                // GIGO case
                debug_assert!(false);
                return Decomposed::Default;
            }
            let len = usize::from(trail_or_complex >> 13) + 1;
            if len > 2 {
                break;
            }
            let offset24 = offset - tables.scalars16.len();
            if let Some(first_c) = tables.scalars24.get(offset24) {
                if len == 1 {
                    if c != first_c {
                        return Decomposed::Singleton(first_c);
                    } else {
                        // Singleton representation used to avoid
                        // NFC passthrough of characters that combine
                        // with starters that can occur as the first
                        // character of an expansion decomposition.
                        // See section 5 of
                        // https://www.unicode.org/L2/L2024/24009-utc178-properties-recs.pdf
                        return Decomposed::Default;
                    }
                }
                if let Some(second_c) = tables.scalars24.get(offset24 + 1) {
                    return Decomposed::Expansion(first_c, second_c);
                }
            }
            // GIGO case
            debug_assert!(false);
            return Decomposed::Default;
        }
        let non_recursive = self.non_recursive.get();
        let non_recursive_decomposition = non_recursive.trie.get(c);
        if non_recursive_decomposition == 0 {
            // GIGO case
            debug_assert!(false);
            return Decomposed::Default;
        }
        let trail_or_complex = (non_recursive_decomposition >> 16) as u16;
        let lead = non_recursive_decomposition as u16;
        if lead != 0 && trail_or_complex != 0 {
            // Decomposition into two BMP characters
            return Decomposed::Expansion(char_from_u16(lead), char_from_u16(trail_or_complex));
        }
        if lead != 0 {
            // Decomposition into one BMP character
            return Decomposed::Singleton(char_from_u16(lead));
        }
        // Decomposition into two non-BMP characters
        // Low is offset into a table plus one to keep it non-zero.
        let offset = usize::from(trail_or_complex - 1);
        if let Some(first) = non_recursive.scalars24.get(offset) {
            if let Some(second) = non_recursive.scalars24.get(offset + 1) {
                return Decomposed::Expansion(first, second);
            }
        }
        // GIGO case
        debug_assert!(false);
        Decomposed::Default
    }

    /// Construct from compiled data.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    pub const fn new() -> Self {
        const _: () = assert!(
            crate::provider::Baked::SINGLETON_NORMALIZER_NFDEX_V1
                .scalars16
                .const_len()
                + crate::provider::Baked::SINGLETON_NORMALIZER_NFDEX_V1
                    .scalars24
                    .const_len()
                <= 0xFFF,
            "NormalizerError::FutureExtension"
        );

        Self {
            decompositions: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_NORMALIZER_NFD_V1,
            ),
            tables: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_NORMALIZER_NFDEX_V1,
            ),
            non_recursive: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_NORMALIZER_DECOMP_V1,
            ),
        }
    }

    icu_provider::gen_any_buffer_data_constructors!(locale: skip, options: skip, error: NormalizerError,
        #[cfg(skip)]
        functions: [
            new,
            try_new_with_any_provider,
            try_new_with_buffer_provider,
            try_new_unstable,
            Self,
        ]
    );

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<D>(provider: &D) -> Result<Self, NormalizerError>
    where
        D: DataProvider<CanonicalDecompositionDataV1Marker>
            + DataProvider<CanonicalDecompositionTablesV1Marker>
            + DataProvider<NonRecursiveDecompositionSupplementV1Marker>
            + ?Sized,
    {
        let decompositions: DataPayload<CanonicalDecompositionDataV1Marker> =
            provider.load(Default::default())?.take_payload()?;
        let tables: DataPayload<CanonicalDecompositionTablesV1Marker> =
            provider.load(Default::default())?.take_payload()?;

        if tables.get().scalars16.len() + tables.get().scalars24.len() > 0xFFF {
            // The data is from a future where there exists a normalization flavor whose
            // complex decompositions take more than 0xFFF but fewer than 0x1FFF code points
            // of space. If a good use case from such a decomposition flavor arises, we can
            // dynamically change the bit masks so that the length mask becomes 0x1FFF instead
            // of 0xFFF and the all-non-starters mask becomes 0 instead of 0x1000. However,
            // since for now the masks are hard-coded, error out.
            return Err(NormalizerError::FutureExtension);
        }

        let non_recursive: DataPayload<NonRecursiveDecompositionSupplementV1Marker> =
            provider.load(Default::default())?.take_payload()?;

        Ok(CanonicalDecomposition {
            decompositions,
            tables,
            non_recursive,
        })
    }
}

/// Lookup of the Canonical_Combining_Class Unicode property.
///
/// # Example
///
/// ```
/// use icu::properties::CanonicalCombiningClass;
/// use icu::normalizer::properties::CanonicalCombiningClassMap;
///
/// let map = CanonicalCombiningClassMap::new();
/// assert_eq!(map.get('a'), CanonicalCombiningClass::NotReordered); // U+0061: LATIN SMALL LETTER A
/// assert_eq!(map.get32(0x0301), CanonicalCombiningClass::Above); // U+0301: COMBINING ACUTE ACCENT
/// ```
#[derive(Debug)]
pub struct CanonicalCombiningClassMap {
    /// The data trie
    decompositions: DataPayload<CanonicalDecompositionDataV1Marker>,
}

#[cfg(feature = "compiled_data")]
impl Default for CanonicalCombiningClassMap {
    fn default() -> Self {
        Self::new()
    }
}

impl CanonicalCombiningClassMap {
    /// Look up the canonical combining class for a scalar value
    #[inline(always)]
    pub fn get(&self, c: char) -> CanonicalCombiningClass {
        self.get32(u32::from(c))
    }

    /// Look up the canonical combining class for a scalar value
    /// represented as `u32`. If the argument is outside the scalar
    /// value range, `CanonicalCombiningClass::NotReordered` is returned.
    pub fn get32(&self, c: u32) -> CanonicalCombiningClass {
        let trie_value = self.decompositions.get().trie.get32(c);
        if trie_value_has_ccc(trie_value) {
            CanonicalCombiningClass(trie_value as u8)
        } else if trie_value_indicates_special_non_starter_decomposition(trie_value) {
            match c {
                0x0340 | 0x0341 | 0x0343 | 0x0344 => CanonicalCombiningClass::Above,
                _ => CanonicalCombiningClass::NotReordered,
            }
        } else {
            CanonicalCombiningClass::NotReordered
        }
    }

    /// Construct from compiled data.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    pub const fn new() -> Self {
        CanonicalCombiningClassMap {
            decompositions: DataPayload::from_static_ref(
                crate::provider::Baked::SINGLETON_NORMALIZER_NFD_V1,
            ),
        }
    }

    icu_provider::gen_any_buffer_data_constructors!(locale: skip, options: skip, error: NormalizerError,
        #[cfg(skip)]
        functions: [
            new,
            try_new_with_any_provider,
            try_new_with_buffer_provider,
            try_new_unstable,
            Self,
    ]);

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<D>(provider: &D) -> Result<Self, NormalizerError>
    where
        D: DataProvider<CanonicalDecompositionDataV1Marker> + ?Sized,
    {
        let decompositions: DataPayload<CanonicalDecompositionDataV1Marker> =
            provider.load(Default::default())?.take_payload()?;
        Ok(CanonicalCombiningClassMap { decompositions })
    }
}
