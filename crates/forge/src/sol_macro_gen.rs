use alloy_json_abi::JsonAbi;
use foundry_common::fs::json_files;
use proc_macro2::{Ident, Span, TokenStream};
use quote::TokenStreamExt;
use serde_json::Value;
use std::path::{Path, PathBuf};
pub struct SolMacroGen {
    pub path: PathBuf,
    pub name: String,
}

impl SolMacroGen {
    pub fn new(path: PathBuf, name: String) -> Self {
        Self { path, name }
    }

    pub fn get_json_abi(&self) -> JsonAbi {
        tracing::info!("Reading JSON file at path {:?}", self.path);

        let json = std::fs::read(&self.path).expect("Failed to read JSON file");

        // Need to do this to get the abi in the next step.
        let json: Value = serde_json::from_slice(&json).expect("Failed to parse JSON file");

        // Get the abi from the json.
        if let Some(abi) = json.get("abi") {
            serde_json::from_str(&abi.clone().to_string()).expect("Failed to parse ABI")
        } else {
            // TODO (yash): Remove panic, throw error.
            panic!("No ABI found in JSON file");
        }
    }
}

pub struct MultiSolMacroGen {
    pub artifacts_path: PathBuf,
    pub instances: Vec<SolMacroGen>,
}

impl MultiSolMacroGen {
    pub fn new(artifacts_path: &Path) -> Self {
        let artifacts_path = artifacts_path.to_path_buf();
        let mut instances = Vec::new();

        let abi_files = json_files(&artifacts_path)
            .filter_map(|path| {
                // Ignore the build info JSON.
                if path.to_str()?.contains("/build-info/") {
                    return None;
                }

                // We don't want `.metadata.json` files.
                let stem = path.file_stem()?.to_str()?;
                if stem.ends_with(".metadata") {
                    return None;
                }

                let name = stem.split('.').next().unwrap();

                // Best effort identifier cleanup.
                let name = name.replace(char::is_whitespace, "").replace('-', "_");

                Some((name, path))
            })
            .collect::<Vec<_>>();

        for (name, path) in abi_files {
            let instance = SolMacroGen::new(path, name);
            instances.push(instance);
        }

        Self { artifacts_path, instances }
    }

    pub fn write_to_crate(&self) {
        for instance in &self.instances {
            let mut json_abi = instance.get_json_abi();

            json_abi.dedup();
            let sol_str = json_abi.to_sol(&instance.name, None);

            let ident_name: Ident = Ident::new(&instance.name, Span::call_site());

            let _tokens = tokens_for_sol(&ident_name, &sol_str);

            // TOOD: Expand TokenStream.
        }
    }
}

/// Returns `sol!` tokens.
/// Taken from alloy-macro-input/json
/// TODO(yash): Remove this after making it pub in alloy.
fn tokens_for_sol(name: &Ident, sol: &str) -> TokenStream {
    let mk_err = |s: &str| {
        let msg = format!(
            "`JsonAbi::to_sol` generated invalid Rust tokens: {s}\n\
             This is a bug. We would appreciate a bug report: \
             https://github.com/alloy-rs/core/issues/new/choose"
        );
        syn::Error::new(name.span(), msg)
    };
    let brace_idx = sol.find('{').ok_or_else(|| mk_err("missing `{`")).unwrap();
    let tts = syn::parse_str::<TokenStream>(&sol[brace_idx..])
        .map_err(|e| mk_err(&e.to_string()))
        .unwrap();

    let mut tokens = TokenStream::new();
    // append `name` manually for the span
    tokens.append::<Ident>(syn::parse_str("interface").unwrap());
    tokens.append(name.clone());
    tokens.extend(tts);
    tokens
}
