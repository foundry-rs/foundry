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

use alloy_sol_macro_expander::expand::expand;
use alloy_sol_macro_input::{SolInput, SolInputKind};
use eyre::{Context, OptionExt, Result};
use foundry_common::fs;
use proc_macro2::{Span, TokenStream};
use std::{
    env::temp_dir,
    fmt::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use heck::ToSnakeCase;

pub struct SolMacroGen {
    pub path: PathBuf,
    pub name: String,
    pub expansion: Option<TokenStream>,
}

impl SolMacroGen {
    pub fn new(path: PathBuf, name: String) -> Self {
        Self { path, name, expansion: None }
    }

    pub fn get_sol_input(&self) -> Result<SolInput> {
        let path = self.path.to_string_lossy().into_owned();
        let name = proc_macro2::Ident::new(&self.name, Span::call_site());
        let tokens = quote::quote! {
            #name,
            #path
        };

        let sol_input: SolInput = syn::parse2(tokens).wrap_err("failed to parse input")?;

        Ok(sol_input)
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

    pub fn generate_bindings(&mut self, all_derives: bool) -> Result<()> {
        for instance in &mut self.instances {
            Self::generate_binding(instance, all_derives).wrap_err_with(|| {
                format!(
                    "failed to generate bindings for {}:{}",
                    instance.path.display(),
                    instance.name
                )
            })?;
        }

        Ok(())
    }

    fn generate_binding(instance: &mut SolMacroGen, all_derives: bool) -> Result<()> {
        // TODO: in `get_sol_input` we currently can't handle unlinked bytecode: <https://github.com/alloy-rs/core/issues/926>
        let input = match instance.get_sol_input() {
            Ok(input) => input.normalize_json()?,
            Err(error) => {
                // TODO(mattsse): remove after <https://github.com/alloy-rs/core/issues/926>
                if error.to_string().contains("expected bytecode, found unlinked bytecode") {
                    // we attempt to do a little hack here until we have this properly supported by
                    // removing the bytecode objects from the json file and using a tmpfile (very
                    // hacky)
                    let content = std::fs::read_to_string(&instance.path)?;
                    let mut value = serde_json::from_str::<serde_json::Value>(&content)?;
                    let obj = value.as_object_mut().expect("valid abi");

                    // clear unlinked bytecode
                    obj.remove("bytecode");
                    obj.remove("deployedBytecode");

                    let tmpdir = temp_dir();
                    let mut tmp_file = tmpdir.join(instance.path.file_name().unwrap());
                    std::fs::write(&tmp_file, serde_json::to_string(&value)?)?;

                    // try again
                    std::mem::swap(&mut tmp_file, &mut instance.path);
                    let input = instance.get_sol_input()?.normalize_json()?;
                    std::mem::swap(&mut tmp_file, &mut instance.path);
                    input.normalize_json()?
                } else {
                    return Err(error);
                }
            }
        };

        let SolInput { attrs: _, path: _, kind } = input;

        let tokens = match kind {
            SolInputKind::Sol(mut file) => {
                let sol_attr: syn::Attribute = if all_derives {
                    syn::parse_quote! {
                            #[sol(rpc, alloy_sol_types = alloy::sol_types, alloy_contract =
                    alloy::contract, all_derives = true, extra_derives(serde::Serialize,
                    serde::Deserialize))]     }
                } else {
                    syn::parse_quote! {
                            #[sol(rpc, alloy_sol_types = alloy::sol_types, alloy_contract =
                    alloy::contract)]     }
                };
                file.attrs.push(sol_attr);
                expand(file).wrap_err("failed to expand")?
            }
            _ => unreachable!(),
        };

        instance.expansion = Some(tokens);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_to_crate(
        &mut self,
        name: &str,
        version: &str,
        description: &str,
        license: &str,
        bindings_path: &Path,
        single_file: bool,
        alloy_version: Option<String>,
        alloy_rev: Option<String>,
        all_derives: bool,
    ) -> Result<()> {
        self.generate_bindings(all_derives)?;

        let src = bindings_path.join("src");
        let _ = fs::create_dir_all(&src);

        // Write Cargo.toml
        let cargo_toml_path = bindings_path.join("Cargo.toml");
        let mut toml_contents = format!(
            r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"
"#
        );

        if !description.is_empty() {
            toml_contents.push_str(&format!("description = \"{description}\"\n"));
        }

        if !license.is_empty() {
            let formatted_licenses: Vec<String> =
                license.split(',').map(Self::parse_license_alias).collect();

            let formatted_license = formatted_licenses.join(" OR ");
            toml_contents.push_str(&format!("license = \"{formatted_license}\"\n"));
        }

        toml_contents.push_str("\n[dependencies]\n");

        let alloy_dep = Self::get_alloy_dep(alloy_version, alloy_rev);
        write!(toml_contents, "{alloy_dep}")?;

        if all_derives {
            let serde_dep = r#"serde = { version = "1.0", features = ["derive"] }"#;
            write!(toml_contents, "\n{serde_dep}")?;
        }

        fs::write(cargo_toml_path, toml_contents).wrap_err("Failed to write Cargo.toml")?;

        let mut lib_contents = String::new();
        write!(
            &mut lib_contents,
            r#"#![allow(unused_imports, clippy::all, rustdoc::all)]
        //! This module contains the sol! generated bindings for solidity contracts.
        //! This is autogenerated code.
        //! Do not manually edit these files.
        //! These files may be overwritten by the codegen system at any time.
        "#
        )?;

        // Write src
        let parse_error = |name: &str| {
            format!("failed to parse generated tokens as an AST for {name};\nthis is likely a bug")
        };
        for instance in &self.instances {
            let contents = instance.expansion.as_ref().unwrap();

            let name = instance.name.to_snake_case();
            let path = src.join(format!("{name}.rs"));
            let file = syn::parse2(contents.clone())
                .wrap_err_with(|| parse_error(&format!("{}:{}", path.display(), name)))?;
            let contents = prettyplease::unparse(&file);
            if single_file {
                write!(&mut lib_contents, "{contents}")?;
            } else {
                fs::write(path, contents).wrap_err("failed to write to file")?;
                write_mod_name(&mut lib_contents, &name)?;
            }
        }

        let lib_path = src.join("lib.rs");
        let lib_file = syn::parse_file(&lib_contents).wrap_err_with(|| parse_error("lib.rs"))?;
        let lib_contents = prettyplease::unparse(&lib_file);
        fs::write(lib_path, lib_contents).wrap_err("Failed to write lib.rs")?;

        Ok(())
    }

    /// Attempts to detect the appropriate license.
    pub fn parse_license_alias(license: &str) -> String {
        match license.trim().to_lowercase().as_str() {
            "mit" => "MIT".to_string(),
            "apache" | "apache2" | "apache20" | "apache2.0" => "Apache-2.0".to_string(),
            "gpl" | "gpl3" => "GPL-3.0".to_string(),
            "lgpl" | "lgpl3" => "LGPL-3.0".to_string(),
            "agpl" | "agpl3" => "AGPL-3.0".to_string(),
            "bsd" | "bsd3" => "BSD-3-Clause".to_string(),
            "bsd2" => "BSD-2-Clause".to_string(),
            "mpl" | "mpl2" => "MPL-2.0".to_string(),
            "isc" => "ISC".to_string(),
            "unlicense" => "Unlicense".to_string(),
            _ => license.trim().to_string(),
        }
    }

    pub fn write_to_module(
        &mut self,
        bindings_path: &Path,
        single_file: bool,
        all_derives: bool,
    ) -> Result<()> {
        self.generate_bindings(all_derives)?;

        let _ = fs::create_dir_all(bindings_path);

        let mut mod_contents = r#"#![allow(unused_imports, clippy::all, rustdoc::all)]
        //! This module contains the sol! generated bindings for solidity contracts.
        //! This is autogenerated code.
        //! Do not manually edit these files.
        //! These files may be overwritten by the codegen system at any time.
        "#
        .to_string();

        for instance in &self.instances {
            let name = instance.name.to_snake_case();
            if !single_file {
                // Module
                write_mod_name(&mut mod_contents, &name)?;
                let mut contents = String::new();

                write!(contents, "{}", instance.expansion.as_ref().unwrap())?;
                let file = syn::parse_file(&contents)?;

                let contents = prettyplease::unparse(&file);
                fs::write(bindings_path.join(format!("{name}.rs")), contents)
                    .wrap_err("Failed to write file")?;
            } else {
                // Single File
                let mut contents = String::new();
                write!(contents, "{}\n\n", instance.expansion.as_ref().unwrap())?;
                write!(mod_contents, "{contents}")?;
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
    #[expect(clippy::too_many_arguments)]
    pub fn check_consistency(
        &self,
        name: &str,
        version: &str,
        crate_path: &Path,
        single_file: bool,
        check_cargo_toml: bool,
        is_mod: bool,
        alloy_version: Option<String>,
        alloy_rev: Option<String>,
    ) -> Result<()> {
        if check_cargo_toml {
            self.check_cargo_toml(name, version, crate_path, alloy_version, alloy_rev)?;
        }

        let mut super_contents = String::new();
        write!(
            &mut super_contents,
            r#"#![allow(unused_imports, clippy::all, rustdoc::all)]
            //! This module contains the sol! generated bindings for solidity contracts.
            //! This is autogenerated code.
            //! Do not manually edit these files.
            //! These files may be overwritten by the codegen system at any time.
            "#
        )?;
        if !single_file {
            for instance in &self.instances {
                let name = instance.name.to_snake_case();
                let path = if is_mod {
                    crate_path.join(format!("{name}.rs"))
                } else {
                    crate_path.join(format!("src/{name}.rs"))
                };
                let tokens = instance
                    .expansion
                    .as_ref()
                    .ok_or_eyre(format!("TokenStream for {path:?} does not exist"))?
                    .to_string();

                self.check_file_contents(&path, &tokens)?;
                write_mod_name(&mut super_contents, &name)?;
            }

            let super_path =
                if is_mod { crate_path.join("mod.rs") } else { crate_path.join("src/lib.rs") };
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

        // Format both
        let file_contents = syn::parse_file(file_contents)?;
        let formatted_file = prettyplease::unparse(&file_contents);

        let expected_contents = syn::parse_file(expected_contents)?;
        let formatted_exp = prettyplease::unparse(&expected_contents);

        eyre::ensure!(
            formatted_file == formatted_exp,
            "File contents do not match expected contents for {file_path:?}"
        );
        Ok(())
    }

    fn check_cargo_toml(
        &self,
        name: &str,
        version: &str,
        crate_path: &Path,
        alloy_version: Option<String>,
        alloy_rev: Option<String>,
    ) -> Result<()> {
        eyre::ensure!(crate_path.is_dir(), "Crate path must be a directory");

        let cargo_toml_path = crate_path.join("Cargo.toml");

        eyre::ensure!(cargo_toml_path.is_file(), "Cargo.toml must exist");
        let cargo_toml_contents =
            fs::read_to_string(cargo_toml_path).wrap_err("Failed to read Cargo.toml")?;

        let name_check = format!("name = \"{name}\"");
        let version_check = format!("version = \"{version}\"");
        let alloy_dep_check = Self::get_alloy_dep(alloy_version, alloy_rev);
        let toml_consistent = cargo_toml_contents.contains(&name_check)
            && cargo_toml_contents.contains(&version_check)
            && cargo_toml_contents.contains(&alloy_dep_check);
        eyre::ensure!(
            toml_consistent,
            r#"The contents of Cargo.toml do not match the expected output of the latest `sol!` version.
                This indicates that the existing bindings are outdated and need to be generated again."#
        );

        Ok(())
    }

    /// Returns the `alloy` dependency string for the Cargo.toml file.
    /// If `alloy_version` is provided, it will use that version from crates.io.
    /// If `alloy_rev` is provided, it will use that revision from the GitHub repository.
    fn get_alloy_dep(alloy_version: Option<String>, alloy_rev: Option<String>) -> String {
        if let Some(alloy_version) = alloy_version {
            format!(
                r#"alloy = {{ version = "{alloy_version}", features = ["sol-types", "contract"] }}"#,
            )
        } else if let Some(alloy_rev) = alloy_rev {
            format!(
                r#"alloy = {{ git = "https://github.com/alloy-rs/alloy", rev = "{alloy_rev}", features = ["sol-types", "contract"] }}"#,
            )
        } else {
            r#"alloy = { version = "1.0", features = ["sol-types", "contract"] }"#.to_string()
        }
    }
}

fn write_mod_name(contents: &mut String, name: &str) -> Result<()> {
    if syn::parse_str::<syn::Ident>(&format!("pub mod {name};")).is_ok() {
        write!(contents, "pub mod {name};")?;
    } else {
        write!(contents, "pub mod r#{name};")?;
    }
    Ok(())
}
