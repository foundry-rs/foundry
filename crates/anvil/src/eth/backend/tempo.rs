use std::{borrow::Cow, collections::HashMap, sync::Arc};

use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_primitives::{Address, B256, Bytes, Log, U256, hex, keccak256};
use parking_lot::Mutex;
use reqwest::Client;
use revm::precompile::{PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult};
use serde_json::{Value, json};

const GAS_READ: u64 = 12_000;
const GAS_WRITE: u64 = 35_000;
const TRANSFER_TOPIC: B256 = B256::new(hex!(
    "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
));
const APPROVAL_TOPIC: B256 = B256::new(hex!(
    "8c5be1e5ebec7d5bd14f714f7e6f8c3cbbf8e5e4a4b9838f5dbdb9fbf1c4f7b5"
));

const SELECTOR_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
const SELECTOR_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
const SELECTOR_ALLOWANCE: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];
const SELECTOR_APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
const SELECTOR_TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
const SELECTOR_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];

pub fn precompiles_from_env(
    chain_id: u64,
    rpc_url: String,
    block_number: u64,
) -> Vec<(Address, DynPrecompile)> {
    if chain_id != 4217 {
        return Vec::new();
    }

    let Some(raw) = std::env::var("ANVIL_TEMPO_TOKENS").ok() else {
        return Vec::new();
    };

    raw.split(',')
        .filter_map(|item| {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed.parse::<Address>().ok()
        })
        .map(|token| {
            let precompile =
                TempoTokenPrecompile::new(token, rpc_url.clone(), block_number).into_dyn();
            (token, precompile)
        })
        .collect()
}

struct TempoTokenPrecompile {
    token: Address,
    rpc_url: String,
    block_number: u64,
    client: Client,
    metadata: Mutex<HashMap<[u8; 4], Bytes>>,
}

impl TempoTokenPrecompile {
    fn new(token: Address, rpc_url: String, block_number: u64) -> Self {
        Self {
            token,
            rpc_url,
            block_number,
            client: Client::new(),
            metadata: Mutex::new(HashMap::new()),
        }
    }

    fn into_dyn(self) -> DynPrecompile {
        let this = Arc::new(self);
        let id = PrecompileId::Custom(Cow::Owned(format!("tempo tip20 {:#x}", this.token)));
        DynPrecompile::new_stateful(id, move |input| this.call(input))
    }

    fn call(&self, mut input: PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() < 4 {
            return Err(other("missing selector"));
        }
        let selector: [u8; 4] = input.data[0..4].try_into().expect("selector length");
        match selector {
            SELECTOR_BALANCE_OF => self.balance_of(&mut input),
            SELECTOR_TRANSFER => self.transfer(&mut input),
            SELECTOR_ALLOWANCE => self.allowance(&mut input),
            SELECTOR_APPROVE => self.approve(&mut input),
            SELECTOR_TRANSFER_FROM => self.transfer_from(&mut input),
            SELECTOR_DECIMALS => self.metadata_call(selector),
            _ => Err(other(format!(
                "unsupported Tempo TIP-20 selector 0x{} for token {:#x}",
                hex::encode(selector),
                self.token
            ))),
        }
    }

    fn balance_of(&self, input: &mut PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() != 36 {
            return Err(other("balanceOf expects 1 address arg"));
        }
        let owner = decode_address(&input.data[4..36])?;
        let balance = self.ensure_balance(input, owner)?;
        Ok(PrecompileOutput::new(GAS_READ, encode_u256(balance)))
    }

    fn allowance(&self, input: &mut PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() != 68 {
            return Err(other("allowance expects 2 address args"));
        }
        let owner = decode_address(&input.data[4..36])?;
        let spender = decode_address(&input.data[36..68])?;
        let allowance = self.ensure_allowance(input, owner, spender)?;
        Ok(PrecompileOutput::new(GAS_READ, encode_u256(allowance)))
    }

    fn approve(&self, input: &mut PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() != 68 {
            return Err(other("approve expects address,uint256"));
        }
        let spender = decode_address(&input.data[4..36])?;
        let value = decode_u256(&input.data[36..68]);
        self.set_allowance(input, input.caller, spender, value)?;
        self.log_approval(input, input.caller, spender, value);
        Ok(PrecompileOutput::new(GAS_WRITE, encode_bool(true)))
    }

    fn transfer(&self, input: &mut PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() != 68 {
            return Err(other("transfer expects address,uint256"));
        }
        let to = decode_address(&input.data[4..36])?;
        let amount = decode_u256(&input.data[36..68]);
        self.move_balance(input, input.caller, to, amount)?;
        Ok(PrecompileOutput::new(GAS_WRITE, encode_bool(true)))
    }

    fn transfer_from(&self, input: &mut PrecompileInput<'_>) -> PrecompileResult {
        if input.data.len() != 100 {
            return Err(other("transferFrom expects address,address,uint256"));
        }
        let from = decode_address(&input.data[4..36])?;
        let to = decode_address(&input.data[36..68])?;
        let amount = decode_u256(&input.data[68..100]);
        let allowance = self.ensure_allowance(input, from, input.caller)?;
        if allowance < amount {
            return Err(other("Tempo TIP-20: insufficient allowance"));
        }
        if allowance != U256::MAX {
            self.set_allowance(input, from, input.caller, allowance - amount)?;
        }
        self.move_balance(input, from, to, amount)?;
        Ok(PrecompileOutput::new(GAS_WRITE, encode_bool(true)))
    }

    fn metadata_call(&self, selector: [u8; 4]) -> PrecompileResult {
        if let Some(cached) = self.metadata.lock().get(&selector).cloned() {
            return Ok(PrecompileOutput::new(GAS_READ, cached));
        }

        let output = self
            .rpc_call(Bytes::copy_from_slice(&selector))
            .map_err(other)?;
        self.metadata.lock().insert(selector, output.clone());
        Ok(PrecompileOutput::new(GAS_READ, output))
    }

    fn move_balance(
        &self,
        input: &mut PrecompileInput<'_>,
        from: Address,
        to: Address,
        amount: U256,
    ) -> Result<(), PrecompileError> {
        let from_balance = self.ensure_balance(input, from)?;
        if from_balance < amount {
            return Err(other("Tempo TIP-20: insufficient balance"));
        }
        let to_balance = self.ensure_balance(input, to)?;
        let new_to = to_balance
            .checked_add(amount)
            .ok_or_else(|| other("Tempo TIP-20: balance overflow"))?;
        self.store_balance(input, from, from_balance - amount)?;
        self.store_balance(input, to, new_to)?;
        self.log_transfer(input, from, to, amount);
        Ok(())
    }

    fn ensure_balance(
        &self,
        input: &mut PrecompileInput<'_>,
        owner: Address,
    ) -> Result<U256, PrecompileError> {
        let init_key = storage_key("tempo.balance.init", &[owner.as_slice()]);
        let balance_key = storage_key("tempo.balance", &[owner.as_slice()]);
        let internals = input.internals_mut();
        let initialized = internals
            .sload(self.token, init_key)
            .map_err(|err| other(err.to_string()))?
            .data;
        if initialized == U256::ZERO {
            let call = encode_address_call(SELECTOR_BALANCE_OF, owner);
            let value = decode_rpc_u256(&self.rpc_call(call).map_err(other)?)?;
            internals
                .sstore(self.token, balance_key, value)
                .map_err(|err| other(err.to_string()))?;
            internals
                .sstore(self.token, init_key, U256::from(1))
                .map_err(|err| other(err.to_string()))?;
            return Ok(value);
        }
        Ok(
            internals
                .sload(self.token, balance_key)
                .map_err(|err| other(err.to_string()))?
                .data,
        )
    }

    fn store_balance(
        &self,
        input: &mut PrecompileInput<'_>,
        owner: Address,
        value: U256,
    ) -> Result<(), PrecompileError> {
        let key = storage_key("tempo.balance", &[owner.as_slice()]);
        input.internals_mut()
            .sstore(self.token, key, value)
            .map_err(|err| other(err.to_string()))?;
        Ok(())
    }

    fn ensure_allowance(
        &self,
        input: &mut PrecompileInput<'_>,
        owner: Address,
        spender: Address,
    ) -> Result<U256, PrecompileError> {
        let init_key = storage_key("tempo.allowance.init", &[owner.as_slice(), spender.as_slice()]);
        let allowance_key = storage_key("tempo.allowance", &[owner.as_slice(), spender.as_slice()]);
        let internals = input.internals_mut();
        let initialized = internals
            .sload(self.token, init_key)
            .map_err(|err| other(err.to_string()))?
            .data;
        if initialized == U256::ZERO {
            let call = encode_two_address_call(SELECTOR_ALLOWANCE, owner, spender);
            let value = decode_rpc_u256(&self.rpc_call(call).map_err(other)?)?;
            internals
                .sstore(self.token, allowance_key, value)
                .map_err(|err| other(err.to_string()))?;
            internals
                .sstore(self.token, init_key, U256::from(1))
                .map_err(|err| other(err.to_string()))?;
            return Ok(value);
        }
        Ok(
            internals
                .sload(self.token, allowance_key)
                .map_err(|err| other(err.to_string()))?
                .data,
        )
    }

    fn set_allowance(
        &self,
        input: &mut PrecompileInput<'_>,
        owner: Address,
        spender: Address,
        value: U256,
    ) -> Result<(), PrecompileError> {
        let init_key = storage_key("tempo.allowance.init", &[owner.as_slice(), spender.as_slice()]);
        let allowance_key = storage_key("tempo.allowance", &[owner.as_slice(), spender.as_slice()]);
        let internals = input.internals_mut();
        internals
            .sstore(self.token, allowance_key, value)
            .map_err(|err| other(err.to_string()))?;
        internals
            .sstore(self.token, init_key, U256::from(1))
            .map_err(|err| other(err.to_string()))?;
        Ok(())
    }

    fn rpc_call(&self, data: Bytes) -> Result<Bytes, String> {
        let url = self.rpc_url.clone();
        let client = self.client.clone();
        let token = self.token;
        let block = format!("0x{:x}", self.block_number);
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let payload = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "eth_call",
                    "params": [
                        {
                            "to": format!("{:#x}", token),
                            "data": format!("0x{}", hex::encode(data)),
                        },
                        block,
                    ],
                });
                let response: reqwest::Response = client
                    .post(url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| err.to_string())?;
                let envelope: Value = response.json().await.map_err(|err| err.to_string())?;
                if let Some(error) = envelope.get("error") {
                    return Err(format!("Tempo token eth_call error: {error}"));
                }
                let result = envelope
                    .get("result")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Tempo token eth_call returned no result".to_string())?;
                let bytes = hex::decode(result.trim_start_matches("0x"))
                    .map_err(|err| err.to_string())?;
                Ok(Bytes::from(bytes))
            })
        })
    }

    fn log_transfer(&self, input: &mut PrecompileInput<'_>, from: Address, to: Address, value: U256) {
        input.internals_mut().log(Log::new_unchecked(
            self.token,
            vec![TRANSFER_TOPIC, topic_address(from), topic_address(to)],
            encode_u256(value),
        ));
    }

    fn log_approval(
        &self,
        input: &mut PrecompileInput<'_>,
        owner: Address,
        spender: Address,
        value: U256,
    ) {
        input.internals_mut().log(Log::new_unchecked(
            self.token,
            vec![APPROVAL_TOPIC, topic_address(owner), topic_address(spender)],
            encode_u256(value),
        ));
    }
}

fn decode_address(word: &[u8]) -> Result<Address, PrecompileError> {
    if word.len() != 32 {
        return Err(other("expected abi-encoded address word"));
    }
    Ok(Address::from_slice(&word[12..32]))
}

fn decode_u256(word: &[u8]) -> U256 {
    U256::from_be_slice(word)
}

fn decode_rpc_u256(bytes: &[u8]) -> Result<U256, PrecompileError> {
    if bytes.len() < 32 {
        return Err(other("expected at least 32 bytes from RPC"));
    }
    Ok(U256::from_be_slice(&bytes[bytes.len() - 32..]))
}

fn encode_u256(value: U256) -> Bytes {
    Bytes::from(value.to_be_bytes_vec())
}

fn encode_bool(value: bool) -> Bytes {
    let mut out = [0u8; 32];
    if value {
        out[31] = 1;
    }
    Bytes::copy_from_slice(&out)
}

fn encode_address_call(selector: [u8; 4], address: Address) -> Bytes {
    let mut out = Vec::with_capacity(4 + 32);
    out.extend_from_slice(&selector);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(address.as_slice());
    Bytes::from(out)
}

fn encode_two_address_call(selector: [u8; 4], first: Address, second: Address) -> Bytes {
    let mut out = Vec::with_capacity(4 + 64);
    out.extend_from_slice(&selector);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(first.as_slice());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(second.as_slice());
    Bytes::from(out)
}

fn storage_key(prefix: &str, parts: &[&[u8]]) -> U256 {
    let mut bytes = Vec::with_capacity(prefix.len() + parts.iter().map(|part| part.len()).sum::<usize>());
    bytes.extend_from_slice(prefix.as_bytes());
    for part in parts {
        bytes.extend_from_slice(part);
    }
    U256::from_be_bytes(keccak256(bytes).into())
}

fn topic_address(address: Address) -> B256 {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(address.as_slice());
    B256::new(out)
}

fn other(msg: impl Into<String>) -> PrecompileError {
    PrecompileError::Other(msg.into().into())
}
