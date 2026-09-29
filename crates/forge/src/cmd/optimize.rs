//! `forge optimize`: rewrites the project's functions with a model through Solar, paying for each
//! model request from the user's Tempo account.
//!
//! The command compiles the project's sources, those in `src` unless paths are given, with Solar's
//! `llm-optimize` pass. The pass offers each eligible function to the model, tests every candidate
//! the model sends back against the original, and keeps a rewrite only when it is cheaper for the
//! objective: runtime gas, or bytecode size with `--size`. Kept rewrites land in a directory,
//! `llm-optimize` in the cache directory unless `--rewrites` names another, which later runs reuse
//! without asking again; `--replay` compiles with those rewrites alone, without a model. Solar's
//! artifacts, the ABI and bytecode of every contract, go to `optimize/combined.json` in the
//! artifacts directory.
//!
//! Model requests go to `--endpoint`, a gateway that charges for each request with the Machine
//! Payments Protocol: it answers an unpaid request with HTTP 402 and a payment challenge, which
//! Forge pays from the Tempo account `cast tempo login` authorized, the one `forge script` deploys
//! with, before sending the request again. No provider API key is read or sent.

use clap::{Parser, ValueHint};
use eyre::{Result, eyre};
use foundry_cli::{
    opts::{BuildOpts, configure_pcx_all_sources},
    utils::{FoundryPathExt, LoadConfig},
};
use foundry_common::provider::mpp::transport::LazyAccountsProvider;
use foundry_compilers::{
    Project,
    utils::{SOLC_EXTENSIONS, source_files},
};
use foundry_config::Config;
use mpp::client::{Fetch, HttpError, PaymentProvider};
use reqwest::{Client, Request, RequestBuilder, Response};
use solar::{interface::Session, sema::ParsingContext};
use solar_cli::{
    CompileOpts, UnstableOpts,
    config::{CompilerOutput, EvmVersion, LlmEffort, LlmOptimizeMode, OptimizationMode},
    llm::{ChatTransport, TransportError, set_transport},
    run_compiler_with_sources,
};
use std::{path::PathBuf, pin::Pin, sync::Arc};
use url::Url;

/// CLI arguments for `forge optimize`.
#[derive(Clone, Debug, Parser)]
pub struct OptimizeArgs {
    /// Source files or directories to optimize. Defaults to the project's sources.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The model to ask, as `PROVIDER/MODEL` with provider `anthropic`, `openai-chat`, or
    /// `opencode`.
    #[arg(long, value_name = "MODEL", required_unless_present = "replay")]
    model: Option<String>,

    /// Base URL of the model's API at a gateway that charges with the Machine Payments Protocol,
    /// such as `https://gateway.example/anthropic/v1`.
    ///
    /// Forge pays each request from the Tempo account `cast tempo login` authorized.
    #[arg(long, value_name = "URL", required_unless_present = "replay")]
    endpoint: Option<Url>,

    /// How much the model reasons before it replies.
    #[arg(long, value_enum, value_name = "EFFORT")]
    effort: Option<LlmEffort>,

    /// Candidates to ask for per function [default: 6].
    #[arg(long, value_name = "N")]
    rounds: Option<usize>,

    /// Optimize for bytecode size instead of runtime gas.
    #[arg(long)]
    size: bool,

    /// Directory of kept rewrites, which later runs reuse [default: <CACHE_PATH>/llm-optimize].
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "DIR")]
    rewrites: Option<PathBuf>,

    /// Compile with the kept rewrites alone, checking each again, without asking a model.
    #[arg(long, conflicts_with_all = ["model", "endpoint", "effort", "rounds"])]
    replay: bool,

    #[command(flatten)]
    build: BuildOpts,
}

foundry_config::impl_figment_convert!(OptimizeArgs, build);

impl OptimizeArgs {
    pub fn run(self) -> Result<()> {
        let payer =
            self.endpoint.as_ref().map(|endpoint| LazyAccountsProvider::new(endpoint.to_string()));
        self.optimize(payer)
    }

    /// Compiles the sources, paying for model requests with `payer`.
    fn optimize<P: PaymentProvider + 'static>(self, payer: Option<P>) -> Result<()> {
        let config = self.load_config()?;
        let project = config.ephemeral_project()?;
        let targets = self.targets(&project)?;
        if targets.is_empty() {
            sh_status!("nothing to optimize")?;
            return Ok(());
        }

        let rewrites = self
            .rewrites
            .clone()
            .unwrap_or_else(|| config.root.join(&config.cache_path).join("llm-optimize"));
        let out = config.root.join(&config.out).join("optimize");
        foundry_common::fs::create_dir_all(&out)?;
        let opts = self.solar_opts(&config, rewrites.clone(), out.clone())?;
        let transport = match payer {
            Some(payer) => {
                let transport = PaidTransport { client: Client::builder().build()?, payer };
                Some(Arc::new(transport) as Arc<dyn ChatTransport>)
            }
            None => None,
        };
        // Solar's own diagnostics, like the linter's, without the compiler-internal note on where
        // each diagnostic was created.
        let mut sess = Session::builder().opts(opts).with_stderr_emitter().build();
        sess.dcx.set_flags_mut(|flags| flags.track_diagnostics = false);
        set_transport(transport);
        let compiled =
            run_compiler_with_sources(sess, |pcx| load_sources(pcx, &config, &project, &targets));
        set_transport(None);
        if compiled.is_err() {
            return Err(eyre!("Solar could not compile the sources"));
        }

        sh_status!("Kept rewrites are in {}", rewrites.display())?;
        sh_println!("{}", out.join("combined.json").display())?;
        Ok(())
    }

    /// The Solidity files to optimize: those `paths` names, or the project's sources.
    fn targets(&self, project: &Project) -> Result<Vec<PathBuf>> {
        if self.paths.is_empty() {
            return Ok(source_files(&project.paths.sources, SOLC_EXTENSIONS));
        }
        let mut targets = Vec::with_capacity(self.paths.len());
        for path in &self.paths {
            if path.is_dir() {
                targets.extend(source_files(path, SOLC_EXTENSIONS));
            } else if path.is_sol() {
                targets.push(path.clone());
            } else {
                sh_warn!("ignoring {}, which is not a Solidity file", path.display())?;
            }
        }
        Ok(targets)
    }

    /// Solar's options: the project's EVM version and optimizer runs, the objective, and the
    /// `llm-optimize` mode, writing rewrites to `rewrites` and artifacts to `out`.
    fn solar_opts(&self, config: &Config, rewrites: PathBuf, out: PathBuf) -> Result<CompileOpts> {
        let evm_version =
            config.evm_version.to_string().parse::<EvmVersion>().map_err(|_| {
                eyre!("Solar does not support EVM version `{}`", config.evm_version)
            })?;
        let llm_optimize =
            if self.replay { LlmOptimizeMode::Replay } else { LlmOptimizeMode::Live };
        Ok(CompileOpts {
            evm_version,
            optimization: if self.size { OptimizationMode::Size } else { OptimizationMode::Gas },
            optimizer_runs: config.optimizer_runs.map(|runs| runs as u64),
            out_dir: Some(out),
            emit: vec![CompilerOutput::Abi, CompilerOutput::Bin, CompilerOutput::BinRuntime],
            unstable: UnstableOpts {
                llm_optimize: Some(llm_optimize),
                llm_cache: Some(rewrites),
                llm_model: self.model.clone(),
                llm_endpoint: self.endpoint.as_ref().map(Url::to_string),
                llm_effort: self.effort,
                llm_rounds: self.rounds,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

/// Adds the sources Solar compiles among `targets` and their imports to `pcx`, with the project's
/// remappings and include paths, as `forge lint` does.
fn load_sources(
    pcx: &mut ParsingContext<'_>,
    config: &Config,
    project: &Project,
    targets: &[PathBuf],
) -> solar::interface::Result {
    match configure_pcx_all_sources(pcx, config, Some(project), Some(targets)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(pcx
            .sess
            .dcx
            .err("no sources Solar can compile")
            .note("Solar compiles Solidity 0.8.0 and later")
            .emit()),
        Err(error) => Err(pcx.sess.dcx.err(error.to_string()).emit()),
    }
}

/// Sends Solar's model requests, paying each HTTP 402 challenge of the endpoint with `payer`.
struct PaidTransport<P> {
    client: Client,
    payer: P,
}

impl<P: PaymentProvider + 'static> ChatTransport for PaidTransport<P> {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, TransportError>> + Send + '_>> {
        Box::pin(async move {
            RequestBuilder::from_parts(self.client.clone(), request)
                .send_with_payment(&self.payer)
                .await
                .map_err(|error| {
                    let transient = matches!(
                        &error,
                        HttpError::Request(error) if error.is_connect() || error.is_timeout()
                    );
                    if transient {
                        TransportError::transient(error.to_string())
                    } else {
                        TransportError::permanent(error.to_string())
                    }
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use mpp::{
        MppError,
        protocol::core::{
            Base64UrlJson, PaymentChallenge, PaymentCredential, PaymentPayload,
            format_www_authenticate,
        },
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{net::TcpListener, runtime::Runtime};

    /// A function the model is offered: `sumBelow` stays out of line with two callers.
    const TRIANGLE: &str = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Triangle {
    function triangle(uint256 n) external pure returns (uint256) {
        return sumBelow(n);
    }

    function triangles(uint256 a, uint256 b) external pure returns (uint256) {
        unchecked {
            return sumBelow(a) + sumBelow(b);
        }
    }

    function sumBelow(uint256 n) internal pure returns (uint256 s) {
        unchecked {
            for (uint256 i; i < n; ++i) {
                s += i;
            }
        }
    }
}
"#;

    /// A streamed Messages reply with nothing cheaper to offer.
    const NO_IMPROVEMENT: &str = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":5,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"NO_IMPROVEMENT"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}

event: message_stop
data: {"type":"message_stop"}

"#;

    /// Pays every Tempo charge with a credential the gateway takes, counting the payments.
    #[derive(Clone, Default)]
    struct Payer(Arc<AtomicUsize>);

    impl PaymentProvider for Payer {
        fn supports(&self, method: &str, intent: &str) -> bool {
            method == "tempo" && intent == "charge"
        }

        async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(PaymentCredential::new(challenge.to_echo(), PaymentPayload::hash("0x01")))
        }
    }

    /// A gateway that answers a request without a credential with a Tempo charge challenge, and
    /// one with a credential with `NO_IMPROVEMENT`. Returns its API base URL and the headers of
    /// the requests it saw.
    async fn gateway() -> (Url, Arc<Mutex<Vec<HeaderMap>>>) {
        let request = Base64UrlJson::from_value(&serde_json::json!({"amount": "1000"})).unwrap();
        let challenge = PaymentChallenge::new("test", "gateway.test", "tempo", "charge", request);
        let challenge = format_www_authenticate(&challenge).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let messages = move |headers: HeaderMap| {
            let paid = headers.contains_key(header::AUTHORIZATION);
            log.lock().unwrap().push(headers);
            let challenge = challenge.clone();
            async move {
                if paid {
                    ([(header::CONTENT_TYPE, "text/event-stream")], NO_IMPROVEMENT).into_response()
                } else {
                    (StatusCode::PAYMENT_REQUIRED, [(header::WWW_AUTHENTICATE, challenge)])
                        .into_response()
                }
            }
        };
        let app = Router::new().route("/v1/messages", post(messages));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url.parse().unwrap(), seen)
    }

    /// Every model request is challenged, paid, and sent again, without a provider key.
    #[test]
    fn pays_the_gateway() {
        foundry_cli::utils::install_crypto_provider();
        let runtime = Runtime::new().unwrap();
        let (endpoint, seen) = runtime.block_on(gateway());
        let root = tempfile::tempdir().unwrap();
        foundry_common::fs::create_dir_all(root.path().join("src")).unwrap();
        foundry_common::fs::write(root.path().join("src/Triangle.sol"), TRIANGLE).unwrap();
        let args = OptimizeArgs::parse_from([
            "optimize",
            "--root",
            root.path().to_str().unwrap(),
            "--model",
            "anthropic/test-model",
            "--endpoint",
            endpoint.as_str(),
        ]);
        let payer = Payer::default();
        args.optimize(Some(payer.clone())).unwrap();

        let payments = payer.0.load(Ordering::Relaxed);
        assert!(payments > 0);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2 * payments);
        for headers in seen.iter() {
            assert!(!headers.contains_key("x-api-key"));
        }
        assert!(root.path().join("out/optimize/combined.json").is_file());
    }
}
