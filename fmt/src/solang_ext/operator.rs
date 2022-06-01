use solang_parser::pt::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd)]
#[repr(u8)]
pub enum Precedence {
    Literal,
    P01,
    P02,
    P03,
    P04,
    P05,
    P06,
    P07,
    P08,
    P09,
    P10,
    P11,
    P12,
    P13,
    P14,
    P15,
}

impl Precedence {
    pub fn is_left_to_right(self) -> bool {
        !matches!(self, Self::P03)
    }
}

pub trait Operator {
    fn precedence(&self) -> Precedence;
    fn operator(&self) -> Option<&'static str>;
    fn has_space_around(&self) -> bool;
    fn left_mut(&mut self) -> Option<&mut Box<Expression>>;
    fn right_mut(&mut self) -> Option<&mut Box<Expression>>;
}

impl Operator for Expression {
    fn precedence(&self) -> Precedence {
        use Expression::*;
        use Precedence::*;
        match self {
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => Literal,
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            PreIncrement(..) |
            PostIncrement(..) |
            New(..) |
            MemberAccess(..) => P01,
            Not(..) | Complement(..) | Delete(..) | UnaryPlus(..) | UnaryMinus(..) |
            PreDecrement(..) | PostDecrement(..) => P02,
            Power(..) => P03,
            Multiply(..) | Divide(..) | Modulo(..) => P04,
            Add(..) | Subtract(..) => P05,
            ShiftLeft(..) | ShiftRight(..) => P06,
            BitwiseAnd(..) => P07,
            BitwiseXor(..) => P08,
            BitwiseOr(..) => P09,
            Less(..) | More(..) | LessEqual(..) | MoreEqual(..) => P10,
            Equal(..) | NotEqual(..) => P11,
            And(..) => P12,
            Or(..) => P13,
            Ternary(..) => P14,
            Assign(..) | AssignOr(..) | AssignAnd(..) | AssignXor(..) | AssignShiftLeft(..) |
            AssignShiftRight(..) | AssignAdd(..) | AssignSubtract(..) | AssignMultiply(..) |
            AssignDivide(..) | AssignModulo(..) => P15,
        }
    }
    fn operator(&self) -> Option<&'static str> {
        use Expression::*;
        Some(match self {
            PreIncrement(..) | PostIncrement(..) => "++",
            PreDecrement(..) | PostDecrement(..) => "--",
            New(..) => "new",
            Not(..) => "!",
            Complement(..) => "~",
            Delete(..) => "delete",
            UnaryPlus(..) | Add(..) => "+",
            UnaryMinus(..) | Subtract(..) => "-",
            Power(..) => "**",
            Multiply(..) => "*",
            Divide(..) => "/",
            Modulo(..) => "%",
            ShiftLeft(..) => "<<",
            ShiftRight(..) => ">>",
            BitwiseAnd(..) => "&",
            BitwiseXor(..) => "^",
            BitwiseOr(..) => "|",
            Less(..) => "<",
            More(..) => ">",
            LessEqual(..) => "<=",
            MoreEqual(..) => ">=",
            And(..) => "&&",
            Or(..) => "||",
            Equal(..) => "==",
            NotEqual(..) => "!=",
            Assign(..) => "=",
            AssignOr(..) => "|=",
            AssignAnd(..) => "&=",
            AssignXor(..) => "^=",
            AssignShiftLeft(..) => "<<=",
            AssignShiftRight(..) => ">>=",
            AssignAdd(..) => "+=",
            AssignSubtract(..) => "-=",
            AssignMultiply(..) => "*=",
            AssignDivide(..) => "/=",
            AssignModulo(..) => "%=",
            MemberAccess(..) |
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            Ternary(..) |
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => return None,
        })
    }
    fn has_space_around(&self) -> bool {
        use Expression::*;
        !matches!(
            self,
            PostIncrement(..) |
                PreIncrement(..) |
                PostDecrement(..) |
                PreDecrement(..) |
                Not(..) |
                Complement(..) |
                UnaryPlus(..) |
                UnaryMinus(..)
        )
    }
    fn left_mut(&mut self) -> Option<&mut Box<Expression>> {
        use Expression::*;
        match self {
            PostDecrement(_, expr) |
            PostIncrement(_, expr) |
            Power(_, expr, _) |
            Multiply(_, expr, _) |
            Divide(_, expr, _) |
            Modulo(_, expr, _) |
            Add(_, expr, _) |
            Subtract(_, expr, _) |
            ShiftLeft(_, expr, _) |
            ShiftRight(_, expr, _) |
            BitwiseAnd(_, expr, _) |
            BitwiseXor(_, expr, _) |
            BitwiseOr(_, expr, _) |
            Less(_, expr, _) |
            More(_, expr, _) |
            LessEqual(_, expr, _) |
            MoreEqual(_, expr, _) |
            Equal(_, expr, _) |
            NotEqual(_, expr, _) |
            And(_, expr, _) |
            Or(_, expr, _) |
            Assign(_, expr, _) |
            AssignOr(_, expr, _) |
            AssignAnd(_, expr, _) |
            AssignXor(_, expr, _) |
            AssignShiftLeft(_, expr, _) |
            AssignShiftRight(_, expr, _) |
            AssignAdd(_, expr, _) |
            AssignSubtract(_, expr, _) |
            AssignMultiply(_, expr, _) |
            AssignDivide(_, expr, _) |
            AssignModulo(_, expr, _) => Some(expr),
            MemberAccess(..) |
            Not(..) |
            Complement(..) |
            Delete(..) |
            UnaryPlus(..) |
            UnaryMinus(..) |
            PreDecrement(..) |
            New(..) |
            PreIncrement(..) |
            Ternary(..) |
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => None,
        }
    }
    fn right_mut(&mut self) -> Option<&mut Box<Expression>> {
        use Expression::*;
        match self {
            Not(_, expr) |
            Complement(_, expr) |
            New(_, expr) |
            Delete(_, expr) |
            UnaryPlus(_, expr) |
            UnaryMinus(_, expr) |
            PreDecrement(_, expr) |
            PreIncrement(_, expr) |
            Power(_, _, expr) |
            Multiply(_, _, expr) |
            Divide(_, _, expr) |
            Modulo(_, _, expr) |
            Add(_, _, expr) |
            Subtract(_, _, expr) |
            ShiftLeft(_, _, expr) |
            ShiftRight(_, _, expr) |
            BitwiseAnd(_, _, expr) |
            BitwiseXor(_, _, expr) |
            BitwiseOr(_, _, expr) |
            Less(_, _, expr) |
            More(_, _, expr) |
            LessEqual(_, _, expr) |
            MoreEqual(_, _, expr) |
            Equal(_, _, expr) |
            NotEqual(_, _, expr) |
            And(_, _, expr) |
            Or(_, _, expr) |
            Assign(_, _, expr) |
            AssignOr(_, _, expr) |
            AssignAnd(_, _, expr) |
            AssignXor(_, _, expr) |
            AssignShiftLeft(_, _, expr) |
            AssignShiftRight(_, _, expr) |
            AssignAdd(_, _, expr) |
            AssignSubtract(_, _, expr) |
            AssignMultiply(_, _, expr) |
            AssignDivide(_, _, expr) |
            AssignModulo(_, _, expr) => Some(expr),
            MemberAccess(..) |
            PostDecrement(..) |
            PostIncrement(..) |
            Ternary(..) |
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => None,
        }
    }
}
