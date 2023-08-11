use super::find::UnsafeCheatcodes;
use forge_fmt::{Visitable, Visitor};
use solang_parser::pt::{
    ContractDefinition, Expression, FunctionDefinition, IdentifierPath, Loc, Parameter, SourceUnit,
    Statement, TypeDefinition, VariableDeclaration, VariableDefinition,
};
use std::convert::Infallible;

/// a [`forge_fmt::Visitor` that scans for invocations of cheatcodes
#[derive(Default)]
pub struct CheatcodeVisitor {
    pub cheatcodes: UnsafeCheatcodes,
}

impl Visitor for CheatcodeVisitor {
    type Error = Infallible;

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> Result<(), Self::Error> {
        source_unit.0.visit(self)
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> Result<(), Self::Error> {
        contract.base.visit(self)?;
        contract.parts.visit(self)
    }

    fn visit_block(
        &mut self,
        _loc: Loc,
        _unchecked: bool,
        statements: &mut Vec<Statement>,
    ) -> Result<(), Self::Error> {
        statements.visit(self)
    }

    fn visit_expr(&mut self, _loc: Loc, expr: &mut Expression) -> Result<(), Self::Error> {
        match expr {
            Expression::PostIncrement(_, expr) => {
                expr.visit(self)?;
            }
            Expression::PostDecrement(_, expr) => {
                expr.visit(self)?;
            }
            Expression::New(_, expr) => {
                expr.visit(self)?;
            }
            Expression::ArraySubscript(_, expr1, expr2) => {
                expr1.visit(self)?;
                expr2.visit(self)?;
            }
            Expression::ArraySlice(_, expr1, expr2, expr3) => {
                expr1.visit(self)?;
                expr2.visit(self)?;
                expr3.visit(self)?;
            }
            Expression::Parenthesis(_, expr) => {
                expr.visit(self)?;
            }
            Expression::MemberAccess(_, expr, _) => {
                expr.visit(self)?;
            }
            Expression::FunctionCall(loc, lhs, rhs) => {
                // all cheatcodes are accessd via <vm>.cheatcode
                if let Expression::MemberAccess(_, expr, identifier) = &**lhs {
                    if let Expression::Variable(_) = &**expr {
                        match identifier.name.as_str() {
                            "ffi" => self.cheatcodes.ffi.push(*loc),
                            "readFile" => self.cheatcodes.read_file.push(*loc),
                            "writeFile" => self.cheatcodes.write_file.push(*loc),
                            "readLine" => self.cheatcodes.read_line.push(*loc),
                            "writeLine" => self.cheatcodes.write_line.push(*loc),
                            "closeFile" => self.cheatcodes.close_file.push(*loc),
                            "removeFile" => self.cheatcodes.remove_file.push(*loc),
                            "setEnv" => self.cheatcodes.set_env.push(*loc),
                            "deriveKey" => self.cheatcodes.derive_key.push(*loc),
                            _ => {}
                        }
                    }
                }
                rhs.visit(self)?;
            }
            Expression::FunctionCallBlock(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::NamedFunctionCall(_, lhs, rhs) => {
                lhs.visit(self)?;
                for arg in rhs.iter_mut() {
                    arg.expr.visit(self)?;
                }
            }
            Expression::Not(_, expr) => {
                expr.visit(self)?;
            }
            Expression::BitwiseNot(_, expr) => {
                expr.visit(self)?;
            }
            Expression::Delete(_, expr) => {
                expr.visit(self)?;
            }
            Expression::PreIncrement(_, expr) => {
                expr.visit(self)?;
            }
            Expression::PreDecrement(_, expr) => {
                expr.visit(self)?;
            }
            Expression::UnaryPlus(_, expr) => {
                expr.visit(self)?;
            }
            Expression::Negate(_, expr) => {
                expr.visit(self)?;
            }
            Expression::Power(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Multiply(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Divide(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Modulo(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Add(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Subtract(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::ShiftLeft(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::ShiftRight(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::BitwiseAnd(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::BitwiseXor(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::BitwiseOr(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Less(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::More(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::LessEqual(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::MoreEqual(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Equal(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::NotEqual(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::And(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Or(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::ConditionalOperator(_, llhs, lhs, rhs) => {
                llhs.visit(self)?;
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::Assign(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignOr(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignAnd(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignXor(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignShiftLeft(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignShiftRight(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignAdd(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignSubtract(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignMultiply(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignDivide(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::AssignModulo(_, lhs, rhs) => {
                lhs.visit(self)?;
                rhs.visit(self)?;
            }
            Expression::List(_, param) => {
                for (_, param) in param.iter_mut() {
                    param.visit(self)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn visit_emit(&mut self, _: Loc, expr: &mut Expression) -> Result<(), Self::Error> {
        expr.visit(self)
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<(), Self::Error> {
        var.ty.visit(self)?;
        var.initializer.visit(self)
    }

    fn visit_var_definition_stmt(
        &mut self,
        _: Loc,
        declaration: &mut VariableDeclaration,
        expr: &mut Option<Expression>,
    ) -> Result<(), Self::Error> {
        declaration.visit(self)?;
        expr.visit(self)
    }

    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> Result<(), Self::Error> {
        var.ty.visit(self)
    }

    fn visit_revert(
        &mut self,
        _: Loc,
        _error: &mut Option<IdentifierPath>,
        args: &mut Vec<Expression>,
    ) -> Result<(), Self::Error> {
        args.visit(self)
    }

    fn visit_if(
        &mut self,
        _loc: Loc,
        cond: &mut Expression,
        if_branch: &mut Box<Statement>,
        else_branch: &mut Option<Box<Statement>>,
        _is_frst_stmt: bool,
    ) -> Result<(), Self::Error> {
        cond.visit(self)?;
        if_branch.visit(self)?;
        else_branch.visit(self)
    }

    fn visit_while(
        &mut self,
        _loc: Loc,
        cond: &mut Expression,
        body: &mut Statement,
    ) -> Result<(), Self::Error> {
        cond.visit(self)?;
        body.visit(self)
    }

    fn visit_for(
        &mut self,
        _loc: Loc,
        init: &mut Option<Box<Statement>>,
        cond: &mut Option<Box<Expression>>,
        update: &mut Option<Box<Expression>>,
        body: &mut Option<Box<Statement>>,
    ) -> Result<(), Self::Error> {
        init.visit(self)?;
        cond.visit(self)?;
        update.visit(self)?;
        body.visit(self)
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<(), Self::Error> {
        if let Some(ref mut body) = func.body {
            body.visit(self)?;
        }
        Ok(())
    }

    fn visit_parameter(&mut self, parameter: &mut Parameter) -> Result<(), Self::Error> {
        parameter.ty.visit(self)
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> Result<(), Self::Error> {
        def.ty.visit(self)
    }
}
