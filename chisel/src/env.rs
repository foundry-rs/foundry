use ethers_solc::project_util::TempProject;
use rustyline::Editor;

pub struct ChiselEnv {
    /// The `TempProject` created for the REPL contract.
    pub project: TempProject,
    /// The `rustyline` Editor
    pub rl: Editor<()>,
    /// The current session
    /// TODO: A vector of strings is insufficient- will need to separate functions from
    /// expressions, etc.
    pub session: Vec<String>,
}

/// A Chisel REPL environment
impl ChiselEnv {
    /// Create a new `ChiselEnv` with a specified `solc` version.
    pub fn new(solc_version: &'static str) -> Self {
        // TODO: Error handling.

        // Create initialized temporary dapptools-style project
        let mut project = TempProject::dapptools_init().unwrap();
        // Set project's solc version explicitly
        project.set_solc(solc_version);

        // Return initialized ChiselEnv with set solc version
        Self { project, rl: Editor::<()>::new().unwrap(), session: Vec::default() }
    }

    /// Create a default `ChiselEnv`.
    pub fn default() -> Self {
        // TODO: Error handling

        Self {
            project: TempProject::dapptools_init().unwrap(),
            rl: Editor::<()>::new().unwrap(),
            session: Vec::default(),
        }
    }

    /// Render the full source code for the current session.
    /// TODO
    pub fn contract_source(&self) -> String {
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {};
contract REPL {{
    {}
}}
        "#,
            "^0.8.17", // TODO: Grab version from TempProject's solc instance.
            self.session.join("\n")
        )
    }
}
