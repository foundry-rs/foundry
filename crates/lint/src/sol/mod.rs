use std::path::PathBuf;

use eyre::Error;
use foundry_compilers::solc::SolcLanguage;
use solar_interface::{
    diagnostics::{DiagnosticBuilder, ErrorGuaranteed},
    ColorChoice, Session,
};
use thiserror::Error;

use crate::{Lint, Linter, LinterOutput, SourceLocation};

pub mod gas;
pub mod high;
pub mod info;
pub mod med;

#[derive(Debug, Hash)]
pub enum SolLint {}

impl Lint for SolLint {
    fn results(&self) -> Vec<SourceLocation> {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct SolidityLinter {}

#[derive(Error, Debug)]
pub enum SolLintError {}

impl Linter for SolidityLinter {
    // TODO: update this to be a solar error
    type LinterError = SolLintError;
    type Lint = SolLint;
    type Language = SolcLanguage;

    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError> {
        // let all_findings = input
        //             .par_iter()
        //             .map(|file| {
        //                 let lints = self.lints.clone();
        //                 let mut local_findings = HashMap::new();

        //                 // Create a new session for this file
        //                 let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();
        //                 let arena = ast::Arena::new();

        //                 // Enter the session context for this thread
        //                 let _ = sess.enter(|| -> solar_interface::Result<()> {
        //                     let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;

        //                     let ast =
        //                         parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

        //                     // Run all lints on the parsed AST and collect findings
        //                     for mut lint in lints {
        //                         let results = lint.lint(&ast);
        //                         local_findings.entry(lint).or_insert_with(Vec::new).extend(results);
        //                     }

        //                     Ok(())
        //                 });

        //                 local_findings
        //             })
        //             .collect::<Vec<HashMap<Lint, Vec<Span>>>>();

        todo!()
    }
}
