//! Visitor helpers to traverse the [solang](https://github.com/hyperledger-labs/solang) solidity AST

use solang::parser::pt::*;

/// The error type a visitor may return
pub type VResult = Result<(), Box<dyn std::error::Error>>;

/// A trait that is invoked while traversing the  solidity AST.
///
/// Each method of the `Visitor` trait is a hook that can be potentially overridden.
pub trait Visitor {
    fn visit_source_unit(&mut self, _source_unit: &SourceUnit) -> VResult {
        Ok(())
    }

    fn visit_stmt(&mut self, _stmt: &Statement) -> VResult {
        Ok(())
    }

    fn visit_assembly(&mut self, _stmt: &AssemblyStatement) -> VResult {
        Ok(())
    }

    fn visit_arg(&mut self, _stmt: &NamedArgument) -> VResult {
        Ok(())
    }

    fn visit_expr(&mut self, _expr: &Expression) -> VResult {
        Ok(())
    }

    fn visit_emit(&mut self, _stmt: &Expression) -> VResult {
        Ok(())
    }

    fn visit_var_def(&mut self, _var: &VariableDefinition) -> VResult {
        Ok(())
    }

    fn visit_return(&mut self, _expr: &Option<Expression>) -> VResult {
        Ok(())
    }

    fn visit_break(&mut self) -> VResult {
        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        Ok(())
    }

    fn visit_do_while(&mut self, _stmt: &Statement, _expr: &Expression) -> VResult {
        Ok(())
    }

    fn visit_while(&mut self, _expr: &Expression, _stmt: &Statement) -> VResult {
        Ok(())
    }

    fn visit_function(&mut self, _func: &FunctionDefinition) -> VResult {
        Ok(())
    }

    fn visit_contract(&mut self, _contract: &ContractDefinition) -> VResult {
        Ok(())
    }

    fn visit_struct(&mut self, _struct: &StructDefinition) -> VResult {
        Ok(())
    }

    fn visit_event(&mut self, _event: &EventDefinition) -> VResult {
        Ok(())
    }

    fn visit_event_parameter(&mut self, _param: &EventParameter) -> VResult {
        Ok(())
    }

    fn visit_enum(&mut self, _enum: &EnumDefinition) -> VResult {
        Ok(())
    }

    fn visit_stray(&mut self) -> VResult {
        Ok(())
    }

    fn visit_using(&mut self, _using: &Using) -> VResult {
        Ok(())
    }

    fn visit_import(&mut self, _import: &Import) -> VResult {
        Ok(())
    }

    fn visit_pragma(&mut self, _ident: &Identifier, _str: &StringLiteral) -> VResult {
        Ok(())
    }

    // TODO more visit callbacks
}

/// The trait for solidity AST nodes
pub trait Visitable {
    fn visit(&self, v: &mut impl Visitor) -> VResult;
}

impl<T: Visitable> Visitable for Vec<T> {
    fn visit(&self, v: &mut impl Visitor) -> VResult {
        for t in self {
            t.visit(v)?;
        }
        Ok(())
    }
}

impl Visitable for SourceUnit {
    fn visit(&self, v: &mut impl Visitor) -> VResult {
        v.visit_source_unit(self)
    }
}

// TODO implement Visitable for all AST nodes

impl Visitable for SourceUnitPart {
    fn visit(&self, v: &mut impl Visitor) -> VResult {
        match self {
            SourceUnitPart::ContractDefinition(contract) => v.visit_contract(contract),
            SourceUnitPart::PragmaDirective(_, ident, str) => v.visit_pragma(ident, str),
            SourceUnitPart::ImportDirective(_, import) => v.visit_import(import),
            SourceUnitPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            SourceUnitPart::StructDefinition(structure) => v.visit_struct(structure),
            SourceUnitPart::EventDefinition(event) => v.visit_event(event),
            SourceUnitPart::FunctionDefinition(function) => v.visit_function(function),
            SourceUnitPart::VariableDefinition(variable) => v.visit_var_def(variable),
            SourceUnitPart::StraySemicolon(_) => v.visit_stray(),
        }
    }
}

impl Visitable for ContractPart {
    fn visit(&self, v: &mut impl Visitor) -> VResult {
        match self {
            ContractPart::StructDefinition(structure) => v.visit_struct(structure),
            ContractPart::EventDefinition(event) => v.visit_event(event),
            ContractPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            ContractPart::VariableDefinition(variable) => v.visit_var_def(variable),
            ContractPart::FunctionDefinition(function) => v.visit_function(function),
            ContractPart::StraySemicolon(_) => v.visit_stray(),
            ContractPart::Using(using) => v.visit_using(using),
        }
    }
}
