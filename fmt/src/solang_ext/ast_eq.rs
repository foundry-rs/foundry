use itertools::Itertools;
use solang_parser::pt::*;

pub trait AstEq {
    fn ast_eq(&self, other: &Self) -> bool;
}

impl AstEq for Loc {
    fn ast_eq(&self, _other: &Self) -> bool {
        true
    }
}

impl AstEq for SourceUnit {
    fn ast_eq(&self, other: &Self) -> bool {
        self.0.ast_eq(&other.0)
    }
}

impl AstEq for VariableDefinition {
    fn ast_eq(&self, other: &Self) -> bool {
        let sort_attrs = |def: &Self| {
            def.attrs
                .iter()
                .sorted_by_key(|attribute| match attribute {
                    VariableAttribute::Visibility(_) => 0,
                    VariableAttribute::Constant(_) => 1,
                    VariableAttribute::Immutable(_) => 2,
                    VariableAttribute::Override(_) => 3,
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let left_sorted_attrs = sort_attrs(self);
        let right_sorted_attrs = sort_attrs(other);
        self.ty.ast_eq(&other.ty) &&
            self.name.ast_eq(&other.name) &&
            self.initializer.ast_eq(&other.initializer) &&
            left_sorted_attrs.ast_eq(&right_sorted_attrs)
    }
}

impl AstEq for FunctionDefinition {
    fn ast_eq(&self, other: &Self) -> bool {
        let sort_attrs = |def: &Self| {
            def.attributes
                .iter()
                .sorted_by_key(|attribute| match attribute {
                    FunctionAttribute::Visibility(_) => 0,
                    FunctionAttribute::Mutability(_) => 1,
                    FunctionAttribute::Virtual(_) => 2,
                    FunctionAttribute::Immutable(_) => 3,
                    FunctionAttribute::Override(_, _) => 4,
                    FunctionAttribute::BaseOrModifier(_, _) => 5,
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let left_sorted_attrs = sort_attrs(self);
        let right_sorted_attrs = sort_attrs(other);
        self.ty.ast_eq(&other.ty) &&
            self.name.ast_eq(&other.name) &&
            self.params.ast_eq(&other.params) &&
            self.return_not_returns.ast_eq(&other.return_not_returns) &&
            self.returns.ast_eq(&other.returns) &&
            self.body.ast_eq(&other.body) &&
            left_sorted_attrs.ast_eq(&right_sorted_attrs)
    }
}

impl AstEq for Base {
    fn ast_eq(&self, other: &Self) -> bool {
        self.name.ast_eq(&other.name) &&
            self.args.clone().unwrap_or_default().ast_eq(&other.args.clone().unwrap_or_default())
    }
}

impl AstEq for DocComment {
    fn ast_eq(&self, other: &Self) -> bool {
        self.ty == other.ty
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
        $($tuple_variant:ident ( $($tuple_field:ident),* $(,)? )),*  $(,)?
        _
        $($struct_variant:ident { $($struct_field:ident),* $(,)? }),*  $(,)?
    }) => {
        impl AstEq for $name {
            fn ast_eq(&self, other: &Self) -> bool {
                match self {
                    $(
                    $name::$unit_variant => {
                        return matches!(other, $name::$unit_variant)
                    }
                    )*
                    $(
                    $name::$tuple_variant($($tuple_field),*) =>  {
                        let left = ($($tuple_field),*);
                        if let $name::$tuple_variant($($tuple_field),*) = other {
                            let right = ($($tuple_field),*);
                            left.ast_eq(&right)
                        } else {
                            false
                        }
                    }
                    )*
                    $(
                    $name::$struct_variant { $($struct_field),* } => {
                        let left = ($($struct_field),*);
                        if let $name::$struct_variant { $($struct_field),* } = other {
                            let right = ($($struct_field),*);
                            left.ast_eq(&right)
                        } else {
                            false
                        }
                    }
                    )*
                }
            }
        }
    }
}

derive_ast_eq! { (0 A) }
derive_ast_eq! { (0 A, 1 B) }
derive_ast_eq! { (0 A, 1 B, 2 C) }
derive_ast_eq! { (0 A, 1 B, 2 C, 3 D) }
derive_ast_eq! { (0 A, 1 B, 2 C, 3 D, 4 E) }
derive_ast_eq! { String }
derive_ast_eq! { bool }
derive_ast_eq! { u8 }
derive_ast_eq! { u16 }
derive_ast_eq! { struct Identifier { loc, name } }
derive_ast_eq! { struct HexLiteral { loc, hex } }
derive_ast_eq! { struct StringLiteral { loc, string } }
derive_ast_eq! { struct Parameter { loc, ty, storage, name } }
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
derive_ast_eq! { struct TypeDefinition { loc, name, ty } }
derive_ast_eq! { struct ContractDefinition { loc, ty, name, base, parts } }
derive_ast_eq! { struct EventParameter { loc, ty, indexed, name } }
derive_ast_eq! { struct ErrorParameter { loc, ty, name } }
derive_ast_eq! { struct EventDefinition { loc, name, fields, anonymous } }
derive_ast_eq! { struct ErrorDefinition { loc, name, fields } }
derive_ast_eq! { struct StructDefinition { loc, name, fields } }
derive_ast_eq! { struct EnumDefinition { loc, name, values } }
derive_ast_eq! { enum CommentType {
    Line,
    Block,
    _
    _
}}
derive_ast_eq! { enum UsingList {
    _
    Library(expr),
    Functions(exprs),
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
derive_ast_eq! { enum Unit {
    _
    Seconds(loc),
    Minutes(loc),
    Hours(loc),
    Days(loc),
    Weeks(loc),
    Wei(loc),
    Gwei(loc),
    Ether(loc),
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
    Mapping(loc, expr1, expr2),
    _
    Function { params, attributes, returns },
}}
derive_ast_eq! { enum Statement {
    _
    Args(loc, args),
    If(loc, expr, stmt1, stmt2),
    While(loc, expr, stmt1),
    Expression(loc, expr),
    VariableDefinition(loc, decl, expr),
    For(loc, stmt1, expr, stmt2, stmt3),
    DoWhile(loc, stmt1, expr),
    Continue(loc, ),
    Break(loc, ),
    Return(loc, expr),
    Revert(loc, expr, expr2),
    RevertNamedArgs(loc, expr, args),
    Emit(loc, expr),
    Try(loc, expr, params, claus),
    DocComment(comment),
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
    },
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
    Complement(loc, expr1),
    Delete(loc, expr1),
    PreIncrement(loc, expr1),
    PreDecrement(loc, expr1),
    UnaryPlus(loc, expr1),
    UnaryMinus(loc, expr1),
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
    Ternary(loc, expr1, expr2, expr3),
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
    NumberLiteral(loc, str1, str2),
    RationalNumberLiteral(loc, str1, str2, str3),
    HexNumberLiteral(loc, str1),
    StringLiteral(strs1),
    Type(loc, ty1),
    HexLiteral(hexs1),
    AddressLiteral(loc, str1),
    Variable(ident1),
    List(loc, params1),
    ArrayLiteral(loc, exprs1),
    Unit(loc, expr1, unit1),
    This(loc)
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
    Member(loc, expr, ident),
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
    DocComment(comment),
    Using(using),
    StraySemicolon(loc),
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
    DocComment(comment),
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
    Override(loc),
    _
}}
