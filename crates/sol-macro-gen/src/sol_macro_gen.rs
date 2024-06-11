//! SolMacroGen and MultiSolMacroGen
//!
//! This type encapsulates the logic for expansion of a Rust TokenStream from Solidity tokens. It
//! uses the `expand` method from `alloy_sol_macro_expander` underneath.
//!
//! It holds info such as `path` to the ABI file, `name` of the file and the rust binding being
//! generated, and lastly the `expansion` itself, i.e the Rust binding for the provided ABI.
//!
//! It contains methods to read the json abi, generate rust bindings from the abi and ultimately
//! write the bindings to a crate or modules.

use alloy_json_abi::JsonAbi;
use alloy_sol_macro_expander::expand::expand;
use alloy_sol_macro_input::{tokens_for_sol, SolInput, SolInputKind};
use eyre::{Ok, OptionExt, Result};
use foundry_common::fs;
use proc_macro2::{Ident, Span, TokenStream};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

pub struct SolMacroGen {
    pub path: PathBuf,
    pub name: String,
    pub expansion: Option<TokenStream>,
}

impl SolMacroGen {
    pub fn new(path: PathBuf, name: String) -> Self {
        Self { path, name, expansion: None }
    }

    pub fn get_json_abi(&self) -> Result<(JsonAbi, Option<String>)> {
        let json = std::fs::read(&self.path)?;

        // Need to do this to get the abi in the next step.
        let json: Value = serde_json::from_slice(&json)?;

        let abi_val = json.get("abi").ok_or_eyre("No ABI found in JSON file")?;
        let json_abi = serde_json::from_str(&abi_val.clone().to_string())?;

        let bytecode = json.get("bytecode").and_then(|b| b.get("object")).map(|o| o.to_string());

        Ok((json_abi, bytecode))
    }
}

pub struct MultiSolMacroGen {
    pub artifacts_path: PathBuf,
    pub instances: Vec<SolMacroGen>,
}

impl MultiSolMacroGen {
    pub fn new(artifacts_path: &Path, instances: Vec<SolMacroGen>) -> Self {
        Self { artifacts_path: artifacts_path.to_path_buf(), instances }
    }

    pub fn populate_expansion(&mut self, bindings_path: &Path) -> Result<()> {
        for instance in &mut self.instances {
            let path = bindings_path.join(format!("{}.rs", instance.name.to_lowercase()));
            let expansion =
                fs::read_to_string(path).map_err(|e| eyre::eyre!("Failed to read file: {e}"))?;

            let tokens = TokenStream::from_str(&expansion)
                .map_err(|e| eyre::eyre!("Failed to parse TokenStream: {e}"))?;
            instance.expansion = Some(tokens);
        }
        Ok(())
    }

    pub fn generate_bindings(&mut self) -> Result<()> {
        for instance in &mut self.instances {
            let (mut json_abi, maybe_bytecode) = instance.get_json_abi()?;

            json_abi.dedup();
            let sol_str = json_abi.to_sol(&instance.name, None);

            let ident_name: Ident = Ident::new(&instance.name, Span::call_site());

            let tokens = tokens_for_sol(&ident_name, &sol_str)
                .map_err(|e| eyre::eyre!("Failed to get sol tokens: {e}"))?;

            let tokens = if let Some(bytecode) = maybe_bytecode {
                let bytecode = proc_macro2::TokenStream::from_str(&bytecode).map_err(|e| {
                    eyre::eyre!("Failed to convert bytecode String to TokenStream {e}")
                })?;
                quote::quote! {
                    #[derive(Debug)]
                    #[sol(rpc, bytecode = #bytecode)]
                    #tokens
                }
            } else {
                quote::quote! {
                    #[derive(Debug)]
                    #[sol(rpc)]
                    #tokens
                }
            };

            let input: SolInput =
                syn::parse2(tokens).map_err(|e| eyre::eyre!("Failed to parse SolInput: {e}"))?;

            let SolInput { attrs: _attrs, path: _path, kind } = input;

            let tokens = match kind {
                SolInputKind::Sol(file) => {
                    expand(file).map_err(|e| eyre::eyre!("Failed to expand SolInput: {e}"))?
                }
                _ => unreachable!(),
            };

            instance.expansion = Some(tokens);
        }

        Ok(())
    }

    pub fn write_to_crate(
        &mut self,
        name: &str,
        version: &str,
        bindings_path: &Path,
        single_file: bool,
        alloy_version: String,
    ) -> Result<()> {
        self.generate_bindings()?;

        let src = bindings_path.join("src");

        let _ = fs::create_dir_all(&src);

        // Write Cargo.toml
        let cargo_toml_path = bindings_path.join("Cargo.toml");
        let toml_contents = format!(
            r#"[package]
name = "{}"
version = "{}"
edition = "2021"

[dependencies]
alloy-sol-types = "{}"
alloy-contract = {{ git = "https://github.com/alloy-rs/alloy", rev = "64feb9b" }}"#,
            name, version, alloy_version
        );

        fs::write(cargo_toml_path, toml_contents)
            .map_err(|e| eyre::eyre!("Failed to write Cargo.toml: {e}"))?;

        // Write src
        let mut lib_contents = if single_file {
            String::from(
                r#"#![allow(unused_imports, clippy::all)]
                //! This module contains the sol! generated bindings for solidity contracts.
                //! This is autogenerated code.
                //! Do not manually edit these files.
                //! These files may be overwritten by the codegen system at any time."#,
            )
        } else {
            String::from(
                r#"#![allow(unused_imports)]
            "#,
            )
        };

        for instance in &self.instances {
            let name = instance.name.to_lowercase();
            let contents = instance.expansion.as_ref().unwrap().to_string();

            if !single_file {
                let path = src.join(format!("{}.rs", name));
                let file = syn::parse_file(&contents)?;
                let contents = prettyplease::unparse(&file);

                fs::write(path, contents).map_err(|e| eyre::eyre!("Failed to write file: {e}"))?;
                lib_contents += &format!(
                    r#"pub mod {};
                    "#,
                    name
                );
            } else {
                lib_contents += &contents;
            }
        }

        if !single_file {
            lib_contents += r#"extern crate alloy_sol_types;
            extern crate core;"#;
        }

        let lib_path = src.join("lib.rs");
        let lib_file = syn::parse_file(&lib_contents)?;

        let lib_contents = prettyplease::unparse(&lib_file);

        fs::write(lib_path, lib_contents)
            .map_err(|e| eyre::eyre!("Failed to write lib.rs: {e}"))?;

        Ok(())
    }

    pub fn write_to_module(&mut self, bindings_path: &Path, single_file: bool) -> Result<()> {
        self.generate_bindings()?;

        let _ = fs::create_dir_all(bindings_path);

        let mut mod_contents = String::from(
            r#"#![allow(clippy::all)]
        //! This module contains the sol! generated bindings for solidity contracts.
        //! This is autogenerated code.
        //! Do not manually edit these files.
        //! These files may be overwritten by the codegen system at any time.
        "#,
        );
        for instance in &self.instances {
            let name = instance.name.to_lowercase();
            if !single_file {
                mod_contents += &format!(
                    r#"pub mod {};
                "#,
                    instance.name.to_lowercase()
                );
                let contents = format!(
                    r#"//! This module was autogenerated by the alloy sol!.
                    //! More information can be found here <https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html>.
                    "#
                ) + &instance.expansion.as_ref().unwrap().to_string();

                let file = syn::parse_file(&contents)?;

                let contents = prettyplease::unparse(&file);
                fs::write(bindings_path.join(format!("{}.rs", name)), contents)
                    .map_err(|e| eyre::eyre!("Failed to write file: {e}"))?;
            } else {
                let contents = format!(
                    r#"pub use {}::*;
                    //! This module was autogenerated by the alloy sol!.
                    //! More information can be found here <https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html>.
                    "#,
                    name
                ) + &instance.expansion.as_ref().unwrap().to_string() +
                    "\n\n";
                mod_contents += &contents;
            }
        }

        let mod_path = bindings_path.join("mod.rs");
        let mod_file = syn::parse_file(&mod_contents)?;
        let mod_contents = prettyplease::unparse(&mod_file);

        fs::write(mod_path, mod_contents)
            .map_err(|e| eyre::eyre!("Failed to write mod.rs: {e}"))?;

        Ok(())
    }

    /// Checks that the generated bindings are up to date with the latest version of
    /// `sol!`.
    ///
    /// Returns `Ok(())` if the generated bindings are up to date, otherwise it returns
    /// `Err(_)`.
    #[allow(clippy::too_many_arguments)]
    pub fn check_consistency(
        &self,
        name: &str,
        version: &str,
        crate_path: &Path,
        single_file: bool,
        check_cargo_toml: bool,
        is_mod: bool,
        alloy_version: String,
    ) -> Result<()> {
        if check_cargo_toml {
            self.check_cargo_toml(name, version, crate_path, alloy_version)?;
        }

        let mut super_contents = if is_mod {
            // mod.rs
            String::from(
                r#"#![allow(clippy::all)]
            //! This module contains the sol! generated bindings for solidity contracts.
            //! This is autogenerated code.
            //! Do not manually edit these files.
            //! These files may be overwritten by the codegen system at any time.
            "#,
            )
        } else {
            // lib.rs
            String::from("#![allow(unused_imports)]\n")
        };
        if !single_file {
            for instance in &self.instances {
                let name = instance.name.to_lowercase();
                let path = crate_path.join(format!("src/{}.rs", name));
                let tokens = instance
                    .expansion
                    .as_ref()
                    .ok_or_eyre(format!("TokenStream for {path:?} does not exist"))?
                    .to_string();

                self.check_file_contents(&path, &tokens)?;

                if !is_mod {
                    super_contents += &format!(
                        r#"pub mod {};
                    "#,
                        name
                    );
                }
            }

            let super_path =
                if is_mod { crate_path.join("src/mod.rs") } else { crate_path.join("src/lib.rs") };
            self.check_file_contents(&super_path, &super_contents)?;
        }

        Ok(())
    }

    fn check_file_contents(&self, file_path: &Path, expected_contents: &str) -> Result<()> {
        eyre::ensure!(
            file_path.is_file() && file_path.exists(),
            "{} is not a file",
            file_path.display()
        );
        let file_contents =
            &fs::read_to_string(file_path).map_err(|e| eyre::eyre!("Failed to read file: {e}"))?;
        eyre::ensure!(
            file_contents == expected_contents,
            "File contents do not match expected contents for {file_path:?}"
        );
        Ok(())
    }

    fn check_cargo_toml(
        &self,
        name: &str,
        version: &str,
        crate_path: &Path,
        alloy_version: String,
    ) -> Result<()> {
        eyre::ensure!(crate_path.is_dir(), "Crate path must be a directory");

        let cargo_toml_path = crate_path.join("Cargo.toml");

        eyre::ensure!(cargo_toml_path.is_file(), "Cargo.toml must exist");
        let cargo_toml_contents = fs::read_to_string(cargo_toml_path)
            .map_err(|e| eyre::eyre!("Failed to read Cargo.toml: {e}"))?;

        let name_check = &format!("name = \"{}\"", name);
        let version_check = &format!("version = \"{}\"", version);
        let sol_types_check = &format!("alloy-sol-types = \"{}\"", alloy_version);
        let alloy_contract_check =
            "alloy-contract = {{ git = \"https://github.com/alloy-rs/alloy\", rev = \"64feb9b\" }}";
        let toml_consistent = cargo_toml_contents.contains(name_check) &&
            cargo_toml_contents.contains(version_check) &&
            cargo_toml_contents.contains(sol_types_check) &&
            cargo_toml_contents.contains(alloy_contract_check);
        eyre::ensure!(
                toml_consistent,
                format!("The contents of Cargo.toml do not match the expected output of the newest `ethers::Abigen` version.\n
This indicates that the existing bindings are outdated and need to be generated again.")
            );

        Ok(())
    }
}
