//! Support for handling/identifying selectors.

#![allow(missing_docs)]

use crate::{abi::abi_decode_calldata, provider::runtime_transport::RuntimeTransportBuilder};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{B256, Selector, map::HashMap};
use eyre::Context;
use itertools::Itertools;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

const BASE_URL: &str = "https://api.4byte.sourcify.dev";
const SELECTOR_LOOKUP_URL: &str = "https://api.4byte.sourcify.dev/signature-database/v1/lookup";
const SELECTOR_IMPORT_URL: &str = "https://api.4byte.sourcify.dev/signature-database/v1/import";

/// The selector registry uses Express's default 100 KiB JSON body limit.
const SELECTOR_IMPORT_BODY_LIMIT: usize = 100 * 1024;

/// The selector registry accepts at most this many signatures of each kind per database insert.
const SELECTOR_IMPORT_SIGNATURE_LIMIT: usize = 1000;

/// The standard request timeout for API requests.
const REQ_TIMEOUT: Duration = Duration::from_secs(15);

/// How many request can time out before we decide this is a spurious connection.
const MAX_TIMEDOUT_REQ: usize = 4usize;

/// List of signatures for a given [`SelectorKind`].
pub type OpenChainSignatures = Vec<String>;

/// A client that can request API data from OpenChain.
#[derive(Clone, Debug)]
pub struct OpenChainClient {
    inner: reqwest::Client,
    /// Whether the connection is spurious, or API is down
    spurious_connection: Arc<AtomicBool>,
    /// How many requests timed out
    timedout_requests: Arc<AtomicUsize>,
    /// Max allowed request that can time out
    max_timedout_requests: usize,
}

impl OpenChainClient {
    /// Creates a new client with default settings.
    pub fn new() -> eyre::Result<Self> {
        let inner = RuntimeTransportBuilder::new(BASE_URL.parse().unwrap())
            .with_timeout(REQ_TIMEOUT)
            .build()
            .reqwest_client()
            .wrap_err("failed to build OpenChain client")?;
        Ok(Self {
            inner,
            spurious_connection: Default::default(),
            timedout_requests: Default::default(),
            max_timedout_requests: MAX_TIMEDOUT_REQ,
        })
    }

    async fn get_text(&self, url: impl reqwest::IntoUrl + fmt::Display) -> reqwest::Result<String> {
        trace!(%url, "GET");
        self.inner
            .get(url)
            .send()
            .await
            .inspect_err(|err| self.on_reqwest_err(err))?
            .error_for_status()
            .inspect_err(|err| self.on_reqwest_err(err))?
            .text()
            .await
            .inspect_err(|err| self.on_reqwest_err(err))
    }

    /// Sends a new post request
    async fn post_json<T: Serialize + std::fmt::Debug, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &T,
    ) -> reqwest::Result<R> {
        trace!(%url, body=?serde_json::to_string(body), "POST");
        self.inner
            .post(url)
            .json(body)
            .send()
            .await
            .inspect_err(|err| self.on_reqwest_err(err))?
            .error_for_status()
            .inspect_err(|err| self.on_reqwest_err(err))?
            .json()
            .await
            .inspect_err(|err| self.on_reqwest_err(err))
    }

    fn on_reqwest_err(&self, err: &reqwest::Error) {
        fn is_connectivity_err(err: &reqwest::Error) -> bool {
            if err.is_timeout() || err.is_connect() {
                return true;
            }
            // Error HTTP codes (5xx) are considered connectivity issues and will prompt retry
            if let Some(status) = err.status() {
                let code = status.as_u16();
                if (500..600).contains(&code) {
                    return true;
                }
            }
            false
        }

        if is_connectivity_err(err) {
            warn!("spurious network detected for OpenChain");
            let previous = self.timedout_requests.fetch_add(1, Ordering::SeqCst);
            if previous + 1 >= self.max_timedout_requests {
                self.set_spurious();
            }
        }
    }

    /// Returns whether the connection was marked as spurious
    fn is_spurious(&self) -> bool {
        self.spurious_connection.load(Ordering::Relaxed)
    }

    /// Marks the connection as spurious
    fn set_spurious(&self) {
        self.spurious_connection.store(true, Ordering::Relaxed)
    }

    fn ensure_not_spurious(&self) -> eyre::Result<()> {
        if self.is_spurious() {
            eyre::bail!("Spurious connection detected");
        }
        Ok(())
    }

    /// Decodes the given function or event selector using OpenChain
    pub async fn decode_selector(
        &self,
        selector: SelectorKind,
    ) -> eyre::Result<OpenChainSignatures> {
        Ok(self.decode_selectors(&[selector]).await?.pop().unwrap())
    }

    /// Decodes the given function, error or event selectors using OpenChain.
    pub async fn decode_selectors(
        &self,
        selectors: &[SelectorKind],
    ) -> eyre::Result<Vec<OpenChainSignatures>> {
        if selectors.is_empty() {
            return Ok(vec![]);
        }

        if enabled!(tracing::Level::TRACE) {
            trace!(?selectors, "decoding selectors");
        } else {
            debug!(len = selectors.len(), "decoding selectors");
        }

        // Exit early if spurious connection.
        self.ensure_not_spurious()?;

        // Build the URL with the query string.
        let mut url: url::Url = SELECTOR_LOOKUP_URL.parse().unwrap();
        {
            let mut query = url.query_pairs_mut();
            let functions = selectors.iter().filter_map(SelectorKind::as_function);
            if functions.clone().next().is_some() {
                query.append_pair("function", &functions.format(",").to_string());
            }
            let events = selectors.iter().filter_map(SelectorKind::as_event);
            if events.clone().next().is_some() {
                query.append_pair("event", &events.format(",").to_string());
            }
            let _ = query.finish();
        }

        let text = self.get_text(url).await?;
        let SignatureResponse { ok, result } = match serde_json::from_str(&text) {
            Ok(response) => response,
            Err(err) => {
                eyre::bail!("could not decode response: {err}: {text}");
            }
        };
        if !ok {
            eyre::bail!("OpenChain returned an error: {text}");
        }

        Ok(selectors
            .iter()
            .map(|selector| {
                let signatures = match selector {
                    SelectorKind::Function(selector) | SelectorKind::Error(selector) => {
                        result.function.get(selector)
                    }
                    SelectorKind::Event(hash) => result.event.get(hash),
                };
                signatures
                    .map(Option::as_deref)
                    .unwrap_or_default()
                    .unwrap_or_default()
                    .iter()
                    .map(|sig| sig.name.clone())
                    .collect()
            })
            .collect())
    }

    /// Fetches a function signature given the selector using OpenChain
    pub async fn decode_function_selector(
        &self,
        selector: Selector,
    ) -> eyre::Result<OpenChainSignatures> {
        self.decode_selector(SelectorKind::Function(selector)).await
    }

    /// Fetches all possible signatures and attempts to abi decode the calldata
    pub async fn decode_calldata(&self, calldata: &str) -> eyre::Result<OpenChainSignatures> {
        let calldata = calldata.strip_prefix("0x").unwrap_or(calldata);
        if calldata.len() < 8 {
            eyre::bail!(
                "Calldata too short: expected at least 8 characters (excluding 0x prefix), got {}.",
                calldata.len()
            );
        }

        let mut sigs = self.decode_function_selector(calldata[..8].parse()?).await?;
        // Retain only signatures that can be decoded.
        sigs.retain(|sig| abi_decode_calldata(sig, calldata, true, true).is_ok());
        Ok(sigs)
    }

    /// Fetches an event signature given the 32 byte topic using OpenChain.
    pub async fn decode_event_topic(&self, topic: B256) -> eyre::Result<OpenChainSignatures> {
        self.decode_selector(SelectorKind::Event(topic)).await
    }

    /// Pretty print calldata and if available, fetch possible function signatures
    ///
    /// ```no_run
    /// use foundry_common::selectors::OpenChainClient;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let pretty_data = OpenChainClient::new()?
    ///     .pretty_calldata(
    ///         "0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5"
    ///             .to_string(),
    ///         false,
    ///     )
    ///     .await?;
    /// println!("{}", pretty_data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pretty_calldata(
        &self,
        calldata: impl AsRef<str>,
        offline: bool,
    ) -> eyre::Result<PossibleSigs> {
        let mut possible_info = PossibleSigs::new();
        let calldata = calldata.as_ref().trim_start_matches("0x");

        let selector =
            calldata.get(..8).ok_or_else(|| eyre::eyre!("calldata cannot be less that 4 bytes"))?;

        let sigs = if offline {
            vec![]
        } else {
            let selector = selector.parse()?;
            self.decode_function_selector(selector).await.unwrap_or_default().into_iter().collect()
        };
        let (_, data) = calldata.split_at(8);

        if !data.len().is_multiple_of(64) {
            eyre::bail!("\nInvalid calldata size");
        }

        let row_length = data.len() / 64;

        for row in 0..row_length {
            possible_info.data.push(data[64 * row..64 * (row + 1)].to_string());
        }
        if sigs.is_empty() {
            possible_info.method = SelectorOrSig::Selector(selector.to_string());
        } else {
            possible_info.method = SelectorOrSig::Sig(sigs);
        }
        Ok(possible_info)
    }

    /// uploads selectors to OpenChain using the given data
    pub async fn import_selectors(
        &self,
        data: SelectorImportData,
    ) -> eyre::Result<SelectorImportResponse> {
        self.ensure_not_spurious()?;

        let request = match data {
            SelectorImportData::Abi(abis) => {
                let functions_and_errors: OpenChainSignatures = abis
                    .iter()
                    .flat_map(|abi| {
                        abi.functions()
                            .map(|func| func.signature())
                            .chain(abi.errors().map(|error| error.signature()))
                    })
                    .unique()
                    .collect();

                let events = abis
                    .iter()
                    .flat_map(|abi| abi.events().map(|event| event.signature()))
                    .unique()
                    .collect::<Vec<_>>();

                SelectorImportRequest { function: functions_and_errors, event: events }
            }
            SelectorImportData::Raw(raw) => {
                let function_and_error =
                    raw.function.iter().chain(raw.error.iter()).cloned().collect::<Vec<_>>();
                SelectorImportRequest { function: function_and_error, event: raw.event }
            }
        };

        let mut response: Option<SelectorImportResponse> = None;
        for request in request.into_chunks()? {
            let chunk: SelectorImportResponse =
                self.post_json(SELECTOR_IMPORT_URL, &request).await?;
            if let Some(response) = &mut response {
                response.result.function.imported.extend(chunk.result.function.imported);
                response.result.function.duplicated.extend(chunk.result.function.duplicated);
                response.result.event.imported.extend(chunk.result.event.imported);
                response.result.event.duplicated.extend(chunk.result.event.duplicated);
            } else {
                response = Some(chunk);
            }
        }
        Ok(response.expect("selector import always contains at least one request"))
    }
}

pub enum SelectorOrSig {
    Selector(String),
    Sig(OpenChainSignatures),
}

pub struct PossibleSigs {
    method: SelectorOrSig,
    data: OpenChainSignatures,
}

impl PossibleSigs {
    fn new() -> Self {
        Self { method: SelectorOrSig::Selector("0x00000000".to_string()), data: vec![] }
    }
}

impl fmt::Display for PossibleSigs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.method {
            SelectorOrSig::Selector(selector) => {
                writeln!(f, "\n Method: {selector}")?;
            }
            SelectorOrSig::Sig(sigs) => {
                writeln!(f, "\n Possible methods:")?;
                for sig in sigs {
                    writeln!(f, " - {sig}")?;
                }
            }
        }

        writeln!(f, " ------------")?;
        for (i, row) in self.data.iter().enumerate() {
            let row_label_decimal = i * 32;
            let row_label_hex = format!("{row_label_decimal:03x}");
            writeln!(f, " [{row_label_hex}]: {row}")?;
        }
        Ok(())
    }
}

/// The kind of selector to fetch from OpenChain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectorKind {
    /// A function selector.
    Function(Selector),
    /// A custom error selector. Behaves the same as a function selector.
    Error(Selector),
    /// An event selector.
    Event(B256),
}

impl SelectorKind {
    /// Returns the function selector if it is a function OR custom error.
    pub const fn as_function(&self) -> Option<Selector> {
        match *self {
            Self::Function(selector) | Self::Error(selector) => Some(selector),
            _ => None,
        }
    }

    /// Returns the event selector if it is an event.
    pub const fn as_event(&self) -> Option<B256> {
        match *self {
            Self::Event(hash) => Some(hash),
            _ => None,
        }
    }
}

/// Decodes the given function or event selector using OpenChain.
pub async fn decode_selector(selector: SelectorKind) -> eyre::Result<OpenChainSignatures> {
    OpenChainClient::new()?.decode_selector(selector).await
}

/// Decodes the given function or event selectors using OpenChain.
pub async fn decode_selectors(
    selectors: &[SelectorKind],
) -> eyre::Result<Vec<OpenChainSignatures>> {
    OpenChainClient::new()?.decode_selectors(selectors).await
}

/// Fetches a function signature given the selector using OpenChain.
pub async fn decode_function_selector(selector: Selector) -> eyre::Result<OpenChainSignatures> {
    OpenChainClient::new()?.decode_function_selector(selector).await
}

/// Fetches all possible signatures and attempts to abi decode the calldata using OpenChain.
pub async fn decode_calldata(calldata: &str) -> eyre::Result<OpenChainSignatures> {
    OpenChainClient::new()?.decode_calldata(calldata).await
}

/// Fetches an event signature given the 32 byte topic using OpenChain.
pub async fn decode_event_topic(topic: B256) -> eyre::Result<OpenChainSignatures> {
    OpenChainClient::new()?.decode_event_topic(topic).await
}

/// Pretty print calldata and if available, fetch possible function signatures.
///
/// ```no_run
/// use foundry_common::selectors::pretty_calldata;
///
/// # async fn foo() -> eyre::Result<()> {
/// let pretty_data = pretty_calldata(
///     "0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5".to_string(),
///     false,
/// )
/// .await?;
/// println!("{}", pretty_data);
/// # Ok(())
/// # }
/// ```
pub async fn pretty_calldata(
    calldata: impl AsRef<str>,
    offline: bool,
) -> eyre::Result<PossibleSigs> {
    OpenChainClient::new()?.pretty_calldata(calldata, offline).await
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct RawSelectorImportData {
    pub function: OpenChainSignatures,
    pub event: OpenChainSignatures,
    pub error: OpenChainSignatures,
}

impl RawSelectorImportData {
    pub const fn is_empty(&self) -> bool {
        self.function.is_empty() && self.event.is_empty() && self.error.is_empty()
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum SelectorImportData {
    Abi(Vec<JsonAbi>),
    Raw(RawSelectorImportData),
}

#[derive(Debug, Default, Serialize)]
struct SelectorImportRequest {
    function: OpenChainSignatures,
    event: OpenChainSignatures,
}

impl SelectorImportRequest {
    fn into_chunks(self) -> eyre::Result<Vec<Self>> {
        let empty_request_len = serde_json::to_vec(&Self::default()).unwrap().len();
        let mut chunks = Vec::new();
        let mut chunk = Self::default();
        let mut chunk_len = empty_request_len;

        for (is_event, signature) in self
            .function
            .into_iter()
            .map(|signature| (false, signature))
            .chain(self.event.into_iter().map(|signature| (true, signature)))
        {
            let signature_len = serde_json::to_vec(&signature).unwrap().len();
            if empty_request_len + signature_len > SELECTOR_IMPORT_BODY_LIMIT {
                eyre::bail!("selector signature exceeds the registry request body limit");
            }
            let needs_comma =
                if is_event { !chunk.event.is_empty() } else { !chunk.function.is_empty() };
            let signatures_full = if is_event {
                chunk.event.len() == SELECTOR_IMPORT_SIGNATURE_LIMIT
            } else {
                chunk.function.len() == SELECTOR_IMPORT_SIGNATURE_LIMIT
            };
            if (signatures_full
                || chunk_len + signature_len + usize::from(needs_comma)
                    > SELECTOR_IMPORT_BODY_LIMIT)
                && chunk_len > empty_request_len
            {
                chunks.push(chunk);
                chunk = Self::default();
                chunk_len = empty_request_len;
            }

            let signatures = if is_event { &mut chunk.event } else { &mut chunk.function };
            chunk_len += signature_len + usize::from(!signatures.is_empty());
            signatures.push(signature);
        }
        chunks.push(chunk);
        Ok(chunks)
    }
}

#[derive(Debug, Deserialize)]
struct SelectorImportEffect {
    imported: HashMap<String, String>,
    duplicated: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SelectorImportResult {
    function: SelectorImportEffect,
    event: SelectorImportEffect,
}

#[derive(Debug, Deserialize)]
pub struct SelectorImportResponse {
    result: SelectorImportResult,
}

impl SelectorImportResponse {
    /// Print info about the functions which were uploaded or already known
    pub fn describe(&self) {
        for (k, v) in &self.result.function.imported {
            let _ = sh_println!("Imported: Function {k}: {v}");
        }
        for (k, v) in &self.result.event.imported {
            let _ = sh_println!("Imported: Event {k}: {v}");
        }
        for (k, v) in &self.result.function.duplicated {
            let _ = sh_println!("Duplicated: Function {k}: {v}");
        }
        for (k, v) in &self.result.event.duplicated {
            let _ = sh_println!("Duplicated: Event {k}: {v}");
        }

        let _ = sh_println!("Selectors successfully uploaded to OpenChain");
    }
}

/// uploads selectors to OpenChain using the given data
pub async fn import_selectors(data: SelectorImportData) -> eyre::Result<SelectorImportResponse> {
    OpenChainClient::new()?.import_selectors(data).await
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedSignatures {
    pub signatures: RawSelectorImportData,
    pub abis: Vec<JsonAbi>,
}

#[derive(Deserialize)]
struct Artifact {
    abi: JsonAbi,
}

/// Parses a list of tokens into function, event, and error signatures.
/// Also handles JSON artifact files
/// Ignores invalid tokens
pub fn parse_signatures(tokens: Vec<String>) -> ParsedSignatures {
    // if any of the given tokens are json artifact files,
    // Parse them and read in the ABI from the file
    let abis = tokens
        .iter()
        .filter(|sig| sig.ends_with(".json"))
        .filter_map(|filename| std::fs::read_to_string(filename).ok())
        .filter_map(|file| serde_json::from_str(file.as_str()).ok())
        .map(|artifact: Artifact| artifact.abi)
        .collect();

    // for tokens that are not json artifact files,
    // try to parse them as raw signatures
    let signatures = tokens.iter().filter(|sig| !sig.ends_with(".json")).fold(
        RawSelectorImportData::default(),
        |mut data, signature| {
            let mut split = signature.split(' ');
            #[allow(clippy::collapsible_match)]
            match split.next() {
                Some("function") => {
                    if let Some(sig) = split.next() {
                        data.function.push(sig.to_string())
                    }
                }
                Some("event") => {
                    if let Some(sig) = split.next() {
                        data.event.push(sig.to_string())
                    }
                }
                Some("error") => {
                    if let Some(sig) = split.next() {
                        data.error.push(sig.to_string())
                    }
                }
                Some(signature) => {
                    // if no type given, assume function
                    data.function.push(signature.to_string());
                }
                None => {}
            }
            data
        },
    );

    ParsedSignatures { signatures, abis }
}

/// [`SELECTOR_LOOKUP_URL`] response.
#[derive(Deserialize)]
struct SignatureResponse {
    ok: bool,
    result: SignatureResult,
}

#[derive(Deserialize)]
struct SignatureResult {
    event: HashMap<B256, Option<Vec<Signature>>>,
    function: HashMap<Selector, Option<Vec<Signature>>>,
}

#[derive(Deserialize)]
struct Signature {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::StatusCode,
        routing::{get, post},
    };

    async fn spawn_status_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/unavailable", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
            .route("/bad-request", get(|| async { StatusCode::BAD_REQUEST }))
            .route("/post-unavailable", post(|| async { StatusCode::SERVICE_UNAVAILABLE }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn test_parse_signatures() {
        let result = parse_signatures(vec!["transfer(address,uint256)".to_string()]);
        assert_eq!(
            result,
            ParsedSignatures {
                signatures: RawSelectorImportData {
                    function: vec!["transfer(address,uint256)".to_string()],
                    ..Default::default()
                },
                ..Default::default()
            }
        );

        let result = parse_signatures(vec![
            "transfer(address,uint256)".to_string(),
            "function approve(address,uint256)".to_string(),
        ]);
        assert_eq!(
            result,
            ParsedSignatures {
                signatures: RawSelectorImportData {
                    function: vec![
                        "transfer(address,uint256)".to_string(),
                        "approve(address,uint256)".to_string()
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }
        );

        let result = parse_signatures(vec![
            "transfer(address,uint256)".to_string(),
            "event Approval(address,address,uint256)".to_string(),
            "error ERC20InsufficientBalance(address,uint256,uint256)".to_string(),
        ]);
        assert_eq!(
            result,
            ParsedSignatures {
                signatures: RawSelectorImportData {
                    function: vec!["transfer(address,uint256)".to_string()],
                    event: vec!["Approval(address,address,uint256)".to_string()],
                    error: vec!["ERC20InsufficientBalance(address,uint256,uint256)".to_string()]
                },
                ..Default::default()
            }
        );

        // skips invalid
        let result = parse_signatures(vec!["event".to_string()]);
        assert_eq!(
            result,
            ParsedSignatures { signatures: Default::default(), ..Default::default() }
        );
    }

    #[test]
    fn selector_import_requests_stay_within_registry_limits() {
        let suffix = "x".repeat(100);
        let function = (0..3_000)
            .map(|i| format!("functionName{i:04}{suffix}(address,uint256)"))
            .collect::<Vec<_>>();
        let expected = function.clone();
        let chunks = SelectorImportRequest { function, event: Vec::new() }.into_chunks().unwrap();

        assert!(chunks.len() > 3);
        assert!(chunks.iter().all(|chunk| chunk.function.len() <= SELECTOR_IMPORT_SIGNATURE_LIMIT
            && serde_json::to_vec(chunk).unwrap().len() <= SELECTOR_IMPORT_BODY_LIMIT));
        assert_eq!(
            chunks.into_iter().flat_map(|chunk| chunk.function).collect::<Vec<_>>(),
            expected
        );
    }

    #[tokio::test]
    async fn spurious_marked_on_timeout_threshold() {
        // Use an unreachable local port to trigger a quick connect error.
        let client = OpenChainClient::new().expect("client must build");
        let url = "http://127.0.0.1:9"; // Discard port; typically closed and fails fast.

        // After MAX_TIMEDOUT_REQ - 1 failures we should NOT be spurious.
        for i in 0..(MAX_TIMEDOUT_REQ - 1) {
            let _ = client.get_text(url).await; // expect an error and internal counter increment
            assert!(!client.is_spurious(), "unexpected spurious after {} failed attempts", i + 1);
        }

        // The Nth failure (N == MAX_TIMEDOUT_REQ) should flip the spurious flag.
        let _ = client.get_text(url).await;
        assert!(client.is_spurious(), "expected spurious after threshold failures");
    }

    #[tokio::test]
    async fn spurious_marked_on_http_5xx_threshold() {
        let (base_url, server_task) = spawn_status_server().await;
        let client = OpenChainClient::new().expect("client must build");

        for i in 0..(MAX_TIMEDOUT_REQ - 1) {
            let result = client.get_text(format!("{base_url}/unavailable")).await;
            assert!(result.is_err(), "expected HTTP 503 on attempt {}", i + 1);
            assert!(!client.is_spurious(), "unexpected spurious after {} failed attempts", i + 1);
        }

        let result = client.get_text(format!("{base_url}/unavailable")).await;
        assert!(result.is_err());
        assert!(client.is_spurious(), "expected spurious after threshold HTTP 503 responses");

        server_task.abort();
    }

    #[tokio::test]
    async fn post_http_5xx_counts_as_connectivity_failure() {
        let (base_url, server_task) = spawn_status_server().await;
        let client = OpenChainClient::new().expect("client must build");
        let body = serde_json::json!({});

        for _ in 0..MAX_TIMEDOUT_REQ {
            let result = client
                .post_json::<_, serde_json::Value>(&format!("{base_url}/post-unavailable"), &body)
                .await;
            assert!(result.is_err(), "expected HTTP 503");
        }

        assert!(client.is_spurious(), "expected HTTP 503 POSTs to mark the connection spurious");
        server_task.abort();
    }

    #[tokio::test]
    async fn http_4xx_does_not_count_as_connectivity_failure() {
        let (base_url, server_task) = spawn_status_server().await;
        let client = OpenChainClient::new().expect("client must build");

        assert!(client.get_text(format!("{base_url}/bad-request")).await.is_err());
        assert!(client.get_text(format!("{base_url}/missing")).await.is_err());
        assert_eq!(client.timedout_requests.load(Ordering::SeqCst), 0);
        assert!(!client.is_spurious());

        server_task.abort();
    }
}
