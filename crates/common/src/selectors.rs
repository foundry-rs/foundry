//! Support for handling/identifying selectors.

#![allow(missing_docs)]

use crate::abi::abi_decode_calldata;
use alloy_json_abi::JsonAbi;
use eyre::Context;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

const SELECTOR_LOOKUP_URL: &str = "https://api.openchain.xyz/signature-database/v1/lookup";
const SELECTOR_IMPORT_URL: &str = "https://api.openchain.xyz/signature-database/v1/import";

/// The standard request timeout for API requests.
const REQ_TIMEOUT: Duration = Duration::from_secs(15);

/// How many request can time out before we decide this is a spurious connection.
const MAX_TIMEDOUT_REQ: usize = 4usize;

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
    /// Creates a new client with default settings
    pub fn new() -> eyre::Result<Self> {
        let inner = reqwest::Client::builder()
            .default_headers(HeaderMap::from_iter([(
                HeaderName::from_static("user-agent"),
                HeaderValue::from_static("forge"),
            )]))
            .timeout(REQ_TIMEOUT)
            .build()
            .wrap_err("failed to build OpenChain client")?;
        Ok(Self {
            inner,
            spurious_connection: Arc::new(Default::default()),
            timedout_requests: Arc::new(Default::default()),
            max_timedout_requests: MAX_TIMEDOUT_REQ,
        })
    }

    async fn get_text(&self, url: &str) -> reqwest::Result<String> {
        trace!(%url, "GET");
        self.inner
            .get(url)
            .send()
            .await
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
            if previous >= self.max_timedout_requests {
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
            eyre::bail!("Spurious connection detected")
        }
        Ok(())
    }

    /// Decodes the given function or event selector using OpenChain
    pub async fn decode_selector(
        &self,
        selector: &str,
        selector_type: SelectorType,
    ) -> eyre::Result<Vec<String>> {
        self.decode_selectors(selector_type, std::iter::once(selector))
            .await?
            .pop() // Not returning on the previous line ensures a vector with exactly 1 element
            .unwrap()
            .ok_or_else(|| eyre::eyre!("No signature found"))
    }

    /// Decodes the given function or event selectors using OpenChain
    pub async fn decode_selectors(
        &self,
        selector_type: SelectorType,
        selectors: impl IntoIterator<Item = impl Into<String>>,
    ) -> eyre::Result<Vec<Option<Vec<String>>>> {
        let selectors: Vec<String> = selectors
            .into_iter()
            .map(Into::into)
            .map(|s| s.to_lowercase())
            .map(|s| if s.starts_with("0x") { s } else { format!("0x{s}") })
            .collect();

        if selectors.is_empty() {
            return Ok(vec![]);
        }

        debug!(len = selectors.len(), "decoding selectors");
        trace!(?selectors, "decoding selectors");

        // exit early if spurious connection
        self.ensure_not_spurious()?;

        let expected_len = match selector_type {
            SelectorType::Function => 10, // 0x + hex(4bytes)
            SelectorType::Event => 66,    // 0x + hex(32bytes)
        };
        if let Some(s) = selectors.iter().find(|s| s.len() != expected_len) {
            eyre::bail!(
                "Invalid selector {s}: expected {expected_len} characters (including 0x prefix)."
            )
        }

        #[derive(Deserialize)]
        struct Decoded {
            name: String,
        }

        #[derive(Deserialize)]
        struct ApiResult {
            event: HashMap<String, Option<Vec<Decoded>>>,
            function: HashMap<String, Option<Vec<Decoded>>>,
        }

        #[derive(Deserialize)]
        struct ApiResponse {
            ok: bool,
            result: ApiResult,
        }

        let url = format!(
            "{SELECTOR_LOOKUP_URL}?{ltype}={selectors_str}",
            ltype = match selector_type {
                SelectorType::Function => "function",
                SelectorType::Event => "event",
            },
            selectors_str = selectors.join(",")
        );

        let res = self.get_text(&url).await?;
        let api_response = match serde_json::from_str::<ApiResponse>(&res) {
            Ok(inner) => inner,
            Err(err) => {
                eyre::bail!("Could not decode response:\n {res}.\nError: {err}")
            }
        };

        if !api_response.ok {
            eyre::bail!("Failed to decode:\n {res}")
        }

        let decoded = match selector_type {
            SelectorType::Function => api_response.result.function,
            SelectorType::Event => api_response.result.event,
        };

        Ok(selectors
            .into_iter()
            .map(|selector| match decoded.get(&selector) {
                Some(Some(r)) => Some(r.iter().map(|d| d.name.clone()).collect()),
                _ => None,
            })
            .collect())
    }

    /// Fetches a function signature given the selector using OpenChain
    pub async fn decode_function_selector(&self, selector: &str) -> eyre::Result<Vec<String>> {
        self.decode_selector(selector, SelectorType::Function).await
    }

    /// Fetches all possible signatures and attempts to abi decode the calldata
    pub async fn decode_calldata(&self, calldata: &str) -> eyre::Result<Vec<String>> {
        let calldata = calldata.strip_prefix("0x").unwrap_or(calldata);
        if calldata.len() < 8 {
            eyre::bail!(
                "Calldata too short: expected at least 8 characters (excluding 0x prefix), got {}.",
                calldata.len()
            )
        }

        let sigs = self.decode_function_selector(&calldata[..8]).await?;

        // filter for signatures that can be decoded
        Ok(sigs
            .iter()
            .filter(|sig| abi_decode_calldata(sig, calldata, true, true).is_ok())
            .cloned()
            .collect::<Vec<String>>())
    }

    /// Fetches an event signature given the 32 byte topic using OpenChain
    pub async fn decode_event_topic(&self, topic: &str) -> eyre::Result<Vec<String>> {
        self.decode_selector(topic, SelectorType::Event).await
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
            self.decode_function_selector(selector).await.unwrap_or_default().into_iter().collect()
        };
        let (_, data) = calldata.split_at(8);

        if data.len() % 64 != 0 {
            eyre::bail!("\nInvalid calldata size")
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
                let functions_and_errors: Vec<String> = abis
                    .iter()
                    .flat_map(|abi| {
                        abi.functions()
                            .map(|func| func.signature())
                            .chain(abi.errors().map(|error| error.signature()))
                            .collect::<Vec<_>>()
                    })
                    .collect();

                let events = abis
                    .iter()
                    .flat_map(|abi| abi.events().map(|event| event.signature()))
                    .collect::<Vec<_>>();

                SelectorImportRequest { function: functions_and_errors, event: events }
            }
            SelectorImportData::Raw(raw) => {
                let function_and_error =
                    raw.function.iter().chain(raw.error.iter()).cloned().collect::<Vec<_>>();
                SelectorImportRequest { function: function_and_error, event: raw.event }
            }
        };

        Ok(self.post_json(SELECTOR_IMPORT_URL, &request).await?)
    }
}

pub enum SelectorOrSig {
    Selector(String),
    Sig(Vec<String>),
}

pub struct PossibleSigs {
    method: SelectorOrSig,
    data: Vec<String>,
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

/// The type of selector fetched from OpenChain.
#[derive(Clone, Copy)]
pub enum SelectorType {
    /// A function selector.
    Function,
    /// An event selector.
    Event,
}

/// Decodes the given function or event selector using OpenChain.
pub async fn decode_selector(
    selector_type: SelectorType,
    selector: &str,
) -> eyre::Result<Vec<String>> {
    OpenChainClient::new()?.decode_selector(selector, selector_type).await
}

/// Decodes the given function or event selectors using OpenChain.
pub async fn decode_selectors(
    selector_type: SelectorType,
    selectors: impl IntoIterator<Item = impl Into<String>>,
) -> eyre::Result<Vec<Option<Vec<String>>>> {
    OpenChainClient::new()?.decode_selectors(selector_type, selectors).await
}

/// Fetches a function signature given the selector using OpenChain.
pub async fn decode_function_selector(selector: &str) -> eyre::Result<Vec<String>> {
    OpenChainClient::new()?.decode_function_selector(selector).await
}

/// Fetches all possible signatures and attempts to abi decode the calldata using OpenChain.
pub async fn decode_calldata(calldata: &str) -> eyre::Result<Vec<String>> {
    OpenChainClient::new()?.decode_calldata(calldata).await
}

/// Fetches an event signature given the 32 byte topic using OpenChain.
pub async fn decode_event_topic(topic: &str) -> eyre::Result<Vec<String>> {
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
    pub function: Vec<String>,
    pub event: Vec<String>,
    pub error: Vec<String>,
}

impl RawSelectorImportData {
    pub fn is_empty(&self) -> bool {
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
    function: Vec<String>,
    event: Vec<String>,
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
        self.result
            .function
            .imported
            .iter()
            .for_each(|(k, v)| println!("Imported: Function {k}: {v}"));
        self.result.event.imported.iter().for_each(|(k, v)| println!("Imported: Event {k}: {v}"));
        self.result
            .function
            .duplicated
            .iter()
            .for_each(|(k, v)| println!("Duplicated: Function {k}: {v}"));
        self.result
            .event
            .duplicated
            .iter()
            .for_each(|(k, v)| println!("Duplicated: Event {k}: {v}"));

        println!("Selectors successfully uploaded to OpenChain");
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

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_return)]

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_decode_selector() {
        let sigs = decode_function_selector("0xa9059cbb").await;
        assert_eq!(sigs.unwrap()[0], "transfer(address,uint256)".to_string());

        let sigs = decode_function_selector("a9059cbb").await;
        assert_eq!(sigs.unwrap()[0], "transfer(address,uint256)".to_string());

        // invalid signature
        decode_function_selector("0xa9059c")
            .await
            .map_err(|e| {
                assert_eq!(
                    e.to_string(),
                    "Invalid selector 0xa9059c: expected 10 characters (including 0x prefix)."
                )
            })
            .map(|_| panic!("Expected fourbyte error"))
            .ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_decode_calldata() {
        let decoded = decode_calldata("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79").await;
        assert_eq!(decoded.unwrap()[0], "transfer(address,uint256)".to_string());

        let decoded = decode_calldata("a9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79").await;
        assert_eq!(decoded.unwrap()[0], "transfer(address,uint256)".to_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_import_selectors() {
        let mut data = RawSelectorImportData::default();
        data.function.push("transfer(address,uint256)".to_string());
        let result = import_selectors(SelectorImportData::Raw(data)).await;
        assert_eq!(
            result.unwrap().result.function.duplicated.get("transfer(address,uint256)").unwrap(),
            "0xa9059cbb"
        );

        let abi: JsonAbi = serde_json::from_str(r#"[{"constant":false,"inputs":[{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transfer","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function", "methodIdentifiers": {"transfer(address,uint256)(uint256)": "0xa9059cbb"}}]"#).unwrap();
        let result = import_selectors(SelectorImportData::Abi(vec![abi])).await;
        println!("{result:?}");
        assert_eq!(
            result.unwrap().result.function.duplicated.get("transfer(address,uint256)").unwrap(),
            "0xa9059cbb"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_parse_signatures() {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_decode_event_topic() {
        let decoded = decode_event_topic(
            "0x7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6",
        )
        .await;
        assert_eq!(decoded.unwrap()[0], "updateAuthority(address,uint8)".to_string());

        let decoded =
            decode_event_topic("7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6")
                .await;
        assert_eq!(decoded.unwrap()[0], "updateAuthority(address,uint8)".to_string());

        let decoded = decode_event_topic(
            "0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd",
        )
        .await;
        assert_eq!(decoded.unwrap()[0], "canCall(address,address,bytes4)".to_string());

        let decoded =
            decode_event_topic("b7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd")
                .await;
        assert_eq!(decoded.unwrap()[0], "canCall(address,address,bytes4)".to_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_decode_selectors() {
        let event_topics = vec![
            "7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6",
            "0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd",
        ];
        let decoded = decode_selectors(SelectorType::Event, event_topics).await;
        let decoded = decoded.unwrap();
        assert_eq!(
            decoded,
            vec![
                Some(vec!["updateAuthority(address,uint8)".to_string()]),
                Some(vec!["canCall(address,address,bytes4)".to_string()]),
            ]
        );

        let function_selectors = vec!["0xa9059cbb", "0x70a08231", "313ce567"];
        let decoded = decode_selectors(SelectorType::Function, function_selectors).await;
        let decoded = decoded.unwrap();
        assert_eq!(
            decoded,
            vec![
                Some(vec!["transfer(address,uint256)".to_string()]),
                Some(vec!["balanceOf(address)".to_string()]),
                Some(vec!["decimals()".to_string()]),
            ]
        );
    }
}
