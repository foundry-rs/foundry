use super::contracts::ISafe;
use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes, U256};
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_common::sh_status;
use reqwest::{Client, Method, StatusCode, Url, header};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::str::FromStr;

#[derive(Args, Clone, Debug)]
pub struct SafeServiceOpts {
    /// Safe Transaction Service URL. Inferred from the RPC chain ID when omitted.
    /// The `/api` suffix is optional.
    #[arg(long, env = "SAFE_TRANSACTION_SERVICE_URL")]
    pub(super) service_url: Option<Url>,

    /// Safe Transaction Service API key.
    #[arg(long, env = "SAFE_API_KEY")]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SafeTransaction {
    pub(super) safe: Address,
    pub(super) to: Address,
    #[serde(deserialize_with = "deserialize_number_string")]
    pub(super) value: String,
    #[serde(default, deserialize_with = "deserialize_bytes")]
    pub(super) data: Bytes,
    pub(super) operation: u8,
    #[serde(deserialize_with = "deserialize_number_string")]
    pub(super) safe_tx_gas: String,
    #[serde(deserialize_with = "deserialize_number_string")]
    pub(super) base_gas: String,
    #[serde(deserialize_with = "deserialize_number_string")]
    pub(super) gas_price: String,
    #[serde(default, deserialize_with = "deserialize_nullable_address")]
    pub(super) gas_token: Address,
    #[serde(default, deserialize_with = "deserialize_nullable_address")]
    pub(super) refund_receiver: Address,
    #[serde(deserialize_with = "deserialize_number_string")]
    pub(super) nonce: String,
    #[serde(alias = "contractTransactionHash")]
    pub(super) safe_tx_hash: B256,
    #[serde(default, deserialize_with = "deserialize_confirmations")]
    pub(super) confirmations: Vec<SafeConfirmation>,
    #[serde(default)]
    pub(super) is_executed: bool,
    #[serde(default)]
    pub(super) transaction_hash: Option<B256>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SafeConfirmation {
    owner: Address,
    #[serde(default)]
    signature: Option<Bytes>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SafeDelegate {
    safe: Option<Address>,
    delegate: Address,
    delegator: Address,
    label: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SafeDelegatesResponse {
    pub(super) results: Vec<SafeDelegate>,
}

fn deserialize_number_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::String(value) => Ok(value),
        Value::Number(value) => Ok(value.to_string()),
        value => Err(serde::de::Error::custom(format!("expected number or string, got {value}"))),
    }
}

fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Bytes>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_nullable_address<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Address>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_confirmations<'de, D>(deserializer: D) -> Result<Vec<SafeConfirmation>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Vec<SafeConfirmation>>::deserialize(deserializer)?.unwrap_or_default())
}

impl SafeTransaction {
    pub(super) fn number(value: &str, field: &str) -> Result<U256> {
        U256::from_str(value).wrap_err_with(|| format!("invalid {field} in transaction"))
    }

    pub(super) async fn calculate_hash<P>(&self, provider: &P) -> Result<B256>
    where
        P: alloy_provider::Provider<Ethereum>,
    {
        ISafe::new(self.safe, provider)
            .getTransactionHash(
                self.to,
                Self::number(&self.value, "value")?,
                self.data.clone(),
                self.operation,
                Self::number(&self.safe_tx_gas, "safeTxGas")?,
                Self::number(&self.base_gas, "baseGas")?,
                Self::number(&self.gas_price, "gasPrice")?,
                self.gas_token,
                self.refund_receiver,
                Self::number(&self.nonce, "nonce")?,
            )
            .call()
            .await
            .wrap_err("failed to calculate Safe transaction hash")
    }

    pub(super) async fn verify_hash<P>(&self, provider: &P) -> Result<()>
    where
        P: alloy_provider::Provider<Ethereum>,
    {
        ensure!(self.operation <= 1, "invalid Safe operation: {}", self.operation);
        let calculated = self.calculate_hash(provider).await?;
        ensure!(
            calculated == self.safe_tx_hash,
            "Safe transaction hash mismatch: service/file returned {}, calculated {calculated}",
            self.safe_tx_hash
        );
        Ok(())
    }

    pub(super) fn proposal_body(
        &self,
        sender: Address,
        signature: String,
        origin: Option<String>,
    ) -> Value {
        let mut body = json!({
            "to": self.to,
            "value": self.value,
            "data": self.data,
            "operation": self.operation,
            "safeTxGas": self.safe_tx_gas,
            "baseGas": self.base_gas,
            "gasPrice": self.gas_price,
            "gasToken": self.gas_token,
            "refundReceiver": self.refund_receiver,
            "nonce": self.nonce,
            "contractTransactionHash": self.safe_tx_hash,
            "sender": sender.to_checksum(None),
            "signature": signature,
        });
        if let Some(origin) = origin {
            body["origin"] = Value::String(origin);
        }
        body
    }

    pub(super) fn packed_signatures(&self) -> Result<Bytes> {
        let mut confirmations = self.confirmations.iter().collect::<Vec<_>>();
        confirmations.sort_unstable_by_key(|confirmation| confirmation.owner);
        let static_len = confirmations.len() * 65;
        let mut signatures = Vec::with_capacity(static_len);
        let mut dynamic = Vec::new();
        for confirmation in confirmations {
            let signature = confirmation.signature.as_ref().ok_or_else(|| {
                eyre::eyre!("confirmation from {} does not contain a signature", confirmation.owner)
            })?;
            ensure!(
                signature.len() >= 65,
                "invalid signature from {}: expected at least 65 bytes, got {}",
                confirmation.owner,
                signature.len()
            );
            if signature[64] != 0 {
                ensure!(
                    signature.len() == 65,
                    "invalid signature from {}: expected 65 bytes, got {}",
                    confirmation.owner,
                    signature.len()
                );
                signatures.extend_from_slice(signature);
                continue;
            }

            ensure!(
                signature.len() >= 97,
                "contract signature from {} does not contain a length",
                confirmation.owner
            );
            let offset = U256::from_be_slice(&signature[32..64]);
            ensure!(
                offset == U256::from(65),
                "invalid contract signature offset from {}: expected 65, got {offset}",
                confirmation.owner
            );
            let data_len = U256::from_be_slice(&signature[65..97]);
            ensure!(
                data_len == U256::from(signature.len() - 97),
                "invalid contract signature length from {}: expected {}, got {data_len}",
                confirmation.owner,
                signature.len() - 97
            );

            signatures.extend_from_slice(&signature[..32]);
            signatures
                .extend_from_slice(&U256::from(static_len + dynamic.len()).to_be_bytes::<32>());
            signatures.push(0);
            dynamic.extend_from_slice(&signature[65..]);
        }
        ensure!(!signatures.is_empty(), "Safe transaction has no confirmations");
        signatures.extend_from_slice(&dynamic);
        Ok(signatures.into())
    }

    pub(super) fn show_transaction_summary(&self) -> Result<()> {
        let operation = if self.operation == 0 { "CALL" } else { "DELEGATECALL" };
        sh_status!("Safe transaction: {}", self.safe_tx_hash)?;
        sh_status!("  Safe:            {}", self.safe)?;
        sh_status!("  To:              {}", self.to)?;
        sh_status!("  Value:           {}", self.value)?;
        sh_status!("  Operation:       {} ({operation})", self.operation)?;
        sh_status!("  Safe tx gas:     {}", self.safe_tx_gas)?;
        sh_status!("  Base gas:        {}", self.base_gas)?;
        sh_status!("  Gas price:       {}", self.gas_price)?;
        sh_status!("  Gas token:       {}", self.gas_token)?;
        sh_status!("  Refund receiver: {}", self.refund_receiver)?;
        sh_status!("  Nonce:           {}", self.nonce)?;
        sh_status!("  Data:            {}", self.data)?;
        Ok(())
    }
}

impl SafeServiceOpts {
    pub(super) fn endpoint(&self, chain_id: u64, path: &str) -> Result<Url> {
        let mut url = match &self.service_url {
            Some(url) => url.clone(),
            None => default_service_url(chain_id)?,
        };
        let mut endpoint = url.path().trim_end_matches('/').to_string();
        if !endpoint.ends_with("/api") {
            endpoint.push_str("/api");
        }
        endpoint.push('/');
        endpoint.push_str(path.trim_start_matches('/'));
        url.set_path(&endpoint);
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    pub(super) fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        let request = Client::new()
            .request(method, url)
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(api_key) = &self.api_key { request.bearer_auth(api_key) } else { request }
    }

    pub(super) async fn response<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T> {
        let response = request.send().await.wrap_err("Safe Transaction Service request failed")?;
        let status = response.status();
        let text = response.text().await.wrap_err("failed to read Transaction Service response")?;
        ensure_success(status, &text)?;
        serde_json::from_str(&text).wrap_err("invalid Transaction Service response")
    }

    pub(super) async fn empty_response(&self, request: reqwest::RequestBuilder) -> Result<()> {
        let response = request.send().await.wrap_err("Safe Transaction Service request failed")?;
        let status = response.status();
        let text = response.text().await.wrap_err("failed to read Transaction Service response")?;
        ensure_success(status, &text)
    }

    pub(super) async fn next_nonce(
        &self,
        chain_id: u64,
        safe: Address,
        onchain_nonce: U256,
    ) -> Result<U256> {
        let response: Value = self
            .response(self.request(Method::GET, self.next_nonce_endpoint(chain_id, safe)?))
            .await?;
        let Some(nonce) = response
            .get("results")
            .and_then(Value::as_array)
            .and_then(|results| results.first())
            .and_then(|tx| tx.get("nonce"))
        else {
            return Ok(onchain_nonce);
        };
        let nonce = match nonce {
            Value::String(nonce) => U256::from_str(nonce),
            Value::Number(nonce) => U256::from_str(&nonce.to_string()),
            _ => eyre::bail!("invalid nonce returned by Safe Transaction Service"),
        }
        .wrap_err("invalid nonce returned by Safe Transaction Service")?;
        Ok(std::cmp::max(onchain_nonce, nonce.saturating_add(U256::from(1))))
    }

    fn next_nonce_endpoint(&self, chain_id: u64, safe: Address) -> Result<Url> {
        let mut url = self.endpoint(
            chain_id,
            &format!("v1/safes/{}/multisig-transactions/", safe.to_checksum(None)),
        )?;
        url.query_pairs_mut()
            .append_pair("executed", "false")
            .append_pair("ordering", "-nonce")
            .append_pair("limit", "1");
        Ok(url)
    }
}

fn ensure_success(status: StatusCode, body: &str) -> Result<()> {
    ensure!(status.is_success(), "Safe Transaction Service returned {status}: {}", body.trim());
    Ok(())
}

fn default_service_url(chain_id: u64) -> Result<Url> {
    let short_name = match chain_id {
        1 => "eth",
        10 => "oeth",
        56 => "bnb",
        100 => "gno",
        130 => "unichain",
        137 => "pol",
        146 => "sonic",
        196 => "okb",
        232 => "lens",
        324 => "zksync",
        480 => "wc",
        4217 => "tempo",
        5000 => "mantle",
        8453 => "base",
        9745 => "plasma",
        10200 => "chi",
        42161 => "arb1",
        42220 => "celo",
        42431 => "tempo-moderato",
        43114 => "avax",
        43111 => "hemi",
        57073 => "ink",
        59144 => "linea",
        747474 => "katana",
        80094 => "berachain",
        84532 => "basesep",
        534352 => "scr",
        11155111 => "sep",
        1313161554 => "aurora",
        _ => eyre::bail!(
            "no known Safe Transaction Service for chain ID {chain_id}; pass --service-url"
        ),
    };
    format!("https://api.safe.global/tx-service/{short_name}")
        .parse()
        .wrap_err("invalid built-in Safe Transaction Service URL")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_provider::{ProviderBuilder, mock::Asserter};

    fn transaction() -> SafeTransaction {
        SafeTransaction {
            safe: Address::ZERO,
            to: Address::ZERO,
            value: "1".to_string(),
            data: Bytes::new(),
            operation: 0,
            safe_tx_gas: "0".to_string(),
            base_gas: "0".to_string(),
            gas_price: "0".to_string(),
            gas_token: Address::ZERO,
            refund_receiver: Address::ZERO,
            nonce: "7".to_string(),
            safe_tx_hash: B256::ZERO,
            confirmations: Vec::new(),
            is_executed: false,
            transaction_hash: None,
        }
    }

    #[test]
    fn normalizes_transaction_service_urls() {
        for base in [
            "https://api.safe.global/tx-service/tempo-moderato/",
            "https://api.safe.global/tx-service/tempo-moderato/api",
        ] {
            let service =
                SafeServiceOpts { service_url: Some(base.parse().unwrap()), api_key: None };
            assert_eq!(
                service.endpoint(42431, "v2/delegates/").unwrap().as_str(),
                "https://api.safe.global/tx-service/tempo-moderato/api/v2/delegates/"
            );
        }
    }

    #[test]
    fn infers_tempo_transaction_service_urls() {
        assert_eq!(
            default_service_url(4217).unwrap().as_str(),
            "https://api.safe.global/tx-service/tempo"
        );
        assert_eq!(
            default_service_url(42431).unwrap().as_str(),
            "https://api.safe.global/tx-service/tempo-moderato"
        );
        for chain_id in [1101, 81457, 31337] {
            assert!(default_service_url(chain_id).is_err());
        }
    }

    #[test]
    fn builds_pending_nonce_query_as_url_parameters() {
        let service = SafeServiceOpts { service_url: None, api_key: None };
        assert_eq!(
            service.next_nonce_endpoint(42431, Address::ZERO).unwrap().as_str(),
            "https://api.safe.global/tx-service/tempo-moderato/api/v1/safes/0x0000000000000000000000000000000000000000/multisig-transactions/?executed=false&ordering=-nonce&limit=1"
        );
    }

    #[test]
    fn parses_transaction_service_response() {
        let transaction: SafeTransaction = serde_json::from_value(json!({
            "safe": Address::ZERO,
            "to": Address::ZERO,
            "value": "1",
            "data": null,
            "operation": 0,
            "safeTxGas": 0,
            "baseGas": "0",
            "gasPrice": 0,
            "gasToken": null,
            "refundReceiver": null,
            "nonce": 7,
            "safeTxHash": B256::ZERO,
        }))
        .unwrap();

        assert_eq!(transaction.data, Bytes::new());
        assert_eq!(transaction.nonce, "7");
        assert_eq!(transaction.gas_token, Address::ZERO);
        assert_eq!(transaction.refund_receiver, Address::ZERO);
    }

    #[test]
    fn packs_confirmations_in_owner_order() {
        let mut transaction = transaction();
        transaction.confirmations = vec![
            SafeConfirmation {
                owner: Address::repeat_byte(2),
                signature: Some(Bytes::from(vec![2; 65])),
            },
            SafeConfirmation {
                owner: Address::repeat_byte(1),
                signature: Some(Bytes::from(vec![1; 65])),
            },
        ];

        let signatures = transaction.packed_signatures().unwrap();
        assert_eq!(&signatures[..65], &[1; 65]);
        assert_eq!(&signatures[65..], &[2; 65]);
    }

    #[test]
    fn packs_contract_signatures_after_static_signatures() {
        let contract_signature = |owner: Address, payload: &[u8]| {
            let mut signature = Vec::with_capacity(65 + 32 + payload.len());
            signature.extend_from_slice(owner.into_word().as_slice());
            signature.extend_from_slice(&U256::from(65).to_be_bytes::<32>());
            signature.push(0);
            signature.extend_from_slice(&U256::from(payload.len()).to_be_bytes::<32>());
            signature.extend_from_slice(payload);
            Bytes::from(signature)
        };
        let first_owner = Address::repeat_byte(1);
        let third_owner = Address::repeat_byte(3);
        let first_payload = [4, 5, 6];
        let third_payload = [7, 8, 9, 10];

        let mut transaction = transaction();
        transaction.confirmations = vec![
            SafeConfirmation {
                owner: third_owner,
                signature: Some(contract_signature(third_owner, &third_payload)),
            },
            SafeConfirmation {
                owner: Address::repeat_byte(2),
                signature: Some(Bytes::from(vec![2; 65])),
            },
            SafeConfirmation {
                owner: first_owner,
                signature: Some(contract_signature(first_owner, &first_payload)),
            },
        ];

        let signatures = transaction.packed_signatures().unwrap();
        assert_eq!(&signatures[..32], first_owner.into_word().as_slice());
        assert_eq!(U256::from_be_slice(&signatures[32..64]), U256::from(195));
        assert_eq!(signatures[64], 0);
        assert_eq!(&signatures[65..130], &[2; 65]);
        assert_eq!(&signatures[130..162], third_owner.into_word().as_slice());
        assert_eq!(U256::from_be_slice(&signatures[162..194]), U256::from(230));
        assert_eq!(signatures[194], 0);
        assert_eq!(U256::from_be_slice(&signatures[195..227]), U256::from(3));
        assert_eq!(&signatures[227..230], &first_payload);
        assert_eq!(U256::from_be_slice(&signatures[230..262]), U256::from(4));
        assert_eq!(&signatures[262..], &third_payload);
    }

    #[test]
    fn rejects_malformed_confirmation_signatures() {
        let owner = Address::repeat_byte(1);
        let mut wrong_offset = vec![0; 97];
        wrong_offset[32..64].copy_from_slice(&U256::from(66).to_be_bytes::<32>());
        let mut wrong_length = vec![0; 97];
        wrong_length[32..64].copy_from_slice(&U256::from(65).to_be_bytes::<32>());
        wrong_length[65..97].copy_from_slice(&U256::from(1).to_be_bytes::<32>());

        for signature in [vec![1; 66], vec![0; 65], wrong_offset, wrong_length] {
            let mut transaction = transaction();
            transaction.confirmations =
                vec![SafeConfirmation { owner, signature: Some(signature.into()) }];
            assert!(transaction.packed_signatures().is_err());
        }
    }

    #[tokio::test]
    async fn calculates_hash_with_safe_contract() {
        let expected = B256::repeat_byte(0x42);
        let asserter = Asserter::new();
        asserter.push_success(&expected);
        let provider = ProviderBuilder::new().connect_mocked_client(asserter);

        assert_eq!(transaction().calculate_hash(&provider).await.unwrap(), expected);
    }
}
