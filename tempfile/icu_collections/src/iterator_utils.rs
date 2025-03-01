// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::codepointtrie::CodePointMapRange;

/// This is an iterator that coalesces adjacent ranges in an iterator over code
/// point ranges
pub(crate) struct RangeListIteratorCoalescer<I, T> {
    iter: I,
    peek: Option<CodePointMapRange<T>>,
}

impl<I, T: Eq> RangeListIteratorCoalescer<I, T>
where
    I: Iterator<Item = CodePointMapRange<T>>,
{
    pub fn new(iter: I) -> Self {
        Self { iter, peek: None }
    }
}

impl<I, T: Eq> Iterator for RangeListIteratorCoalescer<I, T>
where
    I: Iterator<Item = CodePointMapRange<T>>,
{
    type Item = CodePointMapRange<T>;

    fn next(&mut self) -> Option<Self::Item> {
        // Get the initial range we're working with: either a leftover
        // range from last time, or the next range
        let mut ret = if let Some(peek) = self.peek.take() {
            peek
        } else if let Some(next) = self.iter.next() {
            next
        } else {
            // No ranges, exit early
            return None;
        };

        // Keep pulling ranges
        #[allow(clippy::while_let_on_iterator)]
        // can't move the iterator, also we want it to be explicit that we're not draining the iterator
        while let Some(next) = self.iter.next() {
            if *next.range.start() == ret.range.end() + 1 && next.value == ret.value {
                // Range has no gap, coalesce
                ret.range = *ret.range.start()..=*next.range.end();
            } else {
                // Range has a gap, return what we have so far, update
                // peek
                self.peek = Some(next);
                return Some(ret);
            }
        }

        // Ran out of elements, exit
        Some(ret)
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Debug;
    use icu::collections::codepointinvlist::CodePointInversionListBuilder;
    use icu::collections::codepointtrie::TrieValue;
    use icu::properties::maps::{self, CodePointMapDataBorrowed};
    use icu::properties::sets::{self, CodePointSetDataBorrowed};
    use icu::properties::{GeneralCategory, Script};

    fn test_set(data: CodePointSetDataBorrowed<'static>, name: &str) {
        let mut builder = CodePointInversionListBuilder::new();
        let mut builder_complement = CodePointInversionListBuilder::new();

        for range in data.iter_ranges() {
            builder.add_range32(&range)
        }

        for range in data.iter_ranges_complemented() {
            builder_complement.add_range32(&range)
        }

        builder.complement();
        let set1 = builder.build();
        let set2 = builder_complement.build();
        assert_eq!(set1, set2, "Set {name} failed to complement correctly");
    }

    fn test_map<T: TrieValue + Debug>(
        data: &CodePointMapDataBorrowed<'static, T>,
        value: T,
        name: &str,
    ) {
        let mut builder = CodePointInversionListBuilder::new();
        let mut builder_complement = CodePointInversionListBuilder::new();

        for range in data.iter_ranges_for_value(value) {
            builder.add_range32(&range)
        }

        for range in data.iter_ranges_for_value_complemented(value) {
            builder_complement.add_range32(&range)
        }

        builder.complement();
        let set1 = builder.build();
        let set2 = builder_complement.build();
        assert_eq!(
            set1, set2,
            "Map {name} failed to complement correctly with value {value:?}"
        );
    }

    #[test]
    fn test_complement_sets() {
        // Stress test the RangeListIteratorComplementer logic by ensuring it works for
        // a whole bunch of binary properties
        test_set(sets::ascii_hex_digit(), "ASCII_Hex_Digit");
        test_set(sets::alnum(), "Alnum");
        test_set(sets::alphabetic(), "Alphabetic");
        test_set(sets::bidi_control(), "Bidi_Control");
        test_set(sets::bidi_mirrored(), "Bidi_Mirrored");
        test_set(sets::blank(), "Blank");
        test_set(sets::cased(), "Cased");
        test_set(sets::case_ignorable(), "Case_Ignorable");
        test_set(
            sets::full_composition_exclusion(),
            "Full_Composition_Exclusion",
        );
        test_set(sets::changes_when_casefolded(), "Changes_When_Casefolded");
        test_set(sets::changes_when_casemapped(), "Changes_When_Casemapped");
        test_set(
            sets::changes_when_nfkc_casefolded(),
            "Changes_When_NFKC_Casefolded",
        );
        test_set(sets::changes_when_lowercased(), "Changes_When_Lowercased");
        test_set(sets::changes_when_titlecased(), "Changes_When_Titlecased");
        test_set(sets::changes_when_uppercased(), "Changes_When_Uppercased");
        test_set(sets::dash(), "Dash");
        test_set(sets::deprecated(), "Deprecated");
        test_set(
            sets::default_ignorable_code_point(),
            "Default_Ignorable_Code_Point",
        );
        test_set(sets::diacritic(), "Diacritic");
        test_set(sets::emoji_modifier_base(), "Emoji_Modifier_Base");
        test_set(sets::emoji_component(), "Emoji_Component");
        test_set(sets::emoji_modifier(), "Emoji_Modifier");
        test_set(sets::emoji(), "Emoji");
        test_set(sets::emoji_presentation(), "Emoji_Presentation");
        test_set(sets::extender(), "Extender");
        test_set(sets::extended_pictographic(), "Extended_Pictographic");
        test_set(sets::graph(), "Graph");
        test_set(sets::grapheme_base(), "Grapheme_Base");
        test_set(sets::grapheme_extend(), "Grapheme_Extend");
        test_set(sets::grapheme_link(), "Grapheme_Link");
        test_set(sets::hex_digit(), "Hex_Digit");
        test_set(sets::hyphen(), "Hyphen");
        test_set(sets::id_continue(), "Id_Continue");
        test_set(sets::ideographic(), "Ideographic");
        test_set(sets::id_start(), "Id_Start");
        test_set(sets::ids_binary_operator(), "Ids_Binary_Operator");
        test_set(sets::ids_trinary_operator(), "Ids_Trinary_Operator");
        test_set(sets::join_control(), "Join_Control");
        test_set(sets::logical_order_exception(), "Logical_Order_Exception");
        test_set(sets::lowercase(), "Lowercase");
        test_set(sets::math(), "Math");
        test_set(sets::noncharacter_code_point(), "Noncharacter_Code_Point");
        test_set(sets::nfc_inert(), "NFC_Inert");
        test_set(sets::nfd_inert(), "NFD_Inert");
        test_set(sets::nfkc_inert(), "NFKC_Inert");
        test_set(sets::nfkd_inert(), "NFKD_Inert");
        test_set(sets::pattern_syntax(), "Pattern_Syntax");
        test_set(sets::pattern_white_space(), "Pattern_White_Space");
        test_set(
            sets::prepended_concatenation_mark(),
            "Prepended_Concatenation_Mark",
        );
        test_set(sets::print(), "Print");
        test_set(sets::quotation_mark(), "Quotation_Mark");
        test_set(sets::radical(), "Radical");
        test_set(sets::regional_indicator(), "Regional_Indicator");
        test_set(sets::soft_dotted(), "Soft_Dotted");
        test_set(sets::segment_starter(), "Segment_Starter");
        test_set(sets::case_sensitive(), "Case_Sensitive");
        test_set(sets::sentence_terminal(), "Sentence_Terminal");
        test_set(sets::terminal_punctuation(), "Terminal_Punctuation");
        test_set(sets::unified_ideograph(), "Unified_Ideograph");
        test_set(sets::uppercase(), "Uppercase");
        test_set(sets::variation_selector(), "Variation_Selector");
        test_set(sets::white_space(), "White_Space");
        test_set(sets::xdigit(), "Xdigit");
        test_set(sets::xid_continue(), "XID_Continue");
        test_set(sets::xid_start(), "XID_Start");
    }

    #[test]
    fn test_complement_maps() {
        let gc = maps::general_category();
        let script = maps::script();
        test_map(&gc, GeneralCategory::UppercaseLetter, "gc");
        test_map(&gc, GeneralCategory::OtherPunctuation, "gc");
        test_map(&script, Script::Devanagari, "script");
        test_map(&script, Script::Latin, "script");
        test_map(&script, Script::Common, "script");
    }
}
