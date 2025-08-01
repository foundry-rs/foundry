use eyre::Result;

pub struct ForgeCompileOutput;
pub struct ForgeLintOutput;
use tower_lsp::lsp_types::{Diagnostic, Url};

pub async fn get_lint_diagnostics(target_file: &Url) -> Result<Vec<Diagnostic>> {
    // TODO run single file lint
    let _ = target_file;
    Ok(Vec::new())
}

pub async fn get_compile_diagnostics(target_file: &Url) -> Result<Vec<Diagnostic>> {
    // TODO run single file compile or build
    let _ = target_file;
    Ok(Vec::new())
}
