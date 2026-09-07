use async_lsp::{
    LanguageServer,
    lsp_types::{
        ClientCapabilities, DidChangeWatchedFilesParams, FileChangeType, FileEvent,
        InitializeParams, InitializedParams, Url, WorkspaceFolder, WorkspaceSymbolParams,
        WorkspaceSymbolResponse,
    },
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use super::lsp_client::{LspClient, request};

const SYMBOL_TIMEOUT: Duration = Duration::from_secs(10);

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
