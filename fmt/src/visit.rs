//! Visitor helpers to traverse the [solang](https://github.com/hyperledger-labs/solang) Solidity Parse Tree

use crate::loc::LineOfCode;
use solang_parser::pt::*;

/// The error type a [Visitor] may return
pub type VResult = Result<(), Box<dyn std::error::Error>>;

/// A trait that is invoked while traversing the Solidity Parse Tree.
/// Each method of the [Visitor] trait is a hook that can be potentially overridden.
///
/// Currently the main implementor of this trait is the [`Formatter`](crate::Formatter) struct.
pub trait Visitor {
    fn visit_source(&mut self, _loc: Loc) -> VResult;

    fn visit_source_unit(&mut self, _source_unit: &mut SourceUnit) -> VResult {
        Ok(())
    }

    fn visit_doc_comment(&mut self, _doc_comment: &mut DocComment) -> VResult {
        Ok(())
    }

    fn visit_doc_comments(&mut self, _doc_comments: &mut [DocComment]) -> VResult {
        Ok(())
    }

    fn visit_contract(&mut self, _contract: &mut ContractDefinition) -> VResult {
        Ok(())
    }

    fn visit_pragma(&mut self, _ident: &mut Identifier, _str: &mut StringLiteral) -> VResult {
        Ok(())
    }

    fn visit_import_plain(&mut self, _import: &mut StringLiteral) -> VResult {
        Ok(())
    }

    fn visit_import_global(
        &mut self,
        _global: &mut StringLiteral,
        _alias: &mut Identifier,
    ) -> VResult {
        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        _imports: &mut [(Identifier, Option<Identifier>)],
        _from: &mut StringLiteral,
    ) -> VResult {
        Ok(())
    }

    fn visit_enum(&mut self, _enum: &mut EnumDefinition) -> VResult {
        Ok(())
    }

    fn visit_statement(&mut self, stmt: &mut Statement) -> VResult {
        self.visit_source(stmt.loc())?;

        Ok(())
    }

    fn visit_assembly(&mut self, stmt: &mut YulStatement) -> VResult {
        self.visit_source(stmt.loc())?;

        Ok(())
    }

    fn visit_arg(&mut self, arg: &mut NamedArgument) -> VResult {
        self.visit_source(arg.loc)?;

        Ok(())
    }

    fn visit_expr(&mut self, _expr: &mut Expression) -> VResult {
        Ok(())
    }

    fn visit_emit(&mut self, _expr: &mut Expression) -> VResult {
        Ok(())
    }

    fn visit_var_def(&mut self, var: &mut VariableDefinition) -> VResult {
        if !var.doc.is_empty() {
            self.visit_doc_comments(&mut var.doc)?;
            self.visit_newline()?;
        }
        self.visit_source(var.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_return(&mut self, _expr: &mut Option<Expression>) -> VResult {
        Ok(())
    }

    fn visit_break(&mut self) -> VResult {
        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        Ok(())
    }

    fn visit_do_while(&mut self, _stmt: &mut Statement, _expr: &mut Expression) -> VResult {
        Ok(())
    }

    fn visit_while(&mut self, _expr: &mut Expression, _stmt: &mut Statement) -> VResult {
        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> VResult {
        if !func.doc.is_empty() {
            self.visit_doc_comments(&mut func.doc)?;
            self.visit_newline()?;
        }
        self.visit_source(func.loc())?;
        if func.body.is_none() {
            self.visit_stray_semicolon()?;
        }

        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> VResult {
        if !structure.doc.is_empty() {
            self.visit_doc_comments(&mut structure.doc)?;
            self.visit_newline()?;
        }
        self.visit_source(structure.loc)?;

        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> VResult {
        if !event.doc.is_empty() {
            self.visit_doc_comments(&mut event.doc)?;
            self.visit_newline()?;
        }
        self.visit_source(event.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> VResult {
        if !error.doc.is_empty() {
            self.visit_doc_comments(&mut error.doc)?;
            self.visit_newline()?;
        }
        self.visit_source(error.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> VResult {
        self.visit_source(param.loc)?;

        Ok(())
    }

    fn visit_stray_semicolon(&mut self) -> VResult;

    fn visit_newline(&mut self) -> VResult;

    fn visit_using(&mut self, using: &mut Using) -> VResult {
        self.visit_source(using.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }
}

/// All `solang::pt::*` types, such as [Statement](solang::pt::Statement) should implement the
/// [Visitable] trait that accepts a trait [Visitor] implementation, which has various callback
/// handles for Solidity Parse Tree nodes.
///
/// We want to take a `&mut self` to be able to implement some advanced features in the future such
/// as modifying the Parse Tree before formatting it.
pub trait Visitable {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult;
}

impl Visitable for Loc {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_source(*self)?;

        Ok(())
    }
}

impl Visitable for DocComment {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_doc_comment(self)?;

        Ok(())
    }
}

impl Visitable for Vec<DocComment> {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_doc_comments(self)?;

        Ok(())
    }
}

impl Visitable for SourceUnit {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_source_unit(self)
    }
}

impl Visitable for SourceUnitPart {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        match self {
            SourceUnitPart::ContractDefinition(contract) => v.visit_contract(contract),
            SourceUnitPart::PragmaDirective(_, _, ident, str) => v.visit_pragma(ident, str),
            SourceUnitPart::ImportDirective(_, import) => import.visit(v),
            SourceUnitPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            SourceUnitPart::StructDefinition(structure) => v.visit_struct(structure),
            SourceUnitPart::EventDefinition(event) => v.visit_event(event),
            SourceUnitPart::ErrorDefinition(error) => v.visit_error(error),
            SourceUnitPart::FunctionDefinition(function) => v.visit_function(function),
            SourceUnitPart::VariableDefinition(variable) => v.visit_var_def(variable),
            SourceUnitPart::StraySemicolon(_) => v.visit_stray_semicolon(),
        }
    }
}

impl Visitable for Import {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        match self {
            Import::Plain(import, _) => v.visit_import_plain(import),
            Import::GlobalSymbol(global, import_as, _) => v.visit_import_global(global, import_as),
            Import::Rename(from, imports, _) => v.visit_import_renames(imports, from),
        }
    }
}

impl Visitable for ContractPart {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        match self {
            ContractPart::StructDefinition(structure) => v.visit_struct(structure),
            ContractPart::EventDefinition(event) => v.visit_event(event),
            ContractPart::ErrorDefinition(error) => v.visit_error(error),
            ContractPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            ContractPart::VariableDefinition(variable) => v.visit_var_def(variable),
            ContractPart::FunctionDefinition(function) => v.visit_function(function),
            ContractPart::StraySemicolon(_) => v.visit_stray_semicolon(),
            ContractPart::Using(using) => v.visit_using(using),
        }
    }
}

impl Visitable for Statement {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_statement(self)?;

        Ok(())
    }
}
