use crate::compiler::{Compiler, ForgeCompiler};
use std::sync::Arc;
use tower_lsp::{Client, LanguageServer, lsp_types::*};

pub struct ForgeLsp {
    client: Client,
    compiler: Arc<dyn Compiler>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TextDocumentItem<'a> {
    uri: Url,
    text: &'a str,
    version: Option<i32>,
}

impl ForgeLsp {
    pub fn new(client: Client) -> Self {
        let compiler = Arc::new(ForgeCompiler) as Arc<dyn Compiler>;
        Self { client, compiler }
    }

    async fn on_change<'a>(&self, params: TextDocumentItem<'a>) {
        let uri = params.uri.clone();
        let version = params.version;

        let (lint_result, build_result) = tokio::join!(
            self.compiler.get_lint_diagnostics(&uri),
            self.compiler.get_build_diagnostics(&uri)
        );

        let mut all_diagnostics = vec![];

        match lint_result {
            Ok(mut lints) => {
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Found {} linting diagnostics", lints.len()),
                    )
                    .await;
                all_diagnostics.append(&mut lints);
            }
            Err(e) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Forge linting diagnostics failed: {e}"),
                    )
                    .await;
            }
        }

        match build_result {
            Ok(mut builds) => {
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Found {} build diagnostics", builds.len()),
                    )
                    .await;
                all_diagnostics.append(&mut builds);
            }
            Err(e) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Forge build diagnostics failed: {e}"),
                    )
                    .await;
            }
        }

        self.client.publish_diagnostics(uri, all_diagnostics, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ForgeLsp {
    async fn initialize(
        &self,
        _: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "forge lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "lsp server initialized!").await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        self.client.log_message(MessageType::INFO, "lsp server shutting down").await;
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "file opened").await;

        self.on_change(TextDocumentItem {
            uri: params.text_document.uri,
            text: &params.text_document.text,
            version: Some(params.text_document.version),
        })
        .await
    }

    async fn did_change(&self, _params: DidChangeTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "file changed").await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "file saved - running diagnostics").await;

        // Run diagnostics on save, regardless of whether text is provided
        // If text is provided, use it; otherwise read from file system
        let text_content = if let Some(text) = params.text {
            text
        } else {
            // Read the file from disk since many LSP clients don't send text on save
            match std::fs::read_to_string(params.text_document.uri.path()) {
                Ok(content) => content,
                Err(e) => {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("Failed to read file on save: {e}"),
                        )
                        .await;
                    return;
                }
            }
        };

        let item =
            TextDocumentItem { uri: params.text_document.uri, text: &text_content, version: None };

        // Always run diagnostics on save to reflect the current file state
        self.on_change(item).await;
        _ = self.client.semantic_tokens_refresh().await;
    }

    async fn did_close(&self, _: DidCloseTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "file closed").await;
    }

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
        self.client.log_message(MessageType::INFO, "configuration changed!").await;
    }

    async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {
        self.client.log_message(MessageType::INFO, "workspace folders changed!").await;
    }

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        self.client.log_message(MessageType::INFO, "watched files have changed!").await;
    }

    async fn execute_command(
        &self,
        _: ExecuteCommandParams,
    ) -> tower_lsp::jsonrpc::Result<Option<serde_json::Value>> {
        self.client.log_message(MessageType::INFO, "command executed!").await;

        match self.client.apply_edit(WorkspaceEdit::default()).await {
            Ok(res) if res.applied => self.client.log_message(MessageType::INFO, "applied").await,
            Ok(_) => self.client.log_message(MessageType::INFO, "rejected").await,
            Err(err) => self.client.log_message(MessageType::ERROR, err).await,
        }
        Ok(None)
    }
}
