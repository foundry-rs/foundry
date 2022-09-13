use crate::cmd::forge::script::ScriptResult;
use cast::{executor::inspector::DEFAULT_CREATE2_DEPLOYER, trace::CallTraceDecoder, CallKind};
use ethers::{
    abi,
    abi::{Address, Contract as Abi},
    prelude::{NameOrAddress, H256 as TxHash},
    types::transaction::eip2718::TypedTransaction,
};
use foundry_common::SELECTOR_LEN;
use foundry_utils::format_token;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::error;

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    #[serde(serialize_with = "wrapper::serialize_addr")]
    pub address: Address,
    #[serde(with = "hex")]
    pub init_code: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<TxHash>,
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    #[serde(default = "default_string")]
    pub contract_name: Option<String>,
    #[serde(default = "default_address", serialize_with = "wrapper::serialize_opt_addr")]
    pub contract_address: Option<Address>,
    #[serde(default = "default_string")]
    pub function: Option<String>,
    #[serde(default = "default_vec_of_strings")]
    pub arguments: Option<Vec<String>>,
    pub transaction: TypedTransaction,
    pub additional_contracts: Vec<AdditionalContract>,
}

fn default_string() -> Option<String> {
    Some("".to_string())
}

fn default_address() -> Option<Address> {
    Some(Address::zero())
}

fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl TransactionWithMetadata {
    pub fn from_typed_transaction(transaction: TypedTransaction) -> Self {
        Self { transaction, ..Default::default() }
    }

    pub fn new(
        transaction: TypedTransaction,
        result: &ScriptResult,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
        additional_contracts: Vec<AdditionalContract>,
    ) -> eyre::Result<Self> {
        let mut metadata = Self { transaction, ..Default::default() };

        // Specify if any contract was directly created with this transaction
        if let Some(NameOrAddress::Address(to)) = metadata.transaction.to().cloned() {
            if to == DEFAULT_CREATE2_DEPLOYER {
                metadata.set_create(
                    true,
                    Address::from_slice(&result.returned),
                    local_contracts,
                    decoder,
                )?;
            } else {
                metadata.set_call(to, local_contracts, decoder)?;
            }
        } else if metadata.transaction.to().is_none() {
            metadata.set_create(
                false,
                result.address.expect("There should be a contract address."),
                local_contracts,
                decoder,
            )?;
        }

        // Add the additional contracts created in this transaction, so we can verify them later.
        if let Some(tx_address) = metadata.contract_address {
            metadata.additional_contracts = additional_contracts
                .into_iter()
                .filter_map(|contract| {
                    // Filter out the transaction contract repeated init_code.
                    if contract.address != tx_address {
                        Some(contract)
                    } else {
                        None
                    }
                })
                .collect();
        }

        Ok(metadata)
    }

    /// Populate the transaction as CREATE tx
    ///
    /// If this is a CREATE2 transaction this attempt to decode the arguments from the CREATE2
    /// deployer's function
    fn set_create(
        &mut self,
        is_create2: bool,
        address: Address,
        contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<()> {
        if is_create2 {
            self.opcode = CallKind::Create2;
        } else {
            self.opcode = CallKind::Create;
        }

        self.contract_name = contracts.get(&address).map(|(name, _)| name.clone());
        self.contract_address = Some(address);

        if let Some(data) = self.transaction.data() {
            // a CREATE2 transaction is a CALL to the CREATE2 deployer function, so we need to
            // decode the arguments  from the function call
            if is_create2 {
                // this will be the `deploy()` function of the CREATE2 call, we assume the contract
                // is already deployed and will try to fetch it via its signature
                if let Some(Some(function)) = decoder
                    .functions
                    .get(&data.0[..SELECTOR_LEN])
                    .map(|functions| functions.first())
                {
                    self.function = Some(function.signature());
                    self.arguments = Some(
                        function
                            .decode_input(&data.0[SELECTOR_LEN..])
                            .map(|tokens| tokens.iter().map(format_token).collect())
                            .map_err(|_| eyre::eyre!("Failed to decode CREATE2 call arguments"))?,
                    );
                }
            } else {
                // This is a regular CREATE via the constructor, in which case we expect the
                // contract has a constructor and if so decode its input params
                if let Some((_, abi)) = contracts.get(&address) {
                    // set constructor arguments
                    if let Some(constructor) = abi.constructor() {
                        let params =
                            constructor.inputs.iter().map(|p| p.kind.clone()).collect::<Vec<_>>();

                        if let Some(constructor_args) = find_constructor_args(&data.0) {
                            self.arguments = Some(
                                abi::decode(&params, constructor_args)
                                    .map_err(|_| {
                                        eyre::eyre!("Failed to decode constructor arguments")
                                    })?
                                    .iter()
                                    .map(format_token)
                                    .collect(),
                            );
                        } else {
                            error!("Failed to extract constructor args from CREATE data")
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Populate the transaction as CALL tx
    fn set_call(
        &mut self,
        target: Address,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<()> {
        self.opcode = CallKind::Call;

        if let Some(data) = self.transaction.data() {
            if data.0.len() >= SELECTOR_LEN {
                if let Some((contract_name, abi)) = local_contracts.get(&target) {
                    // This CALL is made to a local contract.

                    self.contract_name = Some(contract_name.clone());
                    if let Some(function) = abi
                        .functions()
                        .find(|function| function.short_signature() == data.0[..SELECTOR_LEN])
                    {
                        self.function = Some(function.signature());
                        self.arguments = Some(
                            function
                                .decode_input(&data.0[SELECTOR_LEN..])
                                .map(|tokens| tokens.iter().map(format_token).collect())?,
                        );
                    }
                } else {
                    // This CALL is made to an external contract. We can only decode it, if it has
                    // been verified and identified by etherscan.

                    if let Some(Some(function)) = decoder
                        .functions
                        .get(&data.0[..SELECTOR_LEN])
                        .map(|functions| functions.first())
                    {
                        self.contract_name = decoder.contracts.get(&target).cloned();

                        self.function = Some(function.signature());
                        self.arguments = Some(
                            function
                                .decode_input(&data.0[SELECTOR_LEN..])
                                .map(|tokens| tokens.iter().map(format_token).collect())?,
                        );
                    }
                }
                self.contract_address = Some(target);
            }
        }
        Ok(())
    }

    pub fn set_tx(&mut self, tx: TypedTransaction) {
        self.transaction = tx;
    }

    pub fn change_type(&mut self, is_legacy: bool) {
        self.transaction = if is_legacy {
            TypedTransaction::Legacy(self.transaction.clone().into())
        } else {
            TypedTransaction::Eip1559(self.transaction.clone().into())
        };
    }

    pub fn typed_tx(&self) -> &TypedTransaction {
        &self.transaction
    }

    pub fn typed_tx_mut(&mut self) -> &mut TypedTransaction {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == CallKind::Create2
    }
}

/// Given the transaction data tries to identify the constructor arguments
/// The constructor data is encoded as: Constructor Code + Contract Code +  Constructor arguments
/// decoding the arguments here with only the transaction data is not trivial here, we try to find
/// the beginning of the constructor arguments by finding the length of the code, which is PUSH op
/// code which holds the code size and the code starts after the invalid op code (0xfe)
///
/// finding the `0xfe` (invalid opcode) in the data which should mark the beginning of constructor
/// arguments
fn find_constructor_args(data: &[u8]) -> Option<&[u8]> {
    // ref <https://ethereum.stackexchange.com/questions/126785/how-do-you-identify-the-start-of-constructor-arguments-for-contract-creation-cod>
    static CONSTRUCTOR_CODE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?m)(?:5b)?(?:60([a-z0-9]{2})|61([a-z0-9_]{4})|62([a-z0-9_]{6}))80(?:60([a-z0-9]{2})|61([a-z0-9_]{4})|62([a-z0-9_]{6}))(6000396000f3fe)").unwrap()
    });
    let s = hex::encode(data);

    // we're only interested in the last occurrence which skips additional CREATE inside the
    // constructor itself
    let caps = CONSTRUCTOR_CODE_RE.captures_iter(&s).last()?;

    let contract_len = u64::from_str_radix(
        caps.get(1).or_else(|| caps.get(2)).or_else(|| caps.get(2))?.as_str(),
        16,
    )
    .unwrap();

    // the end position of the constructor code, we use this instead of the contract offset , since
    // there could be multiple CREATE inside the data we need to divide by 2 for hex conversion
    let constructor_end = (caps.get(7)?.end() / 2) as u64;
    let start = (contract_len + constructor_end) as usize;
    let args = &data[start..];

    Some(args)
}

// wrapper for modifying ethers-rs type serialization
pub mod wrapper {
    pub use super::*;

    use ethers::{
        types::{Bloom, Bytes, Log, TransactionReceipt, H256, U256, U64},
        utils::to_checksum,
    };

    pub fn serialize_addr<S>(addr: &Address, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&to_checksum(addr, None))
    }

    pub fn serialize_opt_addr<S>(opt: &Option<Address>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match opt {
            Some(addr) => serialize_addr(addr, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn serialize_vec_with_wrapped<S, T, WrappedType>(
        vec: &[T],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        T: Clone,
        WrappedType: serde::Serialize + From<T>,
    {
        serializer.collect_seq(vec.iter().cloned().map(WrappedType::from))
    }

    // copied from https://github.com/gakonst/ethers-rs
    #[derive(Serialize, Deserialize)]
    struct WrappedLog {
        /// H160. the contract that emitted the log
        #[serde(serialize_with = "serialize_addr")]
        pub address: Address,

        /// topics: Array of 0 to 4 32 Bytes of indexed log arguments.
        /// (In solidity: The first topic is the hash of the signature of the event
        /// (e.g. `Deposit(address,bytes32,uint256)`), except you declared the event
        /// with the anonymous specifier.)
        pub topics: Vec<H256>,

        /// Data
        pub data: Bytes,

        /// Block Hash
        #[serde(rename = "blockHash")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub block_hash: Option<H256>,

        /// Block Number
        #[serde(rename = "blockNumber")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub block_number: Option<U64>,

        /// Transaction Hash
        #[serde(rename = "transactionHash")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_hash: Option<H256>,

        /// Transaction Index
        #[serde(rename = "transactionIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_index: Option<U64>,

        /// Integer of the log index position in the block. None if it's a pending log.
        #[serde(rename = "logIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub log_index: Option<U256>,

        /// Integer of the transactions index position log was created from.
        /// None when it's a pending log.
        #[serde(rename = "transactionLogIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_log_index: Option<U256>,

        /// Log Type
        #[serde(rename = "logType")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub log_type: Option<String>,

        /// True when the log was removed, due to a chain reorganization.
        /// false if it's a valid log.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub removed: Option<bool>,
    }
    impl From<Log> for WrappedLog {
        fn from(log: Log) -> Self {
            Self {
                address: log.address,
                topics: log.topics,
                data: log.data,
                block_hash: log.block_hash,
                block_number: log.block_number,
                transaction_hash: log.transaction_hash,
                transaction_index: log.transaction_index,
                log_index: log.log_index,
                transaction_log_index: log.transaction_log_index,
                log_type: log.log_type,
                removed: log.removed,
            }
        }
    }

    fn serialize_logs<S: serde::Serializer>(
        logs: &[Log],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_vec_with_wrapped::<S, Log, WrappedLog>(logs, serializer)
    }

    // "Receipt" of an executed transaction: details of its execution.
    // copied from https://github.com/gakonst/ethers-rs
    #[derive(Default, Clone, Serialize, Deserialize)]
    pub struct WrappedTransactionReceipt {
        /// Transaction hash.
        #[serde(rename = "transactionHash")]
        pub transaction_hash: H256,
        /// Index within the block.
        #[serde(rename = "transactionIndex")]
        pub transaction_index: U64,
        /// Hash of the block this transaction was included within.
        #[serde(rename = "blockHash")]
        pub block_hash: Option<H256>,
        /// Number of the block this transaction was included within.
        #[serde(rename = "blockNumber")]
        pub block_number: Option<U64>,
        /// address of the sender.
        #[serde(serialize_with = "serialize_addr")]
        pub from: Address,
        // address of the receiver. null when its a contract creation transaction.
        #[serde(serialize_with = "serialize_opt_addr")]
        pub to: Option<Address>,
        /// Cumulative gas used within the block after this was executed.
        #[serde(rename = "cumulativeGasUsed")]
        pub cumulative_gas_used: U256,
        /// Gas used by this transaction alone.
        ///
        /// Gas used is `None` if the the client is running in light client mode.
        #[serde(rename = "gasUsed")]
        pub gas_used: Option<U256>,
        /// Contract address created, or `None` if not a deployment.
        #[serde(rename = "contractAddress", serialize_with = "serialize_opt_addr")]
        pub contract_address: Option<Address>,
        /// Logs generated within this transaction.
        #[serde(serialize_with = "serialize_logs")]
        pub logs: Vec<Log>,
        /// Status: either 1 (success) or 0 (failure). Only present after activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
        pub status: Option<U64>,
        /// State root. Only present before activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub root: Option<H256>,
        /// Logs bloom
        #[serde(rename = "logsBloom")]
        pub logs_bloom: Bloom,
        /// Transaction type, Some(1) for AccessList transaction, None for Legacy
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        pub transaction_type: Option<U64>,
        /// The price paid post-execution by the transaction (i.e. base fee + priority fee).
        /// Both fields in 1559-style transactions are *maximums* (max fee + max priority fee), the
        /// amount that's actually paid by users can only be determined post-execution
        #[serde(rename = "effectiveGasPrice", default, skip_serializing_if = "Option::is_none")]
        pub effective_gas_price: Option<U256>,
    }
    impl From<TransactionReceipt> for WrappedTransactionReceipt {
        fn from(receipt: TransactionReceipt) -> Self {
            Self {
                transaction_hash: receipt.transaction_hash,
                transaction_index: receipt.transaction_index,
                block_hash: receipt.block_hash,
                block_number: receipt.block_number,
                from: receipt.from,
                to: receipt.to,
                cumulative_gas_used: receipt.cumulative_gas_used,
                gas_used: receipt.gas_used,
                contract_address: receipt.contract_address,
                logs: receipt.logs,
                status: receipt.status,
                root: receipt.root,
                logs_bloom: receipt.logs_bloom,
                transaction_type: receipt.transaction_type,
                effective_gas_price: receipt.effective_gas_price,
            }
        }
    }

    pub fn serialize_receipts<S: serde::Serializer>(
        receipts: &[TransactionReceipt],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_vec_with_wrapped::<S, TransactionReceipt, WrappedTransactionReceipt>(
            receipts, serializer,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::abi::ParamType;

    // <https://github.com/foundry-rs/foundry/issues/3053>
    #[test]
    fn test_find_constructor_args() {
        let code = "6080604052348015600f57600080fd5b50604051610121380380610121833981016040819052602c91606e565b600080546001600160a01b0319166001600160a01b0396909616959095179094556001929092556002556003556004805460ff191691151591909117905560d4565b600080600080600060a08688031215608557600080fd5b85516001600160a01b0381168114609b57600080fd5b809550506020860151935060408601519250606086015191506080860151801515811460c657600080fd5b809150509295509295909350565b603f806100e26000396000f3fe6080604052600080fdfea264697066735822122089f2c61beace50d105ec1b6a56a1204301b5595e850e7576f6f3aa8e76f12d0b64736f6c6343000810003300000000000000000000000000a329c0648769a73afac7f9381e08fb43dbea720000000000000000000000000000000000000000000000000000000100000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf60000000000000000000000000000000000000000000000000000000000000001";

        let code = hex::decode(code).unwrap();

        let args = find_constructor_args(&code).unwrap();

        let params = vec![
            ParamType::Address,
            ParamType::Uint(256),
            ParamType::Int(256),
            ParamType::FixedBytes(32),
            ParamType::Bool,
        ];

        let _decoded = abi::decode(&params, args).unwrap();
    }

    #[test]
    fn test_find_constructor_args_nested_deploy() {
        let code = "608060405234801561001057600080fd5b5060405161066d38038061066d83398101604081905261002f9161014a565b868686868686866040516100429061007c565b610052979695949392919061022f565b604051809103906000f08015801561006e573d6000803e3d6000fd5b50505050505050505061028a565b610396806102d783390190565b634e487b7160e01b600052604160045260246000fd5b60005b838110156100ba5781810151838201526020016100a2565b50506000910152565b600082601f8301126100d457600080fd5b81516001600160401b03808211156100ee576100ee610089565b604051601f8301601f19908116603f0116810190828211818310171561011657610116610089565b8160405283815286602085880101111561012f57600080fd5b61014084602083016020890161009f565b9695505050505050565b600080600080600080600060e0888a03121561016557600080fd5b87516001600160a01b038116811461017c57600080fd5b80975050602088015195506040880151945060608801519350608088015180151581146101a857600080fd5b60a08901519093506001600160401b03808211156101c557600080fd5b6101d18b838c016100c3565b935060c08a01519150808211156101e757600080fd5b506101f48a828b016100c3565b91505092959891949750929550565b6000815180845261021b81602086016020860161009f565b601f01601f19169290920160200192915050565b60018060a01b0388168152866020820152856040820152846060820152831515608082015260e060a0820152600061026a60e0830185610203565b82810360c084015261027c8185610203565b9a9950505050505050505050565b603f806102986000396000f3fe6080604052600080fdfea264697066735822122072aeef1567521008007b956bd7c6e9101a9b49fbce1f45210fa929c79d28bd9364736f6c63430008110033608060405234801561001057600080fd5b5060405161039638038061039683398101604081905261002f91610148565b600080546001600160a01b0319166001600160a01b0389161790556001869055600285905560038490556004805460ff19168415151790556005610073838261028a565b506006610080828261028a565b5050505050505050610349565b634e487b7160e01b600052604160045260246000fd5b600082601f8301126100b457600080fd5b81516001600160401b03808211156100ce576100ce61008d565b604051601f8301601f19908116603f011681019082821181831017156100f6576100f661008d565b8160405283815260209250868385880101111561011257600080fd5b600091505b838210156101345785820183015181830184015290820190610117565b600093810190920192909252949350505050565b600080600080600080600060e0888a03121561016357600080fd5b87516001600160a01b038116811461017a57600080fd5b80975050602088015195506040880151945060608801519350608088015180151581146101a657600080fd5b60a08901519093506001600160401b03808211156101c357600080fd5b6101cf8b838c016100a3565b935060c08a01519150808211156101e557600080fd5b506101f28a828b016100a3565b91505092959891949750929550565b600181811c9082168061021557607f821691505b60208210810361023557634e487b7160e01b600052602260045260246000fd5b50919050565b601f82111561028557600081815260208120601f850160051c810160208610156102625750805b601f850160051c820191505b818110156102815782815560010161026e565b5050505b505050565b81516001600160401b038111156102a3576102a361008d565b6102b7816102b18454610201565b8461023b565b602080601f8311600181146102ec57600084156102d45750858301515b600019600386901b1c1916600185901b178555610281565b600085815260208120601f198616915b8281101561031b578886015182559484019460019091019084016102fc565b50858210156103395787850151600019600388901b60f8161c191681555b5050505050600190811b01905550565b603f806103576000396000f3fe6080604052600080fdfea2646970667358221220a468ac913d3ecf191b6559ae7dca58e05ba048434318f393b86640b25cbbf1ed64736f6c6343000811003300000000000000000000000000a329c0648769a73afac7f9381e08fb43dbea720000000000000000000000000000000000000000000000000000000100000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000066162636465660000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000568656c6c6f000000000000000000000000000000000000000000000000000000";

        let code = hex::decode(code).unwrap();

        let args = find_constructor_args(&code).unwrap();

        let params = vec![
            ParamType::Address,
            ParamType::Uint(256),
            ParamType::Int(256),
            ParamType::FixedBytes(32),
            ParamType::Bool,
            ParamType::Bytes,
            ParamType::String,
        ];

        let _decoded = abi::decode(&params, args).unwrap();
    }
}
