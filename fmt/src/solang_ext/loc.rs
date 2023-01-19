use solang_parser::pt::*;

/// Get the location of an Enum
pub trait LineOfCode {
    fn loc(&self) -> Loc;
}

/// Get the location of an Enum if it has one
pub trait OptionalLineOfCode {
    fn loc(&self) -> Option<Loc>;
}

impl<T> LineOfCode for Box<T>
where
    T: LineOfCode,
{
    fn loc(&self) -> Loc {
        T::loc(self)
    }
}

impl<T> LineOfCode for &T
where
    T: LineOfCode,
{
    fn loc(&self) -> Loc {
        T::loc(self)
    }
}

impl<T> LineOfCode for &mut T
where
    T: LineOfCode,
{
    fn loc(&self) -> Loc {
        T::loc(self)
    }
}

impl<T> OptionalLineOfCode for Box<T>
where
    T: OptionalLineOfCode,
{
    fn loc(&self) -> Option<Loc> {
        T::loc(self)
    }
}

impl<T> OptionalLineOfCode for &T
where
    T: OptionalLineOfCode,
{
    fn loc(&self) -> Option<Loc> {
        T::loc(self)
    }
}

impl<T> OptionalLineOfCode for &mut T
where
    T: OptionalLineOfCode,
{
    fn loc(&self) -> Option<Loc> {
        T::loc(self)
    }
}

impl LineOfCode for SourceUnitPart {
    fn loc(&self) -> Loc {
        match self {
            SourceUnitPart::ContractDefinition(contract) => contract.loc,
            SourceUnitPart::PragmaDirective(loc, _, _) | SourceUnitPart::StraySemicolon(loc) => {
                *loc
            }
            SourceUnitPart::ImportDirective(import) => *match import {
                Import::Plain(_, loc) => loc,
                Import::GlobalSymbol(_, _, loc) => loc,
                Import::Rename(_, _, loc) => loc,
            },
            SourceUnitPart::EnumDefinition(enumeration) => enumeration.loc,
            SourceUnitPart::StructDefinition(structure) => structure.loc,
            SourceUnitPart::EventDefinition(event) => event.loc,
            SourceUnitPart::ErrorDefinition(error) => error.loc,
            SourceUnitPart::FunctionDefinition(function) => function.loc(),
            SourceUnitPart::VariableDefinition(variable) => variable.loc,
            SourceUnitPart::TypeDefinition(def) => def.loc,
            SourceUnitPart::Using(using) => using.loc,
            SourceUnitPart::Annotation(annotation) => annotation.loc,
        }
    }
}

impl LineOfCode for ContractPart {
    fn loc(&self) -> Loc {
        match self {
            ContractPart::StructDefinition(structure) => structure.loc,
            ContractPart::EventDefinition(event) => event.loc,
            ContractPart::ErrorDefinition(error) => error.loc,
            ContractPart::EnumDefinition(enumeration) => enumeration.loc,
            ContractPart::VariableDefinition(variable) => variable.loc,
            ContractPart::FunctionDefinition(function) => function.loc(),
            ContractPart::StraySemicolon(loc) => *loc,
            ContractPart::Using(using) => using.loc,
            ContractPart::TypeDefinition(def) => def.loc,
            ContractPart::Annotation(annotation) => annotation.loc,
        }
    }
}

impl LineOfCode for YulStatement {
    fn loc(&self) -> Loc {
        match self {
            YulStatement::Assign(loc, _, _) |
            YulStatement::If(loc, _, _) |
            YulStatement::Leave(loc) |
            YulStatement::Break(loc) |
            YulStatement::VariableDeclaration(loc, _, _) |
            YulStatement::Continue(loc) => *loc,
            YulStatement::For(f) => f.loc,
            YulStatement::Block(b) => b.loc,
            YulStatement::Switch(s) => s.loc,
            YulStatement::FunctionDefinition(f) => f.loc,
            YulStatement::FunctionCall(f) => f.loc,
            YulStatement::Error(loc) => *loc,
        }
    }
}

impl LineOfCode for YulExpression {
    fn loc(&self) -> Loc {
        match self {
            YulExpression::BoolLiteral(loc, _, _) |
            YulExpression::NumberLiteral(loc, _, _, _) |
            YulExpression::HexNumberLiteral(loc, _, _) |
            YulExpression::SuffixAccess(loc, _, _) => *loc,
            YulExpression::StringLiteral(literal, _) => literal.loc,
            YulExpression::Variable(ident) => ident.loc,
            YulExpression::FunctionCall(f) => f.loc,
            YulExpression::HexStringLiteral(lit, _) => lit.loc,
        }
    }
}

impl LineOfCode for FunctionDefinition {
    fn loc(&self) -> Loc {
        Loc::File(
            self.loc.file_no(),
            self.loc.start(),
            self.body
                .as_ref()
                .map(|body| LineOfCode::loc(body).end())
                .unwrap_or_else(|| self.loc.end()),
        )
    }
}

impl LineOfCode for Statement {
    fn loc(&self) -> Loc {
        use Statement::*;
        match self {
            Block { loc, unchecked: _checked, statements: _statements } => *loc,
            Assembly { loc, dialect: _dialect, block: _block, flags: _flags } => *loc,
            Args(loc, _) => *loc,
            If(loc, _, _, _) => *loc,
            While(loc, _, _) => *loc,
            Expression(_, expr) => LineOfCode::loc(&expr),
            VariableDefinition(loc, _, _) => *loc,
            For(loc, _, _, _, _) => *loc,
            DoWhile(loc, _, _) => *loc,
            Continue(loc) => *loc,
            Break(loc) => *loc,
            Return(loc, _) => *loc,
            Revert(loc, _, _) => *loc,
            RevertNamedArgs(loc, _, _) => *loc,
            Emit(loc, _) => *loc,
            Try(loc, _, _, _) => *loc,
            Statement::Error(loc) => *loc,
        }
    }
}

impl LineOfCode for Expression {
    fn loc(&self) -> Loc {
        use Expression::*;
        match self {
            PostIncrement(loc, _) => *loc,
            PostDecrement(loc, _) => *loc,
            New(loc, _) => *loc,
            ArraySubscript(loc, _, _) => *loc,
            ArraySlice(loc, _, _, _) => *loc,
            MemberAccess(loc, _, _) => *loc,
            FunctionCall(loc, _, _) => *loc,
            FunctionCallBlock(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            NamedFunctionCall(loc, _, _) => *loc,
            Not(loc, expr) => Loc::File(loc.file_no(), loc.start(), expr.loc().end()),
            Complement(loc, expr) => Loc::File(loc.file_no(), loc.start(), expr.loc().end()),
            Delete(loc, expr) => Loc::File(loc.file_no(), loc.start(), expr.loc().end()),
            PreIncrement(loc, expr) => Loc::File(loc.file_no(), loc.start(), expr.loc().end()),
            PreDecrement(loc, expr) => Loc::File(loc.file_no(), loc.start(), expr.loc().end()),
            UnaryPlus(loc, _) => *loc,
            UnaryMinus(loc, _) => *loc,
            Power(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Multiply(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Divide(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Modulo(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Add(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Subtract(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            ShiftLeft(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            ShiftRight(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            BitwiseAnd(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            BitwiseXor(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            BitwiseOr(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Less(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            More(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            LessEqual(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            MoreEqual(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Equal(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            NotEqual(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            And(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Or(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            ConditionalOperator(loc, left, _, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            Assign(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignOr(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignAnd(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignXor(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignShiftLeft(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignShiftRight(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignAdd(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignSubtract(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignMultiply(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignDivide(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            AssignModulo(loc, left, right) => Loc::File(
                loc.file_no(),
                LineOfCode::loc(&left).start(),
                LineOfCode::loc(&right).end(),
            ),
            BoolLiteral(loc, _) => *loc,
            NumberLiteral(loc, _, _) => *loc,
            RationalNumberLiteral(loc, _, _, _) => *loc,
            HexNumberLiteral(loc, _) => *loc,
            StringLiteral(literals) => {
                let left_loc = literals.first().unwrap().loc;
                Loc::File(left_loc.file_no(), left_loc.start(), literals.last().unwrap().loc.end())
            }
            HexLiteral(literals) => {
                let left_loc = literals.first().unwrap().loc;
                Loc::File(left_loc.file_no(), left_loc.start(), literals.last().unwrap().loc.end())
            }
            Type(loc, _) => *loc,
            AddressLiteral(loc, _) => *loc,
            Variable(ident) => ident.loc,
            List(loc, _) => *loc,
            ArrayLiteral(loc, _) => *loc,
            Unit(loc, _, _) => *loc,
            This(loc) => *loc,
            Parenthesis(loc, _) => *loc,
        }
    }
}

impl LineOfCode for Comment {
    fn loc(&self) -> Loc {
        match self {
            Comment::Line(loc, _) => *loc,
            Comment::Block(loc, _) => *loc,
            Comment::DocLine(loc, _) => *loc,
            Comment::DocBlock(loc, _) => *loc,
        }
    }
}

impl LineOfCode for FunctionAttribute {
    fn loc(&self) -> Loc {
        match self {
            FunctionAttribute::Mutability(mutability) => mutability.loc(),
            // visibility will always have a location in a function attribute
            FunctionAttribute::Visibility(visibility) => visibility.loc().unwrap(),
            FunctionAttribute::Virtual(loc) |
            FunctionAttribute::Immutable(loc) |
            FunctionAttribute::Override(loc, _) |
            FunctionAttribute::BaseOrModifier(loc, _) |
            FunctionAttribute::Error(loc) => *loc,
        }
    }
}

impl LineOfCode for VariableAttribute {
    fn loc(&self) -> Loc {
        match self {
            // visibility will always have a location in a function attribute
            VariableAttribute::Visibility(visibility) => visibility.loc().unwrap(),
            VariableAttribute::Constant(loc) |
            VariableAttribute::Immutable(loc) |
            VariableAttribute::Override(loc, _) => *loc,
        }
    }
}

impl<T> OptionalLineOfCode for Vec<(Loc, T)> {
    fn loc(&self) -> Option<Loc> {
        if self.is_empty() {
            return None
        }
        let mut locs = self.iter().map(|(loc, _)| loc).collect::<Vec<_>>();
        locs.sort();
        let mut loc = **locs.first().unwrap();
        loc.use_end_from(locs.pop().unwrap());
        Some(loc)
    }
}

impl OptionalLineOfCode for SourceUnit {
    fn loc(&self) -> Option<Loc> {
        self.0.get(0).map(|unit| *unit.loc())
    }
}

impl LineOfCode for Unit {
    fn loc(&self) -> Loc {
        match *self {
            Unit::Seconds(loc) => loc,
            Unit::Minutes(loc) => loc,
            Unit::Hours(loc) => loc,
            Unit::Days(loc) => loc,
            Unit::Weeks(loc) => loc,
            Unit::Wei(loc) => loc,
            Unit::Gwei(loc) => loc,
            Unit::Ether(loc) => loc,
        }
    }
}

macro_rules! impl_loc {
    ($type:ty) => {
        impl LineOfCode for $type {
            fn loc(&self) -> Loc {
                self.loc
            }
        }
    };
}

impl_loc! { IdentifierPath }
impl_loc! { YulTypedIdentifier }
impl_loc! { EventParameter }
impl_loc! { ErrorParameter }

/// Extra helpers for Locs
pub trait LocExt {
    fn with_start_from(self, other: &Self) -> Self;
    fn with_end_from(self, other: &Self) -> Self;
    fn with_start(self, start: usize) -> Self;
    fn with_end(self, end: usize) -> Self;
    fn range(self) -> std::ops::Range<usize>;
}

impl LocExt for Loc {
    fn with_start_from(mut self, other: &Self) -> Self {
        self.use_start_from(other);
        self
    }
    fn with_end_from(mut self, other: &Self) -> Self {
        self.use_end_from(other);
        self
    }
    fn with_start(self, start: usize) -> Self {
        Loc::File(self.file_no(), start, self.end())
    }
    fn with_end(self, end: usize) -> Self {
        Loc::File(self.file_no(), self.start(), end)
    }
    fn range(self) -> std::ops::Range<usize> {
        self.start()..self.end()
    }
}
