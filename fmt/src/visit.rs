//! Visitor helpers to traverse the [solang](https://github.com/hyperledger-labs/solang) Solidity Parse Tree

use crate::solang_ext::*;
use solang_parser::pt::*;

/// The error type a [Visitor] may return
pub type VResult = Result<(), Box<dyn std::error::Error>>;

pub type ParameterList = Vec<(Loc, Option<Parameter>)>;

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

    fn visit_assembly(
        &mut self,
        loc: Loc,
        _dialect: &mut Option<StringLiteral>,
        _block: &mut YulBlock,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_block(
        &mut self,
        loc: Loc,
        _unchecked: bool,
        _statements: &mut Vec<Statement>,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_args(&mut self, loc: Loc, _args: &mut Vec<NamedArgument>) -> VResult {
        self.visit_source(loc)
    }

    /// Don't write semicolon at the end because expressions can appear as both
    /// part of other node and a statement in the function body
    fn visit_expr(&mut self, loc: Loc, _expr: &mut Expression) -> VResult {
        self.visit_source(loc)
    }

    fn visit_emit(&mut self, loc: Loc, _event: &mut Expression) -> VResult {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> VResult {
        self.visit_source(var.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_var_definition_stmt(
        &mut self,
        loc: Loc,
        _declaration: &mut VariableDeclaration,
        _expr: &mut Option<Expression>,
    ) -> VResult {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    /// Don't write semicolon at the end because variable declarations can appear in both
    /// struct definition and function body as a statement
    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> VResult {
        self.visit_source(var.loc)
    }

    fn visit_return(&mut self, loc: Loc, _expr: &mut Option<Expression>) -> VResult {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_revert(
        &mut self,
        loc: Loc,
        _error: &mut Option<Expression>,
        _args: &mut Vec<Expression>,
    ) -> VResult {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_revert_named_args(
        &mut self,
        loc: Loc,
        _error: &mut Option<Expression>,
        _args: &mut Vec<NamedArgument>,
    ) -> VResult {
        self.visit_source(loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_break(&mut self) -> VResult {
        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn visit_try(
        &mut self,
        loc: Loc,
        _expr: &mut Expression,
        _returns: &mut Option<(Vec<(Loc, Option<Parameter>)>, Box<Statement>)>,
        _clauses: &mut Vec<CatchClause>,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_if(
        &mut self,
        loc: Loc,
        _cond: &mut Expression,
        _if_branch: &mut Box<Statement>,
        _else_branch: &mut Option<Box<Statement>>,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_do_while(
        &mut self,
        loc: Loc,
        _cond: &mut Statement,
        _body: &mut Expression,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_while(&mut self, loc: Loc, _cond: &mut Expression, _body: &mut Statement) -> VResult {
        self.visit_source(loc)
    }

    fn visit_for(
        &mut self,
        loc: Loc,
        _init: &mut Option<Box<Statement>>,
        _cond: &mut Option<Box<Expression>>,
        _update: &mut Option<Box<Statement>>,
        _body: &mut Option<Box<Statement>>,
    ) -> VResult {
        self.visit_source(loc)
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> VResult {
        self.visit_source(func.loc())?;
        if func.body.is_none() {
            self.visit_stray_semicolon()?;
        }

        Ok(())
    }

    fn visit_function_attribute(&mut self, attribute: &mut FunctionAttribute) -> VResult {
        if let Some(loc) = attribute.loc() {
            self.visit_source(loc)?;
        }

        Ok(())
    }

    fn visit_function_attribute_list(&mut self, list: &mut Vec<FunctionAttribute>) -> VResult {
        if let (Some(first), Some(last)) = (list.first(), list.last()) {
            if let (Some(first_loc), Some(last_loc)) = (first.loc(), last.loc()) {
                self.visit_source(Loc::File(
                    first_loc.file_no(),
                    first_loc.start(),
                    last_loc.end(),
                ))?;
            }
        }

        Ok(())
    }

    fn visit_base(&mut self, base: &mut Base) -> VResult {
        self.visit_source(base.loc)
    }

    fn visit_parameter(&mut self, parameter: &mut Parameter) -> VResult {
        self.visit_source(parameter.loc)
    }

    /// Write parameter list used in function/constructor/fallback/modifier arguments, return
    /// arguments and try return arguments including opening and closing parenthesis.
    fn visit_parameter_list(&mut self, list: &mut ParameterList) -> VResult {
        self.visit_opening_paren()?;
        if let (Some((first_loc, _)), Some((last_loc, _))) = (list.first(), list.last()) {
            self.visit_source(Loc::File(first_loc.file_no(), first_loc.start(), last_loc.end()))?;
        }
        self.visit_closing_paren()?;

        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> VResult {
        self.visit_source(structure.loc)?;

        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> VResult {
        self.visit_source(event.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> VResult {
        self.visit_source(param.loc)
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> VResult {
        self.visit_source(error.loc)?;
        self.visit_stray_semicolon()?;

        Ok(())
    }

    fn visit_error_parameter(&mut self, param: &mut ErrorParameter) -> VResult {
        self.visit_source(param.loc)
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> VResult {
        self.visit_source(def.loc)
    }

    fn visit_stray_semicolon(&mut self) -> VResult;

    fn visit_opening_paren(&mut self) -> VResult;

    fn visit_closing_paren(&mut self) -> VResult;

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

impl Visitable for SourceUnitPart {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        match self {
            SourceUnitPart::ContractDefinition(contract) => v.visit_contract(contract),
            SourceUnitPart::PragmaDirective(_, ident, str) => v.visit_pragma(ident, str),
            SourceUnitPart::ImportDirective(import) => import.visit(v),
            SourceUnitPart::EnumDefinition(enumeration) => v.visit_enum(enumeration),
            SourceUnitPart::StructDefinition(structure) => v.visit_struct(structure),
            SourceUnitPart::EventDefinition(event) => v.visit_event(event),
            SourceUnitPart::ErrorDefinition(error) => v.visit_error(error),
            SourceUnitPart::FunctionDefinition(function) => v.visit_function(function),
            SourceUnitPart::VariableDefinition(variable) => v.visit_var_definition(variable),
            SourceUnitPart::TypeDefinition(def) => v.visit_type_definition(def),
            SourceUnitPart::StraySemicolon(_) => v.visit_stray_semicolon(),
            SourceUnitPart::DocComment(doc) => v.visit_doc_comment(doc),
            SourceUnitPart::Using(using) => v.visit_using(using),
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
            ContractPart::VariableDefinition(variable) => v.visit_var_definition(variable),
            ContractPart::FunctionDefinition(function) => v.visit_function(function),
            ContractPart::TypeDefinition(def) => v.visit_type_definition(def),
            ContractPart::StraySemicolon(_) => v.visit_stray_semicolon(),
            ContractPart::Using(using) => v.visit_using(using),
            ContractPart::DocComment(doc) => v.visit_doc_comment(doc),
        }
    }
}

impl Visitable for Statement {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        match self {
            Statement::Block { loc, unchecked, statements } => {
                v.visit_block(*loc, *unchecked, statements)
            }
            Statement::Assembly { loc, dialect, block } => v.visit_assembly(*loc, dialect, block),
            Statement::Args(loc, args) => v.visit_args(*loc, args),
            Statement::If(loc, cond, if_branch, else_branch) => {
                v.visit_if(*loc, cond, if_branch, else_branch)
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
            Statement::DoWhile(loc, cond, body) => v.visit_do_while(*loc, cond, body),
            Statement::Continue(_) => v.visit_continue(),
            Statement::Break(_) => v.visit_break(),
            Statement::Return(loc, expr) => v.visit_return(*loc, expr),
            Statement::Revert(loc, error, args) => v.visit_revert(*loc, error, args),
            Statement::RevertNamedArgs(loc, error, args) => {
                v.visit_revert_named_args(*loc, error, args)
            }
            Statement::Emit(loc, event) => v.visit_emit(*loc, event),
            Statement::Try(loc, expr, returns, clauses) => {
                v.visit_try(*loc, expr, returns, clauses)
            }
            Statement::DocComment(doc) => v.visit_doc_comment(doc),
        }
    }
}

impl Visitable for Loc {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_source(*self)
    }
}

impl Visitable for Expression {
    fn visit(&mut self, v: &mut impl Visitor) -> VResult {
        v.visit_expr(LineOfCode::loc(self), self)
    }
}

macro_rules! impl_visitable {
    ($type:ty, $func:ident) => {
        impl Visitable for $type {
            fn visit(&mut self, v: &mut impl Visitor) -> VResult {
                v.$func(self)
            }
        }
    };
}

impl_visitable!(DocComment, visit_doc_comment);
impl_visitable!(Vec<DocComment>, visit_doc_comments);
impl_visitable!(SourceUnit, visit_source_unit);
impl_visitable!(VariableDeclaration, visit_var_declaration);
impl_visitable!(FunctionAttribute, visit_function_attribute);
impl_visitable!(Vec<FunctionAttribute>, visit_function_attribute_list);
impl_visitable!(Parameter, visit_parameter);
impl_visitable!(ParameterList, visit_parameter_list);
impl_visitable!(Base, visit_base);
impl_visitable!(EventParameter, visit_event_parameter);
impl_visitable!(ErrorParameter, visit_error_parameter);
