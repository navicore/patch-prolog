//! Binary entry point for `plgl` — the Language Server Protocol
//! implementation for patch-prolog. Implements `LanguageServer` over a
//! per-URI document cache and publishes parse-error diagnostics.
//!
//! Features: text sync + parse-error diagnostics, completion (built-ins
//! from `plg_shared::builtins` + stdlib + buffer predicates), and hover
//! (built-in docs + user clause heads). Goto-definition is the remaining
//! `buffer.rs` consumer — see docs/design/LSP_PORT.md.

use std::collections::HashMap;
use std::sync::RwLock;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{info, warn};

mod buffer;
mod completion;
mod diagnostics;
mod hover;

/// State for a single document.
struct DocumentState {
    /// Current buffer contents.
    content: String,
}

struct PrologLanguageServer {
    client: Client,
    documents: RwLock<HashMap<Url, DocumentState>>,
}

impl PrologLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: RwLock::new(HashMap::new()),
        }
    }

    /// Replace the cached buffer for `uri` and republish diagnostics.
    /// The diagnostic list is always replaced wholesale so stale errors
    /// clear on the first successful parse.
    async fn update_document(&self, uri: Url, content: String) {
        let diagnostics = diagnostics::compute(&content);
        if let Ok(mut docs) = self.documents.write() {
            docs.insert(uri.clone(), DocumentState { content });
        } else {
            warn!("documents lock poisoned");
        }
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for PrologLanguageServer {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        info!("plgl: initialize");
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // No `trigger_characters` — identifier typing already
                // triggers completion in most clients, and `:` only
                // appears in `:-`/`?-`, not predicate position.
                completion_provider: Some(CompletionOptions::default()),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                // goto-definition capability lands with its handler next.
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "plgl".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("plgl: initialized");
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.update_document(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // FULL sync — the editor sends the entire new buffer in change.text.
        // If a client ever sends multiple updates in one notification, the
        // LAST is the authoritative final state.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.update_document(params.text_document.uri, change.text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Ok(mut docs) = self.documents.write() {
            docs.remove(&params.text_document.uri);
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let response = self
            .documents
            .read()
            .ok()
            .and_then(|docs| {
                docs.get(uri)
                    .map(|doc| completion::compute(&doc.content, position))
            })
            .unwrap_or_default();
        Ok(Some(CompletionResponse::Array(response)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        Ok(self.documents.read().ok().and_then(|docs| {
            docs.get(uri)
                .and_then(|doc| hover::compute(&doc.content, position))
        }))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // `--version` / `-V`: print and exit before claiming stdio for the
    // protocol (lets `plgl --version` verify an install, per the plugin
    // README). Any other args are ignored — the server speaks LSP only.
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("plgl {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Log to stderr; stdout is owned by the LSP protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(PrologLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
