use super::lsp_client::{LspClient, request};
use async_lsp::{
    LanguageServer,
    lsp_types::{
        ClientCapabilities, DiagnosticSeverity, DidChangeTextDocumentParams,
        DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, FileChangeType, FileEvent, FormattingOptions,
        GotoDefinitionParams, GotoDefinitionResponse, InitializeParams, InitializedParams,
        Location, OneOf, Position, Range, ReferenceContext, ReferenceParams, RenameParams,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        TextDocumentPositionParams, TextEdit, Url, VersionedTextDocumentIdentifier, WorkspaceEdit,
        WorkspaceFolder, WorkspaceSymbolParams, WorkspaceSymbolResponse,
    },
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use foundry_test_utils::snapbox::{IntoData, data::DataFormat};
#[cfg(unix)]
use rexpect::{Encoding, process::wait::WaitStatus, reader::Options, spawn_with_options};
#[cfg(unix)]
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

const SYMBOL_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(unix)]
forgetest!(lsp_vscode_opens_current_project_with_bundled_extension, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    let executables = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(prj.root()).unwrap();
    let forge = executables.path().join("standalone forge");
    fs::hard_link(env!("CARGO_BIN_EXE_forge"), &forge).unwrap();
    let forge = dunce::canonicalize(forge).unwrap();
    let code = executables.path().join("code");
    fs::write(
        &code,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$FORGE_LSP_TEST_ARGS"
printf '%s\n' "$FOUNDRY_PROFILE" "$FOUNDRY_LSP_FORGE" > "$FORGE_LSP_TEST_PROFILE"
printf '%s\n' "${VSCODE_APPDATA-unset}" "${VSCODE_EXTENSIONS-unset}" "${VSCODE_PORTABLE-unset}" "${VSCODE_IPC_HOOK_CLI-unset}" > "$FORGE_LSP_TEST_ENV"
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
    cmd.env("XDG_DATA_HOME", home.path().join("data"));
    cmd.env("PATH", executables.path());
    cmd.env("FORGE_LSP_TEST_ARGS", &captured_args);
    cmd.env("FORGE_LSP_TEST_PROFILE", &captured_profile);
    cmd.env("FORGE_LSP_TEST_ENV", &captured_env);
    cmd.env("VSCODE_APPDATA", "/stale/appdata");
    cmd.env("VSCODE_EXTENSIONS", "/stale/extensions");
    cmd.env("VSCODE_PORTABLE", "/stale/portable");
    cmd.env("VSCODE_IPC_HOOK_CLI", "/stale/ipc-hook");
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
    for (name, expected) in [
        ("LICENSE-MIT", include_bytes!("../../../../LICENSE-MIT").as_slice()),
        ("LICENSE-APACHE", include_bytes!("../../../../LICENSE-APACHE").as_slice()),
    ] {
        assert_eq!(fs::read(Path::new(extension).join(name)).unwrap(), expected, "{name}");
    }
    let user_data = arguments
        .windows(2)
        .find(|pair| pair[0] == "--user-data-dir")
        .map(|pair| Path::new(pair[1]))
        .expect("VS Code must use a dedicated profile");
    let session = user_data.parent().unwrap();
    let durable_session = dunce::canonicalize(session).unwrap();
    let data_home = if cfg!(target_os = "macos") {
        home.path().join("Library/Application Support")
    } else {
        home.path().join("data")
    };
    assert_eq!(
        durable_session.parent().unwrap(),
        dunce::canonicalize(data_home.join("foundry/lsp/vscode")).unwrap()
    );
    assert!(fs::symlink_metadata(session).unwrap().file_type().is_symlink());
    let extensions = arguments
        .windows(2)
        .find(|pair| pair[0] == "--extensions-dir")
        .map(|pair| Path::new(pair[1]))
        .expect("VS Code must preserve installed extensions in the managed profile");
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
    assert_eq!(
        fs::read_to_string(&captured_env).unwrap(),
        format!("unset\nunset\n{}\nunset\n", user_data.parent().unwrap().display())
    );
    assert!(!project.join(".vscode").exists());

    // Cache and temporary-link cleanup must preserve the managed profile's editor state.
    let settings = user_data.join("User/settings.json");
    let custom_settings = "{\"editor.fontSize\":17}\n";
    fs::write(&settings, custom_settings).unwrap();
    let history = user_data.join("User/History/entry.sol");
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    fs::write(&history, "contract PreviousVersion {}\n").unwrap();
    let installed_extension = extensions.join("installed-extension.txt");
    fs::write(&installed_extension, "user-installed extension\n").unwrap();
    cmd.forge_fuse();
    cmd.env("HOME", home.path());
    cmd.env("XDG_DATA_HOME", home.path().join("data"));
    cmd.args(["cache", "clean", "all"]).assert_empty_stdout();
    assert!(!Path::new(extension).exists());
    fs::remove_file(session).unwrap();
    assert_eq!(
        fs::read_to_string(durable_session.join("user-data/User/settings.json")).unwrap(),
        custom_settings
    );

    // A terminal needs only `forge lsp`, and reopening restores the short profile link.
    let mut terminal = Command::new(&forge);
    terminal
        .current_dir(&project)
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path().join("data"))
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
    assert_eq!(fs::read_to_string(history).unwrap(), "contract PreviousVersion {}\n");
    assert_eq!(fs::read_to_string(installed_extension).unwrap(), "user-installed extension\n");
    assert_eq!(dunce::canonicalize(session).unwrap(), durable_session);
    assert!(Path::new(extension).join("out/extension.js").is_file());
});

#[cfg(unix)]
forgetest!(lsp_vscode_sessions_follow_resolved_editor, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    let editors = prj.root().join("editors");
    let captured_session = editors.join("session");
    let stable = editors.join("stable/code");
    let insiders = editors.join("insiders/code");
    for code in [&stable, &insiders] {
        fs::create_dir_all(code.parent().unwrap()).unwrap();
        fs::write(
            code,
            "#!/bin/sh\nprintf '%s' \"$VSCODE_PORTABLE\" > \"$FORGE_LSP_TEST_SESSION\"\n",
        )
        .unwrap();
        fs::set_permissions(code, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut launch = |code: &Path, search_path: &Path| {
        cmd.forge_fuse();
        cmd.env("HOME", home.path());
        cmd.env("XDG_DATA_HOME", home.path().join("data"));
        cmd.env("PATH", search_path);
        cmd.env("FORGE_LSP_TEST_SESSION", &captured_session);
        cmd.args(["lsp", "--code-path"]).arg(code).assert_empty_stdout();
        dunce::canonicalize(fs::read_to_string(&captured_session).unwrap()).unwrap()
    };
    let stable_path = stable.parent().unwrap();
    let insiders_path = insiders.parent().unwrap();
    let stable_session = launch(&stable, stable_path);
    let insiders_session = launch(&insiders, insiders_path);
    assert_ne!(stable_session, insiders_session);
    assert_eq!(launch(&stable, insiders_path), stable_session);
    assert_eq!(launch(Path::new("code"), stable_path), stable_session);
    assert_eq!(launch(Path::new("code"), insiders_path), insiders_session);
    assert_eq!(launch(Path::new("editors/stable/code"), insiders_path), stable_session);
    assert!(!prj.root().join(".vscode").exists());
});

#[cfg(unix)]
forgetest!(lsp_vscode_preserves_symlink_launchers, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    let dispatcher = prj.root().join("dispatcher");
    let captured_launcher = prj.root().join("launcher");
    let captured_session = prj.root().join("session");
    fs::write(
        &dispatcher,
        r#"#!/bin/sh
printf '%s' "$0" > "$FORGE_LSP_TEST_LAUNCHER"
printf '%s' "$VSCODE_PORTABLE" > "$FORGE_LSP_TEST_SESSION"
"#,
    )
    .unwrap();
    fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755)).unwrap();
    let stable = prj.root().join("code");
    let insiders = prj.root().join("code-insiders");
    symlink(&dispatcher, &stable).unwrap();
    symlink(&dispatcher, &insiders).unwrap();

    let mut launch = |code: &Path| {
        cmd.forge_fuse();
        cmd.env("HOME", home.path());
        cmd.env("XDG_DATA_HOME", home.path().join("data"));
        cmd.env("FORGE_LSP_TEST_LAUNCHER", &captured_launcher);
        cmd.env("FORGE_LSP_TEST_SESSION", &captured_session);
        cmd.args(["lsp", "--code-path"]).arg(code).assert_empty_stdout();
        assert_eq!(fs::read_to_string(&captured_launcher).unwrap(), code.to_str().unwrap());
        dunce::canonicalize(fs::read_to_string(&captured_session).unwrap()).unwrap()
    };
    let stable_session = launch(&stable);
    let insiders_session = launch(&insiders);
    assert_ne!(stable_session, insiders_session);
    assert_eq!(launch(&stable), stable_session);

    // Repointing the same launcher must not inherit the previous editor's profile.
    let other_dispatcher = prj.root().join("other dispatcher");
    fs::copy(&dispatcher, &other_dispatcher).unwrap();
    fs::remove_file(&stable).unwrap();
    symlink(&other_dispatcher, &stable).unwrap();
    assert_ne!(launch(&stable), stable_session);
    assert_eq!(launch(&insiders), insiders_session);
    assert!(stable_session.join("user-data/User/settings.json").is_file());
});

forgetest!(lsp_stdio_rejects_editor_launch_options, |_prj, cmd| {
    cmd.args(["lsp", "--stdio", "--vscode"]).assert_code(2).stdout_eq(str![""]).stderr_eq(str![[
        r#"
error: the argument '--stdio' cannot be used with '--vscode'

Usage: forge[..] lsp --stdio [PATH]

For more information, try '--help'.

"#
    ]]);
});

#[cfg(unix)]
forgetest!(lsp_code_path_reports_missing_editor, |prj, cmd| {
    let home = tempfile::tempdir().unwrap();
    cmd.env("HOME", home.path());
    cmd.env("XDG_DATA_HOME", home.path().join("data"));
    cmd.args(["lsp", "--code-path"]).arg(prj.root().join("missing-vscode"));
    cmd.assert_failure().stdout_eq(str![""]).stderr_eq(str![[r#"
Error: Could not launch VS Code using [..]/missing-vscode. Install VS Code and its `code` command, or pass --code-path <PATH> to the VS Code CLI. Use `forge lsp --stdio` for another editor.

Context:
- cannot find binary path

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

forgetest!(lsp_indexes_closed_tests_and_scripts, |prj, _cmd| {
    // Test both default directories and directories resolved from the selected profile.
    for (profile, sources, tests, scripts) in
        [("default", "src", "test", "script"), ("custom", "contracts", "checks", "deployments")]
    {
        prj.create_file(
            "foundry.toml",
            "[profile.default]\n[profile.custom]\nsrc = \"contracts\"\ntest = \"checks\"\nscript = \"deployments\"\n",
        );
        let files = [
            (
                format!("{sources}/Counter.sol"),
                "// SPDX-License-Identifier: MIT\npragma solidity >=0.8.0;\ncontract CoverageCounter {\n    function increment() public {}\n}\n".to_owned(),
            ),
            (
                format!("{tests}/Counter.t.sol"),
                format!("// SPDX-License-Identifier: MIT\npragma solidity >=0.8.0;\nimport {{CoverageCounter}} from \"../{sources}/Counter.sol\";\ncontract CoverageTest {{\n    function check(CoverageCounter counter) public {{ counter.increment(); }}\n}}\n"),
            ),
            (
                format!("{scripts}/Counter.s.sol"),
                format!("// SPDX-License-Identifier: MIT\npragma solidity >=0.8.0;\nimport {{CoverageCounter}} from \"../{sources}/Counter.sol\";\ncontract CoverageScript {{\n    function run(CoverageCounter counter) public {{ counter.increment(); }}\n}}\n"),
            ),
        ];
        for (path, source) in &files {
            prj.create_file(path, source);
        }
        let root = dunce::canonicalize(prj.root()).unwrap();
        let locations: Vec<_> = files
            .iter()
            .map(|(path, source)| {
                let (line, column) = source
                    .lines()
                    .enumerate()
                    .find_map(|(line, text)| text.find("increment").map(|column| (line, column)))
                    .unwrap();
                Location {
                    uri: Url::from_file_path(root.join(path)).unwrap(),
                    range: Range::new(
                        Position::new(line as u32, column as u32),
                        Position::new(line as u32, (column + "increment".len()) as u32),
                    ),
                }
            })
            .collect();
        let empty_path = tempfile::tempdir().unwrap();
        let mut client =
            LspClient::spawn(&root, empty_path.path(), &["lsp", "--stdio", "--profile", profile]);
        request(
            &client.runtime,
            client.server.initialize(InitializeParams {
                capabilities: ClientCapabilities::default(),
                workspace_folders: Some(vec![WorkspaceFolder {
                    uri: Url::from_directory_path(&root).unwrap(),
                    name: "fixture".into(),
                }]),
                ..InitializeParams::default()
            }),
        );
        client.server.initialized(InitializedParams {}).unwrap();
        client.wait_for_log_message();
        client
            .server
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: locations[0].uri.clone(),
                    language_id: "solidity".into(),
                    version: 1,
                    text: files[0].1.clone(),
                },
            })
            .unwrap();

        for state in ["never opened", "opened", "closed again"] {
            for (location, (_, source)) in locations[1..].iter().zip(&files[1..]) {
                match state {
                    "opened" => client
                        .server
                        .did_open(DidOpenTextDocumentParams {
                            text_document: TextDocumentItem {
                                uri: location.uri.clone(),
                                language_id: "solidity".into(),
                                version: 1,
                                text: source.clone(),
                            },
                        })
                        .unwrap(),
                    "closed again" => client
                        .server
                        .did_close(DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier { uri: location.uri.clone() },
                        })
                        .unwrap(),
                    _ => {}
                }
            }
            let position = TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: locations[0].uri.clone() },
                position: locations[0].range.start,
            };
            // References and rename wait for the latest analysis, including didClose.
            let mut references = request(
                &client.runtime,
                client.server.references(ReferenceParams {
                    text_document_position: position.clone(),
                    context: ReferenceContext { include_declaration: true },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                }),
            )
            .unwrap();
            let mut expected = locations.clone();
            references.sort_by(|a, b| a.uri.cmp(&b.uri));
            expected.sort_by(|a, b| a.uri.cmp(&b.uri));
            assert_eq!(references, expected, "{profile}: {state}");

            let edits = request(
                &client.runtime,
                client.server.rename(RenameParams {
                    text_document_position: position,
                    new_name: "increase".into(),
                    work_done_progress_params: Default::default(),
                }),
            )
            .unwrap();
            assert_eq!(
                edits,
                WorkspaceEdit {
                    changes: Some(
                        locations
                            .iter()
                            .map(|location| (
                                location.uri.clone(),
                                vec![TextEdit {
                                    range: location.range,
                                    new_text: "increase".into()
                                }],
                            ))
                            .collect()
                    ),
                    ..WorkspaceEdit::default()
                },
                "{profile}: {state}"
            );

            let symbols = request(
                &client.runtime,
                client.server.symbol(WorkspaceSymbolParams {
                    query: "Coverage".into(),
                    ..WorkspaceSymbolParams::default()
                }),
            )
            .unwrap();
            let mut names: Vec<_> = match symbols {
                WorkspaceSymbolResponse::Flat(symbols) => {
                    symbols.into_iter().map(|symbol| symbol.name).collect()
                }
                WorkspaceSymbolResponse::Nested(symbols) => {
                    symbols.into_iter().map(|symbol| symbol.name).collect()
                }
            };
            names.sort();
            assert_eq!(
                names,
                ["CoverageCounter", "CoverageScript", "CoverageTest"],
                "{profile}: {state}"
            );
        }
        client.shutdown();
    }
});

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
