use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd},
};
use clap::Parser;
use ethers::{
    prelude::{
        info::ContractInfo
    },
    solc::{
        utils::canonicalize
    }
};
use foundry_common::compile;
use tracing::trace;

#[derive(Debug, Clone, Parser)]
pub struct SigCollisionArgs {
    #[clap(
        help = "The first of the two contracts for which to look method signature collisions in the form `(<path>:)?<contractname>`.",
        value_name = "FIRST_CONTRACT"
    )]
    pub first_contract: ContractInfo,
    
    #[clap(
        help = "The second of the two contracts for which to look method signature collisions in the form `(<path>:)?<contractname>`.",
        value_name = "SECOND_CONTRACT"
    )]
    pub second_contract: ContractInfo,

    /// Support build arguments
    #[clap(flatten)]
    build: CoreBuildArgs,
}

impl Cmd for SigCollisionArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let SigCollisionArgs {mut first_contract, mut second_contract, build} = self;
        
        trace!(target: "forge", ?first_contract, ?second_contract, "running forge sig-collision");

        println!("{} {}", first_contract.path.as_ref().unwrap(), second_contract.path.as_ref().unwrap());

        // Build first project
        let first_project = build.project()?;
        let first_outcome = if let Some(ref mut contract_path) = first_contract.path {
            let target_path = canonicalize(&*contract_path)?;
            *contract_path = target_path.to_string_lossy().to_string();
            compile::compile_files(&first_project, vec![target_path], true)
        } else {
            compile::suppress_compile(&first_project)
        }?;
        
        // Build second project
        let second_project = build.project()?;
        let second_outcome = if let Some(ref mut contract_path) = second_contract.path {
            let target_path = canonicalize(&*contract_path)?;
            *contract_path = target_path.to_string_lossy().to_string();
            compile::compile_files(&second_project, vec![target_path], true)
        } else {
            compile::suppress_compile(&second_project)
        }?;

        // Find the artifacts
        let first_found_artifact = first_outcome.find_contract(&first_contract);
        let second_found_artifact = second_outcome.find_contract(&second_contract);

        trace!(target: "forge", artifact=?first_found_artifact, input=?first_contract, "Found artifact");
        trace!(target: "forge", artifact=?second_found_artifact, input=?second_contract, "Found artifact");

        // Unwrapping inner artifacts
        let first_artifact = first_found_artifact.ok_or_else( || {
            eyre::eyre!("Failed to extract first artifact bytecode as a string")
        })?;
        let second_artifact = second_found_artifact.ok_or_else( || {
            eyre::eyre!("Failed to extract second artifact bytecode as a string")
        })?;

        let first_method_map = first_artifact.method_identifiers.as_ref().unwrap();
        let second_method_map = second_artifact.method_identifiers.as_ref().unwrap();

        let mut clashing_methods = Vec::new();
        for (k1, v1) in first_method_map {
            if let Some(k2) = second_method_map.iter().find_map(| (k2, v2) | if v1 == v2 {Some(k2)} else {None} ) {
                clashing_methods.push((k1.clone(), k2.clone()))
            };
        }

        if clashing_methods.is_empty() {
            println!("No clashing methods between the two contracts.");
        } else {
            println!("The two contracts have the following methods whose signatures clash: {:#?}", 
                clashing_methods
            );
        }

        Ok(())
    }
}