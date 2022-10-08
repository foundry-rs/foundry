use core::fmt;
use ethers_solc::project_util::TempProject;
use foundry_evm::executor::{backend::Backend, Executor, ExecutorBuilder};
use rustyline::Editor;
use std::rc::Rc;

/// Represents a parsed snippet of Solidity code.
#[derive(Debug)]
pub struct SolSnippet {
    pub source_unit: (solang_parser::pt::SourceUnit, Vec<solang_parser::pt::Comment>),
    pub raw: Rc<String>,
}

/// Display impl for `SolToken`
impl fmt::Display for SolSnippet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// A Chisel REPL environment.
pub struct ChiselEnv {
    /// The `TempProject` created for the REPL contract.
    pub project: TempProject,
    /// The `rustyline` Editor
    pub rl: Editor<()>,
    /// The current session
    /// A session contains an ordered vector of source units, parsed by the solang-parser,
    /// as well as the raw source.
    pub session: Vec<SolSnippet>,
    /// The executor used to run the REPL contract's code.
    pub executor: Executor,
}

/// Chisel REPL environment impl
impl ChiselEnv {
    /// Create a new `ChiselEnv` with a specified `solc` version.
    pub fn new(solc_version: &'static str) -> Self {
        // Create initialized temporary dapptools-style project
        let mut project = Self::create_temp_project();

        // Set project's solc version explicitly
        project.set_solc(solc_version);

        // Create a new rustyline Editor
        let rl = Self::create_rustyline_editor();

        // TODO: Configurable network forking, bonus points
        // if it can be done on-the-fly with a builtin.
        let db = Backend::spawn(None);
        let executor = ExecutorBuilder::default().build(db);

        // Return initialized ChiselEnv with set solc version
        Self { project, rl, session: Vec::default(), executor }
    }

    /// Create a default `ChiselEnv`.
    pub fn default() -> Self {
        // Create an `Executor` with an in-memory DB.
        let db = Backend::spawn(None);
        let executor = ExecutorBuilder::default().build(db);

        Self {
            project: Self::create_temp_project(),
            rl: Self::create_rustyline_editor(),
            session: Vec::default(),
            executor,
        }
    }

    /// Runs the REPL contract within the executor
    /// TODO
    pub fn run_repl(&self) -> Result<(), &str> {
        // Recompile the project and ensure no errors occurred.
        // TODO: This is pretty slow. Def a better way to do this.
        if let Ok(artifacts) = self.project.compile() {
            if artifacts.has_compiler_errors() {
                return Err("Failed to compile REPL contract.")
            }

            // let runner = ContractRunner::new(
            //     self.executor,
            //     contract: /* ... */,
            //
            // );
            Ok(())
        } else {
            Err("Failed to compile REPL contract.")
        }
    }

    /// Render the full source code for the current session.
    /// TODO - Render source correctly rather than throwing
    /// everything into `setUp`.
    pub fn contract_source(&self) -> String {
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {};
// TODO: Inherit `forge-std/Test.sol`
contract REPL {{
    function setUp() public {{
        {}
    }}
}}
        "#,
            "^0.8.17", // TODO: Grab version from TempProject's solc instance.
            self.session.iter().map(|t| t.to_string()).collect::<Vec<String>>().join("\n")
        )
    }

    /// Helper function to create a new temporary project with proper error handling.
    ///
    /// ### Panics
    ///
    /// Panics if the temporary project cannot be created.
    pub(crate) fn create_temp_project() -> TempProject {
        TempProject::dapptools_init().unwrap_or_else(|e| {
            tracing::error!(target: "chisel-env", "Failed to initialize temporary project! {}", e);
            panic!("failed to create a temporary project for the chisel environment! {e}");
        })
    }

    /// Helper function to create a new rustyline Editor with proper error handling.
    ///
    /// ### Panics
    ///
    /// Panics if the rustyline Editor cannot be created.
    pub(crate) fn create_rustyline_editor() -> Editor<()> {
        Editor::<()>::new().unwrap_or_else(|e| {
            tracing::error!(target: "chisel-env", "Failed to initialize rustyline Editor! {}", e);
            panic!("failed to create a rustyline Editor for the chisel environment! {e}");
        })
    }
}
