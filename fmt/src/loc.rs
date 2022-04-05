use solang_parser::pt::{
    CodeLocation, ContractPart, FunctionDefinition, Import, Loc, SourceUnitPart, YulExpression,
    YulStatement,
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
            YulExpression::NumberLiteral(loc, _, _) |
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
            self.body.as_ref().map(|body| body.loc().end()).unwrap_or_else(|| self.loc.end()),
        )
    }
}
