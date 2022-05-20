use crate::{
    cmd::forge::build::{CoreBuildArgs, ProjectPathsArgs},
    compile,
    opts::forge::CompilerArgs,
};
use clap::Parser;
use ethers::prelude::artifacts::{output_selection::ContractOutputSelection, LosslessAbi};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::trace;

static SELECTOR_DATABASE_URL: &str = "https://sig.eth.samczsun.com/api/v1/import";

#[derive(Debug, Clone, Parser)]
pub struct UploadSelectorsArgs {
    #[clap(help = "The name of the contract to upload selectors for.")]
    pub contract: String,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    pub project_paths: ProjectPathsArgs,
}

impl UploadSelectorsArgs {
    /// Builds a contract and uploads the ABI to selector database
    pub async fn run(self) -> eyre::Result<()> {
        let UploadSelectorsArgs { contract, project_paths } = self;

        let build_args = CoreBuildArgs {
            project_paths: project_paths.clone(),
            compiler: CompilerArgs {
                extra_output: vec![ContractOutputSelection::Abi],
                ..Default::default()
            },
            ..Default::default()
        };

        trace!("Building project");
        let project = build_args.project()?;
        let outcome = compile::suppress_compile(&project)?;
        let found_artifact = outcome.find(&contract);
        let artifact = found_artifact.ok_or_else(|| {
            eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
        })?;

        let body = ImportRequest {
            import_type: "abi".to_string(),
            data: vec![artifact.abi.clone().ok_or(eyre::eyre!("Unable to fetch abi"))?],
        };

        // upload abi to selector database
        trace!("Uploading selector args {:?}", body);
        let res: ImportResponse = reqwest::Client::new()
            .post(SELECTOR_DATABASE_URL)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        trace!("Got response: {:?}", res);
        res.describe_upload();

        Ok(())
    }
}

#[derive(Serialize, Debug)]
struct ImportRequest {
    #[serde(rename = "type")]
    import_type: String,
    data: Vec<LosslessAbi>,
}

#[derive(Deserialize, Debug)]
struct ImportTypeData {
    imported: HashMap<String, String>,
    duplicated: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
struct ImportData {
    function: ImportTypeData,
    event: ImportTypeData,
}

#[derive(Deserialize, Debug)]
struct ImportResponse {
    result: ImportData,
}

impl ImportResponse {
    /// Print info about the functions which were uploaded or already known
    pub fn describe_upload(&self) {
        self.result
            .function
            .imported
            .iter()
            .for_each(|(k, v)| println!("Imported: Function {k}: {v}"));
        self.result.event.imported.iter().for_each(|(k, v)| println!("Imported: Event {k}: {v}"));
        self.result
            .function
            .duplicated
            .iter()
            .for_each(|(k, v)| println!("Duplicated: Function {k}: {v}"));
        self.result
            .event
            .duplicated
            .iter()
            .for_each(|(k, v)| println!("Duplicated: Event {k}: {v}"));

        println!("Selectors successfully uploaded to https://sig.eth.samczsun.com");
    }
}
