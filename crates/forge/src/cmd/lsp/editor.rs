//! Launch the bundled VS Code client without requiring a Foundry checkout.

use alloy_primitives::{Keccak256, keccak256};
use eyre::{Context, Result, ensure, eyre};
use flate2::read::GzDecoder;
use foundry_config::Config;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CLIENT: &[u8] = include_bytes!("../../../../../editors/vscode/dist/extension.js.gz");
const ASSETS: &[(&str, &[u8])] = &[
    ("package.json", include_bytes!("../../../../../editors/vscode/package.json")),
    (
        "language-configuration.json",
        include_bytes!("../../../../../editors/vscode/language-configuration.json"),
    ),
    (
        "syntaxes/solidity.json",
        include_bytes!("../../../../../editors/vscode/syntaxes/solidity.json"),
    ),
    (
        "syntaxes/solidity-markdown-injection.json",
        include_bytes!("../../../../../editors/vscode/syntaxes/solidity-markdown-injection.json"),
    ),
    ("syntaxes/LICENSE", include_bytes!("../../../../../editors/vscode/syntaxes/LICENSE")),
    ("LICENSE", include_bytes!("../../../../../editors/vscode/LICENSE")),
    ("LICENSE-MIT", include_bytes!("../../../../../editors/vscode/LICENSE-MIT")),
    ("LICENSE-APACHE", include_bytes!("../../../../../editors/vscode/LICENSE-APACHE")),
    ("NOTICE.md", include_bytes!("../../../../../editors/vscode/NOTICE.md")),
    (
        "THIRD_PARTY_NOTICES.txt",
        include_bytes!("../../../../../editors/vscode/dist/THIRD_PARTY_NOTICES.txt"),
    ),
];

pub(super) fn launch(path: Option<&Path>, code_path: Option<&Path>) -> Result<()> {
    let project =
        dunce::canonicalize(path.map(Path::to_path_buf).unwrap_or(std::env::current_dir()?))
            .wrap_err("Could not open the project directory")?;
    ensure!(project.is_dir(), "Project path must be a directory: {}", project.display());
    let forge = std::env::current_exe()?;
    let profile = Config::selected_profile().to_string();
    let cache = Config::foundry_cache_dir()
        .ok_or_else(|| eyre!("Could not find the Foundry cache directory"))?
        .join("lsp");
    let extension = prepare_extension(&cache)?;

    // Separate projects, executables and profiles cannot inherit a stale VS Code process
    // environment.
    let session_key = keccak256(serde_json::to_vec(&(&project, &forge, &profile))?);
    // VS Code appends a Unix socket name; keep the profile path within platform limits.
    let session = cache.join("vscode").join(&format!("{session_key:x}")[..16]);
    let user_data = session.join("user-data");
    let extensions = session.join("extensions");
    fs::create_dir_all(user_data.join("User"))?;
    fs::create_dir_all(&extensions)?;
    let settings = user_data.join("User/settings.json");
    if !settings.exists() {
        let mut file = tempfile::NamedTempFile::new_in(user_data.join("User"))?;
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({
                "solarLsp.forgePath": forge,
                "workbench.startupEditor": "none",
            }),
        )?;
        file.write_all(b"\n")?;
        if let Err(error) = file.persist_noclobber(&settings)
            && error.error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(error.into());
        }
    }

    let code = code_path.map(Path::to_path_buf).unwrap_or_else(default_code_path);
    sh_status!("Opening VS Code with Forge Solidity support: {}", project.display())?;
    let status = Command::new(&code)
        .arg("--new-window")
        .arg("--extensionDevelopmentPath")
        .arg(&extension)
        .arg("--user-data-dir")
        .arg(&user_data)
        .arg("--extensions-dir")
        .arg(&extensions)
        .arg(&project)
        .env("FOUNDRY_LSP_FORGE", &forge)
        .env("FOUNDRY_PROFILE", &profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .wrap_err_with(|| {
            format!(
                "Could not launch VS Code using {}. Install VS Code and its `code` command, \
                 or pass --code-path <PATH> to the VS Code CLI. \
                 Use `forge lsp --stdio` for another editor.",
                code.display()
            )
        })?;
    ensure!(status.success(), "VS Code launcher exited with {status}");
    Ok(())
}

fn prepare_extension(cache: &Path) -> Result<PathBuf> {
    let mut hash = Keccak256::new();
    hash.update(CLIENT);
    for (name, bytes) in ASSETS {
        hash.update(name.as_bytes());
        hash.update(bytes);
    }
    let parent = cache.join("extensions");
    let directory = parent.join(format!("{:x}", hash.finalize()));
    if directory.is_dir() {
        return Ok(directory);
    }

    fs::create_dir_all(&parent)?;
    let staging = tempfile::Builder::new().prefix(".extract-").tempdir_in(&parent)?;
    fs::create_dir_all(staging.path().join("out"))?;
    io::copy(
        &mut GzDecoder::new(CLIENT),
        &mut fs::File::create(staging.path().join("out/extension.js"))?,
    )?;
    for (name, bytes) in ASSETS {
        let path = staging.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    // Publish a complete directory atomically; concurrent launches may finish extraction first.
    if let Err(error) = fs::rename(staging.path(), &directory)
        && !directory.is_dir()
    {
        return Err(error).wrap_err("Could not cache the bundled VS Code extension");
    }
    Ok(directory)
}

fn default_code_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| path.join("code").is_file())
        }) {
            return PathBuf::from("code");
        }
        // The application includes a CLI even when `code` has not been added to PATH.
        let cli =
            PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code");
        if cli.is_file() {
            return cli;
        }
    }
    PathBuf::from(if cfg!(windows) { "code.cmd" } else { "code" })
}
