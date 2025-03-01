// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! 🚧 \[Experimental\] This module is experimental and currently crate-private. Let us know if you
//! have a use case for this!
//!
//! This module contains utilities for working with properties where the specific property in use
//! is not known at compile time.
//!
//! For regex engines, [`crate::sets::load_for_ecma262_unstable()`] is a convenient API for working
//! with properties at runtime tailored for the use case of ECMA262-compatible regex engines.

#[cfg(doc)]
use super::{maps, script, GeneralCategory, GeneralCategoryGroup, Script};

/// This type can represent any Unicode property.
///
/// This is intended to be used in situations where the exact unicode property needed is
/// only known at runtime, for example in regex engines.
///
/// The values are intended to be identical to ICU4C's UProperty enum
#[allow(clippy::exhaustive_structs)] // newtype
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct UnicodeProperty(pub u32);

#[allow(non_upper_case_globals)]
#[allow(unused)] // experimental, may be made public later
impl UnicodeProperty {
    /// Binary property `Alphabetic`
    pub const Alphabetic: Self = UnicodeProperty(0);
    /// Binary property `ASCII_Hex_Digit`
    pub const AsciiHexDigit: Self = UnicodeProperty(1);
    /// Binary property `Bidi_Control`
    pub const BidiControl: Self = UnicodeProperty(2);
    /// Binary property `Bidi_Mirrored`
    pub const BidiMirrored: Self = UnicodeProperty(3);
    /// Binary property `Dash`
    pub const Dash: Self = UnicodeProperty(4);
    /// Binary property `Default_Ignorable_Code_Point`
    pub const DefaultIgnorableCodePoint: Self = UnicodeProperty(5);
    /// Binary property `Deprecated`
    pub const Deprecated: Self = UnicodeProperty(6);
    /// Binary property `Diacritic`
    pub const Diacritic: Self = UnicodeProperty(7);
    /// Binary property `Extender`
    pub const Extender: Self = UnicodeProperty(8);
    /// Binary property `Full_Composition_Exclusion`
    pub const FullCompositionExclusion: Self = UnicodeProperty(9);
    /// Binary property `Grapheme_Base`
    pub const GraphemeBase: Self = UnicodeProperty(10);
    /// Binary property `Grapheme_Extend`
    pub const GraphemeExtend: Self = UnicodeProperty(11);
    /// Binary property `Grapheme_Link`
    pub const GraphemeLink: Self = UnicodeProperty(12);
    /// Binary property `Hex_Digit`
    pub const HexDigit: Self = UnicodeProperty(13);
    /// Binary property `Hyphen`
    pub const Hyphen: Self = UnicodeProperty(14);
    /// Binary property `ID_Continue`
    pub const IdContinue: Self = UnicodeProperty(15);
    /// Binary property `ID_Start`
    pub const IdStart: Self = UnicodeProperty(16);
    /// Binary property `Ideographic`
    pub const Ideographic: Self = UnicodeProperty(17);
    /// Binary property `IDS_Binary_Operator`
    pub const IdsBinaryOperator: Self = UnicodeProperty(18);
    /// Binary property `IDS_Trinary_Operator`
    pub const IdsTrinaryOperator: Self = UnicodeProperty(19);
    /// Binary property `Join_Control`
    pub const JoinControl: Self = UnicodeProperty(20);
    /// Binary property `Logical_Order_Exception`
    pub const LogicalOrderException: Self = UnicodeProperty(21);
    /// Binary property `Lowercase`
    pub const Lowercase: Self = UnicodeProperty(22);
    /// Binary property `Math`
    pub const Math: Self = UnicodeProperty(23);
    /// Binary property `Noncharacter_Code_Point`
    pub const NoncharacterCodePoint: Self = UnicodeProperty(24);
    /// Binary property `Quotation_Mark`
    pub const QuotationMark: Self = UnicodeProperty(25);
    /// Binary property `Radical`
    pub const Radical: Self = UnicodeProperty(26);
    /// Binary property `Soft_Dotted`
    pub const SoftDotted: Self = UnicodeProperty(27);
    /// Binary property `Terminal_Punctuation`
    pub const TerminalPunctuation: Self = UnicodeProperty(28);
    /// Binary property `Unified_Ideograph`
    pub const UnifiedIdeograph: Self = UnicodeProperty(29);
    /// Binary property `Uppercase`
    pub const Uppercase: Self = UnicodeProperty(30);
    /// Binary property `White_Space`
    pub const WhiteSpace: Self = UnicodeProperty(31);
    /// Binary property `XID_Continue`
    pub const XidContinue: Self = UnicodeProperty(32);
    /// Binary property `XID_Start`
    pub const XidStart: Self = UnicodeProperty(33);
    /// Binary property `Case_Sensitive`
    pub const CaseSensitive: Self = UnicodeProperty(34);
    /// Binary property `Sentence_Terminal`
    pub const SentenceTerminal: Self = UnicodeProperty(35);
    /// Binary property `Variation_Selector`
    pub const VariationSelector: Self = UnicodeProperty(36);
    /// Binary property `NFD_Inert`
    pub const NfdInert: Self = UnicodeProperty(37);
    /// Binary property `NFKD_Inert`
    pub const NfkdInert: Self = UnicodeProperty(38);
    /// Binary property `NFC_Inert`
    pub const NfcInert: Self = UnicodeProperty(39);
    /// Binary property `NFKC_Inert`
    pub const NfkcInert: Self = UnicodeProperty(40);
    /// Binary property `Segment_Starter`
    pub const SegmentStarter: Self = UnicodeProperty(41);
    /// Binary property `Pattern_Syntax`
    pub const PatternSyntax: Self = UnicodeProperty(42);
    /// Binary property `Pattern_White_Space`
    pub const PatternWhiteSpace: Self = UnicodeProperty(43);
    /// Binary property `alnum`
    pub const Alnum: Self = UnicodeProperty(44);
    /// Binary property `blank`
    pub const Blank: Self = UnicodeProperty(45);
    /// Binary property `graph`
    pub const Graph: Self = UnicodeProperty(46);
    /// Binary property `print`
    pub const Print: Self = UnicodeProperty(47);
    /// Binary property `xdigit`
    pub const XDigit: Self = UnicodeProperty(48);
    /// Binary property `Cased`
    pub const Cased: Self = UnicodeProperty(49);
    /// Binary property `Case_Ignorable`
    pub const CaseIgnorable: Self = UnicodeProperty(50);
    /// Binary property `Changes_When_Lowercased`
    pub const ChangesWhenLowercased: Self = UnicodeProperty(51);
    /// Binary property `Changes_When_Uppercased`
    pub const ChangesWhenUppercased: Self = UnicodeProperty(52);
    /// Binary property `Changes_When_Titlecased`
    pub const ChangesWhenTitlecased: Self = UnicodeProperty(53);
    /// Binary property `Changes_When_Casefolded`
    pub const ChangesWhenCasefolded: Self = UnicodeProperty(54);
    /// Binary property `Changes_When_Casemapped`
    pub const ChangesWhenCasemapped: Self = UnicodeProperty(55);
    /// Binary property `Changes_When_NFKC_Casefolded`
    pub const ChangesWhenNfkcCasefolded: Self = UnicodeProperty(56);
    /// Binary property `Emoji`
    pub const Emoji: Self = UnicodeProperty(57);
    /// Binary property `Emoji_Presentation`
    pub const EmojiPresentation: Self = UnicodeProperty(58);
    /// Binary property `Emoji_Modifier`
    pub const EmojiModifier: Self = UnicodeProperty(59);
    /// Binary property `Emoji_Modifier_Base`
    pub const EmojiModifierBase: Self = UnicodeProperty(60);
    /// Binary property `Emoji_Component`
    pub const EmojiComponent: Self = UnicodeProperty(61);
    /// Binary property `Regional_Indicator`
    pub const RegionalIndicator: Self = UnicodeProperty(62);
    /// Binary property `Prepended_Concatenation_Mark`
    pub const PrependedConcatenationMark: Self = UnicodeProperty(63);
    /// Binary property `Extended_Pictographic`
    pub const ExtendedPictographic: Self = UnicodeProperty(64);
    /// Binary property `Basic_Emoji`
    pub const BasicEmoji: Self = UnicodeProperty(65);
    /// Binary property `Emoji_Keycap_Sequence`
    pub const EmojiKeycapSequence: Self = UnicodeProperty(66);
    /// Binary property `RGI_Emoji_Modifier_Sequence`
    pub const RgiEmojiModifierSequence: Self = UnicodeProperty(67);
    /// Binary property `RGI_Emoji_Flag_Sequence`
    pub const RgiEmojiFlagSequence: Self = UnicodeProperty(68);
    /// Binary property `RGI_Emoji_Tag_Sequence`
    pub const RgiEmojiTagSequence: Self = UnicodeProperty(69);
    /// Binary property `RGI_Emoji_ZWJ_Sequence`
    pub const RgiEmojiZWJSequence: Self = UnicodeProperty(70);
    /// Binary property `RGI_Emoji`
    pub const RgiEmoji: Self = UnicodeProperty(71);

    const BINARY_MAX: Self = Self::RgiEmoji;

    /// Enumerated property `Bidi_Class`
    pub const BidiClass: Self = UnicodeProperty(0x1000);
    /// Enumerated property `Block`
    pub const Block: Self = UnicodeProperty(0x1001);
    /// Enumerated property `Canonical_Combining_Class`
    pub const CombiningClass: Self = UnicodeProperty(0x1002);
    /// Enumerated property `Decomposition_Type`
    pub const DecompositionType: Self = UnicodeProperty(0x1003);
    /// Enumerated property `East_Asian_Width`
    pub const EastAsianWidth: Self = UnicodeProperty(0x1004);
    /// Enumerated property `General_Category`
    pub const GeneralCategory: Self = UnicodeProperty(0x1005);
    /// Enumerated property `Joining_Group`
    pub const JoiningGroup: Self = UnicodeProperty(0x1006);
    /// Enumerated property `Joining_Type`
    pub const JoiningType: Self = UnicodeProperty(0x1007);
    /// Enumerated property `Line_Break`
    pub const LineBreak: Self = UnicodeProperty(0x1008);
    /// Enumerated property `Numeric_Type`
    pub const NumericType: Self = UnicodeProperty(0x1009);
    /// Enumerated property `Script`
    pub const Script: Self = UnicodeProperty(0x100A);
    /// Enumerated property `Hangul_Syllable_Type`
    pub const HangulSyllableType: Self = UnicodeProperty(0x100B);
    /// Enumerated property `NFD_Quick_Check`
    pub const NFDQuickCheck: Self = UnicodeProperty(0x100C);
    /// Enumerated property `NFKD_Quick_Check`
    pub const NFKDQuickCheck: Self = UnicodeProperty(0x100D);
    /// Enumerated property `NFC_Quick_Check`
    pub const NFCQuickCheck: Self = UnicodeProperty(0x100E);
    /// Enumerated property `NFKC_Quick_Check`
    pub const NFKCQuickCheck: Self = UnicodeProperty(0x100F);
    /// Enumerated property `Lead_Canonical_Combining_Class`
    pub const LeadCanonicalCombiningClass: Self = UnicodeProperty(0x1010);
    /// Enumerated property `Trail_Canonical_Combining_Class`
    pub const TrailCanonicalCombiningClass: Self = UnicodeProperty(0x1011);
    /// Enumerated property `Grapheme_Cluster_Break`
    pub const GraphemeClusterBreak: Self = UnicodeProperty(0x1012);
    /// Enumerated property `Sentence_Break`
    pub const SentenceBreak: Self = UnicodeProperty(0x1013);
    /// Enumerated property `Word_Break`
    pub const WordBreak: Self = UnicodeProperty(0x1014);
    /// Enumerated property `Bidi_Paired_Bracket_Type`
    pub const BidiPairedBracketType: Self = UnicodeProperty(0x1015);
    /// Enumerated property `Indic_Positional_Category`
    pub const IndicPositionalCategory: Self = UnicodeProperty(0x1016);
    /// Enumerated property `Indic_Syllabic_Category`
    pub const IndicSyllabicCategory: Self = UnicodeProperty(0x1017);
    /// Enumerated property `Vertical_Orientation`
    pub const VerticalOrientation: Self = UnicodeProperty(0x1018);

    const ENUMERATED_MAX: Self = Self::VerticalOrientation;

    /// Mask property `General_Category_Mask`
    pub const GeneralCategoryMask: Self = UnicodeProperty(0x2000);

    /// Double property `Numeric_Value`
    pub const NumericValue: Self = UnicodeProperty(0x3000);

    /// String property `Age`
    pub const Age: Self = UnicodeProperty(0x4000);
    /// String property `Bidi_Mirroring_Glyph`
    pub const BidiMirroringGlyph: Self = UnicodeProperty(0x4001);
    /// String property `Case_Folding`
    pub const CaseFolding: Self = UnicodeProperty(0x4002);
    /// String property `ISO_Comment`
    pub const ISOComment: Self = UnicodeProperty(0x4003);
    /// String property `Lowercase_Mapping`
    pub const LowercaseMapping: Self = UnicodeProperty(0x4004);
    /// String property `Name`
    pub const Name: Self = UnicodeProperty(0x4005);
    /// String property `Simple_Case_Folding`
    pub const SimpleCaseFolding: Self = UnicodeProperty(0x4006);
    /// String property `Simple_Lowercase_Mapping`
    pub const SimpleLowercaseMapping: Self = UnicodeProperty(0x4007);
    /// String property `Simple_Titlecase_Mapping`
    pub const SimpleTitlecaseMapping: Self = UnicodeProperty(0x4008);
    /// String property `Simple_Uppercase_Mapping`
    pub const SimpleUppercaseMapping: Self = UnicodeProperty(0x4009);
    /// String property `Titlecase_Mapping`
    pub const TitlecaseMapping: Self = UnicodeProperty(0x400A);
    /// String property `Unicode_1_Name`
    pub const Unicode1_Name: Self = UnicodeProperty(0x400B);
    /// String property `Uppercase_Mapping`
    pub const UppercaseMapping: Self = UnicodeProperty(0x400C);
    /// String property `Bidi_Paired_Bracket`
    pub const BidiPairedBracket: Self = UnicodeProperty(0x400D);

    const STRING_MAX: Self = Self::BidiPairedBracket;

    /// Misc property `Script_Extensions`
    pub const ScriptExtensions: Self = UnicodeProperty(0x7000);
}

#[allow(unused)] // experimental, may be made public later
impl UnicodeProperty {
    /// Given a property name (long, short, or alias), returns the corresponding [`UnicodeProperty`]
    /// value for it provided it belongs to the [subset relevant for ECMA262 regexes][subset]
    ///
    /// Returns none if the name does not match any of the names in this subset. Performs
    /// strict matching of names.
    ///
    /// If using this to implement an ECMA262-compliant regex engine, please note these caveats:
    ///
    /// - This only returns binary and enumerated properties, as well as [`Self::ScriptExtensions`].
    ///   Lookup can be performed sufficiently with [`Self::load_ecma262_binary_property_unstable()`],
    ///   [`maps::load_general_category()`], [`maps::load_script()`]  and [`script::load_script_with_extensions_unstable()`].
    /// - This does not handle the `Any`, `Assigned`, or `ASCII` pseudoproperties, since they are not
    ///   defined as properties.
    ///    - `Any` can be expressed as the range `[\u{0}-\u{10FFFF}]`
    ///    - `Assigned` can be expressed as the inverse of the set `gc=Cn` (i.e., `\P{gc=Cn}`).
    ///    - `ASCII` can be expressed as the range `[\u{0}-\u{7F}]`
    /// - ECMA262 regexes transparently allow `General_Category_Mask` values for `GeneralCategory`.
    ///   This method does not return [`Self::GeneralCategoryMask`], and instead relies on the caller to use mask-related lookup
    ///   functions where necessary.
    /// - ECMA262 regexes allow treating `General_Category` (and `gcm`) values as binary properties,
    ///   e.g. you can do things like `\p{Lu}` as shortform for `\p{gc=Lu}`. This method does not do so
    ///   since these are property values, not properties, but you can use
    ///   [`GeneralCategory::get_name_to_enum_mapper()`] or  [`GeneralCategoryGroup::get_name_to_enum_mapper()`]
    ///   to handle this.
    ///
    ///
    /// [subset]: https://tc39.es/ecma262/#table-nonbinary-unicode-properties
    pub fn parse_ecma262_name(name: &str) -> Option<Self> {
        let prop = match name {
            "General_Category" | "gc" => Self::GeneralCategory,
            "Script" | "sc" => Self::Script,
            "Script_Extensions" | "scx" => Self::ScriptExtensions,
            "ASCII_Hex_Digit" | "AHex" => Self::AsciiHexDigit,
            "Alphabetic" | "Alpha" => Self::Alphabetic,
            "Bidi_Control" | "Bidi_C" => Self::BidiControl,
            "Bidi_Mirrored" | "Bidi_M" => Self::BidiMirrored,
            "Case_Ignorable" | "CI" => Self::CaseIgnorable,
            "Cased" => Self::Cased,
            "Changes_When_Casefolded" | "CWCF" => Self::ChangesWhenCasefolded,
            "Changes_When_Casemapped" | "CWCM" => Self::ChangesWhenCasemapped,
            "Changes_When_Lowercased" | "CWL" => Self::ChangesWhenLowercased,
            "Changes_When_NFKC_Casefolded" | "CWKCF" => Self::ChangesWhenNfkcCasefolded,
            "Changes_When_Titlecased" | "CWT" => Self::ChangesWhenTitlecased,
            "Changes_When_Uppercased" | "CWU" => Self::ChangesWhenUppercased,
            "Dash" => Self::Dash,
            "Default_Ignorable_Code_Point" | "DI" => Self::DefaultIgnorableCodePoint,
            "Deprecated" | "Dep" => Self::Deprecated,
            "Diacritic" | "Dia" => Self::Diacritic,
            "Emoji" => Self::Emoji,
            "Emoji_Component" | "EComp" => Self::EmojiComponent,
            "Emoji_Modifier" | "EMod" => Self::EmojiModifier,
            "Emoji_Modifier_Base" | "EBase" => Self::EmojiModifierBase,
            "Emoji_Presentation" | "EPres" => Self::EmojiPresentation,
            "Extended_Pictographic" | "ExtPict" => Self::ExtendedPictographic,
            "Extender" | "Ext" => Self::Extender,
            "Grapheme_Base" | "Gr_Base" => Self::GraphemeBase,
            "Grapheme_Extend" | "Gr_Ext" => Self::GraphemeExtend,
            "Hex_Digit" | "Hex" => Self::HexDigit,
            "IDS_Binary_Operator" | "IDSB" => Self::IdsBinaryOperator,
            "IDS_Trinary_Operator" | "IDST" => Self::IdsTrinaryOperator,
            "ID_Continue" | "IDC" => Self::IdContinue,
            "ID_Start" | "IDS" => Self::IdStart,
            "Ideographic" | "Ideo" => Self::Ideographic,
            "Join_Control" | "Join_C" => Self::JoinControl,
            "Logical_Order_Exception" | "LOE" => Self::LogicalOrderException,
            "Lowercase" | "Lower" => Self::Lowercase,
            "Math" => Self::Math,
            "Noncharacter_Code_Point" | "NChar" => Self::NoncharacterCodePoint,
            "Pattern_Syntax" | "Pat_Syn" => Self::PatternSyntax,
            "Pattern_White_Space" | "Pat_WS" => Self::PatternWhiteSpace,
            "Quotation_Mark" | "QMark" => Self::QuotationMark,
            "Radical" => Self::Radical,
            "Regional_Indicator" | "RI" => Self::RegionalIndicator,
            "Sentence_Terminal" | "STerm" => Self::SentenceTerminal,
            "Soft_Dotted" | "SD" => Self::SoftDotted,
            "Terminal_Punctuation" | "Term" => Self::TerminalPunctuation,
            "Unified_Ideograph" | "UIdeo" => Self::UnifiedIdeograph,
            "Uppercase" | "Upper" => Self::Uppercase,
            "Variation_Selector" | "VS" => Self::VariationSelector,
            "White_Space" | "space" => Self::WhiteSpace,
            "XID_Continue" | "XIDC" => Self::XidContinue,
            "XID_Start" | "XIDS" => Self::XidStart,
            _ => return None,
        };

        Some(prop)
    }
}
