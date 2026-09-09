use super::lsp_client::{LspClient, request};
use async_lsp::{
    LanguageServer,
    lsp_types::{
        ClientCapabilities, DiagnosticSeverity, DidChangeTextDocumentParams,
        DidChangeWatchedFilesParams, DidOpenTextDocumentParams, DocumentFormattingParams,
        FileChangeType, FileEvent, FormattingOptions, GotoDefinitionParams, GotoDefinitionResponse,
        InitializeParams, InitializedParams, Location, OneOf, Position, Range,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        TextDocumentPositionParams, TextEdit, Url, VersionedTextDocumentIdentifier,
        WorkspaceFolder, WorkspaceSymbolParams, WorkspaceSymbolResponse,
    },
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

#[cfg(unix)]
use foundry_test_utils::snapbox::{IntoData, data::DataFormat};
#[cfg(unix)]
use rexpect::{Encoding, process::wait::WaitStatus, reader::Options, spawn_with_options};

const SYMBOL_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(unix)]
forgetest!(lsp_vscode_opens_current_project_with_bundled_extension, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    let executables = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(prj.root()).unwrap();
    let forge = executables.path().join("standalone forge");
    fs::hard_link(env!("CARGO_BIN_EXE_forge"), &forge).unwrap();
    let forge = dunce::canonicalize(forge).unwrap();
    let code = executables.path().join("mock code");
    fs::write(
        &code,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$FORGE_LSP_TEST_ARGS"
printf '%s\n' "$FOUNDRY_PROFILE" "$FOUNDRY_LSP_FORGE" > "$FORGE_LSP_TEST_PROFILE"
printf '%s\n' "${VSCODE_APPDATA-unset}" "${VSCODE_EXTENSIONS-unset}" "${VSCODE_PORTABLE-unset}" > "$FORGE_LSP_TEST_ENV"
"#,
    )
    .unwrap();
    fs::set_permissions(&code, fs::Permissions::from_mode(0o755)).unwrap();
    let captured_args = executables.path().join("args");
    let captured_profile = executables.path().join("profile");
    let captured_env = executables.path().join("env");
    let mut standalone = Command::new(&forge);
    standalone.current_dir(&project).env("NO_COLOR", "1");
    cmd.set_cmd(standalone);
    cmd.env("HOME", home.path());
    cmd.env("PATH", executables.path());
    cmd.env("FORGE_LSP_TEST_ARGS", &captured_args);
    cmd.env("FORGE_LSP_TEST_PROFILE", &captured_profile);
    cmd.env("FORGE_LSP_TEST_ENV", &captured_env);
    cmd.env("VSCODE_APPDATA", "/stale/appdata");
    cmd.env("VSCODE_EXTENSIONS", "/stale/extensions");
    cmd.env("VSCODE_PORTABLE", "/stale/portable");
    cmd.args(["lsp", "--vscode", "--profile", "editor", "--code-path"]).arg(&code);
    cmd.assert_empty_stdout();

    let captured = fs::read_to_string(&captured_args).unwrap();
    let arguments = captured.lines().collect::<Vec<_>>();
    assert_eq!(arguments.last().copied(), project.to_str());
    assert!(arguments.contains(&"--new-window"));
    let extension = arguments
        .windows(2)
        .find(|pair| pair[0] == "--extensionDevelopmentPath")
        .map(|pair| pair[1])
        .expect("VS Code must load the bundled extension");
    assert!(Path::new(extension).is_absolute());
    assert!(Path::new(extension).join("package.json").is_file());
    assert!(Path::new(extension).join("out/extension.js").is_file());
    assert!(Path::new(extension).join("syntaxes/solidity.json").is_file());
    let user_data = arguments
        .windows(2)
        .find(|pair| pair[0] == "--user-data-dir")
        .map(|pair| Path::new(pair[1]))
        .expect("VS Code must use a dedicated profile");
    assert_data_eq!(
        fs::read_to_string(user_data.join("User/settings.json"))
            .unwrap()
            .into_data()
            .is(DataFormat::Json),
        serde_json::to_string(&serde_json::json!({
            "solarLsp.forgePath": forge,
            "workbench.startupEditor": "none",
        }))
        .unwrap()
        .into_data()
        .is(DataFormat::Json),
    );
    assert_eq!(
        fs::read_to_string(&captured_profile).unwrap(),
        format!("editor\n{}\n", forge.display())
    );
    assert_eq!(fs::read_to_string(&captured_env).unwrap(), "unset\nunset\nunset\n");
    assert!(!project.join(".vscode").exists());

    // A terminal needs only `forge lsp`, and reopening preserves the managed profile's settings.
    let settings = user_data.join("User/settings.json");
    let custom_settings = "{\"editor.fontSize\":17}\n";
    fs::write(&settings, custom_settings).unwrap();
    symlink(&code, executables.path().join("code")).unwrap();
    let mut terminal = Command::new(&forge);
    terminal
        .current_dir(&project)
        .env("HOME", home.path())
        .env("PATH", executables.path())
        .env("FOUNDRY_PROFILE", "editor")
        .env("FORGE_LSP_TEST_ARGS", &captured_args)
        .env("FORGE_LSP_TEST_PROFILE", &captured_profile)
        .env("FORGE_LSP_TEST_ENV", &captured_env)
        .arg("lsp");
    let mut terminal = spawn_with_options(
        terminal,
        Options {
            timeout_ms: Some(30_000),
            strip_ansi_escape_codes: true,
            encoding: Encoding::UTF8,
        },
    )
    .unwrap();
    terminal.exp_eof().unwrap();
    assert!(matches!(terminal.process.wait().unwrap(), WaitStatus::Exited(_, 0)));
    assert_eq!(fs::read_to_string(captured_args).unwrap(), captured);
    assert_eq!(fs::read_to_string(settings).unwrap(), custom_settings);
});

forgetest!(lsp_stdio_rejects_editor_launch_options, |_prj, cmd| {
    cmd.args(["lsp", "--stdio", "--vscode"]).assert_code(2).stdout_eq(str![""]).stderr_eq(str![[
        r#"
error: the argument '--stdio' cannot be used with '--vscode'

Usage: forge lsp --stdio [PATH]

For more information, try '--help'.

"#
    ]]);
});

#[cfg(unix)]
forgetest!(lsp_code_path_reports_missing_editor, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    cmd.env("HOME", home.path());
    cmd.args(["lsp", "--code-path"]).arg(prj.root().join("missing-vscode"));
    cmd.assert_failure().stdout_eq(str![""]).stderr_eq(str![[r#"
Opening VS Code with Forge Solidity support: [..]
Error: Could not launch VS Code using [..]/missing-vscode. Install VS Code and its `code` command, or pass --code-path <PATH> to the VS Code CLI. Use `forge lsp --stdio` for another editor.

Context:
- No such file or directory (os error 2)

"#]]);
    assert!(!prj.root().join(".vscode").exists());
});

fn wait_for_workspace_symbols(client: &mut LspClient, expected: &str, unexpected: &str) {
    let deadline = Instant::now() + SYMBOL_TIMEOUT;
    let mut last_names = Vec::new();
    while Instant::now() < deadline {
        let response = request(
            &client.runtime,
            client.server.symbol(WorkspaceSymbolParams {
                query: String::new(),
                ..WorkspaceSymbolParams::default()
            }),
        );
        last_names = match response {
            None => Vec::new(),
            Some(WorkspaceSymbolResponse::Flat(symbols)) => {
                symbols.into_iter().map(|symbol| symbol.name).collect()
            }
            Some(WorkspaceSymbolResponse::Nested(symbols)) => {
                symbols.into_iter().map(|symbol| symbol.name).collect()
            }
        };
        if last_names.iter().any(|name| name == expected)
            && last_names.iter().all(|name| name != unexpected)
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }

    panic!(
        "expected workspace symbol `{expected}` without `{unexpected}`; observed: {last_names:?}"
    );
}

#[test]
fn lsp_profile_selects_workspace_sources() {
    let project = tempfile::tempdir().unwrap();
    let project_root = dunce::canonicalize(project.path()).unwrap();
    fs::write(
        project_root.join("foundry.toml"),
        "[profile.default]\nsrc = \"default-src\"\n[profile.custom]\nextends = \"base.toml\"\n",
    )
    .unwrap();
    fs::write(project_root.join("base.toml"), "[profile.custom]\nsrc = \"custom-src\"\n").unwrap();
    fs::create_dir_all(project_root.join("default-src")).unwrap();
    fs::create_dir_all(project_root.join("custom-src")).unwrap();
    fs::write(project_root.join("default-src/Default.sol"), "contract DefaultContract {}\n")
        .unwrap();
    fs::write(project_root.join("custom-src/Custom.sol"), "contract CustomContract {}\n").unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(
        &project_root,
        empty_path.path(),
        &["lsp", "--stdio", "--profile", "custom"],
    );
    let initialize = request(
        &client.runtime,
        client.server.initialize(InitializeParams {
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Url::from_directory_path(&project_root).unwrap(),
                name: "fixture".into(),
            }]),
            ..InitializeParams::default()
        }),
    );
    assert!(initialize.capabilities.workspace_symbol_provider.is_some());
    client.server.initialized(InitializedParams {}).unwrap();
    client.wait_for_log_message();

    wait_for_workspace_symbols(&mut client, "CustomContract", "DefaultContract");
    client.shutdown();
}

#[test]
fn lsp_reloads_host_resolved_config_after_manifest_change() {
    let project = tempfile::tempdir().unwrap();
    let project_root = dunce::canonicalize(project.path()).unwrap();
    let manifest = project_root.join("foundry.toml");
    fs::write(&manifest, "[profile.default]\nsrc = \"old-src\"\n").unwrap();
    fs::create_dir_all(project_root.join("old-src")).unwrap();
    fs::create_dir_all(project_root.join("new-src")).unwrap();
    fs::write(project_root.join("old-src/Old.sol"), "contract OldContract {}\n").unwrap();
    fs::write(project_root.join("new-src/New.sol"), "contract NewContract {}\n").unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(&project_root, empty_path.path(), &["lsp", "--stdio"]);
    let initialize = request(
        &client.runtime,
        client.server.initialize(InitializeParams {
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Url::from_directory_path(&project_root).unwrap(),
                name: "fixture".into(),
            }]),
            ..InitializeParams::default()
        }),
    );
    assert!(initialize.capabilities.workspace_symbol_provider.is_some());
    client.server.initialized(InitializedParams {}).unwrap();
    client.wait_for_log_message();
    wait_for_workspace_symbols(&mut client, "OldContract", "NewContract");

    fs::write(&manifest, "[profile.default]\nsrc = \"new-src\"\n").unwrap();
    client
        .server
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: Url::from_file_path(&manifest).unwrap(),
                typ: FileChangeType::CHANGED,
            }],
        })
        .unwrap();
    wait_for_workspace_symbols(&mut client, "NewContract", "OldContract");
    client.shutdown();
}

#[cfg(unix)]
#[test]
fn lsp_preserves_aliased_workspace_root() {
    let project = tempfile::tempdir().unwrap();
    let project_root = dunce::canonicalize(project.path()).unwrap();
    fs::write(project_root.join("foundry.toml"), "[profile.default]\nsrc = \"src\"\n").unwrap();
    fs::create_dir_all(project_root.join("src")).unwrap();
    fs::write(project_root.join("src/Alias.sol"), "contract AliasContract {}\n").unwrap();

    let alias_parent = tempfile::tempdir().unwrap();
    let alias_root = alias_parent.path().join("project-alias");
    symlink(&project_root, &alias_root).unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(&alias_root, empty_path.path(), &["lsp", "--stdio"]);
    let initialize = request(
        &client.runtime,
        client.server.initialize(InitializeParams {
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Url::from_directory_path(&alias_root).unwrap(),
                name: "fixture".into(),
            }]),
            ..InitializeParams::default()
        }),
    );
    assert!(initialize.capabilities.workspace_symbol_provider.is_some());
    client.server.initialized(InitializedParams {}).unwrap();
    client.wait_for_log_message();

    wait_for_workspace_symbols(&mut client, "AliasContract", "MissingContract");
    client.shutdown();
}

#[test]
fn lsp_stdio_handshake_uses_only_lsp_stdout() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    fs::write(project.path().join("foundry.toml"), "[profile.default]\nevm_version = \"cancun\"\n")
        .unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/Example.sol"), "contract Example {}\n").unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    for args in [&["lsp"][..], &["lsp", "--stdio"][..]] {
        let mut client = LspClient::spawn(project.path(), empty_path.path(), args);
        let initialize = request(
            &client.runtime,
            client.server.initialize(InitializeParams {
                capabilities: ClientCapabilities::default(),
                workspace_folders: Some(vec![WorkspaceFolder {
                    uri: Url::from_directory_path(project.path()).unwrap(),
                    name: "fixture".into(),
                }]),
                ..InitializeParams::default()
            }),
        );
        assert!(initialize.capabilities.workspace_symbol_provider.is_some());
        client.server.initialized(InitializedParams {}).unwrap();
        client.wait_for_log_message();
        client.shutdown();
    }
}

forgetest!(lsp_reports_unsaved_diagnostics_and_resolves_definition, |prj, _cmd| {
    prj.create_file("foundry.toml", "[profile.default]\nsrc = \"src\"\n");
    let saved = "contract Saved {}\n";
    prj.create_file("src/Example.sol", saved);
    let project_root = dunce::canonicalize(prj.root()).unwrap();
    let path = project_root.join("src/Example.sol");
    let uri = Url::from_file_path(&path).unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(&project_root, empty_path.path(), &["lsp", "--stdio"]);
    let initialize = request(
        &client.runtime,
        client.server.initialize(InitializeParams {
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Url::from_directory_path(&project_root).unwrap(),
                name: "fixture".into(),
            }]),
            ..InitializeParams::default()
        }),
    );
    assert!(initialize.capabilities.definition_provider.is_some());
    client.server.initialized(InitializedParams {}).unwrap();
    client.wait_for_log_message();
    client
        .server
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "solidity".into(),
                version: 1,
                text: "contract Example { function read() public { missing(); } }".into(),
            },
        })
        .unwrap();
    client.wait_for_diagnostics(&uri, |params| {
        params.diagnostics.iter().any(|diag| diag.severity == Some(DiagnosticSeverity::ERROR))
    });

    let corrected = r#"// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;
contract Example {
    function read() public pure returns (uint256) { return 42; }
    function caller() public pure returns (uint256) {
        return read();
    }
}
"#;
    client
        .server
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri: uri.clone(), version: 2 },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: corrected.into(),
            }],
        })
        .unwrap();
    client.wait_for_diagnostics(&uri, |params| params.diagnostics.is_empty());
    let definition = request(
        &client.runtime,
        client.server.definition(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(5, 15),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }),
    )
    .expect("the unsaved function call should resolve");
    let locations = match definition {
        GotoDefinitionResponse::Scalar(location) => vec![location],
        GotoDefinitionResponse::Array(locations) => locations,
        GotoDefinitionResponse::Link(links) => links
            .into_iter()
            .map(|link| Location { uri: link.target_uri, range: link.target_selection_range })
            .collect(),
    };
    assert_eq!(
        locations,
        vec![Location { uri, range: Range::new(Position::new(3, 13), Position::new(3, 17)) }]
    );
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
    client.shutdown();
});

forgetest!(lsp_formats_unsaved_document_with_nested_foundry_config, |prj, _cmd| {
    prj.create_file("foundry.toml", "[profile.default]\nsrc = \"src\"\n[fmt]\ntab_width = 6\n");
    prj.create_file(
        "nested/foundry.toml",
        "[profile.default]\nsrc = \"src\"\n[fmt]\ntab_width = 2\n",
    );
    let saved = "contract Saved {}\n";
    prj.create_file("nested/src/Example.sol", saved);
    let project_root = dunce::canonicalize(prj.root()).unwrap();
    let path = project_root.join("nested/src/Example.sol");
    let uri = Url::from_file_path(&path).unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(&project_root, empty_path.path(), &["lsp", "--stdio"]);
    let initialize = request(
        &client.runtime,
        client.server.initialize(InitializeParams {
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Url::from_directory_path(&project_root).unwrap(),
                name: "fixture".into(),
            }]),
            ..InitializeParams::default()
        }),
    );
    assert_eq!(initialize.capabilities.document_formatting_provider, Some(OneOf::Left(true)));
    client.server.initialized(InitializedParams {}).unwrap();
    client.wait_for_log_message();
    client
        .server
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "solidity".into(),
                version: 1,
                text: saved.into(),
            },
        })
        .unwrap();
    let unsaved =
        "contract Example{uint256 public value;function set(uint256 next) public{value=next;}}";
    client
        .server
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri: uri.clone(), version: 2 },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: unsaved.into(),
            }],
        })
        .unwrap();
    let edits = request(
        &client.runtime,
        client.server.formatting(DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: FormattingOptions { tab_size: 8, insert_spaces: true, ..Default::default() },
            work_done_progress_params: Default::default(),
        }),
    )
    .expect("Forge should format the unsaved document");
    assert_eq!(
        edits,
        vec![TextEdit {
            range: Range::new(Position::new(0, 0), Position::new(0, unsaved.len() as u32)),
            new_text: r#"contract Example {
  uint256 public value;

  function set(uint256 next) public {
    value = next;
  }
}
"#
            .into(),
        }]
    );
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
    client.shutdown();
});
