use std::{collections::HashMap, path::Path};
use zed_extension_api::{self as zed, LanguageServerId, Result, serde_json, settings::LspSettings};

const INSTALL_GUIDANCE: &str = "Install or upgrade Foundry from https://getfoundry.sh with an LSP-enabled Forge. Verify `forge lsp --stdio --help` succeeds from the worktree directory; for this checkout, run `cargo build --locked -p forge --bin forge`. Set lsp.solar.settings.forgePath to its absolute path.";
const LEGACY_BINARY_GUIDANCE: &str = "Remove lsp.solar.binary.path and binary.arguments, which override the language server command in Zed. Set lsp.solar.settings.forgePath to an absolute Forge path instead. A former Solar path is not a Forge path.";

struct SolarExtension {
    forge_paths: HashMap<u64, String>,
}

impl SolarExtension {
    fn command(&mut self, worktree: &zed::Worktree) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree("solar", worktree)?;
        let command = forge_command(&settings, worktree.shell_env(), |name| worktree.which(name))?;
        self.forge_paths.insert(worktree.id(), command.command.clone());
        Ok(command)
    }
}

impl zed::Extension for SolarExtension {
    fn new() -> Self {
        Self { forge_paths: HashMap::new() }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        check_server_id(language_server_id)?;
        self.command(worktree)
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
            self.command(worktree)?.command
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
) -> Result<zed::Command> {
    check_binary_settings(settings)?;
    let configured_path = settings.settings.as_ref().and_then(|settings| settings.get("forgePath"));
    let forge_path = match configured_path {
        Some(serde_json::Value::String(path)) if is_absolute_path(path) => path.clone(),
        None | Some(serde_json::Value::Null) => which("forge")
            .ok_or_else(|| format!("Forge was not found on PATH. {INSTALL_GUIDANCE}"))?,
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

zed::register_extension!(SolarExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_dependent_forge_shim_is_not_probed_before_launch() {
        let settings = LspSettings {
            initialization_options: Some(serde_json::json!({ "flycheck": { "enabled": false } })),
            ..Default::default()
        };
        let command =
            forge_command(&settings, vec![("PWD".into(), "/workspace/project".into())], |_| {
                Some("/tools/shims/forge".into())
            })
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
            let command = forge_command(&settings, Vec::new(), |_| {
                panic!("custom Forge must not consult PATH")
            })
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
            let command = forge_command(&LspSettings::default(), Vec::new(), |name| match name {
                "forge" => Some("/tools/forge".into()),
                "solar" if solar_available => panic!("Solar must not be probed"),
                other => panic!("unexpected executable lookup: {other}"),
            })
            .unwrap();
            assert_eq!(command.command, "/tools/forge");
        }
    }

    #[test]
    fn missing_forge_reports_installation_guidance_without_running_solar() {
        let error = forge_command(&LspSettings::default(), Vec::new(), |name| {
            assert_eq!(name, "forge");
            None
        })
        .unwrap_err();
        assert_eq!(error, format!("Forge was not found on PATH. {INSTALL_GUIDANCE}"));
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
            let error = forge_command(&settings, Vec::new(), |_| {
                panic!("legacy configuration must be migrated first")
            })
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
            forge_command(&settings, Vec::new(), |_| Some("/tools/forge".into())).unwrap_err();
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
        let command =
            forge_command(&settings, vec![("FOUNDRY_PROFILE".into(), "default".into())], |_| {
                Some("/tools/forge".into())
            })
            .unwrap();
        assert_eq!(command.env, [("FOUNDRY_PROFILE".into(), "editor".into())]);
    }
}
