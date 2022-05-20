use solang_parser::pt::*;

pub trait LineOfCode {
    fn loc(&self) -> Loc;
}

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
            SourceUnitPart::FunctionDefinition(function) => function.loc,
            SourceUnitPart::VariableDefinition(variable) => variable.loc,
            SourceUnitPart::TypeDefinition(def) => def.loc,
            SourceUnitPart::DocComment(doc) => doc.loc,
            SourceUnitPart::Using(using) => using.loc,
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
            ContractPart::DocComment(doc) => doc.loc,
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
        }
    }
}

impl LineOfCode for YulExpression {
    fn loc(&self) -> Loc {
        match self {
            YulExpression::BoolLiteral(loc, _, _) |
            YulExpression::NumberLiteral(loc, _, _, _) |
            YulExpression::HexNumberLiteral(loc, _, _) |
            YulExpression::Member(loc, _, _) => *loc,
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
            Assembly { loc, dialect: _dialect, block: _block } => *loc,
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
            DocComment(comment) => comment.loc,
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
            Ternary(loc, left, _, right) => Loc::File(
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
        }
    }
}

impl OptionalLineOfCode for FunctionAttribute {
    fn loc(&self) -> Option<Loc> {
        match self {
            FunctionAttribute::Mutability(mutability) => Some(mutability.loc()),
            FunctionAttribute::Visibility(visibility) => visibility.loc(),
            FunctionAttribute::Virtual(loc) |
            FunctionAttribute::Immutable(loc) |
            FunctionAttribute::Override(loc, _) |
            FunctionAttribute::BaseOrModifier(loc, _) => Some(*loc),
        }
    }
}
