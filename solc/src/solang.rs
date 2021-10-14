//! [solang](https://github.com/hyperledger-labs/solang) support used to inject statements into solidity files

use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::Path,
};

use solang::parser::{self, pt::*};

/// A trait that is invoked while traversing the  solidity AST.
///
/// Each method of the `Visitor` trait is a hook that can be potentially overriden.
pub trait Visitor {
    fn visit_stmt(&mut self, stmt: Statement, injector: &mut SolInjector) {
        match stmt {
            Statement::Block { statements, .. } => {
                statements.into_iter().for_each(|stmt| self.visit_stmt(stmt, injector))
            }
            Statement::Assembly { assembly, .. } => {
                assembly.into_iter().for_each(|stmt| self.visit_assembly(stmt, injector))
            }
            Statement::Args(_, args) => {
                args.into_iter().for_each(|stmt| self.visit_arg(stmt, injector))
            }
            Statement::If(loc, expr, stmt, other) => {
                self.visit_if(IfStmt { loc, expr, stmt, other }, injector)
            }
            Statement::While(loc, expr, stmt) => self.visit_while(loc, expr, *stmt, injector),
            Statement::Expression(_, expr) => self.visit_expr(expr, injector),
            Statement::VariableDefinition(loc, v, expr) => {
                self.visit_var_def(loc, v, expr, injector)
            }
            Statement::For(a, b, c, d, e) => self.visit_for(a, b, c, d, e, injector),
            Statement::DoWhile(loc, stmt, expr) => self.visit_do_while(loc, *stmt, expr, injector),
            Statement::Continue(loc) => self.visit_continue(loc, injector),
            Statement::Break(loc) => self.visit_break(loc, injector),
            Statement::Return(loc, r) => self.visit_return(loc, r, injector),
            Statement::Emit(loc, expr) => self.visit_emit(loc, expr, injector),
            Statement::Try(a, b, c, d, e) => self.visit_try(a, b, c, d, e, injector),
        }
    }

    fn visit_assembly(&mut self, _stmt: AssemblyStatement, _injector: &mut SolInjector) {}

    fn visit_arg(&mut self, _stmt: NamedArgument, _injector: &mut SolInjector) {}

    fn visit_if(&mut self, _stmt: IfStmt, _injector: &mut SolInjector) {}

    fn visit_expr(&mut self, _expr: Expression, _injector: &mut SolInjector) {}

    fn visit_emit(&mut self, _loc: Loc, _stmt: Expression, _injector: &mut SolInjector) {}

    fn visit_var_def(
        &mut self,
        _loc: Loc,
        _v: VariableDeclaration,
        _expr: Option<Expression>,
        _injector: &mut SolInjector,
    ) {
    }

    fn visit_return(&mut self, _loc: Loc, _expr: Option<Expression>, _injector: &mut SolInjector) {}

    fn visit_break(&mut self, _loc: Loc, _injector: &mut SolInjector) {}

    fn visit_continue(&mut self, _loc: Loc, _injector: &mut SolInjector) {}

    fn visit_do_while(
        &mut self,
        _loc: Loc,
        _stmt: Statement,
        _expr: Expression,
        _injector: &mut SolInjector,
    ) {
    }

    fn visit_while(
        &mut self,
        _loc: Loc,
        _expr: Expression,
        _stmt: Statement,
        _injector: &mut SolInjector,
    ) {
    }

    fn visit_for(
        &mut self,
        _: Loc,
        _: Option<Box<Statement>>,
        _: Option<Box<Expression>>,
        _: Option<Box<Statement>>,
        _: Option<Box<Statement>>,
        _injector: &mut SolInjector,
    ) {
    }

    #[allow(clippy::type_complexity)]
    fn visit_try(
        &mut self,
        _: Loc,
        _: Expression,
        _: Option<(Vec<(Loc, Option<Parameter>)>, Box<Statement>)>,
        _: Option<Box<(Identifier, Parameter, Statement)>>,
        _: Box<(Option<Parameter>, Statement)>,
        _injector: &mut SolInjector,
    ) {
    }

    fn visit_function(&mut self, fun: FunctionDefinition, injector: &mut SolInjector) {
        if let Some(stmt) = fun.body {
            self.visit_stmt(stmt, injector)
        }
    }

    fn visit_contract(&mut self, contract: ContractDefinition, injector: &mut SolInjector) {
        for part in contract.parts {
            if let ContractPart::FunctionDefinition(fun) = part {
                self.visit_function(*fun, injector)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub loc: Loc,
    pub expr: Expression,
    pub stmt: Box<Statement>,
    pub other: Option<Box<Statement>>,
}

/// The container type that keeps track of all injections
#[derive(Debug, Clone)]
pub struct SolInjector {
    /// The content of the solidity source file
    content: String,
    /// injections into the source content
    injections: Vec<Injection>,
}

impl SolInjector {
    pub fn new(content: impl Into<String>) -> Self {
        Self { content: content.into(), injections: vec![] }
    }

    /// Traverses the solidity AST with the given visitor and pipes the modified source code into
    /// the given output
    pub fn inject<V, O>(mut self, visitor: &mut V, mut output: O) -> eyre::Result<()>
    where
        V: Visitor,
        O: Write,
    {
        let sol = parser::parse(&self.content, 1).map_err(|err| eyre::eyre!("{:?}", err))?;

        for unit in sol.0 {
            if let SourceUnitPart::ContractDefinition(contract) = unit {
                visitor.visit_contract(*contract, &mut self);
            }
        }
        let lookup = LineIdentLookup::new(&self.content);
        let mut buf = Cursor::new(self.content);
        // sort injections
        self.injections.sort_by(|a, b| a.loc().1.cmp(&b.loc().1));

        for injection in self.injections {
            match injection.location {
                InjectLocation::Before(loc) => {
                    // write everything before
                    let num = loc.1 as u64 - buf.position();
                    io::copy(&mut buf.by_ref().take(num), &mut output)?;
                    let ident = lookup.line(loc.1).1;
                    output.write_all(b"\n")?;
                    for _ in 0..ident {
                        output.write_all(b" ")?;
                    }
                    output.write_all(injection.content.as_bytes())?;
                    output.write_all(b"\n")?;
                    for _ in 0..ident {
                        output.write_all(b" ")?;
                    }
                    // write the statement
                    let num = loc.2 as u64 - buf.position();
                    io::copy(&mut buf.by_ref().take(num), &mut output)?;
                }
                InjectLocation::After(loc) => {
                    // write the statement
                    let num = loc.2 as u64 - buf.position();
                    io::copy(&mut buf.by_ref().take(num), &mut output)?;
                    let ident = lookup.line(loc.1).1;
                    output.write_all(b"\n")?;
                    for _ in 0..ident {
                        output.write_all(b" ")?;
                    }
                    output.write_all(injection.content.as_bytes())?;
                    output.write_all(b"\n")?;
                    for _ in 0..ident {
                        output.write_all(b" ")?;
                    }
                }
            }
        }
        io::copy(&mut buf, &mut output)?;

        Ok(())
    }

    /// Inject content after the given location
    pub fn after(&mut self, loc: Loc, content: impl Into<String>) {
        self.injections
            .push(Injection { content: content.into(), location: InjectLocation::After(loc) })
    }

    /// Inject content before the given location
    pub fn before(&mut self, loc: Loc, content: impl Into<String>) {
        self.injections
            .push(Injection { content: content.into(), location: InjectLocation::Before(loc) })
    }
}

#[derive(Debug, Clone)]
struct Injection {
    pub content: String,
    pub location: InjectLocation,
}

impl Injection {
    fn loc(&self) -> &Loc {
        match self.location {
            InjectLocation::Before(ref loc) => loc,
            InjectLocation::After(ref loc) => loc,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InjectLocation {
    Before(Loc),
    After(Loc),
}

/// Helper type to keep track of the lines and their ident
#[derive(Debug, Clone)]
struct LineIdentLookup {
    lines: Vec<usize>,
    ws: Vec<usize>,
}

impl LineIdentLookup {
    fn new(input: &str) -> Self {
        let mut iter = input.chars().enumerate();
        let mut lines = vec![0];
        let mut ws = vec![0];

        while let Some((pos, c)) = iter.next() {
            if c == '\n' {
                let mut ident = 0;
                for (_, s) in iter.by_ref().take_while(|(_, s)| *s != '\n') {
                    match s {
                        '\t' => ident += 2,
                        ' ' | '\x0C' => ident += 1,
                        _ => break,
                    }
                }
                lines.push(pos);
                ws.push(ident);
            }
        }
        Self { lines, ws }
    }

    /// Returns the line number of the given index and the number of white space ident of the line
    fn line(&self, idx: usize) -> (u64, u64) {
        let mut line_range = 0..self.lines.len();
        while line_range.end - line_range.start > 1 {
            let range_middle = line_range.start + (line_range.end - line_range.start) / 2;
            let (left, right) = (line_range.start..range_middle, range_middle..line_range.end);
            if (self.lines[left.start]..self.lines[left.end]).contains(&idx) {
                line_range = left;
            } else {
                line_range = right;
            }
        }
        let line = line_range.start + 1;
        (line as u64, self.ws[line_range.start] as u64)
    }
}

/// Returns the injected solidity file content
pub fn injected_sol<V: Visitor>(path: impl AsRef<Path>, visitor: &mut V) -> eyre::Result<Vec<u8>> {
    let content = fs::read_to_string(path.as_ref())?;
    let mut output = Vec::with_capacity(content.len());
    SolInjector::new(content).inject(visitor, &mut output)?;
    Ok(output)
}

/// Writes the injected solidity file to the given file
pub fn write_injected_sol<V: Visitor>(
    path: impl AsRef<Path>,
    visitor: &mut V,
    output: impl AsRef<Path>,
) -> eyre::Result<Vec<u8>> {
    let injected = injected_sol(path, visitor)?;
    fs::write(output.as_ref(), &injected)?;
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Visitor for () {
        fn visit_expr(&mut self, expr: Expression, injector: &mut SolInjector) {
            if let Expression::Assign(_, var, val) = expr {
                injector.before(var.loc(), "// The Statement begins:");
                injector.after(val.loc(), "// The Statement ended");
            }
        }
    }

    #[test]
    fn can_inject() {
        let contract = r#"// SPDX-License-Identifier: UNLICENSED
pragma abicoder v2;
pragma solidity =0.7.6;

contract Greeter {
    string public greeting;

    function greet(string memory _greeting) public {
        greeting = _greeting;
    }

    function gm() public {
        greeting = "gm";
    }
}"#;

        let mut output = Vec::with_capacity(contract.len());
        SolInjector::new(contract).inject(&mut (), &mut output).unwrap();
        println!("{}", String::from_utf8_lossy(&output));
    }

    #[test]
    fn line_look_up_works() {
        let text = "x\nxy\n    xyz";
        let lookup = LineIdentLookup::new(text);
        dbg!(lookup.clone());
        assert_eq!(lookup.line(0), (1, 0));
        assert_eq!(lookup.line(5), (3, 4));
    }
}
