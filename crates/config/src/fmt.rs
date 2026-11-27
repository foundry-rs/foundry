//! Configuration specific to the `forge fmt` command and the `forge_fmt` package

use serde::{Deserialize, Serialize};

/// Contains the config and rule set
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatterConfig {
    /// Maximum line length where the formatter will try to wrap the line.
    pub line_length: usize,
    /// Number of spaces per indentation level.
    ///
    /// When `style = "tab"`, each indentation level is written as one tab, and a tab counts as
    /// `tab_width` columns when the formatter measures lines against `line_length`.
    pub tab_width: usize,
    /// Indentation style.
    ///
    /// - `space`: indent with spaces, `tab_width` per level.
    /// - `tab`: indent with tabs, one per level.
    pub style: IndentStyle,
    /// Whether to print spaces between brackets.
    ///
    /// When enabled, the formatter writes `{ a, b }` instead of `{a, b}` in braced lists such as
    /// import lists.
    pub bracket_spacing: bool,
    /// Style of `uint256`/`int256` types.
    ///
    /// - `long`: use the explicit `uint256` or `int256`.
    /// - `short`: use the implicit alias `uint` or `int`.
    /// - `preserve`: use the type as written in the source code.
    pub int_types: IntTypes,
    /// Style of a function header that doesn't fit on one line.
    ///
    /// - `attributes_first`: write the function attributes multiline first.
    /// - `params_always`: always write the function parameters multiline. `params_first` is
    ///   accepted as an alias.
    /// - `params_first_multi`: write the function parameters multiline first when there is more
    ///   than one parameter.
    /// - `all`: if the function parameters or attributes are multiline, write everything
    ///   multiline.
    /// - `all_params`: same as `all`, but writes the function parameters multiline even when there
    ///   is a single parameter.
    ///
    /// For example, with `params_always`:
    ///
    /// ```solidity
    /// function myFunction(
    ///     uint256 param1,
    ///     uint256 param2,
    ///     uint256 param3
    /// ) public returns (uint256) {
    ///     // ...
    /// }
    /// ```
    ///
    /// And with `all`:
    ///
    /// ```solidity
    /// function myFunction(
    ///     uint256 param1,
    ///     uint256 param2,
    ///     uint256 param3
    /// )
    ///     public
    ///     returns (uint256)
    /// {
    ///     // ...
    /// }
    /// ```
    pub multiline_func_header: MultilineFuncHeaderStyle,
    /// Style of quotation marks around string literals.
    ///
    /// - `double`: use double quotes (`"`) where possible.
    /// - `single`: use single quotes (`'`) where possible.
    /// - `preserve`: use the quotation mark written in the source code.
    ///
    /// A literal keeps its other quotation mark when switching would change its contents, for
    /// example a string that contains the preferred quote character.
    pub quote_style: QuoteStyle,
    /// Style of underscores in number literals.
    ///
    /// - `preserve`: keep the underscores written in the source code.
    /// - `thousands`: add an underscore every thousand, if the number is greater than 9999, so
    ///   `1000` stays `1000` and `10000` becomes `10_000`.
    /// - `remove`: remove all underscores.
    ///
    /// Thousands separators are only added to decimal numbers, never to hex numbers or to
    /// literals in inline assembly, where underscores are always removed.
    pub number_underscore: NumberUnderscore,
    /// Style of underscores in hex string literals (`hex"..."`).
    ///
    /// - `preserve`: keep the underscores written in the source code.
    /// - `remove`: remove all underscores.
    /// - `bytes`: add an underscore between bytes, so `hex"deadbeef"` becomes `hex"de_ad_be_ef"`.
    pub hex_underscore: HexUnderscore,
    /// Style of single-line blocks in statements.
    ///
    /// - `preserve`: keep the existing single-line or multiline formatting of statement blocks.
    /// - `single`: format statement blocks on a single line where possible.
    /// - `multi`: always format statement blocks on multiple lines.
    ///
    /// For example, with `single`:
    ///
    /// ```solidity
    /// if (true) { return true; }
    /// ```
    ///
    /// With `multi`:
    ///
    /// ```solidity
    /// if (true) {
    ///     return true;
    /// }
    /// ```
    pub single_line_statement_blocks: SingleLineBlockStyle,
    /// Whether to print a space in the `override` attribute of state variables, functions and
    /// modifiers.
    ///
    /// When enabled:
    ///
    /// ```solidity
    /// function foo() public override (Parent) {}
    /// ```
    ///
    /// When disabled:
    ///
    /// ```solidity
    /// function foo() public override(Parent) {}
    /// ```
    pub override_spacing: bool,
    /// Whether to wrap comments that exceed `line_length`.
    pub wrap_comments: bool,
    /// Style of doc comments.
    ///
    /// - `preserve`: keep the style written in the source code.
    /// - `line`: use single-line style (`///`).
    /// - `block`: use block style (`/** ... */`).
    pub docs_style: DocCommentStyle,
    /// Glob patterns of files to skip when formatting.
    ///
    /// Patterns are resolved relative to the project root.
    ///
    /// ```toml
    /// [fmt]
    /// ignore = ["src/vendor/**/*.sol", "lib/**/*.sol"]
    /// ```
    pub ignore: Vec<String>,
    /// Whether to add an empty line at the start and end of contract bodies.
    ///
    /// When enabled:
    ///
    /// ```solidity
    /// contract MyContract {
    ///
    ///     uint256 x;
    ///
    /// }
    /// ```
    pub contract_new_lines: bool,
    /// Whether to sort import statements alphabetically by path within their groups.
    ///
    /// A group is a run of consecutive imports; a blank line or a comment between imports starts a
    /// new group. Names inside an import list (`import {b, a} from "x";`) are sorted too.
    pub sort_imports: bool,
    /// Style of imports that bring a whole file into a namespace.
    ///
    /// - `prefer_plain`: prefer `import "source" as name;`.
    /// - `prefer_glob`: prefer `import * as name from "source";`.
    /// - `preserve`: keep the style written in the source code.
    pub namespace_import_style: NamespaceImportStyle,
    /// Whether to remove the spaces around the power operator (`**`).
    ///
    /// When enabled:
    ///
    /// ```solidity
    /// uint256 result = base**exponent;
    /// ```
    ///
    /// When disabled:
    ///
    /// ```solidity
    /// uint256 result = base ** exponent;
    /// ```
    pub pow_no_space: bool,
    /// Which lists keep their elements together on their own line before breaking each element
    /// onto its own line.
    ///
    /// A list that doesn't fit on one line is either broken compactly (all elements together on
    /// the next line, if they fit) or consistently (one element per line).
    ///
    /// - `all`: all lists are preferred compact.
    /// - `none`: all lists break consistently.
    /// - `calls`: calls are preferred compact; events and errors break consistently.
    /// - `events`: events are preferred compact; calls and errors break consistently.
    /// - `errors`: errors are preferred compact; calls and events break consistently.
    /// - `events_errors`: events and errors are preferred compact; calls break consistently.
    pub prefer_compact: PreferCompact,
    /// Whether to keep an import of a single name on one line even if it exceeds `line_length`.
    ///
    /// Applies to imports like `import {Name} from "path";`.
    pub single_line_imports: bool,
}

/// Style of integer types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntTypes {
    /// Use the type defined in the source code.
    Preserve,
    /// Print the full length `uint256` or `int256`.
    #[default]
    Long,
    /// Print the alias `uint` or `int`.
    Short,
}

/// Style of underscores in number literals
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberUnderscore {
    /// Use the underscores defined in the source code
    #[default]
    Preserve,
    /// Remove all underscores
    Remove,
    /// Add an underscore every thousand, if greater than 9999
    /// e.g. 1000 -> 1000 and 10000 -> 10_000
    Thousands,
}

impl NumberUnderscore {
    /// Returns true if the option is `Preserve`
    #[inline]
    pub const fn is_preserve(self) -> bool {
        matches!(self, Self::Preserve)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub const fn is_remove(self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub const fn is_thousands(self) -> bool {
        matches!(self, Self::Thousands)
    }
}

/// Style of underscores in hex literals
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexUnderscore {
    /// Use the underscores defined in the source code
    Preserve,
    /// Remove all underscores
    #[default]
    Remove,
    /// Add underscore as separator between byte boundaries
    Bytes,
}

/// Style of doc comments
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocCommentStyle {
    /// Preserve the source code style
    #[default]
    Preserve,
    /// Use single-line style (`///`)
    Line,
    /// Use block style (`/** .. */`)
    Block,
}

/// Style of string quotes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
    /// Use quotation mark defined in the source code.
    Preserve,
    /// Use double quotes where possible.
    #[default]
    Double,
    /// Use single quotes where possible.
    Single,
}

impl QuoteStyle {
    /// Returns the associated quotation mark character.
    pub const fn quote(self) -> Option<char> {
        match self {
            Self::Preserve => None,
            Self::Double => Some('"'),
            Self::Single => Some('\''),
        }
    }
}

/// Style of single line blocks in statements
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleLineBlockStyle {
    /// Preserve the original style
    #[default]
    Preserve,
    /// Prefer single line block when possible
    Single,
    /// Always use multiline block
    Multi,
}

/// Style of function header in case it doesn't fit
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultilineFuncHeaderStyle {
    /// Always write function parameters multiline.
    #[serde(alias = "params_first")] // alias for backwards compatibility
    ParamsAlways,
    /// Write function parameters multiline first when there is more than one param.
    ParamsFirstMulti,
    /// Write function attributes multiline first.
    #[default]
    AttributesFirst,
    /// If function params or attrs are multiline.
    /// split the rest
    All,
    /// Same as `All` but writes function params multiline even when there is a single param.
    AllParams,
}

impl MultilineFuncHeaderStyle {
    pub const fn all(&self) -> bool {
        matches!(self, Self::All | Self::AllParams)
    }

    pub const fn params_first(&self) -> bool {
        matches!(self, Self::ParamsAlways | Self::ParamsFirstMulti)
    }

    pub const fn attrib_first(&self) -> bool {
        matches!(self, Self::AttributesFirst)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceImportStyle {
    /// prefer plain imports: `import "source" as name;`
    #[default]
    PreferPlain,
    /// prefer glob imports: `import * as name from "source";`
    PreferGlob,
    /// preserve the original style
    Preserve,
}

/// Style that determines if a broken list, should keep its elements together on their own line,
/// before breaking individually.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferCompact {
    /// All elements are preferred consistent.
    None,
    /// Calls are preferred compact. Events and errors break consistently.
    Calls,
    /// Events are preferred compact. Calls and errors break consistently.
    Events,
    /// Errors are preferred compact. Calls and events break consistently.
    Errors,
    /// Events and errors are preferred compact. Calls break consistently.
    EventsErrors,
    /// All elements are preferred compact.
    #[default]
    All,
}

impl PreferCompact {
    pub const fn calls(&self) -> bool {
        matches!(self, Self::All | Self::Calls)
    }

    pub const fn events(&self) -> bool {
        matches!(self, Self::All | Self::Events | Self::EventsErrors)
    }

    pub const fn errors(&self) -> bool {
        matches!(self, Self::All | Self::Errors | Self::EventsErrors)
    }
}

/// Style of indent
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndentStyle {
    /// Indent with spaces.
    #[default]
    Space,
    /// Indent with tabs.
    Tab,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            line_length: 120,
            tab_width: 4,
            style: IndentStyle::Space,
            bracket_spacing: false,
            int_types: IntTypes::default(),
            multiline_func_header: MultilineFuncHeaderStyle::default(),
            quote_style: QuoteStyle::default(),
            number_underscore: NumberUnderscore::default(),
            hex_underscore: HexUnderscore::default(),
            single_line_statement_blocks: SingleLineBlockStyle::default(),
            override_spacing: false,
            wrap_comments: false,
            ignore: vec![],
            contract_new_lines: false,
            sort_imports: false,
            namespace_import_style: NamespaceImportStyle::default(),
            pow_no_space: false,
            prefer_compact: PreferCompact::default(),
            docs_style: DocCommentStyle::default(),
            single_line_imports: false,
        }
    }
}
