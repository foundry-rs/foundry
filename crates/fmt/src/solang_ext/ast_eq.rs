use alloy_primitives::{Address, I256, U256};
use solang_parser::pt::*;
use std::str::FromStr;

/// Helper to convert a string number into a comparable one
fn to_num(string: &str) -> I256 {
    if string.is_empty() {
        return I256::ZERO
    }
    string.replace('_', "").trim().parse().unwrap()
}

/// Helper to convert the fractional part of a number into a comparable one.
/// This will reverse the number so that 0's can be ignored
fn to_num_reversed(string: &str) -> U256 {
    if string.is_empty() {
        return U256::from(0)
    }
    string.replace('_', "").trim().chars().rev().collect::<String>().parse().unwrap()
}

/// Helper to filter [ParameterList] to omit empty
/// parameters
fn filter_params(list: &ParameterList) -> ParameterList {
    list.iter().filter(|(_, param)| param.is_some()).cloned().collect::<Vec<_>>()
}

/// Check if two ParseTrees are equal ignoring location information or ordering if ordering does
/// not matter
pub trait AstEq {
    fn ast_eq(&self, other: &Self) -> bool;
}

impl AstEq for Loc {
    fn ast_eq(&self, _other: &Self) -> bool {
        true
    }
}

impl AstEq for IdentifierPath {
    fn ast_eq(&self, other: &Self) -> bool {
        self.identifiers.ast_eq(&other.identifiers)
    }
}

impl AstEq for SourceUnit {
    fn ast_eq(&self, other: &Self) -> bool {
        self.0.ast_eq(&other.0)
    }
}

impl AstEq for VariableDefinition {
    fn ast_eq(&self, other: &Self) -> bool {
        let sorted_attrs = |def: &Self| {
            let mut attrs = def.attrs.clone();
            attrs.sort();
            attrs
        };
        self.ty.ast_eq(&other.ty) &&
            self.name.ast_eq(&other.name) &&
            self.initializer.ast_eq(&other.initializer) &&
            sorted_attrs(self).ast_eq(&sorted_attrs(other))
    }
}

impl AstEq for FunctionDefinition {
    fn ast_eq(&self, other: &Self) -> bool {
        // attributes
        let sorted_attrs = |def: &Self| {
            let mut attrs = def.attributes.clone();
            attrs.sort();
            attrs
        };

        // params
        let left_params = filter_params(&self.params);
        let right_params = filter_params(&other.params);
        let left_returns = filter_params(&self.returns);
        let right_returns = filter_params(&other.returns);

        self.ty.ast_eq(&other.ty) &&
            self.name.ast_eq(&other.name) &&
            left_params.ast_eq(&right_params) &&
            self.return_not_returns.ast_eq(&other.return_not_returns) &&
            left_returns.ast_eq(&right_returns) &&
            self.body.ast_eq(&other.body) &&
            sorted_attrs(self).ast_eq(&sorted_attrs(other))
    }
}

impl AstEq for Base {
    fn ast_eq(&self, other: &Self) -> bool {
        self.name.ast_eq(&other.name) &&
            self.args.clone().unwrap_or_default().ast_eq(&other.args.clone().unwrap_or_default())
    }
}

impl<T> AstEq for Vec<T>
where
    T: AstEq,
{
    fn ast_eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            false
        } else {
            self.iter().zip(other.iter()).all(|(left, right)| left.ast_eq(right))
        }
    }
}

impl<T> AstEq for Option<T>
where
    T: AstEq,
{
    fn ast_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Some(left), Some(right)) => left.ast_eq(right),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T> AstEq for Box<T>
where
    T: AstEq,
{
    fn ast_eq(&self, other: &Self) -> bool {
        T::ast_eq(self, other)
    }
}

impl AstEq for () {
    fn ast_eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T> AstEq for &T
where
    T: AstEq,
{
    fn ast_eq(&self, other: &Self) -> bool {
        T::ast_eq(self, other)
    }
}

impl AstEq for String {
    fn ast_eq(&self, other: &Self) -> bool {
        match (Address::from_str(self), Address::from_str(other)) {
            (Ok(left), Ok(right)) => left == right,
            _ => self == other,
        }
    }
}

macro_rules! ast_eq_field {
    (#[ast_eq_use($convert_func:ident)] $field:ident) => {
        $convert_func($field)
    };
    ($field:ident) => {
        $field
    };
}

macro_rules! gen_ast_eq_enum {
    ($self:expr, $other:expr, $name:ident {
        $($unit_variant:ident),* $(,)?
        _
        $($tuple_variant:ident ( $($(#[ast_eq_use($tuple_convert_func:ident)])? $tuple_field:ident),* $(,)? )),*  $(,)?
        _
        $($struct_variant:ident { $($(#[ast_eq_use($struct_convert_func:ident)])? $struct_field:ident),* $(,)? }),*  $(,)?
    }) => {
        match $self {
            $($name::$unit_variant => gen_ast_eq_enum!($other, $name, $unit_variant),)*
            $($name::$tuple_variant($($tuple_field),*) =>
                gen_ast_eq_enum!($other, $name, $tuple_variant ($($(#[ast_eq_use($tuple_convert_func)])? $tuple_field),*)),)*
            $($name::$struct_variant { $($struct_field),* } =>
                gen_ast_eq_enum!($other, $name, $struct_variant {$($(#[ast_eq_use($struct_convert_func)])? $struct_field),*}),)*
        }
    };
    ($other:expr, $name:ident, $unit_variant:ident) => {
        {
            matches!($other, $name::$unit_variant)
        }
    };
    ($other:expr, $name:ident, $tuple_variant:ident ( $($(#[ast_eq_use($tuple_convert_func:ident)])? $tuple_field:ident),* $(,)? ) ) => {
        {
            let left = ($(ast_eq_field!($(#[ast_eq_use($tuple_convert_func)])? $tuple_field)),*);
            if let $name::$tuple_variant($($tuple_field),*) = $other {
                let right = ($(ast_eq_field!($(#[ast_eq_use($tuple_convert_func)])? $tuple_field)),*);
                left.ast_eq(&right)
            } else {
                false
            }
        }
    };
    ($other:expr, $name:ident, $struct_variant:ident { $($(#[ast_eq_use($struct_convert_func:ident)])? $struct_field:ident),* $(,)? } ) => {
        {
            let left = ($(ast_eq_field!($(#[ast_eq_use($struct_convert_func)])? $struct_field)),*);
            if let $name::$struct_variant { $($struct_field),* } = $other {
                let right = ($(ast_eq_field!($(#[ast_eq_use($struct_convert_func)])? $struct_field)),*);
                left.ast_eq(&right)
            } else {
                false
            }
        }
    };
}

macro_rules! wrap_in_box {
    ($stmt:expr, $loc:expr) => {
        if !matches!(**$stmt, Statement::Block { .. }) {
            Box::new(Statement::Block {
                loc: $loc,
                unchecked: false,
                statements: vec![*$stmt.clone()],
            })
        } else {
            $stmt.clone()
        }
    };
}

impl AstEq for Statement {
    fn ast_eq(&self, other: &Self) -> bool {
        match self {
            Self::If(loc, expr, stmt1, stmt2) => {
                #[allow(clippy::borrowed_box)]
                let wrap_if = |stmt1: &Box<Self>, stmt2: &Option<Box<Self>>| {
                    (
                        wrap_in_box!(stmt1, *loc),
                        stmt2.as_ref().map(|stmt2| {
                            if matches!(**stmt2, Self::If(..)) {
                                stmt2.clone()
                            } else {
                                wrap_in_box!(stmt2, *loc)
                            }
                        }),
                    )
                };
                let (stmt1, stmt2) = wrap_if(stmt1, stmt2);
                let left = (loc, expr, &stmt1, &stmt2);
                if let Self::If(loc, expr, stmt1, stmt2) = other {
                    let (stmt1, stmt2) = wrap_if(stmt1, stmt2);
                    let right = (loc, expr, &stmt1, &stmt2);
                    left.ast_eq(&right)
                } else {
                    false
                }
            }
            Self::While(loc, expr, stmt1) => {
                let stmt1 = wrap_in_box!(stmt1, *loc);
                let left = (loc, expr, &stmt1);
                if let Self::While(loc, expr, stmt1) = other {
                    let stmt1 = wrap_in_box!(stmt1, *loc);
                    let right = (loc, expr, &stmt1);
                    left.ast_eq(&right)
                } else {
                    false
                }
            }
            Self::DoWhile(loc, stmt1, expr) => {
                let stmt1 = wrap_in_box!(stmt1, *loc);
                let left = (loc, &stmt1, expr);
                if let Self::DoWhile(loc, stmt1, expr) = other {
                    let stmt1 = wrap_in_box!(stmt1, *loc);
                    let right = (loc, &stmt1, expr);
                    left.ast_eq(&right)
                } else {
                    false
                }
            }
            Self::For(loc, stmt1, expr, stmt2, stmt3) => {
                let stmt3 = stmt3.as_ref().map(|stmt3| wrap_in_box!(stmt3, *loc));
                let left = (loc, stmt1, expr, stmt2, &stmt3);
                if let Self::For(loc, stmt1, expr, stmt2, stmt3) = other {
                    let stmt3 = stmt3.as_ref().map(|stmt3| wrap_in_box!(stmt3, *loc));
                    let right = (loc, stmt1, expr, stmt2, &stmt3);
                    left.ast_eq(&right)
                } else {
                    false
                }
            }
            Self::Try(loc, expr, returns, catch) => {
                let left_returns =
                    returns.as_ref().map(|(params, stmt)| (filter_params(params), stmt));
                let left = (loc, expr, left_returns, catch);
                if let Self::Try(loc, expr, returns, catch) = other {
                    let right_returns =
                        returns.as_ref().map(|(params, stmt)| (filter_params(params), stmt));
                    let right = (loc, expr, right_returns, catch);
                    left.ast_eq(&right)
                } else {
                    false
                }
            }
            _ => gen_ast_eq_enum!(self, other, Statement {
                _
                Args(loc, args),
                Expression(loc, expr),
                VariableDefinition(loc, decl, expr),
                Continue(loc, ),
                Break(loc, ),
                Return(loc, expr),
                Revert(loc, expr, expr2),
                RevertNamedArgs(loc, expr, args),
                Emit(loc, expr),
                // provide overridden variants regardless
                If(loc, expr, stmt1, stmt2),
                While(loc, expr, stmt1),
                DoWhile(loc, stmt1, expr),
                For(loc, stmt1, expr, stmt2, stmt3),
                Try(loc, expr, params, clause),
                Error(loc)
                _
                Block {
                    loc,
                    unchecked,
                    statements,
                },
                Assembly {
                    loc,
                    dialect,
                    block,
                    flags,
                },
            }),
        }
    }
}

macro_rules! derive_ast_eq {
    ($name:ident) => {
        impl AstEq for $name {
            fn ast_eq(&self, other: &Self) -> bool {
                self == other
            }
        }
    };
    (($($index:tt $gen:tt),*)) => {
        impl < $( $gen ),* > AstEq for ($($gen,)*) where $($gen: AstEq),* {
            fn ast_eq(&self, other: &Self) -> bool {
                $(
                if !self.$index.ast_eq(&other.$index) {
                    return false
                }
                )*
                true
            }
        }
    };
    (struct $name:ident { $($field:ident),* $(,)? }) => {
        impl AstEq for $name {
            fn ast_eq(&self, other: &Self) -> bool {
                let $name { $($field),* } = self;
                let left = ($($field),*);
                let $name { $($field),* } = other;
                let right = ($($field),*);
                left.ast_eq(&right)
            }
        }
    };
    (enum $name:ident {
        $($unit_variant:ident),* $(,)?
        _
        $($tuple_variant:ident ( $($(#[ast_eq_use($tuple_convert_func:ident)])? $tuple_field:ident),* $(,)? )),*  $(,)?
        _
        $($struct_variant:ident { $($(#[ast_eq_use($struct_convert_func:ident)])? $struct_field:ident),* $(,)? }),*  $(,)?
    }) => {
        impl AstEq for $name {
            fn ast_eq(&self, other: &Self) -> bool {
                gen_ast_eq_enum!(self, other, $name {
                    $($unit_variant),*
                    _
                    $($tuple_variant ( $($(#[ast_eq_use($tuple_convert_func)])? $tuple_field),* )),*
                    _
                    $($struct_variant { $($(#[ast_eq_use($struct_convert_func)])? $struct_field),* }),*
                })
            }
        }
    }
}

derive_ast_eq! { (0 A) }
derive_ast_eq! { (0 A, 1 B) }
derive_ast_eq! { (0 A, 1 B, 2 C) }
derive_ast_eq! { (0 A, 1 B, 2 C, 3 D) }
derive_ast_eq! { (0 A, 1 B, 2 C, 3 D, 4 E) }
derive_ast_eq! { bool }
derive_ast_eq! { u8 }
derive_ast_eq! { u16 }
derive_ast_eq! { I256 }
derive_ast_eq! { U256 }
derive_ast_eq! { struct Identifier { loc, name } }
derive_ast_eq! { struct HexLiteral { loc, hex } }
derive_ast_eq! { struct StringLiteral { loc, unicode, string } }
derive_ast_eq! { struct Parameter { loc, annotation, ty, storage, name } }
derive_ast_eq! { struct NamedArgument { loc, name, expr } }
derive_ast_eq! { struct YulBlock { loc, statements } }
derive_ast_eq! { struct YulFunctionCall { loc, id, arguments } }
derive_ast_eq! { struct YulFunctionDefinition { loc, id, params, returns, body } }
derive_ast_eq! { struct YulSwitch { loc, condition, cases, default } }
derive_ast_eq! { struct YulFor {
    loc,
    init_block,
    condition,
    post_block,
    execution_block,
}}
derive_ast_eq! { struct YulTypedIdentifier { loc, id, ty } }
derive_ast_eq! { struct VariableDeclaration { loc, ty, storage, name } }
derive_ast_eq! { struct Using { loc, list, ty, global } }
derive_ast_eq! { struct UsingFunction { loc, path, oper } }
derive_ast_eq! { struct TypeDefinition { loc, name, ty } }
derive_ast_eq! { struct ContractDefinition { loc, ty, name, base, parts } }
derive_ast_eq! { struct EventParameter { loc, ty, indexed, name } }
derive_ast_eq! { struct ErrorParameter { loc, ty, name } }
derive_ast_eq! { struct EventDefinition { loc, name, fields, anonymous } }
derive_ast_eq! { struct ErrorDefinition { loc, keyword, name, fields } }
derive_ast_eq! { struct StructDefinition { loc, name, fields } }
derive_ast_eq! { struct EnumDefinition { loc, name, values } }
derive_ast_eq! { struct Annotation { loc, id, value } }
derive_ast_eq! { enum UsingList {
    Error,
    _
    Library(expr),
    Functions(exprs),
    _
}}
derive_ast_eq! { enum UserDefinedOperator {
    BitwiseAnd,
    BitwiseNot,
    Negate,
    BitwiseOr,
    BitwiseXor,
    Add,
    Divide,
    Modulo,
    Multiply,
    Subtract,
    Equal,
    More,
    MoreEqual,
    Less,
    LessEqual,
    NotEqual,
    _
    _
}}
derive_ast_eq! { enum Visibility {
    _
    External(loc),
    Public(loc),
    Internal(loc),
    Private(loc),
    _
}}
derive_ast_eq! { enum Mutability {
    _
    Pure(loc),
    View(loc),
    Constant(loc),
    Payable(loc),
    _
}}
derive_ast_eq! { enum FunctionAttribute {
    _
    Mutability(muta),
    Visibility(visi),
    Virtual(loc),
    Immutable(loc),
    Override(loc, idents),
    BaseOrModifier(loc, base),
    Error(loc),
    _
}}
derive_ast_eq! { enum StorageLocation {
    _
    Memory(loc),
    Storage(loc),
    Calldata(loc),
    _
}}
derive_ast_eq! { enum Type {
    Address,
    AddressPayable,
    Payable,
    Bool,
    Rational,
    DynamicBytes,
    String,
    _
    Int(int),
    Uint(int),
    Bytes(int),
    _
    Mapping{ loc, key, key_name, value, value_name },
    Function { params, attributes, returns },
}}
derive_ast_eq! { enum Expression {
    _
    PostIncrement(loc, expr1),
    PostDecrement(loc, expr1),
    New(loc, expr1),
    ArraySubscript(loc, expr1, expr2),
    ArraySlice(
        loc,
        expr1,
        expr2,
        expr3,
    ),
    MemberAccess(loc, expr1, ident1),
    FunctionCall(loc, expr1, exprs1),
    FunctionCallBlock(loc, expr1, stmt),
    NamedFunctionCall(loc, expr1, args),
    Not(loc, expr1),
    BitwiseNot(loc, expr1),
    Delete(loc, expr1),
    PreIncrement(loc, expr1),
    PreDecrement(loc, expr1),
    UnaryPlus(loc, expr1),
    Negate(loc, expr1),
    Power(loc, expr1, expr2),
    Multiply(loc, expr1, expr2),
    Divide(loc, expr1, expr2),
    Modulo(loc, expr1, expr2),
    Add(loc, expr1, expr2),
    Subtract(loc, expr1, expr2),
    ShiftLeft(loc, expr1, expr2),
    ShiftRight(loc, expr1, expr2),
    BitwiseAnd(loc, expr1, expr2),
    BitwiseXor(loc, expr1, expr2),
    BitwiseOr(loc, expr1, expr2),
    Less(loc, expr1, expr2),
    More(loc, expr1, expr2),
    LessEqual(loc, expr1, expr2),
    MoreEqual(loc, expr1, expr2),
    Equal(loc, expr1, expr2),
    NotEqual(loc, expr1, expr2),
    And(loc, expr1, expr2),
    Or(loc, expr1, expr2),
    ConditionalOperator(loc, expr1, expr2, expr3),
    Assign(loc, expr1, expr2),
    AssignOr(loc, expr1, expr2),
    AssignAnd(loc, expr1, expr2),
    AssignXor(loc, expr1, expr2),
    AssignShiftLeft(loc, expr1, expr2),
    AssignShiftRight(loc, expr1, expr2),
    AssignAdd(loc, expr1, expr2),
    AssignSubtract(loc, expr1, expr2),
    AssignMultiply(loc, expr1, expr2),
    AssignDivide(loc, expr1, expr2),
    AssignModulo(loc, expr1, expr2),
    BoolLiteral(loc, bool1),
    NumberLiteral(loc, #[ast_eq_use(to_num)] str1, #[ast_eq_use(to_num)] str2, unit),
    RationalNumberLiteral(
        loc,
        #[ast_eq_use(to_num)] str1,
        #[ast_eq_use(to_num_reversed)] str2,
        #[ast_eq_use(to_num)] str3,
        unit
    ),
    HexNumberLiteral(loc, str1, unit),
    StringLiteral(strs1),
    Type(loc, ty1),
    HexLiteral(hexs1),
    AddressLiteral(loc, str1),
    Variable(ident1),
    List(loc, params1),
    ArrayLiteral(loc, exprs1),
    Parenthesis(loc, expr)
    _
}}
derive_ast_eq! { enum CatchClause {
    _
    Simple(param, ident, stmt),
    Named(loc, ident, param, stmt),
    _
}}
derive_ast_eq! { enum YulStatement {
    _
    Assign(loc, exprs, expr),
    VariableDeclaration(loc, idents, expr),
    If(loc, expr, block),
    For(yul_for),
    Switch(switch),
    Leave(loc),
    Break(loc),
    Continue(loc),
    Block(block),
    FunctionDefinition(def),
    FunctionCall(func),
    Error(loc),
    _
}}
derive_ast_eq! { enum YulExpression {
    _
    BoolLiteral(loc, boo, ident),
    NumberLiteral(loc, string1, string2, ident),
    HexNumberLiteral(loc, string, ident),
    HexStringLiteral(hex, ident),
    StringLiteral(string, ident),
    Variable(ident),
    FunctionCall(func),
    SuffixAccess(loc, expr, ident),
    _
}}
derive_ast_eq! { enum YulSwitchOptions {
    _
    Case(loc, expr, block),
    Default(loc, block),
    _
}}
derive_ast_eq! { enum SourceUnitPart {
    _
    ContractDefinition(def),
    PragmaDirective(loc, ident, string),
    ImportDirective(import),
    EnumDefinition(def),
    StructDefinition(def),
    EventDefinition(def),
    ErrorDefinition(def),
    FunctionDefinition(def),
    VariableDefinition(def),
    TypeDefinition(def),
    Using(using),
    StraySemicolon(loc),
    Annotation(annotation),
    _
}}
derive_ast_eq! { enum ImportPath {
    _
    Filename(lit),
    Path(path),
    _
}}
derive_ast_eq! { enum Import {
    _
    Plain(string, loc),
    GlobalSymbol(string, ident, loc),
    Rename(string, idents, loc),
    _
}}
derive_ast_eq! { enum FunctionTy {
    Constructor,
    Function,
    Fallback,
    Receive,
    Modifier,
    _
    _
}}
derive_ast_eq! { enum ContractPart {
    _
    StructDefinition(def),
    EventDefinition(def),
    EnumDefinition(def),
    ErrorDefinition(def),
    VariableDefinition(def),
    FunctionDefinition(def),
    TypeDefinition(def),
    StraySemicolon(loc),
    Using(using),
    Annotation(annotation),
    _
}}
derive_ast_eq! { enum ContractTy {
    _
    Abstract(loc),
    Contract(loc),
    Interface(loc),
    Library(loc),
    _
}}
derive_ast_eq! { enum VariableAttribute {
    _
    Visibility(visi),
    Constant(loc),
    Immutable(loc),
    Override(loc, idents),
    _
}}
