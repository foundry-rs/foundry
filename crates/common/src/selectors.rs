//! Support for handling/identifying selectors
#![allow(missing_docs)]

use crate::abi::abi_decode;
use ethers_solc::artifacts::LosslessAbi;
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
use tracing::warn;

static SELECTOR_DATABASE_URL: &str = "https://api.openchain.xyz/signature-database/v1/";
static SELECTOR_IMPORT_URL: &str = "https://api.openchain.xyz/signature-database/v1/import";

/// The standard request timeout for API requests
const REQ_TIMEOUT: Duration = Duration::from_secs(15);

/// How many request can time out before we decide this is a spurious connection
const MAX_TIMEDOUT_REQ: usize = 4usize;

/// A client that can request API data from `https://api.openchain.xyz`
#[derive(Debug, Clone)]
pub struct SignEthClient {
    inner: reqwest::Client,
    /// Whether the connection is spurious, or API is down
    spurious_connection: Arc<AtomicBool>,
    /// How many requests timed out
    timedout_requests: Arc<AtomicUsize>,
    /// Max allowed request that can time out
    max_timedout_requests: usize,
}

impl SignEthClient {
    /// Creates a new client with default settings
    pub fn new() -> reqwest::Result<Self> {
        let inner = reqwest::Client::builder()
            .default_headers(HeaderMap::from_iter([(
                HeaderName::from_static("user-agent"),
                HeaderValue::from_static("forge"),
            )]))
            .timeout(REQ_TIMEOUT)
            .build()?;
        Ok(Self {
            inner,
            spurious_connection: Arc::new(Default::default()),
            timedout_requests: Arc::new(Default::default()),
            max_timedout_requests: MAX_TIMEDOUT_REQ,
        })
    }

    async fn get_text(&self, url: &str) -> reqwest::Result<String> {
        self.inner
            .get(url)
            .send()
            .await
            .map_err(|err| {
                self.on_reqwest_err(&err);
                err
            })?
            .text()
            .await
            .map_err(|err| {
                self.on_reqwest_err(&err);
                err
            })
    }

    /// Sends a new post request
    async fn post_json<T: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &T,
    ) -> reqwest::Result<R> {
        self.inner
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|err| {
                self.on_reqwest_err(&err);
                err
            })?
            .json()
            .await
            .map_err(|err| {
                self.on_reqwest_err(&err);
                err
            })
    }

    fn on_reqwest_err(&self, err: &reqwest::Error) {
        fn is_connectivity_err(err: &reqwest::Error) -> bool {
            if err.is_timeout() || err.is_connect() {
                return true
            }
            // Error HTTP codes (5xx) are considered connectivity issues and will prompt retry
            if let Some(status) = err.status() {
                let code = status.as_u16();
                if (500..600).contains(&code) {
                    return true
                }
            }
            false
        }

        if is_connectivity_err(err) {
            warn!("spurious network detected for https://api.openchain.xyz");
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

    /// Decodes the given function or event selector using https://api.openchain.xyz
    pub async fn decode_selector(
        &self,
        selector: &str,
        selector_type: SelectorType,
    ) -> eyre::Result<Vec<String>> {
        // exit early if spurious connection
        self.ensure_not_spurious()?;

        #[derive(Deserialize)]
        struct Decoded {
            name: String,
            filtered: bool,
        }

        #[derive(Deserialize)]
        struct ApiResult {
            event: HashMap<String, Vec<Decoded>>,
            function: HashMap<String, Vec<Decoded>>,
        }

        #[derive(Deserialize)]
        struct ApiResponse {
            ok: bool,
            result: ApiResult,
        }

        // using openchain.xyz signature database over 4byte
        // see https://github.com/foundry-rs/foundry/issues/1672
        let url = match selector_type {
            SelectorType::Function => format!("{SELECTOR_DATABASE_URL}lookup?function={selector}"),
            SelectorType::Event => format!("{SELECTOR_DATABASE_URL}lookup?event={selector}"),
        };

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

        Ok(decoded
            .get(selector)
            .ok_or(eyre::eyre!("No signature found"))?
            .iter()
            .filter(|&d| !d.filtered)
            .map(|d| d.name.clone())
            .collect::<Vec<String>>())
    }

    /// Fetches a function signature given the selector using https://api.openchain.xyz
    pub async fn decode_function_selector(&self, selector: &str) -> eyre::Result<Vec<String>> {
        let stripped_selector = selector.strip_prefix("0x").unwrap_or(selector);
        let prefixed_selector = format!("0x{}", stripped_selector);
        if prefixed_selector.len() != 10 {
            eyre::bail!(
                "Invalid selector: expected 8 characters (excluding 0x prefix), got {}.",
                stripped_selector.len()
            )
        }

        self.decode_selector(&prefixed_selector[..10], SelectorType::Function).await
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
            .cloned()
            .filter(|sig| abi_decode(sig, calldata, true, true).is_ok())
            .collect::<Vec<String>>())
    }

    /// Fetches an event signature given the 32 byte topic using https://api.openchain.xyz
    pub async fn decode_event_topic(&self, topic: &str) -> eyre::Result<Vec<String>> {
        let prefixed_topic = format!("0x{}", topic.strip_prefix("0x").unwrap_or(topic));
        if prefixed_topic.len() != 66 {
            eyre::bail!("Invalid topic: expected 64 characters (excluding 0x prefix), got {} characters (including 0x prefix).", prefixed_topic.len())
        }
        self.decode_selector(&prefixed_topic[..66], SelectorType::Event).await
    }

    /// Pretty print calldata and if available, fetch possible function signatures
    ///
    /// ```
    /// use foundry_common::selectors::SignEthClient;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// SignEthClient::new()?.pretty_calldata(
    ///     "0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5",
    ///     false,
    /// ).await?;
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

    /// uploads selectors to https://api.openchain.xyz using the given data
    pub async fn import_selectors(
        &self,
        data: SelectorImportData,
    ) -> eyre::Result<SelectorImportResponse> {
        self.ensure_not_spurious()?;

        let request = match data {
            SelectorImportData::Abi(abis) => {
                let names: Vec<String> = abis
                    .iter()
                    .flat_map(|abi| {
                        abi.abi
                            .functions()
                            .map(|func| {
                                func.signature().split(':').next().unwrap_or("").to_string()
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                SelectorImportRequest { function: names, event: Default::default() }
            }
            SelectorImportData::Raw(raw) => {
                SelectorImportRequest { function: raw.function, event: raw.event }
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
        PossibleSigs { method: SelectorOrSig::Selector("0x00000000".to_string()), data: vec![] }
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

#[derive(Clone, Copy)]
pub enum SelectorType {
    Function,
    Event,
}

/// Decodes the given function or event selector using https://api.openchain.xyz
pub async fn decode_selector(
    selector: &str,
    selector_type: SelectorType,
) -> eyre::Result<Vec<String>> {
    SignEthClient::new()?.decode_selector(selector, selector_type).await
}

/// Fetches a function signature given the selector https://api.openchain.xyz
pub async fn decode_function_selector(selector: &str) -> eyre::Result<Vec<String>> {
    SignEthClient::new()?.decode_function_selector(selector).await
}

/// Fetches all possible signatures and attempts to abi decode the calldata
pub async fn decode_calldata(calldata: &str) -> eyre::Result<Vec<String>> {
    SignEthClient::new()?.decode_calldata(calldata).await
}

/// Fetches an event signature given the 32 byte topic using https://api.openchain.xyz
pub async fn decode_event_topic(topic: &str) -> eyre::Result<Vec<String>> {
    SignEthClient::new()?.decode_event_topic(topic).await
}

/// Pretty print calldata and if available, fetch possible function signatures
///
/// ```
/// use foundry_common::selectors::pretty_calldata;
///
/// # async fn foo() -> eyre::Result<()> {
/// pretty_calldata(
///     "0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5",
///     false,
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn pretty_calldata(
    calldata: impl AsRef<str>,
    offline: bool,
) -> eyre::Result<PossibleSigs> {
    SignEthClient::new()?.pretty_calldata(calldata, offline).await
}

#[derive(Default, Serialize, PartialEq, Debug, Eq)]
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
    Abi(Vec<LosslessAbi>),
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
    pub fn describe(&self) -> eyre::Result<()> {
        for (k, v) in &self.result.function.imported {
            sh_status!("Imported" => "function {k}: {v}")?;
        }
        for (k, v) in &self.result.event.imported {
            sh_status!("Imported" => "event {k}: {v}")?;
        }
        for (k, v) in &self.result.function.duplicated {
            sh_status!("Duplicated" => "function {k}: {v}")?;
        }
        for (k, v) in &self.result.event.duplicated {
            sh_status!("Duplicated" => "event {k}: {v}")?;
        }

        sh_eprintln!("Selectors successfully uploaded to https://api.openchain.xyz")
    }
}

/// uploads selectors to https://api.openchain.xyz using the given data
pub async fn import_selectors(data: SelectorImportData) -> eyre::Result<SelectorImportResponse> {
    SignEthClient::new()?.import_selectors(data).await
}

#[derive(PartialEq, Default, Debug)]
pub struct ParsedSignatures {
    pub signatures: RawSelectorImportData,
    pub abis: Vec<LosslessAbi>,
}

#[derive(Deserialize)]
struct Artifact {
    abi: LosslessAbi,
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
                    "Invalid selector: expected 8 characters (excluding 0x prefix), got 6."
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

        let abi: LosslessAbi = serde_json::from_str(r#"[{"constant":false,"inputs":[{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transfer","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function", "methodIdentifiers": {"transfer(address,uint256)(uint256)": "0xa9059cbb"}}]"#).unwrap();
        let result = import_selectors(SelectorImportData::Abi(vec![abi])).await;
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
        ]);
        assert_eq!(
            result,
            ParsedSignatures {
                signatures: RawSelectorImportData {
                    function: vec!["transfer(address,uint256)".to_string()],
                    event: vec!["Approval(address,address,uint256)".to_string()],
                    ..Default::default()
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
}
