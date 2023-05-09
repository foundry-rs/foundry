//! Visitor helpers to traverse the [solang Solidity Parse Tree](solang_parser::pt).

use crate::solang_ext::pt::*;

/// A trait that is invoked while traversing the Solidity Parse Tree.
/// Each method of the [Visitor] trait is a hook that can be potentially overridden.
///
/// Currently the main implementor of this trait is the [`Formatter`](crate::Formatter) struct.
pub trait Visitor {
    type Error: std::error::Error;

    fn visit_source(&mut self, _loc: Loc) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_source_unit(&mut self, _source_unit: &mut SourceUnit) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_contract(&mut self, _contract: &mut ContractDefinition) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_annotation(&mut self, annotation: &mut Annotation) -> Result<(), Self::Error> {
        self.visit_source(annotation.loc)
    }

    fn visit_pragma(
        &mut self,
        loc: Loc,
        _ident: &mut Option<Identifier>,
        _str: &mut Option<StringLiteral>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_import_plain(
        &mut self,
        _loc: Loc,
        _import: &mut StringLiteral,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_import_global(
        &mut self,
        _loc: Loc,
        _global: &mut StringLiteral,
        _alias: &mut Identifier,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        _loc: Loc,
        _imports: &mut [(Identifier, Option<Identifier>)],
        _from: &mut StringLiteral,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_enum(&mut self, _enum: &mut EnumDefinition) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_assembly(
        &mut self,
        loc: Loc,
        _dialect: &mut Option<StringLiteral>,
        _block: &mut YulBlock,
        _flags: &mut Option<Vec<StringLiteral>>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_block(
        &mut self,
        loc: Loc,
        _unchecked: bool,
        _statements: &mut Vec<Statement>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_args(&mut self, loc: Loc, _args: &mut Vec<NamedArgument>) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    /// Don't write semicolon at the end because expressions can appear as both
    /// part of other node and a statement in the function body
    fn visit_expr(&mut self, loc: Loc, _expr: &mut Expression) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_ident(&mut self, loc: Loc, _ident: &mut Identifier) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_ident_path(&mut self, idents: &mut IdentifierPath) -> Result<(), Self::Error> {
        self.visit_source(idents.loc)
    }

    fn visit_emit(&mut self, loc: Loc, _event: &mut Expression) -> Result<(), Self::Error> {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<(), Self::Error> {
        self.visit_source(var.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_var_definition_stmt(
        &mut self,
        loc: Loc,
        _declaration: &mut VariableDeclaration,
        _expr: &mut Option<Expression>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()
    }

    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> Result<(), Self::Error> {
        self.visit_source(var.loc)
    }

    fn visit_return(
        &mut self,
        loc: Loc,
        _expr: &mut Option<Expression>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_revert(
        &mut self,
        loc: Loc,
        _error: &mut Option<IdentifierPath>,
        _args: &mut Vec<Expression>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_revert_named_args(
        &mut self,
        loc: Loc,
        _error: &mut Option<IdentifierPath>,
        _args: &mut Vec<NamedArgument>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_break(&mut self, loc: Loc, _semicolon: bool) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_continue(&mut self, loc: Loc, _semicolon: bool) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    #[allow(clippy::type_complexity)]
    fn visit_try(
        &mut self,
        loc: Loc,
        _expr: &mut Expression,
        _returns: &mut Option<(Vec<(Loc, Option<Parameter>)>, Box<Statement>)>,
        _clauses: &mut Vec<CatchClause>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_if(
        &mut self,
        loc: Loc,
        _cond: &mut Expression,
        _if_branch: &mut Box<Statement>,
        _else_branch: &mut Option<Box<Statement>>,
        _is_first_stmt: bool,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_do_while(
        &mut self,
        loc: Loc,
        _body: &mut Statement,
        _cond: &mut Expression,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_while(
        &mut self,
        loc: Loc,
        _cond: &mut Expression,
        _body: &mut Statement,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_for(
        &mut self,
        loc: Loc,
        _init: &mut Option<Box<Statement>>,
        _cond: &mut Option<Box<Expression>>,
        _update: &mut Option<Box<Statement>>,
        _body: &mut Option<Box<Statement>>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<(), Self::Error> {
        self.visit_source(func.loc())?;
        if func.body.is_none() {
            self.visit_stray_semicolon()?;
        }

        Ok(())
    }

    fn visit_function_attribute(
        &mut self,
        attribute: &mut FunctionAttribute,
    ) -> Result<(), Self::Error> {
        self.visit_source(attribute.loc())?;
        Ok(())
    }

    fn visit_var_attribute(
        &mut self,
        attribute: &mut VariableAttribute,
    ) -> Result<(), Self::Error> {
        self.visit_source(attribute.loc())?;
        Ok(())
    }

    fn visit_base(&mut self, base: &mut Base) -> Result<(), Self::Error> {
        self.visit_source(base.loc)
    }

    fn visit_parameter(&mut self, parameter: &mut Parameter) -> Result<(), Self::Error> {
        self.visit_source(parameter.loc)
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> Result<(), Self::Error> {
        self.visit_source(structure.loc)?;

        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> Result<(), Self::Error> {
        self.visit_source(event.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> Result<(), Self::Error> {
        self.visit_source(param.loc)
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> Result<(), Self::Error> {
        self.visit_source(error.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_error_parameter(&mut self, param: &mut ErrorParameter) -> Result<(), Self::Error> {
        self.visit_source(param.loc)
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> Result<(), Self::Error> {
        self.visit_source(def.loc)
    }

    fn visit_stray_semicolon(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_opening_paren(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_closing_paren(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_newline(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn visit_using(&mut self, using: &mut Using) -> Result<(), Self::Error> {
        self.visit_source(using.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_yul_block(
        &mut self,
        loc: Loc,
        _stmts: &mut Vec<YulStatement>,
        _attempt_single_line: bool,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_yul_expr(&mut self, expr: &mut YulExpression) -> Result<(), Self::Error> {
        self.visit_source(expr.loc())
    }

    fn visit_yul_assignment<T>(
        &mut self,
        loc: Loc,
        _exprs: &mut Vec<T>,
        _expr: &mut Option<&mut YulExpression>,
    ) -> Result<(), Self::Error>
    where
        T: Visitable + CodeLocation,
    {
        self.visit_source(loc)
    }

    fn visit_yul_for(&mut self, stmt: &mut YulFor) -> Result<(), Self::Error> {
        self.visit_source(stmt.loc)
    }

    fn visit_yul_function_call(&mut self, stmt: &mut YulFunctionCall) -> Result<(), Self::Error> {
        self.visit_source(stmt.loc)
    }

    fn visit_yul_fun_def(&mut self, stmt: &mut YulFunctionDefinition) -> Result<(), Self::Error> {
        self.visit_source(stmt.loc)
    }

    fn visit_yul_if(
        &mut self,
        loc: Loc,
        _expr: &mut YulExpression,
        _block: &mut YulBlock,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_yul_leave(&mut self, loc: Loc) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_yul_switch(&mut self, stmt: &mut YulSwitch) -> Result<(), Self::Error> {
        self.visit_source(stmt.loc)
    }

    fn visit_yul_var_declaration(
        &mut self,
        loc: Loc,
        _idents: &mut Vec<YulTypedIdentifier>,
        _expr: &mut Option<YulExpression>,
    ) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }

    fn visit_yul_typed_ident(&mut self, ident: &mut YulTypedIdentifier) -> Result<(), Self::Error> {
        self.visit_source(ident.loc)
    }

    fn visit_parser_error(&mut self, loc: Loc) -> Result<(), Self::Error> {
        self.visit_source(loc)
    }
}

/// All [`solang_parser::pt`] types, such as [Statement], should implement the [Visitable] trait
/// that accepts a trait [Visitor] implementation, which has various callback handles for Solidity
/// Parse Tree nodes.
///
/// We want to take a `&mut self` to be able to implement some advanced features in the future such
/// as modifying the Parse Tree before formatting it.
pub trait Visitable {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor;
}

impl<T> Visitable for &mut T
where
    T: Visitable,
{
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        T::visit(self, v)
    }
}

impl<T> Visitable for Option<T>
where
    T: Visitable,
{
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        if let Some(inner) = self.as_mut() {
            inner.visit(v)
        } else {
            Ok(())
        }
    }
}

impl<T> Visitable for Box<T>
where
    T: Visitable,
{
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        T::visit(self, v)
    }
}

impl<T> Visitable for Vec<T>
where
    T: Visitable,
{
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        for item in self.iter_mut() {
            item.visit(v)?;
        }
        Ok(())
    }
}

impl Visitable for SourceUnitPart {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        match self {
            SourceUnitPart::ContractDefinition(contract) => v.visit_contract(contract),
            SourceUnitPart::PragmaDirective(loc, ident, str) => v.visit_pragma(*loc, ident, str),
            SourceUnitPart::ImportDirective(import) => import.visit(v),
            SourceUnitPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            SourceUnitPart::StructDefinition(structure) => v.visit_struct(structure),
            SourceUnitPart::EventDefinition(event) => v.visit_event(event),
            SourceUnitPart::ErrorDefinition(error) => v.visit_error(error),
            SourceUnitPart::FunctionDefinition(function) => v.visit_function(function),
            SourceUnitPart::VariableDefinition(variable) => v.visit_var_definition(variable),
            SourceUnitPart::TypeDefinition(def) => v.visit_type_definition(def),
            SourceUnitPart::StraySemicolon(_) => v.visit_stray_semicolon(),
            SourceUnitPart::Using(using) => v.visit_using(using),
            SourceUnitPart::Annotation(annotation) => v.visit_annotation(annotation),
        }
    }
}

impl Visitable for Import {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        match self {
            Import::Plain(import, loc) => v.visit_import_plain(*loc, import),
            Import::GlobalSymbol(global, import_as, loc) => {
                v.visit_import_global(*loc, global, import_as)
            }
            Import::Rename(from, imports, loc) => v.visit_import_renames(*loc, imports, from),
        }
    }
}

impl Visitable for ContractPart {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        match self {
            ContractPart::StructDefinition(structure) => v.visit_struct(structure),
            ContractPart::EventDefinition(event) => v.visit_event(event),
            ContractPart::ErrorDefinition(error) => v.visit_error(error),
            ContractPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            ContractPart::VariableDefinition(variable) => v.visit_var_definition(variable),
            ContractPart::FunctionDefinition(function) => v.visit_function(function),
            ContractPart::TypeDefinition(def) => v.visit_type_definition(def),
            ContractPart::StraySemicolon(_) => v.visit_stray_semicolon(),
            ContractPart::Using(using) => v.visit_using(using),
            ContractPart::Annotation(annotation) => v.visit_annotation(annotation),
        }
    }
}

impl Visitable for Statement {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        match self {
            Statement::Block { loc, unchecked, statements } => {
                v.visit_block(*loc, *unchecked, statements)
            }
            Statement::Assembly { loc, dialect, block, flags } => {
                v.visit_assembly(*loc, dialect, block, flags)
            }
            Statement::Args(loc, args) => v.visit_args(*loc, args),
            Statement::If(loc, cond, if_branch, else_branch) => {
                v.visit_if(*loc, cond, if_branch, else_branch, true)
            }
            Statement::While(loc, cond, body) => v.visit_while(*loc, cond, body),
            Statement::Expression(loc, expr) => {
                v.visit_expr(*loc, expr)?;
                v.visit_stray_semicolon()
            }
            Statement::VariableDefinition(loc, declaration, expr) => {
                v.visit_var_definition_stmt(*loc, declaration, expr)
            }
            Statement::For(loc, init, cond, update, body) => {
                v.visit_for(*loc, init, cond, update, body)
            }
            Statement::DoWhile(loc, body, cond) => v.visit_do_while(*loc, body, cond),
            Statement::Continue(loc) => v.visit_continue(*loc, true),
            Statement::Break(loc) => v.visit_break(*loc, true),
            Statement::Return(loc, expr) => v.visit_return(*loc, expr),
            Statement::Revert(loc, error, args) => v.visit_revert(*loc, error, args),
            Statement::RevertNamedArgs(loc, error, args) => {
                v.visit_revert_named_args(*loc, error, args)
            }
            Statement::Emit(loc, event) => v.visit_emit(*loc, event),
            Statement::Try(loc, expr, returns, clauses) => {
                v.visit_try(*loc, expr, returns, clauses)
            }
            Statement::Error(loc) => v.visit_parser_error(*loc),
        }
    }
}

impl Visitable for Loc {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        v.visit_source(*self)
    }
}

impl Visitable for Expression {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        v.visit_expr(self.loc(), self)
    }
}

impl Visitable for Identifier {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        v.visit_ident(self.loc, self)
    }
}

impl Visitable for VariableDeclaration {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        v.visit_var_declaration(self)
    }
}

impl Visitable for YulBlock {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        v.visit_yul_block(self.loc, self.statements.as_mut(), false)
    }
}

impl Visitable for YulStatement {
    fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
    where
        V: Visitor,
    {
        match self {
            YulStatement::Assign(loc, exprs, expr) => {
                v.visit_yul_assignment(*loc, exprs, &mut Some(expr))
            }
            YulStatement::Block(block) => {
                v.visit_yul_block(block.loc, block.statements.as_mut(), false)
            }
            YulStatement::Break(loc) => v.visit_break(*loc, false),
            YulStatement::Continue(loc) => v.visit_continue(*loc, false),
            YulStatement::For(stmt) => v.visit_yul_for(stmt),
            YulStatement::FunctionCall(stmt) => v.visit_yul_function_call(stmt),
            YulStatement::FunctionDefinition(stmt) => v.visit_yul_fun_def(stmt),
            YulStatement::If(loc, expr, block) => v.visit_yul_if(*loc, expr, block),
            YulStatement::Leave(loc) => v.visit_yul_leave(*loc),
            YulStatement::Switch(stmt) => v.visit_yul_switch(stmt),
            YulStatement::VariableDeclaration(loc, idents, expr) => {
                v.visit_yul_var_declaration(*loc, idents, expr)
            }
            YulStatement::Error(loc) => v.visit_parser_error(*loc),
        }
    }
}

macro_rules! impl_visitable {
    ($type:ty, $func:ident) => {
        impl Visitable for $type {
            fn visit<V>(&mut self, v: &mut V) -> Result<(), V::Error>
            where
                V: Visitor,
            {
                v.$func(self)
            }
        }
    };
}

impl_visitable!(SourceUnit, visit_source_unit);
impl_visitable!(FunctionAttribute, visit_function_attribute);
impl_visitable!(VariableAttribute, visit_var_attribute);
impl_visitable!(Parameter, visit_parameter);
impl_visitable!(Base, visit_base);
impl_visitable!(EventParameter, visit_event_parameter);
impl_visitable!(ErrorParameter, visit_error_parameter);
impl_visitable!(IdentifierPath, visit_ident_path);
impl_visitable!(YulExpression, visit_yul_expr);
impl_visitable!(YulTypedIdentifier, visit_yul_typed_ident);
