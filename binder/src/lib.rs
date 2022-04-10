//! Generate [ethers-rs]("https://github.com/gakonst/ethers-rs") bindings for solidity projects in a build script.


use std::path::PathBuf;

/// Contains all the options to configure the gen process
#[derive(Debug, Clone)]
pub struct Binder {
    location: SourceLocation,

    /// Whether to include the bytecode in the bindings to be able to deploy them
    deployable: bool,
    /// Contains the directory where the artifacts should be written, if `None`, the artifacts will be cleaned up
    keep_artifacts: Option<PathBuf>
}

// == impl Binder ==

impl Binder {

    /// Generates the bindings
    pub fn generate(&self) {
        todo!()
    }
}


pub struct Builder {
    location: SourceLocation,
}

/// Where to find the source project
#[derive(Debug, Clone)]
pub enum SourceLocation {
    Local(PathBuf),
    Remote(Repository)
}

#[derive(Debug, Clone)]
pub struct Repository {
    /// github project url like <https://github.com/aave/aave-v3-core/>
    pub url: String,
    /// The version tag, branch or rev to checkout
    pub checkout: Option<Checkout>
}

#[derive(Debug, Clone)]
pub enum Checkout {
    Tag(String),
    Branch(String),
    Commit(String)
}