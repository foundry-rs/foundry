use solang_parser::pt::{
    AssemblyExpression, AssemblyStatement, ContractPart, FunctionDefinition, Import, Loc,
    SourceUnitPart, Statement,
};

pub trait LineOfCode {
    fn loc(&self) -> Loc;
}

impl LineOfCode for SourceUnitPart {
    fn loc(&self) -> Loc {
        match self {
            SourceUnitPart::ContractDefinition(contract) => contract.loc,
            SourceUnitPart::PragmaDirective(loc, _, _, _) | SourceUnitPart::StraySemicolon(loc) => {
                *loc
            }
            SourceUnitPart::ImportDirective(_, import) => *match import {
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
            AssemblyStatement::Assign(loc, _, _) |
            AssemblyStatement::If(loc, _, _) |
            AssemblyStatement::For(loc, _, _, _, _) |
            AssemblyStatement::Switch(loc, _, _, _) |
            AssemblyStatement::Leave(loc) |
            AssemblyStatement::Break(loc) |
            AssemblyStatement::VariableDeclaration(loc, _, _) |
            AssemblyStatement::Block(loc, _) |
            AssemblyStatement::Continue(loc) => *loc,
            AssemblyStatement::Expression(expr) => expr.loc(),
            AssemblyStatement::FunctionDefinition(f) => f.loc,
            AssemblyStatement::FunctionCall(f) => f.loc,
        }
    }
}

impl LineOfCode for AssemblyExpression {
    fn loc(&self) -> Loc {
        *match self {
            AssemblyExpression::BoolLiteral(loc, _, _) |
            AssemblyExpression::NumberLiteral(loc, _, _) |
            AssemblyExpression::HexNumberLiteral(loc, _, _) |
            AssemblyExpression::Assign(loc, _, _) |
            AssemblyExpression::LetAssign(loc, _, _) |
            AssemblyExpression::Member(loc, _, _) |
            AssemblyExpression::Subscript(loc, _, _) => loc,
            AssemblyExpression::StringLiteral(literal, _) => &literal.loc,
            AssemblyExpression::Variable(ident) => &ident.loc,
            AssemblyExpression::FunctionCall(f) => &f.loc,
        }
    }
}

impl LineOfCode for FunctionDefinition {
    fn loc(&self) -> Loc {
        Loc::File(
            self.loc.file_no(),
            self.loc.start(),
            self.body.as_ref().map(|body| body.loc().end()).unwrap_or_else(|| self.loc.end()),
        )
    }
}
