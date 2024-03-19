use std::marker::PhantomData;

use crate::errors::FunctionSignatureError;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256, U64};
use alloy_provider::{
    network::{Ethereum, TransactionBuilder},
    Provider,
};
use alloy_rpc_types::request::{TransactionInput, TransactionRequest};
use alloy_transport::Transport;
use eyre::Result;
use foundry_common::{
    abi::{encode_function_args, get_func, get_func_etherscan},
    ens::NameOrAddress,
    types::ToEthers,
};
use foundry_config::Chain;
use futures::future::join_all;

pub type TxBuilderOutput = (TransactionRequest, Option<Function>);
pub type TxBuilderPeekOutput<'a> = (&'a TransactionRequest, &'a Option<Function>);

/// Transaction builder
///
/// # Examples
///
/// ```
/// # async fn foo() -> eyre::Result<()> {
/// # use alloy_primitives::{Address, U256};
/// # use cast::TxBuilder;
/// # use foundry_config::NamedChain;
/// # use std::str::FromStr;
/// let provider = foundry_common::provider::alloy::get_http_provider("http://localhost:8545");
/// let from = Address::from_str("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045").unwrap();
/// let to = Address::from_str("0xb8c2c29ee19d8307cb7255e1cd9cbde883a267d5").unwrap();
/// let mut builder = TxBuilder::new(&provider, from, Some(to), NamedChain::Mainnet, false).await?;
/// builder.gas(Some(U256::from(1)));
/// let (tx, _) = builder.build();
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct TxBuilder<'a, P, T> {
    to: Option<Address>,
    chain: Chain,
    tx: TransactionRequest,
    func: Option<Function>,
    etherscan_api_key: Option<String>,
    provider: &'a P,
    transport: PhantomData<T>,
}

impl<'a, T: Transport + Clone, P: Provider<Ethereum, T>> TxBuilder<'a, P, T> {
    /// Create a new TxBuilder
    /// `provider` - provider to use
    /// `from` - 'from' field. Could be an ENS name
    /// `to` - `to`. Could be a ENS
    /// `chain` - chain to construct the tx for
    /// `legacy` - use type 1 transaction
    pub async fn new(
        provider: &'a P,
        from: Address,
        to: Option<Address>,
        chain: impl Into<Chain>,
        legacy: bool,
    ) -> Result<Self> {
        let chain = chain.into();

        let tx = TransactionRequest::default()
            .from(from)
            .transaction_type(if chain.is_legacy() || legacy { 0 } else { 2 })
            .to(to);

        Ok(Self {
            to,
            chain,
            tx,
            func: None,
            etherscan_api_key: None,
            provider,
            transport: PhantomData,
        })
    }

    /// Set gas for tx
    pub fn set_gas(&mut self, v: U256) -> &mut Self {
        self.tx.set_gas_limit(v);
        self
    }

    /// Set gas for tx, if `v` is not None
    pub fn gas(&mut self, v: Option<U256>) -> &mut Self {
        if let Some(value) = v {
            self.set_gas(value);
        }
        self
    }

    /// Set gas price
    pub fn set_gas_price(&mut self, v: U256) -> &mut Self {
        self.tx.set_gas_price(v);
        self
    }

    /// Set gas price, if `v` is not None
    pub fn gas_price(&mut self, v: Option<U256>) -> &mut Self {
        if let Some(value) = v {
            self.set_gas_price(value);
        }
        self
    }

    /// Set priority gas price
    pub fn set_priority_gas_price(&mut self, v: U256) -> &mut Self {
        if let Some(2) = self.tx.transaction_type.map(|v| v.to::<u8>()) {
            self.tx.set_max_priority_fee_per_gas(v);
        }

        self
    }

    /// Set priority gas price, if `v` is not None
    pub fn priority_gas_price(&mut self, v: Option<U256>) -> &mut Self {
        if let Some(value) = v {
            self.set_priority_gas_price(value);
        }
        self
    }

    /// Set value
    pub fn set_value(&mut self, v: U256) -> &mut Self {
        self.tx.set_value(v);
        self
    }

    /// Set value, if `v` is not None
    pub fn value(&mut self, v: Option<U256>) -> &mut Self {
        if let Some(value) = v {
            self.set_value(value);
        }
        self
    }

    /// Set nonce
    pub fn set_nonce(&mut self, v: U64) -> &mut Self {
        self.tx.set_nonce(v);
        self
    }

    /// Set nonce, if `v` is not None
    pub fn nonce(&mut self, v: Option<U64>) -> &mut Self {
        if let Some(value) = v {
            self.set_nonce(value);
        }
        self
    }

    /// Set etherscan API key. Used to look up function signature buy name
    pub fn set_etherscan_api_key(&mut self, v: String) -> &mut Self {
        self.etherscan_api_key = Some(v);
        self
    }

    /// Set etherscan API key, if `v` is not None
    pub fn etherscan_api_key(&mut self, v: Option<String>) -> &mut Self {
        if let Some(value) = v {
            self.set_etherscan_api_key(value);
        }
        self
    }

    pub fn set_data(&mut self, v: Bytes) -> &mut Self {
        self.tx.set_input(v);
        self
    }

    pub async fn create_args(
        &mut self,
        sig: &str,
        args: Vec<String>,
    ) -> Result<(Vec<u8>, Function)> {
        if sig.trim().is_empty() {
            return Err(FunctionSignatureError::MissingSignature.into())
        }

        let args = resolve_name_args(&args, self.provider).await;

        let func = if sig.contains('(') {
            // a regular function signature with parentheses
            get_func(sig)?
        } else if sig.starts_with("0x") {
            // if only calldata is provided, returning a dummy function
            get_func("x()")?
        } else {
            get_func_etherscan(
                sig,
                self.to.ok_or(FunctionSignatureError::MissingToAddress)?,
                &args,
                self.chain,
                self.etherscan_api_key.as_ref().ok_or_else(|| {
                    FunctionSignatureError::MissingEtherscan { sig: sig.to_string() }
                })?,
            )
            .await?
        };

        if sig.starts_with("0x") {
            Ok((hex::decode(sig)?, func))
        } else {
            Ok((encode_function_args(&func, &args)?, func))
        }
    }

    /// Set function arguments
    /// `sig` can be:
    ///  * a fragment (`do(uint32,string)`)
    ///  * selector + abi-encoded calldata
    ///    (`0xcdba2fd40000000000000000000000000000000000000000000000000000000000007a69`)
    ///  * only function name (`do`) - in this case, etherscan lookup is performed on `tx.to`'s
    ///    contract
    pub async fn set_args(&mut self, sig: &str, args: Vec<String>) -> Result<&mut Self> {
        let (data, func) = self.create_args(sig, args).await?;
        self.tx.set_input(data.into());
        self.func = Some(func);
        Ok(self)
    }

    /// Set function arguments, if `value` is not None
    pub async fn args(&mut self, value: Option<(&str, Vec<String>)>) -> Result<&mut Self> {
        if let Some((sig, args)) = value {
            return self.set_args(sig, args).await
        }
        Ok(self)
    }

    /// Consuming build: returns typed transaction and optional function call
    pub fn build(self) -> TxBuilderOutput {
        (self.tx, self.func)
    }

    pub fn peek(&self) -> TxBuilderPeekOutput {
        (&self.tx, &self.func)
    }
}

async fn resolve_name_args<T: Transport + Clone, P: Provider<Ethereum, T>>(
    args: &[String],
    provider: &P,
) -> Vec<String> {
    join_all(args.iter().map(|arg| async {
        if arg.contains('.') {
            let addr = NameOrAddress::Name(arg.to_string()).resolve(provider).await;
            match addr {
                Ok(addr) => addr.to_string(),
                Err(_) => arg.to_string(),
            }
        } else {
            arg.to_string()
        }
    }))
    .await
}

#[cfg(test)]
mod tests {
    use crate::TxBuilder;
    use alloy_primitives::{Address, U256};
    use ethers_core::types::{transaction::eip2718::TypedTransaction, NameOrAddress};
    use foundry_common::types::ToEthers;
    use foundry_config::NamedChain;

    const ADDR_1: Address = Address::with_last_byte(1);
    const ADDR_2: Address = Address::with_last_byte(2);

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_new_non_legacy() -> eyre::Result<()> {
        // Instanciate a local provider although it'll do nothing.
        let provider = foundry_common::provider::alloy::get_http_provider("http://localhost:8545");
        let builder =
            TxBuilder::new(&provider, ADDR_1, Some(ADDR_2), NamedChain::Mainnet, false).await?;
        let (tx, args) = builder.build();
        assert_eq!(*tx.from().unwrap(), ADDR_1.to_ethers());
        assert_eq!(*tx.to().unwrap(), NameOrAddress::Address(ADDR_2.to_ethers()));
        assert_eq!(args, None);

        match tx {
            TypedTransaction::Eip1559(_) => {}
            _ => {
                panic!("Wrong tx type");
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_new_legacy() -> eyre::Result<()> {
        let provider = foundry_common::provider::alloy::get_http_provider("http://localhost:8545");
        let builder =
            TxBuilder::new(&provider, ADDR_1, Some(ADDR_2), NamedChain::Mainnet, true).await?;
        // don't check anything other than the tx type - the rest is covered in the non-legacy case
        let (tx, _) = builder.build();
        match tx {
            TypedTransaction::Legacy(_) => {}
            _ => {
                panic!("Wrong tx type");
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_fields() -> eyre::Result<()> {
        let provider = foundry_common::provider::alloy::get_http_provider("http://localhost:8545");
        let mut builder =
            TxBuilder::new(&provider, ADDR_1, Some(ADDR_2), NamedChain::Mainnet, false)
                .await
                .unwrap();
        builder
            .gas(Some(U256::from(12u32)))
            .gas_price(Some(U256::from(34u32)))
            .value(Some(U256::from(56u32)))
            .nonce(Some(U256::from(78u32)));

        builder.etherscan_api_key(Some(String::from("what a lovely day"))); // not testing for this :-/
        let (tx, _) = builder.build();

        assert_eq!(tx.gas().unwrap().as_u32(), 12);
        assert_eq!(tx.gas_price().unwrap().as_u32(), 34);
        assert_eq!(tx.value().unwrap().as_u32(), 56);
        assert_eq!(tx.nonce().unwrap().as_u32(), 78);
        assert_eq!(tx.chain_id().unwrap().as_u32(), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_args() -> eyre::Result<()> {
        let provider = foundry_common::provider::alloy::get_http_provider("http://localhost:8545");
        let mut builder =
            TxBuilder::new(&provider, ADDR_1, Some(ADDR_2), NamedChain::Mainnet, false)
                .await
                .unwrap();
        builder.args(Some(("what_a_day(int)", vec![String::from("31337")]))).await?;
        let (_, function_maybe) = builder.build();

        assert_ne!(function_maybe, None);
        let function = function_maybe.unwrap();
        assert_eq!(function.name, String::from("what_a_day"));
        // could test function.inputs() but that should be covered by utils's unit test
        Ok(())
    }
}
