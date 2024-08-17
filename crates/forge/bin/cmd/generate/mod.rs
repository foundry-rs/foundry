use clap::{Parser, Subcommand};
use eyre::Result;
use foundry_common::fs;
use std::path::Path;
use yansi::Paint;

/// CLI arguments for `forge generate`.
#[derive(Debug, Parser)]
pub struct GenerateArgs {
    #[command(subcommand)]
    pub sub: GenerateSubcommands,
}

#[derive(Debug, Subcommand)]
pub enum GenerateSubcommands {
    /// Scaffolds test file for given contract.
    Test(GenerateTestArgs),
}

#[derive(Debug, Parser)]
pub struct GenerateTestArgs {
    /// Contract name for test generation.
    #[arg(long, short, value_name = "CONTRACT_NAME")]
    pub contract_name: String,
}

impl GenerateTestArgs {
    pub fn run(self) -> Result<()> {
        let contract_name = format_identifier(&self.contract_name, true);
        let instance_name = format_identifier(&self.contract_name, false);

        // Create the test file content.
        let test_content = include_str!("../../../assets/generated/TestTemplate.t.sol");
        let test_content = test_content
            .replace("{contract_name}", &contract_name)
            .replace("{instance_name}", &instance_name);

        // Create the test directory if it doesn't exist.
        fs::create_dir_all("test")?;

        // Define the test file path
        let test_file_path = Path::new("test").join(format!("{contract_name}.t.sol"));

        // Write the test content to the test file.
        fs::write(&test_file_path, test_content)?;

        println!("{} test file: {}", "Generated".green(), test_file_path.to_str().unwrap());
        Ok(())
    }
}

/// Utility function to convert an identifier to pascal or camel case.
fn format_identifier(input: &str, is_pascal_case: bool) -> String {
    let mut result = String::new();
    let mut capitalize_next = is_pascal_case;

    for word in input.split_whitespace() {
        if !word.is_empty() {
            let (first, rest) = word.split_at(1);
            let formatted_word = if capitalize_next {
                format!("{}{}", first.to_uppercase(), rest)
            } else {
                format!("{}{}", first.to_lowercase(), rest)
            };
            capitalize_next = true;
            result.push_str(&formatted_word);
        }
    }
    result
}
