use crate::errors::FunctionSignatureError;
use ethers_core::{
    abi::Function,
    types::{
        transaction::eip2718::TypedTransaction, Eip1559TransactionRequest, NameOrAddress,
        TransactionRequest, H160, U256,
    },
};
use ethers_providers::Middleware;
use eyre::{eyre, Result};
use foundry_common::abi::{encode_function_args, get_func, get_func_etherscan};
use foundry_config::Chain;
use futures::future::join_all;

pub struct TxBuilder<'a, M: Middleware> {
    to: Option<H160>,
    chain: Chain,
    tx: TypedTransaction,
    func: Option<Function>,
    etherscan_api_key: Option<String>,
    provider: &'a M,
}

pub type TxBuilderOutput = (TypedTransaction, Option<Function>);
pub type TxBuilderPeekOutput<'a> = (&'a TypedTransaction, &'a Option<Function>);

/// Transaction builder
/// ```
/// async fn foo() -> eyre::Result<()> {
///   use ethers_core::types::{Chain, U256};
///   use cast::TxBuilder;
///   let provider = ethers_providers::test_provider::MAINNET.provider();
///   let mut builder = TxBuilder::new(&provider, "a.eth", Some("b.eth"), Chain::Mainnet, false).await?;
///   builder
///       .gas(Some(U256::from(1)));
///   let (tx, _) = builder.build();
///   Ok(())
/// }
/// ```
impl<'a, M: Middleware> TxBuilder<'a, M> {
    /// Create a new TxBuilder
    /// `provider` - provider to use
    /// `from` - 'from' field. Could be an ENS name
    /// `to` - `to`. Could be a ENS
    /// `chain` - chain to construct the tx for
    /// `legacy` - use type 1 transaction
    pub async fn new<F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
        provider: &'a M,
        from: F,
        to: Option<T>,
        chain: impl Into<Chain>,
        legacy: bool,
    ) -> Result<TxBuilder<'a, M>> {
        let chain = chain.into();
        let from_addr = resolve_ens(provider, from).await?;

        let mut tx: TypedTransaction = if chain.is_legacy() || legacy {
            TransactionRequest::new().from(from_addr).chain_id(chain.id()).into()
        } else {
            Eip1559TransactionRequest::new().from(from_addr).chain_id(chain.id()).into()
        };

        let to_addr = if let Some(to) = to {
            let addr =
                resolve_ens(provider, foundry_utils::resolve_addr(to, chain.try_into().ok())?)
                    .await?;
            tx.set_to(addr);
            Some(addr)
        } else {
            None
        };
        Ok(Self { to: to_addr, chain, tx, func: None, etherscan_api_key: None, provider })
    }

    /// Set gas for tx
    pub fn set_gas(&mut self, v: U256) -> &mut Self {
        self.tx.set_gas(v);
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
        if let TypedTransaction::Eip1559(tx) = &mut self.tx {
            tx.max_priority_fee_per_gas = Some(v)
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
    pub fn set_nonce(&mut self, v: U256) -> &mut Self {
        self.tx.set_nonce(v);
        self
    }

    /// Set nonce, if `v` is not None
    pub fn nonce(&mut self, v: Option<U256>) -> &mut Self {
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

    pub fn set_data(&mut self, v: Vec<u8>) -> &mut Self {
        self.tx.set_data(v.into());
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
            let chain = self
                .chain
                .try_into()
                .map_err(|_| FunctionSignatureError::UnknownChain(self.chain))?;
            get_func_etherscan(
                sig,
                self.to.ok_or(FunctionSignatureError::MissingToAddress)?,
                &args,
                chain,
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
    pub async fn set_args(
        &mut self,
        sig: &str,
        args: Vec<String>,
    ) -> Result<&mut TxBuilder<'a, M>> {
        let (data, func) = self.create_args(sig, args).await?;
        self.tx.set_data(data.into());
        self.func = Some(func);
        Ok(self)
    }

    /// Set function arguments, if `value` is not None
    pub async fn args(
        &mut self,
        value: Option<(&str, Vec<String>)>,
    ) -> Result<&mut TxBuilder<'a, M>> {
        if let Some((sig, args)) = value {
            return self.set_args(sig, args).await
        }
        Ok(self)
    }

    /// Consuming build: returns typed transaction and optional function call
    pub fn build(self) -> TxBuilderOutput {
        (self.tx, self.func)
    }

    /// Non-consuming build: peek into the tx content
    pub fn peek(&self) -> TxBuilderPeekOutput {
        (&self.tx, &self.func)
    }
}

async fn resolve_ens<M: Middleware, T: Into<NameOrAddress>>(provider: &M, addr: T) -> Result<H160> {
    let from_addr = match addr.into() {
        NameOrAddress::Name(ref ens_name) => provider.resolve_name(ens_name).await,
        NameOrAddress::Address(addr) => Ok(addr),
    }
    .map_err(|x| eyre!("Failed to resolve ENS name: {x}"))?;
    Ok(from_addr)
}

async fn resolve_name_args<M: Middleware>(args: &[String], provider: &M) -> Vec<String> {
    join_all(args.iter().map(|arg| async {
        if arg.contains('.') {
            let addr = provider.resolve_name(arg).await;
            match addr {
                Ok(addr) => format!("{addr:?}"),
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
    use async_trait::async_trait;
    use ethers_core::types::{
        transaction::eip2718::TypedTransaction, Address, Chain, NameOrAddress, H160, U256,
    };
    use ethers_providers::{JsonRpcClient, Middleware, ProviderError};
    use serde::{de::DeserializeOwned, Serialize};
    use std::str::FromStr;

    const ADDR_1: &str = "0000000000000000000000000000000000000001";
    const ADDR_2: &str = "0000000000000000000000000000000000000002";

    #[derive(Debug)]
    struct MyProvider {}

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl JsonRpcClient for MyProvider {
        type Error = ProviderError;

        async fn request<T: Serialize + Send + Sync, R: DeserializeOwned>(
            &self,
            _method: &str,
            _params: T,
        ) -> Result<R, Self::Error> {
            Err(ProviderError::CustomError("There is no request".to_string()))
        }
    }
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl Middleware for MyProvider {
        type Error = ProviderError;
        type Provider = MyProvider;
        type Inner = MyProvider;

        fn inner(&self) -> &Self::Inner {
            self
        }

        async fn resolve_name(&self, ens_name: &str) -> Result<Address, Self::Error> {
            match ens_name {
                "a.eth" => Ok(H160::from_str(ADDR_1).unwrap()),
                "b.eth" => Ok(H160::from_str(ADDR_2).unwrap()),
                _ => unreachable!("don't know how to resolve {ens_name}"),
            }
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn builder_new_non_legacy() -> eyre::Result<()> {
        let provider = MyProvider {};
        let builder =
            TxBuilder::new(&provider, "a.eth", Some("b.eth"), Chain::Mainnet, false).await?;
        let (tx, args) = builder.build();
        assert_eq!(*tx.from().unwrap(), H160::from_str(ADDR_1).unwrap());
        assert_eq!(*tx.to().unwrap(), NameOrAddress::Address(H160::from_str(ADDR_2).unwrap()));
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
        let provider = MyProvider {};
        let builder =
            TxBuilder::new(&provider, "a.eth", Some("b.eth"), Chain::Mainnet, true).await?;
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
        let provider = MyProvider {};
        let mut builder =
            TxBuilder::new(&provider, "a.eth", Some("b.eth"), Chain::Mainnet, false).await.unwrap();
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
        let provider = MyProvider {};
        let mut builder =
            TxBuilder::new(&provider, "a.eth", Some("b.eth"), Chain::Mainnet, false).await.unwrap();
        builder.args(Some(("what_a_day(int)", vec![String::from("31337")]))).await?;
        let (_, function_maybe) = builder.build();

        assert_ne!(function_maybe, None);
        let function = function_maybe.unwrap();
        assert_eq!(function.name, String::from("what_a_day"));
        // could test function.inputs() but that should be covered by utils's unit test
        Ok(())
    }
}
