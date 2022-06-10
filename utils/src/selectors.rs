use crate::abi_decode;
use ethers_solc::artifacts::LosslessAbi;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};

static SELECTOR_DATABASE_URL: &str = "https://sig.eth.samczsun.com/api/v1/signatures";
static SELECTOR_IMPORT_URL: &str = "https://sig.eth.samczsun.com/api/v1/import";

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
                writeln!(f, "\n Method: {}", selector)?;
            }
            SelectorOrSig::Sig(sigs) => {
                writeln!(f, "\n Possible methods:")?;
                for sig in sigs {
                    writeln!(f, " - {}", sig)?;
                }
            }
        }

        writeln!(f, " ------------")?;
        for (i, row) in self.data.iter().enumerate() {
            let pad = if i < 10 { "  " } else { " " };
            writeln!(f, " [{}]:{}{}", i, pad, row)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub enum SelectorType {
    Function,
    Event,
}

/// Decodes the given function or event selector using sig.eth.samczsun.com
pub async fn decode_selector(selector: &str, selector_type: SelectorType) -> Result<Vec<String>> {
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

    // using samczsun signature database over 4byte
    // see https://github.com/foundry-rs/foundry/issues/1672
    let url = match selector_type {
        SelectorType::Function => format!("{SELECTOR_DATABASE_URL}?function={selector}"),
        SelectorType::Event => format!("{SELECTOR_DATABASE_URL}?event={selector}"),
    };

    let res = reqwest::get(url).await?.text().await?;
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
        .filter_map(|d| {
            if !d.filtered {
                return Some(d.name.clone())
            }
            None
        })
        .collect::<Vec<String>>())
}

/// Fetches a function signature given the selector using sig.eth.samczsun.com
pub async fn decode_function_selector(selector: &str) -> Result<Vec<String>> {
    let prefixed_selector = format!("0x{}", selector.strip_prefix("0x").unwrap_or(selector));
    if prefixed_selector.len() < 10 {
        return Err(eyre::eyre!("Invalid selector"))
    }

    decode_selector(&prefixed_selector[..10], SelectorType::Function).await
}

/// Fetches all possible signatures and attempts to abi decode the calldata
pub async fn decode_calldata(calldata: &str) -> Result<Vec<String>> {
    let sigs = decode_function_selector(calldata).await?;

    // filter for signatures that can be decoded
    Ok(sigs
        .iter()
        .cloned()
        .filter(|sig| {
            let res = abi_decode(sig, calldata, true);
            res.is_ok()
        })
        .collect::<Vec<String>>())
}

/// Fetches a event signature given the 32 byte topic using sig.eth.samczsun.com
pub async fn decode_event_topic(topic: &str) -> Result<Vec<String>> {
    let prefixed_topic = format!("0x{}", topic.strip_prefix("0x").unwrap_or(topic));
    if prefixed_topic.len() < 66 {
        return Err(eyre::eyre!("Invalid topic"))
    }
    decode_selector(&prefixed_topic[..66], SelectorType::Event).await
}

/// Pretty print calldata and if available, fetch possible function signatures
///
/// ```no_run
/// 
/// use foundry_utils::selectors::pretty_calldata;
///
/// # async fn foo() -> eyre::Result<()> {
///   let pretty_data = pretty_calldata("0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5".to_string(), false).await?;
///   println!("{}",pretty_data);
/// # Ok(())
/// # }
/// ```

pub async fn pretty_calldata(calldata: impl AsRef<str>, offline: bool) -> Result<PossibleSigs> {
    let mut possible_info = PossibleSigs::new();
    let calldata = calldata.as_ref().trim_start_matches("0x");

    let selector =
        calldata.get(..8).ok_or_else(|| eyre::eyre!("calldata cannot be less that 4 bytes"))?;

    let sigs = if offline {
        vec![]
    } else {
        decode_function_selector(selector).await.unwrap_or_default().into_iter().collect()
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

#[derive(Serialize)]
struct SelectorImportRequest {
    #[serde(rename = "type")]
    import_type: String,
    data: SelectorImportData,
}

#[derive(Deserialize)]
struct SelectorImportEffect {
    imported: HashMap<String, String>,
    duplicated: HashMap<String, String>,
}

#[derive(Deserialize)]
struct SelectorImportResult {
    function: SelectorImportEffect,
    event: SelectorImportEffect,
}

#[derive(Deserialize)]
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

        println!("Selectors successfully uploaded to https://sig.eth.samczsun.com");
    }
}

/// uploads selectors to sig.eth.samczsun.com using the given data
pub async fn import_selectors(data: SelectorImportData) -> Result<SelectorImportResponse> {
    let request = match &data {
        SelectorImportData::Abi(_) => {
            SelectorImportRequest { import_type: "abi".to_string(), data }
        }
        SelectorImportData::Raw(_) => {
            SelectorImportRequest { import_type: "raw".to_string(), data }
        }
    };

    Ok(reqwest::Client::new().post(SELECTOR_IMPORT_URL).json(&request).send().await?.json().await?)
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

#[tokio::test]
async fn test_decode_selector() {
    let sigs = decode_function_selector("0xa9059cbb").await;
    assert_eq!(sigs.unwrap()[0], "transfer(address,uint256)".to_string());

    let sigs = decode_function_selector("a9059cbb").await;
    assert_eq!(sigs.unwrap()[0], "transfer(address,uint256)".to_string());

    // invalid signature
    decode_function_selector("0xa9059c")
        .await
        .map_err(|e| assert_eq!(e.to_string(), "Invalid selector"))
        .map(|_| panic!("Expected fourbyte error"))
        .ok();
}

#[tokio::test]
async fn test_decode_calldata() {
    let decoded = decode_calldata("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79").await;
    assert_eq!(decoded.unwrap()[0], "transfer(address,uint256)".to_string());

    let decoded = decode_calldata("a9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79").await;
    assert_eq!(decoded.unwrap()[0], "transfer(address,uint256)".to_string());
}

#[tokio::test]
async fn test_import_selectors() {
    let mut data = RawSelectorImportData::default();
    data.function.push("transfer(address,uint256)".to_string());
    let result = import_selectors(SelectorImportData::Raw(data)).await;
    assert_eq!(
        result.unwrap().result.function.duplicated.get("transfer(address,uint256)").unwrap(),
        "0xa9059cbb"
    );

    let abi: LosslessAbi = serde_json::from_str(r#"[{"constant":false,"inputs":[{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transfer","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"}]"#).unwrap();
    let result = import_selectors(SelectorImportData::Abi(vec![abi])).await;
    assert_eq!(
        result.unwrap().result.function.duplicated.get("transfer(address,uint256)").unwrap(),
        "0xa9059cbb"
    );
}

#[tokio::test]
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
        ParsedSignatures {
            signatures: RawSelectorImportData { ..Default::default() },
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn test_decode_event_topic() {
    let decoded =
        decode_event_topic("0x7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6")
            .await;
    assert_eq!(decoded.unwrap()[0], "updateAuthority(address,uint8)".to_string());

    let decoded =
        decode_event_topic("7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6")
            .await;
    assert_eq!(decoded.unwrap()[0], "updateAuthority(address,uint8)".to_string());

    let decoded =
        decode_event_topic("0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd")
            .await;
    assert_eq!(decoded.unwrap()[0], "canCall(address,address,bytes4)".to_string());

    let decoded =
        decode_event_topic("b7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd")
            .await;
    assert_eq!(decoded.unwrap()[0], "canCall(address,address,bytes4)".to_string());
}
