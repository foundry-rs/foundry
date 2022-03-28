use ethers_core::{
    abi::Function,
    types::{
        transaction::eip2718::TypedTransaction, Chain, Eip1559TransactionRequest, NameOrAddress,
        TransactionRequest, H160, U256,
    },
};
use ethers_providers::Middleware;

use eyre::{eyre, Result};
use foundry_utils::{encode_args, get_func, get_func_etherscan};
use futures::future::join_all;

use crate::strip_0x;

pub struct TxBuilder {
    pub from: H160,
    pub to: H160,
    pub chain: Chain,
    pub tx: TypedTransaction,
    pub func: Option<Function>,
    pub etherscan_api_key: Option<String>,
}

impl TxBuilder {
    pub async fn new<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
        provider: &M,
        from: F,
        to: T,
        chain: Chain,
        legacy: bool,
    ) -> Result<TxBuilder> {
        let from_addr = resolve_ens(provider, from).await?;
        let to_addr = resolve_ens(provider, foundry_utils::resolve_addr(to, chain)?).await?;

        let tx: TypedTransaction = if chain.is_legacy() || legacy {
            TransactionRequest::new().from(from_addr).to(to_addr).into()
        } else {
            Eip1559TransactionRequest::new().from(from_addr).to(to_addr).into()
        };

        Ok(TxBuilder {
            from: from_addr,
            to: to_addr,
            chain,
            tx,
            func: None,
            etherscan_api_key: None,
        })
    }
    pub fn set_gas<'a>(&'a mut self, v: U256) -> &'a mut TxBuilder {
        self.tx.set_gas(v);
        self
    }
    pub fn gas<'a>(&'a mut self, v: Option<U256>) -> &'a mut TxBuilder {
        match v {
            Some(x) => self.set_gas(x),
            None => self,
        }
    }

    pub fn set_gas_price<'a>(&'a mut self, v: U256) -> &'a mut TxBuilder {
        self.tx.set_gas_price(v);
        self
    }

    pub fn gas_price<'a>(&'a mut self, v: Option<U256>) -> &'a mut TxBuilder {
        match v {
            Some(x) => self.set_gas_price(x),
            None => self,
        }
    }

    pub fn set_value<'a>(&'a mut self, v: U256) -> &'a mut TxBuilder {
        self.tx.set_value(v);
        self
    }
    pub fn value<'a>(&'a mut self, v: Option<U256>) -> &'a mut TxBuilder {
        match v {
            Some(x) => self.set_value(x),
            None => self,
        }
    }
    pub fn set_nonce<'a>(&'a mut self, v: U256) -> &'a mut TxBuilder {
        self.tx.set_nonce(v);
        self
    }
    pub fn nonce<'a>(&'a mut self, v: Option<U256>) -> &'a mut TxBuilder {
        match v {
            Some(x) => self.set_nonce(x),
            None => self,
        }
    }

    pub fn set_etherscan_api_key<'a>(&'a mut self, v: String) -> &'a mut TxBuilder {
        self.etherscan_api_key = Some(v);
        self
    }
    pub fn etherscan_api_key<'a>(&'a mut self, v: Option<String>) -> &'a mut TxBuilder {
        match v {
            Some(x) => self.set_etherscan_api_key(x),
            None => self,
        }
    }

    pub async fn set_args<'a, M: Middleware>(
        &'a mut self,
        provider: &M,
        sig: &str,
        args: Vec<String>,
    ) -> Result<&'a mut TxBuilder> {
        let args = resolve_name_args(&args, provider).await;

        let func = if sig.contains('(') {
            get_func(&sig)?
        } else if sig.starts_with("0x") {
            // if only calldata is provided, returning a dummy function
            get_func("x()")?
        } else {
            get_func_etherscan(
                &sig,
                self.to,
                &args,
                self.chain,
                self.etherscan_api_key.as_ref().expect("Must set ETHERSCAN_API_KEY"),
            )
            .await?
        };

        let data = if sig.starts_with("0x") {
            hex::decode(strip_0x(&sig))?
        } else {
            encode_args(&func, &args)?
        };

        self.tx.set_data(data.into());
        self.func = Some(func);
        Ok(self)
    }
    pub async fn args<'a, M: Middleware>(
        &'a mut self,
        provider: &M,
        value: Option<(&str, Vec<String>)>,
    ) -> Result<&'a mut TxBuilder> {
        match value {
            Some((sig, args)) => self.set_args(provider, sig, args).await,
            None => Ok(self),
        }
    }
}

async fn resolve_ens<M: Middleware, T: Into<NameOrAddress>>(provider: &M, addr: T) -> Result<H160> {
    let from_addr = match addr.into() {
        NameOrAddress::Name(ref ens_name) => provider.resolve_name(ens_name).await,
        NameOrAddress::Address(addr) => Ok(addr),
    }
    .map_err(|x| eyre!("Failed to resolve ENS name: {}", x))?;
    return Ok(from_addr);
}

async fn resolve_name_args<M: Middleware>(args: &[String], provider: &M) -> Vec<String> {
    join_all(args.iter().map(|arg| async {
        if arg.contains('.') {
            let addr = provider.resolve_name(arg).await;
            match addr {
                Ok(addr) => format!("0x{}", hex::encode(addr.as_bytes())),
                Err(_) => arg.to_string(),
            }
        } else {
            arg.to_string()
        }
    }))
    .await
}
