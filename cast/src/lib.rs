//! Cast
//!
//! Contains core function implementation for `cast`
use crate::rlp_converter::Item;
use base::{Base, NumberWithBase, ToBase};
use chrono::NaiveDateTime;
use ethers_contract::RawAbi;
use ethers_core::{
    abi::{
        token::{LenientTokenizer, Tokenizer},
        Function, HumanReadableParser, Token,
    },
    types::{Chain, *},
    utils::{
        self, format_bytes32_string, format_units, get_contract_address, keccak256,
        parse_bytes32_string, parse_units, rlp, Units,
    },
};
use ethers_etherscan::Client;
use ethers_providers::{Middleware, PendingTransaction};
use eyre::{Context, Result};
use foundry_common::{abi::encode_args, fmt::*};
pub use foundry_evm::*;
use rustc_hex::{FromHexIter, ToHex};
use std::{path::PathBuf, str::FromStr};
pub use tx::TxBuilder;
use tx::{TxBuilderOutput, TxBuilderPeekOutput};

pub mod base;
pub mod errors;
mod rlp_converter;
mod tx;

// TODO: CastContract with common contract initializers? Same for CastProviders?

pub struct Cast<M> {
    provider: M,
}

impl<M: Middleware> Cast<M>
where
    M::Error: 'static,
{
    /// Creates a new Cast instance from the provided client
    ///
    /// # Example
    ///
    /// ```
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(provider: M) -> Self {
        Self { provider }
    }

    /// Makes a read-only call to the specified address
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::{Cast, TxBuilder};
    /// use ethers_core::types::{Address, Chain};
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "function greeting(uint256 i) public returns (string)";
    /// let args = vec!["5".to_owned()];
    /// let mut builder = TxBuilder::new(&provider, Address::zero(), Some(to), Chain::Mainnet, false).await?;
    /// builder
    ///     .set_args(sig, args).await?;
    /// let builder_output = builder.build();
    /// let cast = Cast::new(provider);
    /// let data = cast.call(builder_output, None).await?;
    /// println!("{}", data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call<'a>(
        &self,
        builder_output: TxBuilderOutput,
        block: Option<BlockId>,
    ) -> Result<String> {
        let (tx, func) = builder_output;
        let res = self.provider.call(&tx, block).await?;

        // decode args into tokens
        let func = func.expect("no valid function signature was provided.");
        let decoded = match func.decode_output(res.as_ref()) {
            Ok(decoded) => decoded,
            Err(err) => {
                // ensure the address is a contract
                if res.is_empty() {
                    // check that the recipient is a contract that can be called
                    if let Some(NameOrAddress::Address(addr)) = tx.to() {
                        let code = self.provider.get_code(*addr, block).await?;
                        if code.is_empty() {
                            eyre::bail!("Contract {:?} does not exist", addr)
                        }
                    }
                }
                return Err(err).wrap_err(
                    "could not decode output. did you specify the wrong function return data type perhaps?"
                )
            }
        };
        // handle case when return type is not specified
        Ok(if decoded.is_empty() {
            format!("{res}\n")
        } else {
            // seth compatible user-friendly return type conversions
            decoded
                .iter()
                .map(TokenDisplay)
                .map(|token| token.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// Generates an access list for the specified transaction
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::{Cast, TxBuilder};
    /// use ethers_core::types::{Address, Chain};
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "greeting(uint256)(string)";
    /// let args = vec!["5".to_owned()];
    /// let mut builder = TxBuilder::new(&provider, Address::zero(), Some(to), Chain::Mainnet, false).await?;
    /// builder
    ///     .set_args(sig, args).await?;
    /// let builder_output = builder.peek();
    /// let cast = Cast::new(&provider);
    /// let access_list = cast.access_list(builder_output, None, false).await?;
    /// println!("{}", access_list);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn access_list(
        &self,
        builder_output: TxBuilderPeekOutput<'_>,
        block: Option<BlockId>,
        to_json: bool,
    ) -> Result<String> {
        let (tx, _) = builder_output;
        let access_list = self.provider.create_access_list(tx, block).await?;
        let res = if to_json {
            serde_json::to_string(&access_list)?
        } else {
            let mut s =
                vec![format!("gas used: {}", access_list.gas_used), "access list:".to_string()];
            for al in access_list.access_list.0 {
                s.push(format!("- address: {}", SimpleCast::to_checksum_address(&al.address)));
                if !al.storage_keys.is_empty() {
                    s.push("  keys:".to_string());
                    for key in al.storage_keys {
                        s.push(format!("    {:?}", key));
                    }
                }
            }
            s.join("\n")
        };

        Ok(res)
    }

    pub async fn balance<T: Into<NameOrAddress> + Send + Sync>(
        &self,
        who: T,
        block: Option<BlockId>,
    ) -> Result<U256> {
        Ok(self.provider.get_balance(who, block).await?)
    }

    /// Sends a transaction to the specified address
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::{Cast, TxBuilder};
    /// use ethers_core::types::{Address, Chain, U256};
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let from = "vitalik.eth";
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "greet(string)()";
    /// let args = vec!["hello".to_owned()];
    /// let gas = U256::from_str("200000").unwrap();
    /// let value = U256::from_str("1").unwrap();
    /// let nonce = U256::from_str("1").unwrap();
    /// let mut builder = TxBuilder::new(&provider, Address::zero(), Some(to), Chain::Mainnet, false).await?;
    /// builder
    ///     .set_args(sig, args).await?
    ///     .set_gas(gas)
    ///     .set_value(value)
    ///     .set_nonce(nonce);
    /// let builder_output = builder.build();
    /// let cast = Cast::new(provider);
    /// let data = cast.send(builder_output).await?;
    /// println!("{}", *data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send<'a>(
        &self,
        builder_output: TxBuilderOutput,
    ) -> Result<PendingTransaction<'_, M::Provider>> {
        let (tx, _) = builder_output;
        let res = self.provider.send_transaction(tx, None).await?;

        Ok::<_, eyre::Error>(res)
    }

    /// Publishes a raw transaction to the network
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let res = cast.publish("0x1234".to_string()).await?;
    /// println!("{:?}", res);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(&self, mut raw_tx: String) -> Result<PendingTransaction<'_, M::Provider>> {
        raw_tx = match raw_tx.strip_prefix("0x") {
            Some(s) => s.to_string(),
            None => raw_tx,
        };
        let tx = Bytes::from(hex::decode(raw_tx)?);
        let res = self.provider.send_raw_transaction(tx).await?;

        Ok::<_, eyre::Error>(res)
    }

    /// Estimates the gas cost of a transaction
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::{Cast, TxBuilder};
    /// use ethers_core::types::{Address, Chain, U256};
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let from = "vitalik.eth";
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "greet(string)()";
    /// let args = vec!["5".to_owned()];
    /// let value = U256::from_str("1").unwrap();
    /// let mut builder = TxBuilder::new(&provider, from, Some(to), Chain::Mainnet, false).await?;
    /// builder
    ///     .set_value(value)
    ///     .set_args(sig, args).await?;
    /// let builder_output = builder.peek();
    /// let cast = Cast::new(&provider);
    /// let data = cast.estimate(builder_output).await?;
    /// println!("{}", data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn estimate(&self, builder_output: TxBuilderPeekOutput<'_>) -> Result<U256> {
        let (tx, _) = builder_output;

        let res = self.provider.estimate_gas(tx, None).await?;

        Ok::<_, eyre::Error>(res)
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let block = cast.block(5, true, None, false).await?;
    /// println!("{}", block);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn block<T: Into<BlockId>>(
        &self,
        block: T,
        full: bool,
        field: Option<String>,
        to_json: bool,
    ) -> Result<String> {
        let block = block.into();
        let block = if full {
            let block = self
                .provider
                .get_block_with_txs(block)
                .await?
                .ok_or_else(|| eyre::eyre!("block {:?} not found", block))?;
            if let Some(ref field) = field {
                get_pretty_block_attr(&block, field)
                    .unwrap_or_else(|| format!("{field} is not a valid block field"))
            } else if to_json {
                serde_json::to_value(&block).unwrap().to_string()
            } else {
                block.pretty()
            }
        } else {
            let block = self
                .provider
                .get_block(block)
                .await?
                .ok_or_else(|| eyre::eyre!("block {:?} not found", block))?;

            if let Some(ref field) = field {
                if field == "transactions" {
                    "use --full to view transactions".to_string()
                } else {
                    get_pretty_block_attr(&block, field)
                        .unwrap_or_else(|| format!("{field} is not a valid block field"))
                }
            } else if to_json {
                serde_json::to_value(&block).unwrap().to_string()
            } else {
                block.pretty()
            }
        };

        Ok(block)
    }

    async fn block_field_as_num<T: Into<BlockId>>(&self, block: T, field: String) -> Result<U256> {
        let block = block.into();
        let block_field = Cast::block(
            self,
            block,
            false,
            // Select only select field
            Some(field),
            false,
        )
        .await?;

        let ret = if block_field.starts_with("0x") {
            U256::from_str_radix(strip_0x(&block_field), 16).expect("Unable to convert hex to U256")
        } else {
            U256::from_str_radix(&block_field, 10).expect("Unable to convert decimal to U256")
        };
        Ok(ret)
    }

    pub async fn base_fee<T: Into<BlockId>>(&self, block: T) -> Result<U256> {
        Cast::block_field_as_num(self, block, String::from("baseFeePerGas")).await
    }

    pub async fn age<T: Into<BlockId>>(&self, block: T) -> Result<String> {
        let timestamp_str =
            Cast::block_field_as_num(self, block, String::from("timestamp")).await?.to_string();
        let datetime = NaiveDateTime::from_timestamp(timestamp_str.parse::<i64>().unwrap(), 0);
        Ok(datetime.format("%a %b %e %H:%M:%S %Y").to_string())
    }

    pub async fn timestamp<T: Into<BlockId>>(&self, block: T) -> Result<U256> {
        Cast::block_field_as_num(self, block, "timestamp".to_string()).await
    }

    pub async fn chain(&self) -> Result<&str> {
        let genesis_hash = Cast::block(
            self,
            0,
            false,
            // Select only block hash
            Some(String::from("hash")),
            false,
        )
        .await?;

        Ok(match &genesis_hash[..] {
            "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" => {
                match &(Cast::block(self, 1920000, false, Some("hash".to_string()), false).await?)[..]
                {
                    "0x94365e3a8c0b35089c1d1195081fe7489b528a84b22199c916180db8b28ade7f" => {
                        "etclive"
                    }
                    _ => "ethlive",
                }
            }
            "0xa3c565fc15c7478862d50ccd6561e3c06b24cc509bf388941c25ea985ce32cb9" => "kovan",
            "0x41941023680923e0fe4d74a34bdac8141f2540e3ae90623718e47d66d1ca4a2d" => "ropsten",
            "0x7ca38a1916c42007829c55e69d3e9a73265554b586a499015373241b8a3fa48b" => {
                "optimism-mainnet"
            }
            "0x02adc9b449ff5f2467b8c674ece7ff9b21319d76c4ad62a67a70d552655927e5" => {
                "optimism-kovan"
            }
            "0x7ee576b35482195fc49205cec9af72ce14f003b9ae69f6ba0faef4514be8b442" => {
                "arbitrum-mainnet"
            }
            "0x0cd786a2425d16f152c658316c423e6ce1181e15c3295826d7c9904cba9ce303" => "morden",
            "0x6341fd3daf94b748c72ced5a5b26028f2474f5f00d824504e4fa37a75767e177" => "rinkeby",
            "0xbf7e331f7f7c1dd2e05159666b3bf8bc7a8a3a9eb1d518969eab529dd9b88c1a" => "goerli",
            "0x14c2283285a88fe5fce9bf5c573ab03d6616695d717b12a127188bcacfc743c4" => "kotti",
            "0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b" => "polygon",
            "0x7b66506a9ebdbf30d32b43c5f15a3b1216269a1ec3a75aa3182b86176a2b1ca7" => {
                "polygon-mumbai"
            }
            "0x6d3c66c5357ec91d5c43af47e234a939b22557cbb552dc45bebbceeed90fbe34" => "bsctest",
            "0x0d21840abff46b96c84b2ac9e10e4f5cdaeb5693cb665db62a2f3b02d2d57b5b" => "bsc",
            "0x31ced5b9beb7f8782b014660da0cb18cc409f121f408186886e1ca3e8eeca96b" => {
                match &(Cast::block(self, 1, false, Some(String::from("hash")), false).await?)[..] {
                    "0x738639479dc82d199365626f90caa82f7eafcfe9ed354b456fb3d294597ceb53" => {
                        "avalanche-fuji"
                    }
                    _ => "avalanche",
                }
            }
            _ => "unknown",
        })
    }

    pub async fn chain_id(&self) -> Result<U256> {
        Ok(self.provider.get_chainid().await?)
    }

    pub async fn block_number(&self) -> Result<U64> {
        Ok(self.provider.get_block_number().await?)
    }

    pub async fn gas_price(&self) -> Result<U256> {
        Ok(self.provider.get_gas_price().await?)
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use ethers_core::types::Address;
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let addr = Address::from_str("0x7eD52863829AB99354F3a0503A622e82AcD5F7d3")?;
    /// let nonce = cast.nonce(addr, None).await?;
    /// println!("{}", nonce);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn nonce<T: Into<NameOrAddress> + Send + Sync>(
        &self,
        who: T,
        block: Option<BlockId>,
    ) -> Result<U256> {
        Ok(self.provider.get_transaction_count(who, block).await?)
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use ethers_core::types::Address;
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let addr = Address::from_str("0x7eD52863829AB99354F3a0503A622e82AcD5F7d3")?;
    /// let nonce = cast.nonce(addr, None).await? + 5;
    /// let computed_address = cast.compute_address(addr, Some(nonce)).await?;
    /// println!("Computed address for address {} with nonce {}: {}", addr, nonce, computed_address);
    /// let computed_address_no_nonce = cast.compute_address(addr, None).await?;
    /// println!("Computed address for address {} with nonce {}: {}", addr, nonce, computed_address_no_nonce);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn compute_address<T: Into<Address> + Copy + Send + Sync>(
        &self,
        address: T,
        nonce: Option<U256>,
    ) -> Result<Address> {
        let unpacked = if let Some(n) = nonce {
            n
        } else {
            self.provider.get_transaction_count(address.into(), None).await?
        };

        Ok(get_contract_address(address, unpacked))
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use ethers_core::types::Address;
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let addr = Address::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")?;
    /// let code = cast.code(addr, None).await?;
    /// println!("{}", code);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn code<T: Into<NameOrAddress> + Send + Sync>(
        &self,
        who: T,
        block: Option<BlockId>,
    ) -> Result<String> {
        Ok(format!("{}", self.provider.get_code(who, block).await?))
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let tx_hash = "0xf8d1713ea15a81482958fb7ddf884baee8d3bcc478c5f2f604e008dc788ee4fc";
    /// let tx = cast.transaction(tx_hash.to_string(), None, false).await?;
    /// println!("{}", tx);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction(
        &self,
        tx_hash: String,
        field: Option<String>,
        to_json: bool,
    ) -> Result<String> {
        let transaction_result = self
            .provider
            .get_transaction(H256::from_str(&tx_hash)?)
            .await?
            .ok_or_else(|| eyre::eyre!("transaction {:?} not found", tx_hash))?;

        let transaction = if let Some(ref field) = field {
            serde_json::to_value(&transaction_result)?
                .get(field)
                .cloned()
                .ok_or_else(|| eyre::eyre!("field {field} not found"))?
        } else {
            serde_json::to_value(&transaction_result)?
        };

        let transaction = if let Some(ref field) = field {
            get_pretty_tx_attr(&transaction_result, field)
                .unwrap_or_else(|| format!("{field} is not a valid tx field"))
        } else if to_json {
            serde_json::to_string(&transaction)?
        } else {
            transaction_result.pretty()
        };
        Ok(transaction)
    }

    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let tx_hash = "0xf8d1713ea15a81482958fb7ddf884baee8d3bcc478c5f2f604e008dc788ee4fc";
    /// let receipt = cast.receipt(tx_hash.to_string(), None, 1, false, false).await?;
    /// println!("{}", receipt);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receipt(
        &self,
        tx_hash: String,
        field: Option<String>,
        confs: usize,
        cast_async: bool,
        to_json: bool,
    ) -> Result<String> {
        let tx_hash = H256::from_str(&tx_hash)?;

        // try to get the receipt
        let receipt = self.provider.get_transaction_receipt(tx_hash).await?;

        // if the async flag is provided, immediately exit if no tx is found,
        // otherwise try to poll for it
        let receipt_result = if cast_async {
            match receipt {
                Some(inner) => inner,
                None => return Ok("receipt not found".to_string()),
            }
        } else {
            match receipt {
                Some(inner) => inner,
                None => {
                    let tx = PendingTransaction::new(tx_hash, self.provider.provider());
                    match tx.confirmations(confs).await? {
                        Some(inner) => inner,
                        None => return Ok("receipt not found when polling pending tx. was the transaction dropped from the mempool?".to_string())
                    }
                }
            }
        };

        let receipt = if let Some(ref field) = field {
            serde_json::to_value(&receipt_result)?
                .get(field)
                .cloned()
                .ok_or_else(|| eyre::eyre!("field {field} not found"))?
        } else {
            serde_json::to_value(&receipt_result)?
        };

        let receipt = if let Some(ref field) = field {
            get_pretty_tx_receipt_attr(&receipt_result, field)
                .unwrap_or_else(|| format!("{field} is not a valid tx receipt field"))
        } else if to_json {
            serde_json::to_string(&receipt)?
        } else {
            receipt_result.pretty()
        };
        Ok(receipt)
    }

    /// Perform a raw JSON-RPC request
    ///
    /// # Example
    ///
    /// ```no_run
    /// use cast::Cast;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let cast = Cast::new(provider);
    /// let result = cast.rpc("eth_getBalance", &["0xc94770007dda54cF92009BFF0dE90c06F603a09f", "latest"])
    ///     .await?;
    /// println!("{}", result);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rpc<T>(&self, method: &str, params: T) -> Result<String>
    where
        T: std::fmt::Debug + serde::Serialize + Send + Sync,
    {
        let res = self.provider.provider().request::<T, serde_json::Value>(method, params).await?;
        Ok(serde_json::to_string(&res)?)
    }
}

pub struct InterfaceSource {
    pub name: String,
    pub source: String,
}

pub enum InterfacePath {
    Local { path: String, name: Option<String> },
    Etherscan { address: Address, chain: Chain, api_key: String },
}

pub struct SimpleCast;
impl SimpleCast {
    /// Converts UTF-8 text input to hex
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::from_utf8("yo"), "0x796f");
    ///     assert_eq!(Cast::from_utf8("Hello, World!"), "0x48656c6c6f2c20576f726c6421");
    ///     assert_eq!(Cast::from_utf8("TurboDappTools"), "0x547572626f44617070546f6f6c73");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn from_utf8(s: &str) -> String {
        let s: String = s.as_bytes().to_hex();
        format!("0x{s}")
    }

    /// Converts hex data into text data
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_ascii("0x796f")?, "yo");
    ///     assert_eq!(Cast::to_ascii("48656c6c6f2c20576f726c6421")?, "Hello, World!");
    ///     assert_eq!(Cast::to_ascii("0x547572626f44617070546f6f6c73")?, "TurboDappTools");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_ascii(hex: &str) -> Result<String> {
        let hex_trimmed = hex.trim_start_matches("0x");
        let iter = FromHexIter::new(hex_trimmed);
        let mut ascii = String::new();
        for letter in iter.collect::<Vec<_>>() {
            ascii.push(letter.unwrap() as char);
        }
        Ok(ascii)
    }

    /// Converts fixed point number into specified number of decimals
    /// ```
    /// use cast::SimpleCast as Cast;
    /// use ethers_core::types::U256;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::from_fixed_point("10", "0")?, "10");
    ///     assert_eq!(Cast::from_fixed_point("1.0", "1")?, "10");
    ///     assert_eq!(Cast::from_fixed_point("0.10", "2")?, "10");
    ///     assert_eq!(Cast::from_fixed_point("0.010", "3")?, "10");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn from_fixed_point(value: &str, decimals: &str) -> Result<String> {
        // first try u32 as Units assumes a string can only be "ether", "gwei"... and not a number
        let units = match decimals.parse::<u32>() {
            Ok(d) => Units::Other(d),
            Err(_) => Units::try_from(decimals)?,
        };
        let n: NumberWithBase = parse_units(value, units.as_num())?.into();
        Ok(format!("{}", n))
    }

    /// Converts integers with specified decimals into fixed point numbers
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    /// use ethers_core::types::U256;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_fixed_point("10", "0")?, "10.");
    ///     assert_eq!(Cast::to_fixed_point("10", "1")?, "1.0");
    ///     assert_eq!(Cast::to_fixed_point("10", "2")?, "0.10");
    ///     assert_eq!(Cast::to_fixed_point("10", "3")?, "0.010");
    ///
    ///     assert_eq!(Cast::to_fixed_point("-10", "0")?, "-10.");
    ///     assert_eq!(Cast::to_fixed_point("-10", "1")?, "-1.0");
    ///     assert_eq!(Cast::to_fixed_point("-10", "2")?, "-0.10");
    ///     assert_eq!(Cast::to_fixed_point("-10", "3")?, "-0.010");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_fixed_point(value: &str, decimals: &str) -> Result<String> {
        let (sign, mut value, value_len) = {
            let number = NumberWithBase::parse_int(value, None)?;
            let sign = if number.is_nonnegative() { "" } else { "-" };
            let value = format!("{:#}", number);
            let value_stripped = value.strip_prefix('-').unwrap_or(&value).to_string();
            let value_len = value_stripped.len();
            (sign, value_stripped, value_len)
        };
        let decimals = NumberWithBase::parse_uint(decimals, None)?.number().low_u64() as usize;

        let value = if decimals >= value_len {
            // Add "0." and pad with 0s
            format!("0.{:0>1$}", value, decimals)
        } else {
            // Insert decimal at -idx (i.e 1 => decimal idx = -1)
            value.insert(value_len - decimals, '.');
            value
        };

        Ok(format!("{}{}", sign, value))
    }

    /// Concatencates hex strings
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::concat_hex(vec!["0x00".to_string(), "0x01".to_string()]), "0x0001");
    ///     assert_eq!(Cast::concat_hex(vec!["1".to_string(), "2".to_string()]), "0x12");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn concat_hex(values: Vec<String>) -> String {
        format!(
            "0x{}",
            values
                .into_iter()
                .map(|s| s.strip_prefix("0x").unwrap_or(&s).to_string())
                .collect::<Vec<String>>()
                .join("")
        )
    }

    /// Converts an Ethereum address to its checksum format
    /// according to [EIP-55](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md)
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    /// use ethers_core::types::Address;
    /// use std::str::FromStr;
    ///
    /// # fn main() -> eyre::Result<()> {
    /// let addr = Address::from_str("0xb7e390864a90b7b923c9f9310c6f98aafe43f707")?;
    /// let addr = Cast::to_checksum_address(&addr);
    /// assert_eq!(addr, "0xB7e390864a90b7b923C9f9310C6F98aafE43F707");
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_checksum_address(address: &Address) -> String {
        utils::to_checksum(address, None)
    }

    /// Converts a number into uint256 hex string with 0x prefix
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_uint256("100")?, "0x0000000000000000000000000000000000000000000000000000000000000064");
    ///     assert_eq!(Cast::to_uint256("192038293923")?, "0x0000000000000000000000000000000000000000000000000000002cb65fd1a3");
    ///     assert_eq!(
    ///         Cast::to_uint256("115792089237316195423570985008687907853269984665640564039457584007913129639935")?,
    ///         "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    ///     );
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_uint256(value: &str) -> Result<String> {
        let n = NumberWithBase::parse_uint(value, None)?;
        Ok(format!("{:#066x}", n))
    }

    /// Converts a number into int256 hex string with 0x prefix
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_int256("0")?, "0x0000000000000000000000000000000000000000000000000000000000000000");
    ///     assert_eq!(Cast::to_int256("100")?, "0x0000000000000000000000000000000000000000000000000000000000000064");
    ///     assert_eq!(Cast::to_int256("-100")?, "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff9c");
    ///     assert_eq!(Cast::to_int256("192038293923")?, "0x0000000000000000000000000000000000000000000000000000002cb65fd1a3");
    ///     assert_eq!(Cast::to_int256("-192038293923")?, "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffd349a02e5d");
    ///     assert_eq!(
    ///         Cast::to_int256("57896044618658097711785492504343953926634992332820282019728792003956564819967")?,
    ///         "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    ///     );
    ///     assert_eq!(
    ///         Cast::to_int256("-57896044618658097711785492504343953926634992332820282019728792003956564819968")?,
    ///         "0x8000000000000000000000000000000000000000000000000000000000000000"
    ///     );
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_int256(value: &str) -> Result<String> {
        let n = NumberWithBase::parse_int(value, None)?;
        Ok(format!("{:#066x}", n))
    }

    /// Converts an eth amount into a specified unit
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_unit("1 wei", "wei")?, "1");
    ///     assert_eq!(Cast::to_unit("1", "wei")?, "1");
    ///     assert_eq!(Cast::to_unit("1ether", "wei")?, "1000000000000000000");
    ///     assert_eq!(Cast::to_unit("100 gwei", "gwei")?, "100");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_unit(value: &str, unit: &str) -> Result<String> {
        let value = U256::from(LenientTokenizer::tokenize_uint(value)?);

        Ok(match unit {
            "eth" | "ether" => ethers_core::utils::format_units(value, 18)?
                .trim_end_matches(".000000000000000000")
                .to_string(),
            "gwei" | "nano" | "nanoether" => ethers_core::utils::format_units(value, 9)?
                .trim_end_matches(".000000000")
                .to_string(),
            "wei" => ethers_core::utils::format_units(value, 0)?.trim_end_matches(".0").to_string(),
            _ => eyre::bail!("invalid unit: \"{}\"", unit),
        })
    }

    /// Converts wei into an eth amount
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::from_wei("1", "gwei")?, "0.000000001");
    ///     assert_eq!(Cast::from_wei("12340000005", "gwei")?, "12.340000005");
    ///     assert_eq!(Cast::from_wei("10", "ether")?, "0.000000000000000010");
    ///     assert_eq!(Cast::from_wei("100", "eth")?, "0.000000000000000100");
    ///     assert_eq!(Cast::from_wei("17", "")?, "0.000000000000000017");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn from_wei(value: &str, unit: &str) -> Result<String> {
        let value = NumberWithBase::parse_int(value, None)?.number();

        Ok(match unit {
            "gwei" => format_units(value, 9),
            _ => format_units(value, 18),
        }?)
    }

    /// Converts an eth amount into wei
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_wei("1", "")?, "1000000000000000000");
    ///     assert_eq!(Cast::to_wei("100", "gwei")?, "100000000000");
    ///     assert_eq!(Cast::to_wei("100", "eth")?, "100000000000000000000");
    ///     assert_eq!(Cast::to_wei("1000", "ether")?, "1000000000000000000000");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_wei(value: &str, unit: &str) -> Result<String> {
        let wei = match unit {
            "gwei" => parse_units(value, 9),
            _ => parse_units(value, 18),
        }?;
        Ok(wei.to_string())
    }

    /// Decodes rlp encoded list with hex data
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::from_rlp("0xc0".to_string()).unwrap(), "[]");
    ///     assert_eq!(Cast::from_rlp("0x0f".to_string()).unwrap(), "\"0x0f\"");
    ///     assert_eq!(Cast::from_rlp("0x33".to_string()).unwrap(), "\"0x33\"");
    ///     assert_eq!(Cast::from_rlp("0xc161".to_string()).unwrap(), "[\"0x61\"]");
    ///     assert_eq!(Cast::from_rlp("0xc26162".to_string()).unwrap(), "[\"0x61\",\"0x62\"]");
    ///     Ok(())
    /// }
    /// ```
    pub fn from_rlp(value: impl AsRef<str>) -> Result<String> {
        let value = value.as_ref();
        let striped_value = strip_0x(value);
        let bytes = hex::decode(striped_value).expect("Could not decode hex");
        let item = rlp::decode::<Item>(&bytes).expect("Could not decode rlp");
        Ok(format!("{}", item))
    }

    /// Encodes hex data or list of hex data to hexadecimal rlp
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_rlp("[]").unwrap(),"0xc0".to_string());
    ///     assert_eq!(Cast::to_rlp("0x22").unwrap(),"0x22".to_string());
    ///     assert_eq!(Cast::to_rlp("[\"0x61\"]",).unwrap(), "0xc161".to_string());
    ///     assert_eq!(Cast::to_rlp( "[\"0xf1\",\"f2\"]").unwrap(), "0xc481f181f2".to_string());
    ///     Ok(())
    /// }
    /// ```
    pub fn to_rlp(value: &str) -> Result<String> {
        let val = serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.parse()?));
        let item = Item::value_to_item(&val)?;
        Ok(format!("0x{}", hex::encode(rlp::encode(&item))))
    }

    /// Converts a number of one base to another
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    /// use ethers_core::types::{I256, U256};
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::to_base("100", Some("10".to_string()), "16")?, "0x64");
    ///     assert_eq!(Cast::to_base("100", Some("10".to_string()), "oct")?, "0o144");
    ///     assert_eq!(Cast::to_base("100", Some("10".to_string()), "binary")?, "0b1100100");
    ///
    ///     assert_eq!(Cast::to_base("0xffffffffffffffff", None, "10")?, u64::MAX.to_string());
    ///     assert_eq!(Cast::to_base("0xffffffffffffffffffffffffffffffff", None, "dec")?, u128::MAX.to_string());
    ///     // U256::MAX overflows as internally it is being parsed as I256
    ///     assert_eq!(Cast::to_base("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", None, "decimal")?, I256::MAX.to_string());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_base(value: &str, base_in: Option<String>, base_out: &str) -> Result<String> {
        let base_in = Base::unwrap_or_detect(base_in, value)?;
        let base_out: Base = base_out.parse()?;
        if base_in == base_out {
            return Ok(value.to_string())
        }

        let mut n = NumberWithBase::parse_int(value, Some(base_in.to_string()))?;
        n.set_base(base_out);

        // Use Debug fmt
        Ok(format!("{:#?}", n))
    }

    /// Converts hexdata into bytes32 value
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// # fn main() -> eyre::Result<()> {
    /// let bytes = Cast::to_bytes32("1234")?;
    /// assert_eq!(bytes, "0x1234000000000000000000000000000000000000000000000000000000000000");
    ///
    /// let bytes = Cast::to_bytes32("0x1234")?;
    /// assert_eq!(bytes, "0x1234000000000000000000000000000000000000000000000000000000000000");
    ///
    /// let err = Cast::to_bytes32("0x123400000000000000000000000000000000000000000000000000000000000011").unwrap_err();
    /// assert_eq!(err.to_string(), "string >32 bytes");
    ///
    /// # Ok(())
    /// # }
    pub fn to_bytes32(s: &str) -> Result<String> {
        let s = strip_0x(s);
        if s.len() > 64 {
            eyre::bail!("string >32 bytes");
        }

        let padded = format!("{:0<64}", s);
        // need to use the Debug implementation
        Ok(format!("{:?}", H256::from_str(&padded)?))
    }

    /// Encodes string into bytes32 value
    pub fn format_bytes32_string(s: &str) -> Result<String> {
        let formatted = format_bytes32_string(s)?;
        Ok(format!("0x{}", hex::encode(formatted)))
    }

    /// Decodes string from bytes32 value
    pub fn parse_bytes32_string(s: &str) -> Result<String> {
        let s = strip_0x(s);
        if s.len() != 64 {
            eyre::bail!("string not 32 bytes");
        }

        let bytes = hex::decode(s)?;
        let mut buffer = [0u8; 32];
        buffer.copy_from_slice(&bytes);

        Ok(parse_bytes32_string(&buffer)?.to_owned())
    }

    /// Decodes abi-encoded hex input or output
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     // Passing `input = false` will decode the data as the output type.
    ///     // The input data types and the full function sig are ignored, i.e.
    ///     // you could also pass `balanceOf()(uint256)` and it'd still work.
    ///     let data = "0x0000000000000000000000000000000000000000000000000000000000000001";
    ///     let sig = "balanceOf(address, uint256)(uint256)";
    ///     let decoded = Cast::abi_decode(sig, data, false)?[0].to_string();
    ///     assert_eq!(decoded, "1");
    ///
    ///     // Passing `input = true` will decode the data with the input function signature.
    ///     let data = "0xf242432a0000000000000000000000008dbd1b711dc621e1404633da156fcc779e1c6f3e000000000000000000000000d9f3c9cc99548bf3b44a43e0a2d07399eb918adc000000000000000000000000000000000000000000000000000000000000002a000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000";
    ///     let sig = "safeTransferFrom(address, address, uint256, uint256, bytes)";
    ///     let decoded = Cast::abi_decode(sig, data, true)?;
    ///     let decoded = decoded.iter().map(ToString::to_string).collect::<Vec<_>>();
    ///     assert_eq!(
    ///         decoded,
    ///         vec!["8dbd1b711dc621e1404633da156fcc779e1c6f3e", "d9f3c9cc99548bf3b44a43e0a2d07399eb918adc", "2a", "1", ""]
    ///     );
    ///
    ///
    ///     # Ok(())
    /// }
    /// ```
    pub fn abi_decode(sig: &str, calldata: &str, input: bool) -> Result<Vec<Token>> {
        foundry_common::abi::abi_decode(sig, calldata, input)
    }

    /// Performs ABI encoding based off of the function signature. Does not include
    /// the function selector in the result.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    ///
    /// # fn main() -> eyre::Result<()> {
    ///     assert_eq!(
    ///         "0x0000000000000000000000000000000000000000000000000000000000000001",
    ///         Cast::abi_encode("f(uint a)", &["1"]).unwrap().as_str()
    ///     );
    ///     assert_eq!(
    ///         "0x0000000000000000000000000000000000000000000000000000000000000001",
    ///         Cast::abi_encode("constructor(uint a)", &["1"]).unwrap().as_str()
    ///     );
    /// #    Ok(())
    /// # }
    /// ```
    pub fn abi_encode(sig: &str, args: &[impl AsRef<str>]) -> Result<String> {
        let func = match HumanReadableParser::parse_function(sig) {
            Ok(func) => func,
            Err(err) => {
                if let Ok(constructor) = HumanReadableParser::parse_constructor(sig) {
                    #[allow(deprecated)]
                    Function {
                        name: "constructor".to_string(),
                        inputs: constructor.inputs,
                        outputs: vec![],
                        constant: None,
                        state_mutability: Default::default(),
                    }
                } else {
                    // we return the `Function` parse error as this case is more likely
                    return Err(err.into())
                }
            }
        };
        let calldata = encode_args(&func, args)?.to_hex::<String>();
        let encoded = &calldata[8..];
        Ok(format!("0x{encoded}"))
    }

    /// Performs ABI encoding to produce the hexadecimal calldata with the given arguments.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    ///
    /// # fn main() -> eyre::Result<()> {
    ///     assert_eq!(
    ///         "0xb3de648b0000000000000000000000000000000000000000000000000000000000000001",
    ///         Cast::calldata_encode("f(uint a)", &["1"]).unwrap().as_str()
    ///     );
    /// #    Ok(())
    /// # }
    /// ```
    pub fn calldata_encode(sig: impl AsRef<str>, args: &[impl AsRef<str>]) -> Result<String> {
        let func = HumanReadableParser::parse_function(sig.as_ref())?;
        let calldata = encode_args(&func, args)?;
        Ok(format!("0x{}", calldata.to_hex::<String>()))
    }

    /// Generates an interface in solidity from either a local file ABI or a verified contract on
    /// Etherscan. It returns a vector of [`InterfaceSource`] structs that contain the source of the
    /// interface and their name.
    /// ```no_run
    /// use cast::SimpleCast as Cast;
    /// use cast::InterfacePath;
    /// # async fn foo() -> eyre::Result<()> {
    /// let path = InterfacePath::Local {
    ///     path: "utils/testdata/interfaceTestABI.json".to_owned(),
    ///     name: None,
    /// };
    /// let interfaces= Cast::generate_interface(path).await?;
    /// println!("interface {} {{\n {}\n}}", interfaces[0].name, interfaces[0].source);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn generate_interface(
        address_or_path: InterfacePath,
    ) -> Result<Vec<InterfaceSource>> {
        let (contract_abis, contract_names): (Vec<RawAbi>, Vec<String>) = match address_or_path {
            InterfacePath::Local { path, name } => {
                let file = std::fs::read_to_string(&path).wrap_err("unable to read abi file")?;

                let mut json: serde_json::Value = serde_json::from_str(&file)?;
                let json = if !json["abi"].is_null() { json["abi"].take() } else { json };

                let abi: RawAbi =
                    serde_json::from_value(json).wrap_err("unable to parse json ABI from file")?;

                (vec![abi], vec![name.unwrap_or_else(|| "Interface".to_owned())])
            }
            InterfacePath::Etherscan { address, chain, api_key } => {
                let client = Client::new(chain, api_key)?;

                // get the source
                let contract_source = match client.contract_source_code(address).await {
                    Ok(src) => src,
                    Err(ethers_etherscan::errors::EtherscanError::InvalidApiKey) => {
                        eyre::bail!("Invalid Etherscan API key. Did you set it correctly? You may be using an API key for another Etherscan API chain (e.g. Ethereum API key for Polygonscan).")
                    }
                    Err(err) => {
                        eyre::bail!(err)
                    }
                };

                if contract_source
                    .items
                    .iter()
                    .any(|item| item.abi == "Contract source code not verified")
                {
                    eyre::bail!("Contract source code at {:?} on {} not verified. Maybe you have selected the wrong chain?", address, chain)
                }

                let contract_source_names = contract_source
                    .items
                    .iter()
                    .map(|item| item.contract_name.clone())
                    .collect::<Vec<String>>();

                let mut abis = Vec::with_capacity(contract_source.items.len());
                for item in &contract_source.items {
                    abis.push(serde_json::from_str(&item.abi)?);
                }

                (abis, contract_source_names)
            }
        };
        contract_abis
            .iter()
            .zip(&contract_names)
            .map(|(contract_abi, contract_name)| {
                let interface_source =
                    foundry_utils::abi::abi_to_solidity(contract_abi, contract_name)?;
                Ok(InterfaceSource { name: contract_name.to_owned(), source: interface_source })
            })
            .collect::<Result<Vec<InterfaceSource>>>()
    }

    /// Prints the slot number for the specified mapping type and input data
    /// Uses abi_encode to pad the data to 32 bytes.
    /// For value types v, slot number of v is keccak256(concat(h(v) , p)) where h is the padding
    /// function and p is slot number of the mapping.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    ///
    /// # fn main() -> eyre::Result<()> {
    ///
    ///    assert_eq!(Cast::index("address", "0xD0074F4E6490ae3f888d1d4f7E3E43326bD3f0f5" ,"2").unwrap().as_str(),"0x9525a448a9000053a4d151336329d6563b7e80b24f8e628e95527f218e8ab5fb");
    ///    assert_eq!(Cast::index("uint256","42" ,"6").unwrap().as_str(),"0xfc808b0f31a1e6b9cf25ff6289feae9b51017b392cc8e25620a94a38dcdafcc1");
    /// #    Ok(())
    /// # }
    /// ```
    pub fn index(from_type: &str, from_value: &str, slot_number: &str) -> Result<String> {
        let sig = format!("x({from_type},uint256)");
        let encoded = Self::abi_encode(&sig, &[from_value, slot_number])?;
        let location: String = Self::keccak(&encoded)?;
        Ok(location)
    }

    /// Converts ENS names to their namehash representation
    /// [Namehash reference](https://docs.ens.domains/contract-api-reference/name-processing#hashing-names)
    /// [namehash-rust reference](https://github.com/InstateDev/namehash-rust/blob/master/src/lib.rs)
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::namehash("")?, "0x0000000000000000000000000000000000000000000000000000000000000000");
    ///     assert_eq!(Cast::namehash("eth")?, "0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae");
    ///     assert_eq!(Cast::namehash("foo.eth")?, "0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f");
    ///     assert_eq!(Cast::namehash("sub.foo.eth")?, "0x500d86f9e663479e5aaa6e99276e55fc139c597211ee47d17e1e92da16a83402");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn namehash(ens: &str) -> Result<String> {
        let mut node = vec![0u8; 32];

        if !ens.is_empty() {
            let ens_lower = ens.to_lowercase();
            let mut labels: Vec<&str> = ens_lower.split('.').collect();
            labels.reverse();

            for label in labels {
                let mut label_hash = keccak256(label.as_bytes());
                node.append(&mut label_hash.to_vec());

                label_hash = keccak256(node.as_slice());
                node = label_hash.to_vec();
            }
        }

        let namehash: String = node.to_hex();
        Ok(format!("0x{namehash}"))
    }

    /// Keccak-256 hashes arbitrary data
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::keccak("foo")?, "0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d");
    ///     assert_eq!(Cast::keccak("123abc")?, "0xb1f1c74a1ba56f07a892ea1110a39349d40f66ca01d245e704621033cb7046a4");
    ///     assert_eq!(Cast::keccak("0x12")?, "0x5fa2358263196dbbf23d1ca7a509451f7a2f64c15837bfbb81298b1e3e24e4fa");
    ///     assert_eq!(Cast::keccak("12")?, "0x7f8b6b088b6d74c2852fc86c796dca07b44eed6fb3daf5e6b59f7c364db14528");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn keccak(data: &str) -> Result<String> {
        let hash = match data.as_bytes() {
            // 0x prefix => read as hex data
            [b'0', b'x', rest @ ..] => keccak256(hex::decode(rest)?),
            // No 0x prefix => read as text
            _ => keccak256(data),
        };

        Ok(format!("{:?}", H256(hash)))
    }

    /// Performs the left shift operation (<<) on a number
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::left_shift("16", "10", Some("10".to_string()), "hex")?, "0x4000");
    ///     assert_eq!(Cast::left_shift("255", "16", Some("dec".to_string()), "hex")?, "0xff0000");
    ///     assert_eq!(Cast::left_shift("0xff", "16", None, "hex")?, "0xff0000");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn left_shift(
        value: &str,
        bits: &str,
        base_in: Option<String>,
        base_out: &str,
    ) -> Result<String> {
        let base_out: Base = base_out.parse()?;
        let value = NumberWithBase::parse_uint(value, base_in)?;
        let bits = NumberWithBase::parse_uint(bits, None)?;

        let res = value.number() << bits.number();

        Ok(res.to_base(base_out, true)?)
    }

    /// Performs the right shift operation (>>) on a number
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Cast::right_shift("0x4000", "10", None, "dec")?, "16");
    ///     assert_eq!(Cast::right_shift("16711680", "16", Some("10".to_string()), "hex")?, "0xff");
    ///     assert_eq!(Cast::right_shift("0xff0000", "16", None, "hex")?, "0xff");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn right_shift(
        value: &str,
        bits: &str,
        base_in: Option<String>,
        base_out: &str,
    ) -> Result<String> {
        let base_out: Base = base_out.parse()?;
        let value = NumberWithBase::parse_uint(value, base_in)?;
        let bits = NumberWithBase::parse_uint(bits, None)?;

        let res = value.number() >> bits.number();

        Ok(res.to_base(base_out, true)?)
    }

    /// Fetches source code of verified contracts from etherscan.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    /// # use ethers_core::types::Chain;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    ///     assert_eq!(
    ///             "/*
    ///             - Bytecode Verification performed was compared on second iteration -
    ///             This file is part of the DAO.....",
    ///         Cast::etherscan_source(Chain::Mainnet, "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".to_string(), "<etherscan_api_key>".to_string()).await.unwrap().as_str()
    ///     );
    /// #    Ok(())
    /// # }
    /// ```
    pub async fn etherscan_source(
        chain: Chain,
        contract_address: String,
        etherscan_api_key: String,
    ) -> Result<String> {
        let client = Client::new(chain, etherscan_api_key)?;
        let meta = client.contract_source_code(contract_address.parse()?).await?;
        let code = meta.source_code();

        if code.is_empty() {
            return Err(eyre::eyre!("unverified contract"))
        }

        Ok(code)
    }

    /// Fetches the source code of verified contracts from etherscan and expands the resulting
    /// files to a directory for easy perusal.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    /// # use ethers_core::types::Chain;
    /// # use std::path::PathBuf;
    ///
    /// # async fn expand() -> eyre::Result<()> {
    ///      Cast::expand_etherscan_source_to_directory(Chain::Mainnet, "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".to_string(), "<etherscan_api_key>".to_string(), PathBuf::from("output_dir")).await?;
    /// #    Ok(())
    /// # }
    /// ```
    pub async fn expand_etherscan_source_to_directory(
        chain: Chain,
        contract_address: String,
        etherscan_api_key: String,
        output_directory: PathBuf,
    ) -> eyre::Result<()> {
        let client = Client::new(chain, etherscan_api_key)?;
        let meta = client.contract_source_code(contract_address.parse()?).await?;
        let source_tree = meta.source_tree()?;
        source_tree.write_to(&output_directory)?;
        Ok(())
    }
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::SimpleCast as Cast;

    #[test]
    fn calldata_uint() {
        assert_eq!(
            "0xb3de648b0000000000000000000000000000000000000000000000000000000000000001",
            Cast::calldata_encode("f(uint a)", &["1"]).unwrap().as_str()
        );
    }

    // <https://github.com/foundry-rs/foundry/issues/2681>
    #[test]
    fn calldata_array() {
        assert_eq!(
            "0xcde2baba0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000",
            Cast::calldata_encode("propose(string[])", &["[\"\"]"]).unwrap().as_str()
        );
    }

    #[test]
    fn calldata_bool() {
        assert_eq!(
            "0x6fae94120000000000000000000000000000000000000000000000000000000000000000",
            Cast::calldata_encode("bar(bool)", &["false"]).unwrap().as_str()
        );
    }

    #[test]
    fn concat_hex() {
        assert_eq!(Cast::concat_hex(vec!["0x00".to_string(), "0x01".to_string()]), "0x0001");
        assert_eq!(Cast::concat_hex(vec!["1".to_string(), "2".to_string()]), "0x12");
    }

    #[test]
    fn from_rlp() {
        let rlp = "0xf8b1a02b5df5f0757397573e8ff34a8b987b21680357de1f6c8d10273aa528a851eaca8080a02838ac1d2d2721ba883169179b48480b2ba4f43d70fcf806956746bd9e83f90380a0e46fff283b0ab96a32a7cc375cecc3ed7b6303a43d64e0a12eceb0bc6bd8754980a01d818c1c414c665a9c9a0e0c0ef1ef87cacb380b8c1f6223cb2a68a4b2d023f5808080a0236e8f61ecde6abfebc6c529441f782f62469d8a2cc47b7aace2c136bd3b1ff08080808080";
        let item = Cast::from_rlp(rlp).unwrap();
        assert_eq!(
            item,
            r#"["0x2b5df5f0757397573e8ff34a8b987b21680357de1f6c8d10273aa528a851eaca","0x","0x","0x2838ac1d2d2721ba883169179b48480b2ba4f43d70fcf806956746bd9e83f903","0x","0xe46fff283b0ab96a32a7cc375cecc3ed7b6303a43d64e0a12eceb0bc6bd87549","0x","0x1d818c1c414c665a9c9a0e0c0ef1ef87cacb380b8c1f6223cb2a68a4b2d023f5","0x","0x","0x","0x236e8f61ecde6abfebc6c529441f782f62469d8a2cc47b7aace2c136bd3b1ff0","0x","0x","0x","0x","0x"]"#
        )
    }
}
