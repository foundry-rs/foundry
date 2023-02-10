use ethers_core::{types::H160, utils::to_checksum};
use forge_fmt::solang_ext::{AsStr, AttrSortKeyIteratorExt, Operator, OperatorComponents};
use itertools::Itertools;
use solang_parser::pt::{Expression, FunctionAttribute, IdentifierPath, Loc, Parameter, Type};
use std::str::FromStr;

/// Trait for rendering parse tree items as strings.
#[auto_impl::auto_impl(&)]
pub trait AsString {
    /// Render parse tree item as string.
    fn as_string(&self) -> String;
}

impl AsString for Expression {
    fn as_string(&self) -> String {
        match self {
            Expression::Type(_, ty) => match ty {
                Type::Address => "address".to_owned(),
                Type::AddressPayable => "address payable".to_owned(),
                Type::Payable => "payable".to_owned(),
                Type::Bool => "bool".to_owned(),
                Type::String => "string".to_owned(),
                Type::Bytes(n) => format!("bytes{n}"),
                Type::Rational => "rational".to_owned(),
                Type::DynamicBytes => "bytes".to_owned(),
                Type::Int(n) => format!("int{n}"),
                Type::Uint(n) => format!("uint{n}"),
                Type::Mapping { key, key_name, value, value_name, .. } => {
                    let mut key = key.as_string();
                    if let Some(name) = key_name {
                        key.push(' ');
                        key.push_str(&name.to_string());
                    }
                    let mut value = value.as_string();
                    if let Some(name) = value_name {
                        value.push(' ');
                        value.push_str(&name.to_string());
                    }
                    format!("mapping({key} => {value})")
                }
                Type::Function { params, attributes, returns } => {
                    let params = params.as_string();
                    let mut attributes = attributes.as_string();
                    if !attributes.is_empty() {
                        attributes.insert(0, ' ');
                    }
                    let mut returns_str = String::new();
                    if let Some((params, _attrs)) = returns {
                        returns_str = params.as_string();
                        if !returns_str.is_empty() {
                            returns_str = format!(" returns ({returns_str})")
                        }
                    }
                    format!("function ({params}){attributes}{returns_str}")
                }
            },
            Expression::Variable(ident) => ident.name.to_owned(),
            Expression::ArraySubscript(_, expr1, expr2) => format!(
                "{}[{}]",
                expr1.as_string(),
                expr2.as_ref().map(|expr| expr.as_string()).unwrap_or_default()
            ),
            Expression::MemberAccess(_, expr, ident) => {
                format!("{}.{}", ident.name, expr.as_string())
            }
            Expression::Parenthesis(_, expr) => {
                format!("({})", expr.as_string())
            }
            Expression::HexNumberLiteral(_, val) => {
                // ref: https://docs.soliditylang.org/en/latest/types.html?highlight=address%20literal#address-literals
                if val.len() == 42 {
                    to_checksum(&H160::from_str(val).expect(""), None)
                } else {
                    val.to_owned()
                }
            }
            Expression::NumberLiteral(_, val, exp) => {
                let mut val = val.replace('_', "");
                if !exp.is_empty() {
                    val.push_str(&format!("e{}", exp.replace('_', "")));
                }
                val
            }
            Expression::StringLiteral(vals) => vals
                .iter()
                .map(|val| {
                    format!("{}\"{}\"", if val.unicode { "unicode" } else { "" }, val.string)
                })
                .join(" "),
            Expression::BoolLiteral(_, bool) => {
                let val = if *bool { "true" } else { "false" };
                val.to_owned()
            }
            Expression::HexLiteral(vals) => {
                vals.iter().map(|val| format!("hex\"{}\"", val.hex)).join(" ")
            }
            Expression::ArrayLiteral(_, exprs) => {
                format!("[{}]", exprs.iter().map(AsString::as_string).join(", "))
            }
            Expression::RationalNumberLiteral(_, val, fraction, exp) => {
                let mut val = val.replace('_', "");
                if val.is_empty() {
                    val = "0".to_owned();
                }

                let mut fraction = fraction.trim_end_matches('0').to_owned();
                if fraction.is_empty() {
                    fraction.push('0')
                }
                val.push_str(&format!(".{fraction}"));

                if !exp.is_empty() {
                    val.push_str(&format!("e{}", exp.replace('_', "")));
                }
                val
            }
            Expression::FunctionCall(_, expr, exprs) => {
                format!(
                    "{}({})",
                    expr.as_string(),
                    exprs.iter().map(AsString::as_string).join(", ")
                )
            }
            Expression::Unit(_, expr, unit) => {
                format!("{} {}", expr.as_string(), unit.as_str())
            }
            Expression::PreIncrement(..) |
            Expression::PostIncrement(..) |
            Expression::PreDecrement(..) |
            Expression::PostDecrement(..) |
            Expression::Not(..) |
            Expression::Complement(..) |
            Expression::UnaryPlus(..) |
            Expression::Add(..) |
            Expression::UnaryMinus(..) |
            Expression::Subtract(..) |
            Expression::Power(..) |
            Expression::Multiply(..) |
            Expression::Divide(..) |
            Expression::Modulo(..) |
            Expression::ShiftLeft(..) |
            Expression::ShiftRight(..) |
            Expression::BitwiseAnd(..) |
            Expression::BitwiseXor(..) |
            Expression::BitwiseOr(..) |
            Expression::Less(..) |
            Expression::More(..) |
            Expression::LessEqual(..) |
            Expression::MoreEqual(..) |
            Expression::And(..) |
            Expression::Or(..) |
            Expression::Equal(..) |
            Expression::NotEqual(..) => {
                let spaced = self.has_space_around();

                let (left, right) = self.components();

                let mut val = String::from(self.operator().unwrap());
                if let Some(left) = left {
                    if spaced {
                        val.insert(0, ' ');
                    }
                    val.insert_str(0, &left.as_string());
                }
                if let Some(right) = right {
                    if spaced {
                        val.push(' ');
                    }
                    val.push_str(&right.as_string())
                }

                val
            }
            item => {
                panic!("Attempted to format unsupported item: {item:?}")
            }
        }
    }
}

impl AsString for Parameter {
    fn as_string(&self) -> String {
        [
            Some(self.ty.as_string()),
            self.storage.as_ref().map(|storage| storage.to_string()),
            self.name.as_ref().map(|name| name.name.clone()),
        ]
        .into_iter()
        .flatten()
        .join(" ")
    }
}

impl AsString for Vec<(Loc, Option<Parameter>)> {
    fn as_string(&self) -> String {
        self.iter()
            .map(|(_, param)| param.as_ref().map(AsString::as_string).unwrap_or_default())
            .join(", ")
    }
}

impl AsString for Vec<FunctionAttribute> {
    fn as_string(&self) -> String {
        self.iter().attr_sorted().map(|attr| attr.as_string()).join(" ")
    }
}

impl AsString for FunctionAttribute {
    fn as_string(&self) -> String {
        match self {
            Self::Mutability(mutability) => mutability.to_string(),
            Self::Visibility(visibility) => visibility.to_string(),
            Self::Virtual(_) => "virtual".to_owned(),
            Self::Immutable(_) => "immutable".to_owned(),
            Self::Override(_, idents) => {
                let mut val = "override".to_owned();
                if !idents.is_empty() {
                    val.push_str(&format!(
                        "({})",
                        idents.iter().map(AsString::as_string).join(", ")
                    ));
                }
                val
            }
            Self::BaseOrModifier(_, base) => {
                base.name.identifiers.iter().map(|i| i.name.to_owned()).join(".")
            }
            Self::Error(_) => unreachable!(),
        }
    }
}

impl AsString for IdentifierPath {
    fn as_string(&self) -> String {
        self.identifiers.iter().map(|ident| ident.name.to_owned()).join(".")
    }
}
