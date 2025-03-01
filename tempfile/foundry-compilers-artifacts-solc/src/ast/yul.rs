use super::{macros::node_group, misc::SourceLocation};
use crate::serde_helpers;
use serde::{Deserialize, Serialize};

node_group! {
    YulStatement;

    YulAssignment,
    YulBlock,
    YulBreak,
    YulContinue,
    YulExpressionStatement,
    YulLeave,
    YulForLoop,
    YulFunctionDefinition,
    YulIf,
    YulSwitch,
    YulVariableDeclaration,
}

node_group! {
    YulExpression;

    YulFunctionCall,
    YulIdentifier,
    YulLiteral,
}

/// A Yul block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulBlock {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub statements: Vec<YulStatement>,
}

/// A Yul assignment statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulAssignment {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub value: YulExpression,
    pub variable_names: Vec<YulIdentifier>,
}

/// A Yul function call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulFunctionCall {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub arguments: Vec<YulExpression>,
    pub function_name: YulIdentifier,
}

/// A Yul identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulIdentifier {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub name: String,
}

/// A literal Yul value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulLiteral {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub hex_value: Option<String>, // TODO
    pub value: Option<String>,     // TODO
    pub kind: YulLiteralKind,
    pub type_name: Option<String>, // TODO
}

/// Yul literal value kinds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum YulLiteralKind {
    /// A number literal.
    Number,
    /// A string literal.
    String,
    /// A boolean literal.
    Bool,
}

/// A Yul keyword.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulKeyword {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
}

/// The Yul break keyword.
pub type YulBreak = YulKeyword;
/// The Yul continue keyword.
pub type YulContinue = YulKeyword;
/// The Yul leave keyword.
pub type YulLeave = YulKeyword;

/// A Yul expression statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulExpressionStatement {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub expression: YulExpression,
}

/// A Yul for loop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulForLoop {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub body: YulBlock,
    pub condition: YulExpression,
    pub post: YulBlock,
    pub pre: YulBlock,
}

/// A Yul function definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulFunctionDefinition {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub body: YulBlock,
    pub name: String,
    #[serde(default)]
    pub parameters: Vec<YulTypedName>,
    #[serde(default)]
    pub return_variables: Vec<YulTypedName>,
}

/// A Yul type name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulTypedName {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String, // TODO
}

/// A Yul if statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulIf {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub body: YulBlock,
    pub condition: YulExpression,
}

/// A Yul switch statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulSwitch {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub cases: Vec<YulCase>,
    pub expression: YulExpression,
}

/// A Yul switch statement case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulCase {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub body: YulBlock,
    pub value: YulCaseValue,
}

/// A Yul switch case value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YulCaseValue {
    /// A case defined by a literal value.
    YulLiteral(YulLiteral),
    /// The default case
    // TODO: How do we make this only match "default"?
    Default(String),
}

/// A Yul variable declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YulVariableDeclaration {
    #[serde(with = "serde_helpers::display_from_str")]
    pub src: SourceLocation,
    pub value: Option<YulExpression>,
    pub variables: Vec<YulTypedName>,
}
