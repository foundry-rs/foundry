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
use eyre::{Context, Ok, OptionExt, Result};
use foundry_common::fs;
use proc_macro2::{Ident, Span, TokenStream};
use serde_json::Value;
use std::{
    fmt::Write,
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
            let expansion = fs::read_to_string(path).wrap_err("Failed to read file")?;

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

            let tokens =
                tokens_for_sol(&ident_name, &sol_str).wrap_err("Failed to get sol tokens")?;

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

            let input: SolInput = syn::parse2(tokens).wrap_err("Failed to parse SolInput")?;

            let SolInput { attrs: _attrs, path: _path, kind } = input;

            let tokens = match kind {
                SolInputKind::Sol(file) => expand(file).wrap_err("Failed to expand SolInput")?,
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
alloy-contract = {{ git = "https://github.com/alloy-rs/alloy" }}"#,
            name, version, alloy_version
        );

        fs::write(cargo_toml_path, toml_contents).wrap_err("Failed to write Cargo.toml")?;

        let mut lib_contents = String::new();
        if single_file {
            write!(
                &mut lib_contents,
                r#"#![allow(unused_imports, clippy::all)]
            //! This module contains the sol! generated bindings for solidity contracts.
            //! This is autogenerated code.
            //! Do not manually edit these files.
            //! These files may be overwritten by the codegen system at any time."#
            )?;
        } else {
            write!(
                &mut lib_contents,
                r#"#![allow(unused_imports)]
            "#
            )?;
        };

        // Write src
        for instance in &self.instances {
            let name = instance.name.to_lowercase();
            let contents = instance.expansion.as_ref().unwrap().to_string();

            if !single_file {
                let path = src.join(format!("{}.rs", name));
                let file = syn::parse_file(&contents)?;
                let contents = prettyplease::unparse(&file);

                fs::write(path, contents).wrap_err("Failed to write file")?;
                writeln!(&mut lib_contents, "pub mod {};", name)?;
            } else {
                write!(&mut lib_contents, "{}", contents)?;
            }
        }

        if !single_file {
            write!(
                &mut lib_contents,
                r#"extern crate alloy_sol_types;
            extern crate core;"#
            )?;
        }

        let lib_path = src.join("lib.rs");
        let lib_file = syn::parse_file(&lib_contents)?;

        let lib_contents = prettyplease::unparse(&lib_file);

        fs::write(lib_path, lib_contents).wrap_err("Failed to write lib.rs")?;

        Ok(())
    }

    pub fn write_to_module(&mut self, bindings_path: &Path, single_file: bool) -> Result<()> {
        self.generate_bindings()?;

        let _ = fs::create_dir_all(bindings_path);

        let mut mod_contents = r#"#![allow(clippy::all)]
        //! This module contains the sol! generated bindings for solidity contracts.
        //! This is autogenerated code.
        //! Do not manually edit these files.
        //! These files may be overwritten by the codegen system at any time.
        "#
        .to_string();

        for instance in &self.instances {
            let name = instance.name.to_lowercase();
            if !single_file {
                write!(
                    mod_contents,
                    r#"pub mod {};
                "#,
                    instance.name.to_lowercase()
                )?;
                let mut contents =
                    r#"//! This module was autogenerated by the alloy sol!.
                    //! More information can be found here <https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html>.
                    "#.to_string();

                write!(contents, "{}", instance.expansion.as_ref().unwrap())?;
                let file = syn::parse_file(&contents)?;

                let contents = prettyplease::unparse(&file);
                fs::write(bindings_path.join(format!("{}.rs", name)), contents)
                    .wrap_err("Failed to write file")?;
            } else {
                let mut contents = format!(
                    r#"pub use {}::*;
                    //! This module was autogenerated by the alloy sol!.
                    //! More information can be found here <https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html>.
                    "#,
                    name
                );
                write!(contents, "{}\n\n", instance.expansion.as_ref().unwrap())?;
                write!(mod_contents, "{}", contents)?;
            }
        }

        let mod_path = bindings_path.join("mod.rs");
        let mod_file = syn::parse_file(&mod_contents)?;
        let mod_contents = prettyplease::unparse(&mod_file);

        fs::write(mod_path, mod_contents).wrap_err("Failed to write mod.rs")?;

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

        let mut super_contents = String::new();
        if is_mod {
            // mod.rs
            write!(
                &mut super_contents,
                r#"#![allow(clippy::all)]
                //! This module contains the sol! generated bindings for solidity contracts.
                //! This is autogenerated code.
                //! Do not manually edit these files.
                //! These files may be overwritten by the codegen system at any time.
                "#
            )?;
        } else {
            // lib.rs
            write!(
                &mut super_contents,
                r#"#![allow(unused_imports)]
            "#
            )?;
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
                    write!(
                        &mut super_contents,
                        r#"pub mod {};
                    "#,
                        name
                    )?;
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
        let file_contents = &fs::read_to_string(file_path).wrap_err("Failed to read file")?;
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
        let cargo_toml_contents =
            fs::read_to_string(cargo_toml_path).wrap_err("Failed to read Cargo.toml")?;

        let name_check = &format!("name = \"{}\"", name);
        let version_check = &format!("version = \"{}\"", version);
        let sol_types_check = &format!("alloy-sol-types = \"{}\"", alloy_version);
        let alloy_contract_check =
            "alloy-contract = {{ git = \"https://github.com/alloy-rs/alloy\" }}";
        let toml_consistent = cargo_toml_contents.contains(name_check) &&
            cargo_toml_contents.contains(version_check) &&
            cargo_toml_contents.contains(sol_types_check) &&
            cargo_toml_contents.contains(alloy_contract_check);
        eyre::ensure!(
            toml_consistent,
            r#"The contents of Cargo.toml do not match the expected output of the latest `sol!` version.
                This indicates that the existing bindings are outdated and need to be generated again."#
        );

        Ok(())
    }
}