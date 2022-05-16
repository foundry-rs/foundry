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
                $(
                if !self.$field.ast_eq(&other.$field) {
                    return false;
                }
                )*
                true
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
                    $name::$tuple_variant(.., $($tuple_field),*) =>  {
                        let left = ($($tuple_field),*);
                        if let $name::$tuple_variant(.., $($tuple_field),*) = other {
                            let right = ($($tuple_field),*);
                            left.ast_eq(&right)
                        } else {
                            false
                        }
                    }
                    )*
                    $(
                    $name::$struct_variant { $($struct_field),* , .. } => {
                        let left = ($($struct_field),*);
                        if let $name::$struct_variant { $($struct_field),* , .. } = other {
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
derive_ast_eq! { struct Identifier { name } }
derive_ast_eq! { struct HexLiteral { hex } }
derive_ast_eq! { struct StringLiteral { string } }
derive_ast_eq! { struct Parameter { ty, storage, name } }
derive_ast_eq! { struct NamedArgument { name, expr } }
derive_ast_eq! { struct YulBlock { statements } }
derive_ast_eq! { struct DocComment { ty } }
derive_ast_eq! { struct YulFunctionCall { id, arguments } }
derive_ast_eq! { struct YulFunctionDefinition { id, params, returns, body } }
derive_ast_eq! { struct YulSwitch { condition, cases, default } }
derive_ast_eq! { struct YulFor {
    init_block,
    condition,
    post_block,
    execution_block,
}}
derive_ast_eq! { struct YulTypedIdentifier { id, ty } }
derive_ast_eq! { struct VariableDeclaration { ty, storage, name } }
derive_ast_eq! { struct Using { list, ty, global } }
derive_ast_eq! { struct TypeDefinition { name, ty } }
derive_ast_eq! { struct ContractDefinition { ty, name, base, parts } }
derive_ast_eq! { struct EventParameter { ty, indexed, name } }
derive_ast_eq! { struct ErrorParameter { ty, name } }
derive_ast_eq! { struct EventDefinition { name, fields, anonymous } }
derive_ast_eq! { struct ErrorDefinition { name, fields } }
derive_ast_eq! { struct StructDefinition { name, fields } }
derive_ast_eq! { struct EnumDefinition { name, values } }
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
    External(),
    Public(),
    Internal(),
    Private(),
    _
}}
derive_ast_eq! { enum Mutability {
    _
    Pure(),
    View(),
    Constant(),
    Payable(),
    _
}}
derive_ast_eq! { enum Unit {
    _
    Seconds(),
    Minutes(),
    Hours(),
    Days(),
    Weeks(),
    Wei(),
    Gwei(),
    Ether(),
    _
}}
derive_ast_eq! { enum FunctionAttribute {
    _
    Mutability(muta),
    Visibility(visi),
    Virtual(),
    Immutable(),
    Override(idents),
    BaseOrModifier(base),
    _
}}
derive_ast_eq! { enum StorageLocation {
    _
    Memory(),
    Storage(),
    Calldata(),
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
    Mapping(expr1, expr2),
    _
    Function { params, attributes, returns },
}}
derive_ast_eq! { enum Statement {
    _
    Args(args),
    If(expr, stmt1, stmt2),
    While(expr, stmt1),
    Expression(expr),
    VariableDefinition(decl, expr),
    For(stmt1, expr, stmt2, stmt3),
    DoWhile(stmt1, expr),
    Continue(),
    Break(),
    Return(expr),
    Revert(expr, expr2),
    RevertNamedArgs(expr, args),
    Emit(expr),
    Try(expr, params, claus),
    DocComment(comment),
    _
    Block {
        unchecked,
        statements,
    },
    Assembly {
        dialect,
        block,
    },
}}
derive_ast_eq! { enum Expression {
    _
    PostIncrement(expr1),
    PostDecrement(expr1),
    New(expr1),
    ArraySubscript(expr1, expr2),
    ArraySlice(
        expr1,
        expr2,
        expr3,
    ),
    MemberAccess(expr1, ident1),
    FunctionCall(expr1, exprs1),
    FunctionCallBlock(expr1, stmt),
    NamedFunctionCall(expr1, args),
    Not(expr1),
    Complement(expr1),
    Delete(expr1),
    PreIncrement(expr1),
    PreDecrement(expr1),
    UnaryPlus(expr1),
    UnaryMinus(expr1),
    Power(expr1, expr2),
    Multiply(expr1, expr2),
    Divide(expr1, expr2),
    Modulo(expr1, expr2),
    Add(expr1, expr2),
    Subtract(expr1, expr2),
    ShiftLeft(expr1, expr2),
    ShiftRight(expr1, expr2),
    BitwiseAnd(expr1, expr2),
    BitwiseXor(expr1, expr2),
    BitwiseOr(expr1, expr2),
    Less(expr1, expr2),
    More(expr1, expr2),
    LessEqual(expr1, expr2),
    MoreEqual(expr1, expr2),
    Equal(expr1, expr2),
    NotEqual(expr1, expr2),
    And(expr1, expr2),
    Or(expr1, expr2),
    Ternary(expr1, expr2, expr3),
    Assign(expr1, expr2),
    AssignOr(expr1, expr2),
    AssignAnd(expr1, expr2),
    AssignXor(expr1, expr2),
    AssignShiftLeft(expr1, expr2),
    AssignShiftRight(expr1, expr2),
    AssignAdd(expr1, expr2),
    AssignSubtract(expr1, expr2),
    AssignMultiply(expr1, expr2),
    AssignDivide(expr1, expr2),
    AssignModulo(expr1, expr2),
    BoolLiteral(bool1),
    NumberLiteral(str1, str2),
    RationalNumberLiteral(str1, str2, str3),
    HexNumberLiteral(str1),
    StringLiteral(strs1),
    Type(ty1),
    HexLiteral(hexs1),
    AddressLiteral(str1),
    Variable(ident1),
    List(params1),
    ArrayLiteral(exprs1),
    Unit(expr1, unit1),
    This()
    _
}}
derive_ast_eq! { enum CatchClause {
    _
    Simple(param, ident, stmt),
    Named(ident, param, stmt),
    _
}}
derive_ast_eq! { enum YulStatement {
    _
    Assign(exprs, expr),
    VariableDeclaration(idents, expr),
    If(expr, block),
    For(yul_for),
    Switch(switch),
    Leave(),
    Break(),
    Continue(),
    Block(block),
    FunctionDefinition(def),
    FunctionCall(func),
    _
}}
derive_ast_eq! { enum YulExpression {
    _
    BoolLiteral(boo, ident),
    NumberLiteral(string1, string2, ident),
    HexNumberLiteral(string, ident),
    HexStringLiteral(hex, ident),
    StringLiteral(string, ident),
    Variable(ident),
    FunctionCall(func),
    Member(expr, ident),
    _
}}
derive_ast_eq! { enum YulSwitchOptions {
    _
    Case(expr, block),
    Default(block),
    _
}}
derive_ast_eq! { enum SourceUnitPart {
    _
    ContractDefinition(def),
    PragmaDirective(ident, string),
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
    StraySemicolon(),
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
    StraySemicolon(),
    Using(using),
    DocComment(comment),
    _
}}
derive_ast_eq! { enum ContractTy {
    _
    Abstract(),
    Contract(),
    Interface(),
    Library(),
    _
}}
derive_ast_eq! { enum VariableAttribute {
    _
    Visibility(visi),
    Constant(),
    Immutable(),
    Override(),
    _
}}
