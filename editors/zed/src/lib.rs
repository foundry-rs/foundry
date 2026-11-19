use std::{collections::HashMap, fs, path::Path};
use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, LanguageServerId,
    LanguageServerInstallationStatus, Os, Result,
    http_client::{HttpMethod, HttpRequest, RedirectPolicy},
    serde_json,
    settings::LspSettings,
};

const FOUNDRY_REPO: &str = "foundry-rs/foundry";
const DOWNLOAD_DIR_PREFIX: &str = "forge-";
const COMPLETE_MARKER: &str = ".complete";
const INSTALL_GUIDANCE: &str = "Install or upgrade Foundry from https://getfoundry.sh with an LSP-enabled Forge. Verify `forge lsp --stdio --help` succeeds from the worktree directory; for this checkout, run `cargo build --locked -p forge --bin forge`. Set lsp.solar.settings.forgePath to its absolute path.";
const LEGACY_BINARY_GUIDANCE: &str = "Remove lsp.solar.binary.path and binary.arguments, which override the language server command in Zed. Set lsp.solar.settings.forgePath to an absolute Forge path instead. A former Solar path is not a Forge path.";

struct SolarExtension {
    forge_paths: HashMap<u64, String>,
    downloaded_forge: Option<String>,
}

impl SolarExtension {
    fn command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree("solar", worktree)?;
        let command = forge_command(
            &settings,
            worktree.shell_env(),
            |name| worktree.which(name),
            || self.downloaded_forge(language_server_id),
        )?;
        self.forge_paths.insert(worktree.id(), command.command.clone());
        Ok(command)
    }

    fn downloaded_forge(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.downloaded_forge
            && is_file(path)
        {
            return Ok(path.clone());
        }
        let result = download_forge(language_server_id);
        let status = match &result {
            Ok(_) => LanguageServerInstallationStatus::None,
            Err(err) => LanguageServerInstallationStatus::Failed(err.clone()),
        };
        zed::set_language_server_installation_status(language_server_id, &status);
        let path = absolute_work_path(&result?)?;
        self.downloaded_forge = Some(path.clone());
        Ok(path)
    }
}

impl zed::Extension for SolarExtension {
    fn new() -> Self {
        Self { forge_paths: HashMap::new(), downloaded_forge: None }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        check_server_id(language_server_id)?;
        self.command(language_server_id, worktree)
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        check_server_id(language_server_id)?;
        let settings = LspSettings::for_worktree("solar", worktree)?;
        check_binary_settings(&settings)?;
        let forge_path = if let Some(path) = self.forge_paths.get(&worktree.id()) {
            path.clone()
        } else {
            self.command(language_server_id, worktree)?.command
        };
        initialization_options(&settings, &forge_path).map(Some)
    }
}

fn check_server_id(language_server_id: &LanguageServerId) -> Result<()> {
    if language_server_id.as_ref() == "solar" {
        Ok(())
    } else {
        Err(format!("unknown language server: {language_server_id}"))
    }
}

fn check_binary_settings(settings: &LspSettings) -> Result<()> {
    if settings
        .binary
        .as_ref()
        .is_some_and(|binary| binary.path.is_some() || binary.arguments.is_some())
    {
        return Err(LEGACY_BINARY_GUIDANCE.into());
    }
    Ok(())
}

fn forge_command(
    settings: &LspSettings,
    mut env: Vec<(String, String)>,
    which: impl FnOnce(&str) -> Option<String>,
    download: impl FnOnce() -> Result<String>,
) -> Result<zed::Command> {
    check_binary_settings(settings)?;
    let configured_path = settings.settings.as_ref().and_then(|settings| settings.get("forgePath"));
    let forge_path = match configured_path {
        Some(serde_json::Value::String(path)) if is_absolute_path(path) => path.clone(),
        None | Some(serde_json::Value::Null) => match which("forge") {
            Some(path) => path,
            None => download().map_err(|err| {
                format!("Forge was not found on PATH and could not be downloaded: {err}. {INSTALL_GUIDANCE}")
            })?,
        },
        Some(_) => {
            return Err("lsp.solar.settings.forgePath must be an absolute Forge path.".into());
        }
    };
    initialization_options(settings, &forge_path)?;
    if let Some(binary_env) = settings.binary.as_ref().and_then(|binary| binary.env.as_ref()) {
        for (key, value) in binary_env {
            env.retain(|(existing, _)| existing != key);
            env.push((key.clone(), value.clone()));
        }
    }
    // Zed starts the server in the worktree. Its process API cannot set that cwd for a probe.
    Ok(zed::Command { command: forge_path, args: vec!["lsp".into(), "--stdio".into()], env })
}

// WASI path parsing uses Unix rules even when the host is Windows.
fn is_absolute_path(path: &str) -> bool {
    Path::new(path).is_absolute()
        || path.starts_with(r"\\")
        || (path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && path.as_bytes().get(1) == Some(&b':')
            && path.as_bytes().get(2).is_some_and(|byte| matches!(byte, b'/' | b'\\')))
}

fn initialization_options(settings: &LspSettings, forge_path: &str) -> Result<serde_json::Value> {
    let mut options = match settings.initialization_options.clone() {
        None | Some(serde_json::Value::Null) => serde_json::Map::new(),
        Some(serde_json::Value::Object(options)) => options,
        Some(_) => return Err("lsp.solar.initialization_options must be an object.".into()),
    };
    if let Some(previous) = options.get("forgePath")
        && previous.as_str() != Some(forge_path)
    {
        return Err("Remove lsp.solar.initialization_options.forgePath and set lsp.solar.settings.forgePath instead, so the language server, formatter, and checks use the same Forge.".into());
    }
    options.insert("forgePath".into(), forge_path.into());
    Ok(serde_json::Value::Object(options))
}

/// Downloads the latest stable Forge release into the extension's work directory, falling back to
/// an earlier download when that fails. Returns a path relative to the work directory.
fn download_forge(language_server_id: &LanguageServerId) -> Result<String> {
    zed::set_language_server_installation_status(
        language_server_id,
        &LanguageServerInstallationStatus::CheckingForUpdate,
    );
    let (os, arch) = zed::current_platform();
    download_latest_forge(language_server_id, os, arch).or_else(|err| cached_forge(os).ok_or(err))
}

fn download_latest_forge(
    language_server_id: &LanguageServerId,
    os: Os,
    arch: Architecture,
) -> Result<String> {
    let release = latest_release()?;
    let version = release["tag_name"].as_str().ok_or("the latest Foundry release has no tag")?;
    let (asset_name, file_type) = release_asset(version, os, arch)?;
    let download_url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"] == asset_name.as_str())
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or_else(|| format!("Foundry {version} has no {asset_name} asset"))?;
    let dir = format!("{DOWNLOAD_DIR_PREFIX}{version}");
    let binary = format!("{dir}/{}", forge_binary_name(os));
    if !is_complete(&dir, os) {
        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        // Clear any partial download before extracting again.
        fs::remove_dir_all(&dir).ok();
        zed::download_file(download_url, &dir, file_type)?;
        if os != Os::Windows {
            zed::make_file_executable(&binary)?;
        }
        // The archive also holds anvil, cast, chisel and solar; keep only Forge.
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_name() != forge_binary_name(os) {
                fs::remove_file(entry.path()).ok();
            }
        }
        fs::write(format!("{dir}/{COMPLETE_MARKER}"), "").map_err(|err| err.to_string())?;
        for stale in download_dirs().filter(|name| *name != dir) {
            fs::remove_dir_all(stale).ok();
        }
    }
    Ok(binary)
}

// `latest_github_release` only scans the first page of releases, which daily nightlies can fill.
fn latest_release() -> Result<serde_json::Value> {
    let response = HttpRequest::builder()
        .method(HttpMethod::Get)
        .url(format!("https://api.github.com/repos/{FOUNDRY_REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "zed-forge-lsp")
        .redirect_policy(RedirectPolicy::FollowAll)
        .build()?
        .fetch()?;
    serde_json::from_slice(&response.body).map_err(|err| err.to_string())
}

fn cached_forge(os: Os) -> Option<String> {
    download_dirs()
        .find(|dir| is_complete(dir, os))
        .map(|dir| format!("{dir}/{}", forge_binary_name(os)))
}

fn download_dirs() -> impl Iterator<Item = String> {
    fs::read_dir(".")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(DOWNLOAD_DIR_PREFIX))
}

fn is_complete(dir: &str, os: Os) -> bool {
    is_file(&format!("{dir}/{COMPLETE_MARKER}"))
        && is_file(&format!("{dir}/{}", forge_binary_name(os)))
}

fn release_asset(
    version: &str,
    os: Os,
    arch: Architecture,
) -> Result<(String, DownloadedFileType)> {
    let platform = match (os, arch) {
        (Os::Mac, Architecture::Aarch64) => "darwin_arm64",
        (Os::Mac, Architecture::X8664) => "darwin_amd64",
        // The static musl builds also run on distros with a glibc older than the release runners'.
        (Os::Linux, Architecture::Aarch64) => "alpine_arm64",
        (Os::Linux, Architecture::X8664) => "alpine_amd64",
        (Os::Windows, Architecture::X8664) => "win32_amd64",
        _ => return Err(format!("Foundry does not publish Forge for {os:?} {arch:?}")),
    };
    let (extension, file_type) = if os == Os::Windows {
        ("zip", DownloadedFileType::Zip)
    } else {
        ("tar.gz", DownloadedFileType::GzipTar)
    };
    Ok((format!("foundry_{version}_{platform}.{extension}"), file_type))
}

fn forge_binary_name(os: Os) -> &'static str {
    if os == Os::Windows { "forge.exe" } else { "forge" }
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

// The server runs Forge for formatting from the worktree, so it needs an absolute path.
fn absolute_work_path(path: &str) -> Result<String> {
    let dir = std::env::current_dir().map_err(|err| err.to_string())?;
    Ok(dir.join(path).to_string_lossy().into_owned())
}

zed::register_extension!(SolarExtension);

#[cfg(test)]
mod tests {
    use super::*;

    fn no_download() -> Result<String> {
        panic!("Forge must not be downloaded")
    }

    #[test]
    fn worktree_dependent_forge_shim_is_not_probed_before_launch() {
        let settings = LspSettings {
            initialization_options: Some(serde_json::json!({ "flycheck": { "enabled": false } })),
            ..Default::default()
        };
        let command = forge_command(
            &settings,
            vec![("PWD".into(), "/workspace/project".into())],
            |_| Some("/tools/shims/forge".into()),
            no_download,
        )
        .unwrap();
        assert_eq!(command.command, "/tools/shims/forge");
        assert_eq!(command.args, ["lsp", "--stdio"]);
        assert_eq!(command.env, [("PWD".into(), "/workspace/project".into())]);
        assert_eq!(
            initialization_options(&settings, &command.command).unwrap(),
            serde_json::json!({
                "forgePath": "/tools/shims/forge",
                "flycheck": { "enabled": false }
            })
        );
    }

    #[test]
    fn default_command_resolves_forge_and_preserves_worktree_environment() {
        let command = forge_command(
            &zed::settings::LspSettings::default(),
            vec![("FOUNDRY_PROFILE".into(), "editor".into())],
            |name| {
                assert_eq!(name, "forge");
                Some("/tools/forge".into())
            },
            no_download,
        )
        .unwrap();
        assert_eq!(command.command, "/tools/forge");
        assert_eq!(command.args, ["lsp", "--stdio"]);
        assert_eq!(command.env, [("FOUNDRY_PROFILE".into(), "editor".into())]);
    }

    #[test]
    fn custom_forge_uses_one_binary_and_preserves_initialization_options() {
        for forge_path in ["/custom/forge", r"C:\Foundry\forge.exe"] {
            let settings = LspSettings {
                settings: Some(serde_json::json!({ "forgePath": forge_path })),
                initialization_options: Some(serde_json::json!({
                    "flycheck": { "enabled": false },
                    "indexing": { "exclude": ["vendor"] }
                })),
                ..Default::default()
            };
            let command = forge_command(
                &settings,
                Vec::new(),
                |_| panic!("custom Forge must not consult PATH"),
                no_download,
            )
            .unwrap();
            assert_eq!(command.command, forge_path);
            assert_eq!(command.args, ["lsp", "--stdio"]);
            assert_eq!(
                initialization_options(&settings, &command.command).unwrap(),
                serde_json::json!({
                    "forgePath": forge_path,
                    "flycheck": { "enabled": false },
                    "indexing": { "exclude": ["vendor"] }
                })
            );
        }
    }

    #[test]
    fn solar_on_path_never_changes_default_forge_selection() {
        for solar_available in [false, true] {
            let command = forge_command(
                &LspSettings::default(),
                Vec::new(),
                |name| match name {
                    "forge" => Some("/tools/forge".into()),
                    "solar" if solar_available => panic!("Solar must not be probed"),
                    other => panic!("unexpected executable lookup: {other}"),
                },
                no_download,
            )
            .unwrap();
            assert_eq!(command.command, "/tools/forge");
        }
    }

    #[test]
    fn missing_forge_falls_back_to_download() {
        let command = forge_command(
            &LspSettings::default(),
            Vec::new(),
            |name| {
                assert_eq!(name, "forge");
                None
            },
            || Ok("/extension/forge-v1.8.6/forge".into()),
        )
        .unwrap();
        assert_eq!(command.command, "/extension/forge-v1.8.6/forge");
        assert_eq!(command.args, ["lsp", "--stdio"]);
    }

    #[test]
    fn failed_download_reports_installation_guidance_without_running_solar() {
        let error =
            forge_command(&LspSettings::default(), Vec::new(), |_| None, || Err("offline".into()))
                .unwrap_err();
        assert_eq!(
            error,
            format!(
                "Forge was not found on PATH and could not be downloaded: offline. {INSTALL_GUIDANCE}"
            )
        );
    }

    #[test]
    fn release_assets_match_published_names() {
        for (os, arch, name) in [
            (Os::Mac, Architecture::Aarch64, "foundry_v1.8.6_darwin_arm64.tar.gz"),
            (Os::Mac, Architecture::X8664, "foundry_v1.8.6_darwin_amd64.tar.gz"),
            (Os::Linux, Architecture::Aarch64, "foundry_v1.8.6_alpine_arm64.tar.gz"),
            (Os::Linux, Architecture::X8664, "foundry_v1.8.6_alpine_amd64.tar.gz"),
            (Os::Windows, Architecture::X8664, "foundry_v1.8.6_win32_amd64.zip"),
        ] {
            assert_eq!(release_asset("v1.8.6", os, arch).unwrap().0, name);
        }
        assert!(release_asset("v1.8.6", Os::Windows, Architecture::Aarch64).is_err());
        assert!(release_asset("v1.8.6", Os::Linux, Architecture::X86).is_err());
    }

    #[test]
    fn legacy_binary_overrides_require_explicit_migration() {
        for binary in [
            serde_json::json!({ "path": "/tools/solar" }),
            serde_json::json!({ "arguments": ["lsp"] }),
        ] {
            let settings =
                serde_json::from_value::<LspSettings>(serde_json::json!({ "binary": binary }))
                    .unwrap();
            let error = forge_command(
                &settings,
                Vec::new(),
                |_| panic!("legacy configuration must be migrated first"),
                no_download,
            )
            .unwrap_err();
            assert_eq!(error, LEGACY_BINARY_GUIDANCE);
        }
    }

    #[test]
    fn mismatched_formatter_binary_is_rejected_before_launch() {
        let settings = LspSettings {
            initialization_options: Some(serde_json::json!({ "forgePath": "/other/forge" })),
            ..Default::default()
        };
        let error =
            forge_command(&settings, Vec::new(), |_| Some("/tools/forge".into()), no_download)
                .unwrap_err();
        assert_eq!(
            error,
            "Remove lsp.solar.initialization_options.forgePath and set lsp.solar.settings.forgePath instead, so the language server, formatter, and checks use the same Forge."
        );
    }

    #[test]
    fn configured_environment_overrides_worktree_environment() {
        let settings = serde_json::from_value::<LspSettings>(serde_json::json!({
            "binary": { "env": { "FOUNDRY_PROFILE": "editor" } }
        }))
        .unwrap();
        let command = forge_command(
            &settings,
            vec![("FOUNDRY_PROFILE".into(), "default".into())],
            |_| Some("/tools/forge".into()),
            no_download,
        )
        .unwrap();
        assert_eq!(command.env, [("FOUNDRY_PROFILE".into(), "editor".into())]);
    }
}
