use solang_parser::pt::{AssemblyExpression, AssemblyStatement, ContractPart, Loc, Statement};

pub trait LineOfCode {
    fn loc(&self) -> Loc;
}

impl LineOfCode for ContractPart {
    fn loc(&self) -> Loc {
        match self {
            ContractPart::StructDefinition(structure) => structure.loc,
            ContractPart::EventDefinition(event) => event.loc,
            ContractPart::EnumDefinition(enumeration) => enumeration.loc,
            ContractPart::VariableDefinition(variable) => variable.loc,
            ContractPart::FunctionDefinition(function) => function.loc,
            ContractPart::StraySemicolon(loc) => *loc,
            ContractPart::Using(using) => using.loc,
        }
    }
}

impl LineOfCode for Statement {
    fn loc(&self) -> Loc {
        self.loc()
    }
}

impl LineOfCode for AssemblyStatement {
    fn loc(&self) -> Loc {
        match self {
            AssemblyStatement::Assign(loc, _, _) | AssemblyStatement::LetAssign(loc, _, _) => *loc,
            AssemblyStatement::Expression(expr) => expr.loc(),
        }
    }
}

impl LineOfCode for AssemblyExpression {
    fn loc(&self) -> Loc {
        *match self {
            AssemblyExpression::BoolLiteral(loc, _) |
            AssemblyExpression::NumberLiteral(loc, _) |
            AssemblyExpression::HexNumberLiteral(loc, _) |
            AssemblyExpression::Assign(loc, _, _) |
            AssemblyExpression::LetAssign(loc, _, _) |
            AssemblyExpression::Function(loc, _, _) |
            AssemblyExpression::Member(loc, _, _) |
            AssemblyExpression::Subscript(loc, _, _) => loc,
            AssemblyExpression::StringLiteral(literal) => &literal.loc,
            AssemblyExpression::Variable(ident) => &ident.loc,
        }
    }
}
