use solang_parser::pt::*;

/// Describes the precedence and the operator token of an Expression if it has one
pub trait Operator: Sized {
    fn unsplittable(&self) -> bool;
    fn operator(&self) -> Option<&'static str>;
    fn has_space_around(&self) -> bool;
}

/// Splits the iterator into components
pub trait OperatorComponents: Sized {
    fn components(&self) -> (Option<&Self>, Option<&Self>);
    fn components_mut(&mut self) -> (Option<&mut Self>, Option<&mut Self>);
}

impl Operator for Expression {
    fn unsplittable(&self) -> bool {
        use Expression::*;
        matches!(
            self,
            BoolLiteral(..) |
                NumberLiteral(..) |
                RationalNumberLiteral(..) |
                HexNumberLiteral(..) |
                StringLiteral(..) |
                HexLiteral(..) |
                AddressLiteral(..) |
                Variable(..) |
                This(..)
        )
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
            Negate(..) | Subtract(..) => "-",
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
            ConditionalOperator(..) |
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
            This(..) |
            Parenthesis(..) => return None,
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
                Negate(..)
        )
    }
}

impl OperatorComponents for Expression {
    fn components(&self) -> (Option<&Self>, Option<&Self>) {
        use Expression::*;
        match self {
            PostDecrement(_, expr) | PostIncrement(_, expr) => (Some(expr), None),
            Not(_, expr) |
            Complement(_, expr) |
            New(_, expr) |
            Delete(_, expr) |
            UnaryPlus(_, expr) |
            Negate(_, expr) |
            PreDecrement(_, expr) |
            Parenthesis(_, expr) |
            PreIncrement(_, expr) => (None, Some(expr)),
            Power(_, left, right) |
            Multiply(_, left, right) |
            Divide(_, left, right) |
            Modulo(_, left, right) |
            Add(_, left, right) |
            Subtract(_, left, right) |
            ShiftLeft(_, left, right) |
            ShiftRight(_, left, right) |
            BitwiseAnd(_, left, right) |
            BitwiseXor(_, left, right) |
            BitwiseOr(_, left, right) |
            Less(_, left, right) |
            More(_, left, right) |
            LessEqual(_, left, right) |
            MoreEqual(_, left, right) |
            Equal(_, left, right) |
            NotEqual(_, left, right) |
            And(_, left, right) |
            Or(_, left, right) |
            Assign(_, left, right) |
            AssignOr(_, left, right) |
            AssignAnd(_, left, right) |
            AssignXor(_, left, right) |
            AssignShiftLeft(_, left, right) |
            AssignShiftRight(_, left, right) |
            AssignAdd(_, left, right) |
            AssignSubtract(_, left, right) |
            AssignMultiply(_, left, right) |
            AssignDivide(_, left, right) |
            AssignModulo(_, left, right) => (Some(left), Some(right)),
            MemberAccess(..) |
            ConditionalOperator(..) |
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
            This(..) => (None, None),
        }
    }

    fn components_mut(&mut self) -> (Option<&mut Self>, Option<&mut Self>) {
        use Expression::*;

        match self {
            PostDecrement(_, expr) | PostIncrement(_, expr) => (Some(expr.as_mut()), None),
            Not(_, expr) |
            Complement(_, expr) |
            New(_, expr) |
            Delete(_, expr) |
            UnaryPlus(_, expr) |
            Negate(_, expr) |
            PreDecrement(_, expr) |
            Parenthesis(_, expr) |
            PreIncrement(_, expr) => (None, Some(expr.as_mut())),
            Power(_, left, right) |
            Multiply(_, left, right) |
            Divide(_, left, right) |
            Modulo(_, left, right) |
            Add(_, left, right) |
            Subtract(_, left, right) |
            ShiftLeft(_, left, right) |
            ShiftRight(_, left, right) |
            BitwiseAnd(_, left, right) |
            BitwiseXor(_, left, right) |
            BitwiseOr(_, left, right) |
            Less(_, left, right) |
            More(_, left, right) |
            LessEqual(_, left, right) |
            MoreEqual(_, left, right) |
            Equal(_, left, right) |
            NotEqual(_, left, right) |
            And(_, left, right) |
            Or(_, left, right) |
            Assign(_, left, right) |
            AssignOr(_, left, right) |
            AssignAnd(_, left, right) |
            AssignXor(_, left, right) |
            AssignShiftLeft(_, left, right) |
            AssignShiftRight(_, left, right) |
            AssignAdd(_, left, right) |
            AssignSubtract(_, left, right) |
            AssignMultiply(_, left, right) |
            AssignDivide(_, left, right) |
            AssignModulo(_, left, right) => (Some(left.as_mut()), Some(right.as_mut())),
            MemberAccess(..) |
            ConditionalOperator(..) |
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
            This(..) => (None, None),
        }
    }
}
