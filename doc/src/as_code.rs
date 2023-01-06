use forge_fmt::solang_ext::AttrSortKeyIteratorExt;
use itertools::Itertools;
use solang_parser::pt::{
    EnumDefinition, ErrorDefinition, ErrorParameter, EventDefinition, EventParameter, Expression,
    FunctionAttribute, FunctionDefinition, Identifier, IdentifierPath, Loc, Parameter,
    StructDefinition, Type, VariableAttribute, VariableDeclaration, VariableDefinition,
};

/// Display Solidity parse tree unit as code string.
/// [AsCode::as_code] formats the unit into
/// a valid Solidity code block.
#[auto_impl::auto_impl(&)]
pub(crate) trait AsCode {
    fn as_code(&self) -> String;
}

impl AsCode for VariableDefinition {
    fn as_code(&self) -> String {
        let ty = self.ty.as_code();
        let mut attrs = self.attrs.iter().attr_sorted().map(|attr| attr.as_code()).join(" ");
        if !attrs.is_empty() {
            attrs.insert(0, ' ');
        }
        let name = self.name.name.to_owned();
        format!("{ty}{attrs} {name}")
    }
}

impl AsCode for VariableAttribute {
    fn as_code(&self) -> String {
        match self {
            VariableAttribute::Visibility(visibility) => visibility.to_string(),
            VariableAttribute::Constant(_) => "constant".to_owned(),
            VariableAttribute::Immutable(_) => "immutable".to_owned(),
            VariableAttribute::Override(_, idents) => {
                format!("override({})", idents.iter().map(AsCode::as_code).join(", "))
            }
        }
    }
}

impl AsCode for FunctionDefinition {
    fn as_code(&self) -> String {
        let ty = self.ty.to_string();
        let name = self.name.as_ref().map(|n| format!(" {}", n.name)).unwrap_or_default();
        let params = self.params.as_code();
        let mut attributes = self.attributes.as_code();
        if !attributes.is_empty() {
            attributes.insert(0, ' ');
        }
        let mut returns = self.returns.as_code();
        if !returns.is_empty() {
            returns = format!(" returns ({returns})")
        }
        format!("{ty}{name}({params}){attributes}{returns}")
    }
}

impl AsCode for Expression {
    fn as_code(&self) -> String {
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
                Type::Mapping(_, from, to) => {
                    format!("mapping({} => {})", from.as_code(), to.as_code())
                }
                Type::Function { params, attributes, returns } => {
                    let params = params.as_code();
                    let mut attributes = attributes.as_code();
                    if !attributes.is_empty() {
                        attributes.insert(0, ' ');
                    }
                    let mut returns_str = String::new();
                    if let Some((params, _attrs)) = returns {
                        returns_str = params.as_code();
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
                expr1.as_code(),
                expr2.as_ref().map(|expr| expr.as_code()).unwrap_or_default()
            ),
            Expression::MemberAccess(_, expr, ident) => {
                format!("{}.{}", ident.name, expr.as_code())
            }
            item => {
                panic!("Attempted to format unsupported item: {item:?}")
            }
        }
    }
}

impl AsCode for FunctionAttribute {
    fn as_code(&self) -> String {
        match self {
            Self::Mutability(mutability) => mutability.to_string(),
            Self::Visibility(visibility) => visibility.to_string(),
            Self::Virtual(_) => "virtual".to_owned(),
            Self::Immutable(_) => "immutable".to_owned(),
            Self::Override(_, idents) => {
                format!("override({})", idents.iter().map(AsCode::as_code).join(", "))
            }
            Self::BaseOrModifier(_, _base) => "".to_owned(), // TODO:
            Self::NameValue(..) => unreachable!(),
        }
    }
}

impl AsCode for Parameter {
    fn as_code(&self) -> String {
        [
            Some(self.ty.as_code()),
            self.storage.as_ref().map(|storage| storage.to_string()),
            self.name.as_ref().map(|name| name.name.clone()),
        ]
        .into_iter()
        .flatten()
        .join(" ")
    }
}

impl AsCode for Vec<(Loc, Option<Parameter>)> {
    fn as_code(&self) -> String {
        self.iter()
            .map(|(_, param)| param.as_ref().map(AsCode::as_code).unwrap_or_default())
            .join(", ")
    }
}

impl AsCode for Vec<FunctionAttribute> {
    fn as_code(&self) -> String {
        self.iter().attr_sorted().map(|attr| attr.as_code()).join(" ")
    }
}

impl AsCode for EventDefinition {
    fn as_code(&self) -> String {
        let name = &self.name.name;
        let fields = self.fields.as_code();
        let anonymous = if self.anonymous { " anonymous" } else { "" };
        format!("event {name}({fields}){anonymous}")
    }
}

impl AsCode for EventParameter {
    fn as_code(&self) -> String {
        let ty = self.ty.as_code();
        let indexed = if self.indexed { " indexed" } else { "" };
        let name = self.name.as_ref().map(|name| name.name.to_owned()).unwrap_or_default();
        format!("{ty}{indexed} {name}")
    }
}

impl AsCode for Vec<EventParameter> {
    fn as_code(&self) -> String {
        self.iter().map(AsCode::as_code).join(", ")
    }
}

impl AsCode for ErrorDefinition {
    fn as_code(&self) -> String {
        let name = &self.name.name;
        let fields = self.fields.as_code();
        format!("error {name}({fields})")
    }
}

impl AsCode for ErrorParameter {
    fn as_code(&self) -> String {
        let ty = self.ty.as_code();
        let name = self.name.as_ref().map(|name| name.name.to_owned()).unwrap_or_default();
        format!("{ty} {name}")
    }
}

impl AsCode for Vec<ErrorParameter> {
    fn as_code(&self) -> String {
        self.iter().map(AsCode::as_code).join(", ")
    }
}

impl AsCode for IdentifierPath {
    fn as_code(&self) -> String {
        self.identifiers.iter().map(|ident| ident.name.to_owned()).join(".")
    }
}

impl AsCode for StructDefinition {
    fn as_code(&self) -> String {
        let name = &self.name.name;
        let fields = self.fields.iter().map(AsCode::as_code).join(";\n\t");
        format!("struct {name} {{\n\t{fields};\n}}")
    }
}

impl AsCode for VariableDeclaration {
    fn as_code(&self) -> String {
        format!("{} {}", self.ty.as_code(), self.name.name)
    }
}

impl AsCode for EnumDefinition {
    fn as_code(&self) -> String {
        let name = &self.name.name;
        let values = self.values.iter().map(AsCode::as_code).join("\n\t");
        format!("enum {name} {{\n\t{values}\n}}")
    }
}

impl AsCode for Identifier {
    fn as_code(&self) -> String {
        self.name.to_string()
    }
}
