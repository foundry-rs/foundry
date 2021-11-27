//! Visitor helpers to traverse the [solang](https://github.com/hyperledger-labs/solang) solidity AST

use solang::parser::{self, pt::*};

/// The error type a visitor may return
pub type VResult = Result<(), Box<dyn std::error::Error>>;

/// A trait that is invoked while traversing the  solidity AST.
///
/// Each method of the `Visitor` trait is a hook that can be potentially overridden.
pub trait Visitor {
    fn visit_stmt(&mut self, _stmt: &mut Statement) -> VResult {
        Ok(())
    }

    fn visit_assembly(&mut self, _stmt: &mut AssemblyStatement) -> VResult {
        Ok(())
    }

    fn visit_arg(&mut self, _stmt: &mut NamedArgument) -> VResult {
        Ok(())
    }

    fn visit_expr(&mut self, _expr: &mut Expression) -> VResult {
        Ok(())
    }

    fn visit_emit(&mut self, _stmt: &mut Expression) -> VResult {
        Ok(())
    }

    fn visit_var_def(
        &mut self,
        _v: &mut VariableDeclaration,
        _expr: &mut Option<Expression>,
    ) -> VResult {
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

    fn visit_function(&mut self, _fun: &mut FunctionDefinition) -> VResult {
        Ok(())
    }

    fn visit_contract(&mut self, _contract: &mut ContractDefinition) -> VResult {
        Ok(())
    }

    // TODO more visit callbacks
}

/// The trait for solidity AST nodes
pub trait Visitable {
    fn visit(&mut self, v: &mut dyn Visitor) -> VResult;
}

impl<T: Visitable> Visitable for Vec<T> {
    fn visit(&mut self, v: &mut dyn Visitor) -> VResult {
        for t in self {
            t.visit(v)?;
        }
        Ok(())
    }
}

impl Visitable for SourceUnit {
    fn visit(&mut self, v: &mut dyn Visitor) -> VResult {
        self.0.visit(v)
    }
}

// TODO implement Visitable for all AST nodes

impl Visitable for SourceUnitPart {
    fn visit(&mut self, v: &mut dyn Visitor) -> VResult {
        todo!()
    }
}
