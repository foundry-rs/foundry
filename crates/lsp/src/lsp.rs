use crate::{
    analyzer::Analyzer,
    runner::{ForgeRunner, Runner},
};
use foundry_common::version::SHORT_VERSION;
use std::sync::{Arc, Mutex};
use tower_lsp::{Client, LanguageServer, jsonrpc, lsp_types::*};

pub struct ForgeLsp {
    client: Client,
    compiler: Arc<dyn Runner>,
    analyzer: Arc<Mutex<Analyzer>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TextDocumentItem<'a> {
    uri: Url,
    text: &'a str,
    version: Option<i32>,
}

impl ForgeLsp {
    pub fn new(client: Client, analyzer: Analyzer) -> Self {
        let compiler = Arc::new(ForgeRunner) as Arc<dyn Runner>;
        let analyzer = Arc::new(Mutex::new(analyzer));
        Self { client, compiler, analyzer }
    }

    async fn on_change<'a>(&self, params: TextDocumentItem<'a>) {
        let update_result = { self.analyzer.lock().unwrap().analyze() };
        if let Err(e) = update_result {
            self.client
                .log_message(MessageType::ERROR, format!("Failed to update project: {e}"))
                .await;
        }

        let uri = params.uri.clone();
        let version = params.version;
        let mut all_diagnostics = vec![];

        match self.compiler.get_build_diagnostics(&uri).await {
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
        let lint_diagnostics = self.analyzer.lock().unwrap().get_lint_diagnostics(&uri);
        if let Some(mut lints) = lint_diagnostics {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Found {} linting diagnostics", lints.len()),
                )
                .await;
            all_diagnostics.append(&mut lints);
        } else {
            self.client
                .log_message(MessageType::WARNING, format!("Forge linting diagnostics failed"))
                .await;
        }

        self.client.publish_diagnostics(uri, all_diagnostics, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ForgeLsp {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "forge lsp".to_string(),
                version: Some(SHORT_VERSION.to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "lsp server initialized!").await;
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
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
    ) -> jsonrpc::Result<Option<serde_json::Value>> {
        self.client.log_message(MessageType::INFO, "command executed!").await;

        match self.client.apply_edit(WorkspaceEdit::default()).await {
            Ok(res) if res.applied => self.client.log_message(MessageType::INFO, "applied").await,
            Ok(_) => self.client.log_message(MessageType::INFO, "rejected").await,
            Err(err) => self.client.log_message(MessageType::ERROR, err).await,
        }
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let location = self.analyzer.lock().unwrap().goto_definition(&uri, position);

        Ok(location.map(GotoDefinitionResponse::Scalar))
    }
}
