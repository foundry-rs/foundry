// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
use super::visitor::AssignVarTypes;
use serde::{Deserialize, Serialize};
use solar::{
    interface::BytePos,
    parse::ast::{BinOpKind, LitKind, Span, StrKind, UnOpKind},
};
use std::{fmt::Display, path::PathBuf};

/// Wraps an unary operator mutated, to easily store pre/post-fix op swaps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnaryOpMutated {
    /// String containing the whole new expression (operator and its target)
    /// eg `a++`
    new_expression: String,

    /// The underlying operator used by this mutant
    #[serde(serialize_with = "serialize_unop_kind", deserialize_with = "deserialize_unop_kind")]
    pub resulting_op_kind: UnOpKind,
}

// Custom serialization for UnOpKind
fn serialize_unop_kind<S>(value: &UnOpKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = format!("{value:?}");
    serializer.serialize_str(&s)
}

fn deserialize_unop_kind<'de, D>(deserializer: D) -> Result<UnOpKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
        "PreInc" => Ok(UnOpKind::PreInc),
        "PostInc" => Ok(UnOpKind::PostInc),
        "PreDec" => Ok(UnOpKind::PreDec),
        "PostDec" => Ok(UnOpKind::PostDec),
        "Not" => Ok(UnOpKind::Not),
        "BitNot" => Ok(UnOpKind::BitNot),
        "Neg" => Ok(UnOpKind::Neg),
        other => Err(serde::de::Error::custom(format!("Unknown UnOpKind: {other}"))),
    }
}

impl UnaryOpMutated {
    pub fn new(new_expression: String, resulting_op_kind: UnOpKind) -> Self {
        Self { new_expression, resulting_op_kind }
    }
}

impl Display for UnaryOpMutated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.new_expression)
    }
}

// Custom serialization for BinOpKind
fn serialize_binop<S>(value: &BinOpKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = format!("{value:?}");
    serializer.serialize_str(&s)
}

fn deserialize_binop<'de, D>(deserializer: D) -> Result<BinOpKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
        "Add" => Ok(BinOpKind::Add),
        "Sub" => Ok(BinOpKind::Sub),
        "Mul" => Ok(BinOpKind::Mul),
        "Div" => Ok(BinOpKind::Div),
        "And" => Ok(BinOpKind::And),
        "Or" => Ok(BinOpKind::Or),
        "Eq" => Ok(BinOpKind::Eq),
        "Ne" => Ok(BinOpKind::Ne),
        "Lt" => Ok(BinOpKind::Lt),
        "Le" => Ok(BinOpKind::Le),
        "Gt" => Ok(BinOpKind::Gt),
        "Ge" => Ok(BinOpKind::Ge),
        "BitAnd" => Ok(BinOpKind::BitAnd),
        "BitOr" => Ok(BinOpKind::BitOr),
        "BitXor" => Ok(BinOpKind::BitXor),
        "Shl" => Ok(BinOpKind::Shl),
        "Shr" => Ok(BinOpKind::Shr),
        "Sar" => Ok(BinOpKind::Sar),
        other => Err(serde::de::Error::custom(format!("Unknown BinOpKind: {other}"))),
    }
}

// @todo add a mutation from universalmutator: line swap (swap two lines of code, as it
// could theoretically uncover untested reentrancies
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum OwnedStrKind {
    Str,
    Unicode,
    Hex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OwnedLiteral {
    Str { kind: OwnedStrKind, text: String },
    Number(alloy_primitives::U256),
    Rational(String),
    Address(String),
    Bool(bool),
    Err(String),
}

impl From<&LitKind<'_>> for OwnedLiteral {
    fn from(lit_kind: &LitKind<'_>) -> Self {
        match lit_kind {
            LitKind::Bool(b) => Self::Bool(*b),
            LitKind::Number(n) => Self::Number(*n),
            LitKind::Rational(r) => Self::Rational(r.to_string()),
            LitKind::Address(addr) => Self::Address(addr.to_string()),
            LitKind::Str(sk, bytesym, _extras) => {
                let text = String::from_utf8_lossy(bytesym.as_byte_str()).into_owned();
                let kind = match sk {
                    StrKind::Str => OwnedStrKind::Str,
                    StrKind::Unicode => OwnedStrKind::Unicode,
                    StrKind::Hex => OwnedStrKind::Hex,
                };
                Self::Str { kind, text }
            }
            LitKind::Err(_) => Self::Err("parse_error".to_string()),
        }
    }
}

impl Display for OwnedLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(val) => write!(f, "{val}"),
            Self::Number(val) => write!(f, "{val}"),
            Self::Rational(s) => write!(f, "{s}"),
            Self::Address(s) => write!(f, "{s}"),
            Self::Str { kind, text } => match kind {
                OwnedStrKind::Str => write!(f, "\"{text}\""),
                OwnedStrKind::Unicode => write!(f, "unicode\"{text}\""),
                OwnedStrKind::Hex => write!(f, "hex\"{text}\""),
            },
            Self::Err(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MutationType {
    // @todo Solar doesn't differentiate numeric type in LitKind (only on declaration?) -> for
    // now, planket and let solc filter out the invalid mutants -> we might/should add a
    // hashtable of the var to their underlying type (signed or not) so we avoid *a lot* of
    // invalid mutants
    /// For an initializer x, of type
    /// bool: replace x with !x
    /// uint: replace x with 0
    /// int: replace x with 0; replace x with -x (temp: this is mutated for uint as well)
    ///
    /// For a binary op y: apply BinaryOp(y)
    Assignment(AssignVarTypes),

    /// For a binary op y in BinOpKind ("+", "-", ">=", etc)
    /// replace y with each non-y in op (legacy, kept for cache compatibility)
    #[serde(serialize_with = "serialize_binop", deserialize_with = "deserialize_binop")]
    BinaryOp(BinOpKind),

    /// Binary operator mutation with full expression context
    /// Stores both the new operator and the full mutated expression for display
    BinaryOpExpr {
        #[serde(serialize_with = "serialize_binop", deserialize_with = "deserialize_binop")]
        new_op: BinOpKind,
        mutated_expr: String,
    },

    /// For a delete expr x `delete foo`, replace x with `assert(true)`
    DeleteExpression,

    /// replace "delegatecall" with "call"
    ElimDelegate,

    /// Gambit doesn't implement nor define it?
    FunctionCall,

    // /// For a if(x) condition x:
    // /// replace x with true; replace x with false
    // This mutation is not used anymore, as we mutate the condition as an expression,
    // which will creates true/false mutant as well as more complex conditions (eg if(foo++ >
    // --bar) ) IfStatementMutation,
    /// For a require(x) condition:
    /// replace x with true; replace x with false
    // Same as for IfStatementMutation, the expression inside the require is mutated as an
    // expression to handle increment etc
    Require,

    // @todo review if needed -> this might creates *a lot* of combinations for super-polyadic fn
    // tho       only swapping same type (to avoid obvious compilation failure), but should
    // take into account       implicit casting too...
    /// For 2 args of the same type x,y in a function args:
    /// swap(x, y)
    SwapArgumentsFunction,

    // @todo same remark as above, might end up in a space too big to explore + filtering out
    // based on type
    /// For an expr taking 2 expression x, y (x+y, x-y, x = x + ...):
    /// swap(x, y)
    SwapArgumentsOperator,

    /// For an unary operator x in UnOpKind (eg "++", "--", "~", "!"):
    /// replace x with all other operator in op
    /// Pre or post- are different UnOp
    UnaryOperator(UnaryOpMutated),
}

impl Display for MutationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Assignment(kind) => match kind {
                AssignVarTypes::Literal(lit) => write!(f, "{lit}"),
                AssignVarTypes::Identifier(ident) => write!(f, "{ident}"),
            },
            Self::BinaryOp(kind) => write!(f, "{}", kind.to_str()),
            Self::BinaryOpExpr { mutated_expr, .. } => write!(f, "{mutated_expr}"),
            Self::DeleteExpression => write!(f, "assert(true)"),
            Self::ElimDelegate => write!(f, "call"),
            Self::UnaryOperator(mutated) => write!(f, "{mutated}"),

            Self::FunctionCall
            | Self::Require
            | Self::SwapArgumentsFunction
            | Self::SwapArgumentsOperator => write!(f, ""),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MutationResult {
    Dead,
    Alive,
    Invalid,
    Skipped,
}

/// A given mutation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutant {
    /// The path to the project root where this mutant (tries to) live
    pub path: PathBuf,
    #[serde(serialize_with = "serialize_span", deserialize_with = "deserialize_span")]
    pub span: Span,
    pub mutation: MutationType,
    /// The original source text that will be replaced by this mutation
    #[serde(default)]
    pub original: String,
    /// The full source line for context (e.g., "uint256 x = a * b;")
    #[serde(default)]
    pub source_line: String,
    /// Line number in the source file (1-indexed)
    #[serde(default)]
    pub line_number: usize,
}

// Custom serialization for Span (since solar::parse::ast::Span doesn't implement Serialize)
fn serialize_span<S>(span: &Span, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    #[derive(Serialize)]
    struct SpanHelper {
        lo: u32,
        hi: u32,
    }
    SpanHelper { lo: span.lo().0, hi: span.hi().0 }.serialize(serializer)
}

fn deserialize_span<'de, D>(deserializer: D) -> Result<Span, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct SpanHelper {
        lo: u32,
        hi: u32,
    }
    let helper = SpanHelper::deserialize(deserializer)?;
    Ok(Span::new(BytePos(helper.lo), BytePos(helper.hi)))
}

impl Mutant {
    /// Returns a relative path string (strips common prefixes like absolute paths)
    pub fn relative_path(&self) -> String {
        let path_str = self.path.display().to_string();
        // Try to find src/, test/, or lib/ and show from there
        for prefix in ["src/", "test/", "lib/", "contracts/"] {
            if let Some(idx) = path_str.find(prefix) {
                return path_str[idx..].to_string();
            }
        }
        // Fallback to just filename
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string()
    }

    /// Compute line number from byte position given the source content
    pub fn line_number(&self, source: &str) -> usize {
        let pos = self.span.lo().0 as usize;
        source.get(..pos).map(|s| s.lines().count()).unwrap_or(1).max(1)
    }

    /// Returns a formatted Solidity diff showing the mutation.
    /// Format:
    /// ```
    /// - original_code
    /// + mutated_code
    /// ```
    pub fn format_diff(&self) -> String {
        let original =
            if self.original.is_empty() { "<unknown>".to_string() } else { self.original.clone() };
        let mutated = self.mutation.to_string();

        format!("- {}\n+ {}", original.trim(), mutated.trim())
    }

    /// Returns a concise one-line description of the mutation (full original code)
    pub fn short_description(&self) -> String {
        let original = if self.original.is_empty() {
            "<unknown>".to_string()
        } else {
            self.original.trim().to_string()
        };
        let mutated = self.mutation.to_string();

        format!("`{}` → `{}`", original, mutated.trim())
    }

    /// Returns a description with line context
    pub fn description_with_context(&self) -> String {
        let mutation_desc = self.short_description();
        if self.source_line.is_empty() {
            mutation_desc
        } else {
            format!("{}\n     in: {}", mutation_desc, self.source_line)
        }
    }
}

impl Display for Mutant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line_number > 0 {
            write!(f, "{}:{}: {}", self.relative_path(), self.line_number, self.short_description())
        } else {
            write!(f, "{}: {}", self.relative_path(), self.short_description())
        }
    }
}
