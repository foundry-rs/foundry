use async_lsp::{
    LanguageServer,
    lsp_types::{
        ClientCapabilities, InitializeParams, InitializedParams, Url, WorkspaceFolder,
        WorkspaceSymbolParams, WorkspaceSymbolResponse,
    },
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

use super::lsp_client::{LspClient, request};

const SYMBOL_TIMEOUT: Duration = Duration::from_secs(10);

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
        if last_names.iter().any(|name| name == "CustomContract")
            && last_names.iter().all(|name| name != "DefaultContract")
        {
            client.shutdown();
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }

    panic!("custom profile was not used for workspace indexing; observed symbols: {last_names:?}");
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
