//! Seth
//!
//! TODO
use chrono::NaiveDateTime;
use ethers_core::{
    abi::{
        token::{LenientTokenizer, StrictTokenizer, Tokenizer},
        AbiParser, ParamType, Token,
    },
    types::*,
    utils::{self, keccak256},
};
use ethers_providers::{Middleware, PendingTransaction};
use eyre::Result;
use rustc_hex::{FromHexIter, ToHex};
use std::str::FromStr;

use dapp_utils::{encode_args, get_func, to_table};
use eyre::WrapErr;

// TODO: SethContract with common contract initializers? Same for SethProviders?

pub struct Seth<M> {
    provider: M,
}

impl<M: Middleware> Seth<M>
where
    M::Error: 'static,
{
    /// Converts ASCII text input to hex
    ///
    /// ```
    /// use seth::Seth;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let seth = Seth::new(provider);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(provider: M) -> Self {
        Self { provider }
    }

    /// Makes a read-only call to the specified address
    ///
    /// ```no_run
    /// 
    /// use seth::Seth;
    /// use ethers_core::types::Address;
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let seth = Seth::new(provider);
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "function greeting(uint256 i) public returns (string)";
    /// let args = vec!["5".to_owned()];
    /// let data = seth.call(to, sig, args).await?;
    /// println!("{}", data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call<T: Into<NameOrAddress>>(
        &self,
        to: T,
        sig: &str,
        args: Vec<String>,
    ) -> Result<String> {
        let func = get_func(sig)?;
        let data = encode_args(&func, &args)?;

        // make the call
        let tx = Eip1559TransactionRequest::new().to(to).data(data).into();
        let res = self.provider.call(&tx, None).await?;

        // decode args into tokens
        let res = func.decode_output(res.as_ref())?;

        // concatenate them
        let mut s = String::new();
        for output in res {
            s.push_str(&format!("{}\n", output));
        }

        // return string
        Ok(s)
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
    /// ```no_run
    /// use seth::Seth;
    /// use ethers_core::types::Address;
    /// use ethers_providers::{Provider, Http};
    /// use std::{str::FromStr, convert::TryFrom};
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let seth = Seth::new(provider);
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let sig = "function greet(string memory) public returns (string)";
    /// let args = vec!["5".to_owned()];
    /// let data = seth.call(to, sig, args).await?;
    /// println!("{}", data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send<F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
        &self,
        from: F,
        to: T,
        args: Option<(&str, Vec<String>)>,
    ) -> Result<PendingTransaction<'_, M::Provider>> {
        let from = match from.into() {
            NameOrAddress::Name(ref ens_name) => self.provider.resolve_name(ens_name).await?,
            NameOrAddress::Address(addr) => addr,
        };

        // make the call
        let mut tx = Eip1559TransactionRequest::new().from(from).to(to);

        if let Some((sig, args)) = args {
            let func = get_func(sig)?;
            let data = encode_args(&func, &args)?;
            tx = tx.data(data);
        }

        let res = self.provider.send_transaction(tx, None).await?;

        Ok::<_, eyre::Error>(res)
    }

    /// ```no_run
    /// use seth::Seth;
    /// use ethers_providers::{Provider, Http};
    /// use std::convert::TryFrom;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    /// let seth = Seth::new(provider);
    /// let block = seth.block(5, true, None, false).await?;
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
                // TODO: Use custom serializer to serialize
                // u256s as decimals
                serde_json::to_value(&block)?
                    .get(field)
                    .cloned()
                    .ok_or_else(|| eyre::eyre!("field {} not found", field))?
            } else {
                serde_json::to_value(&block)?
            }
        } else {
            let block = self
                .provider
                .get_block(block)
                .await?
                .ok_or_else(|| eyre::eyre!("block {:?} not found", block))?;
            if let Some(ref field) = field {
                serde_json::to_value(block)?
                    .get(field)
                    .cloned()
                    .ok_or_else(|| eyre::eyre!("field {} not found", field))?
            } else {
                serde_json::to_value(&block)?
            }
        };

        let block = if to_json { serde_json::to_string(&block)? } else { to_table(block) };

        Ok(block)
    }

    async fn block_field_as_num<T: Into<BlockId>>(&self, block: T, field: String) -> Result<U256> {
        let block = block.into();
        let block_field = Seth::block(
            self,
            block,
            false,
            // Select only select field
            Some(field),
            false,
        )
        .await?;
        Ok(U256::from_str_radix(strip_0x(&block_field), 16)
            .expect("Unable to convert hexadecimal to U256"))
    }

    pub async fn base_fee<T: Into<BlockId>>(&self, block: T) -> Result<U256> {
        Ok(Seth::block_field_as_num(self, block, String::from("baseFeePerGas")).await?)
    }

    pub async fn age<T: Into<BlockId>>(&self, block: T) -> Result<String> {
        let timestamp_str =
            Seth::block_field_as_num(self, block, String::from("timestamp")).await?.to_string();
        let datetime = NaiveDateTime::from_timestamp(timestamp_str.parse::<i64>().unwrap(), 0);
        Ok(datetime.format("%a %b %e %H:%M:%S %Y").to_string())
    }

    pub async fn chain(&self) -> Result<&str> {
        let genesis_hash = Seth::block(
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
                match &(Seth::block(self, 1920000, false, Some(String::from("hash")), false)
                    .await?)[..]
                {
                    "0x94365e3a8c0b35089c1d1195081fe7489b528a84b22199c916180db8b28ade7f" => {
                        "etclive"
                    }
                    _ => "ethlive",
                }
            }
            "0xa3c565fc15c7478862d50ccd6561e3c06b24cc509bf388941c25ea985ce32cb9" => "kovan",
            "0x41941023680923e0fe4d74a34bdac8141f2540e3ae90623718e47d66d1ca4a2d" => "ropsten",
            "0x39e1b9259598b65c8c71d1ea153de17e89222e64e8b271213dfb92c231f7fb88" => {
                "optimism-mainnet"
            }
            "0x2510549c5c30f15472b55dbae139122e2e593f824217eefc7a53f78698ac5c1e" => {
                "optimism-kovan"
            }
            "0x7ee576b35482195fc49205cec9af72ce14f003b9ae69f6ba0faef4514be8b442" => {
                "arbitrum-mainnet"
            }
            "0x0cd786a2425d16f152c658316c423e6ce1181e15c3295826d7c9904cba9ce303" => "morden",
            "0x6341fd3daf94b748c72ced5a5b26028f2474f5f00d824504e4fa37a75767e177" => "rinkeby",
            "0xbf7e331f7f7c1dd2e05159666b3bf8bc7a8a3a9eb1d518969eab529dd9b88c1a" => "goerli",
            "0x14c2283285a88fe5fce9bf5c573ab03d6616695d717b12a127188bcacfc743c4" => "kotti",
            "0x6d3c66c5357ec91d5c43af47e234a939b22557cbb552dc45bebbceeed90fbe34" => "bsctest",
            "0x0d21840abff46b96c84b2ac9e10e4f5cdaeb5693cb665db62a2f3b02d2d57b5b" => "bsc",
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
}

pub struct SimpleSeth;
impl SimpleSeth {
    /// Converts UTF-8 text input to hex
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// let bin = Seth::from_utf8("yo");
    /// assert_eq!(bin, "0x796f")
    /// ```
    pub fn from_utf8(s: &str) -> String {
        let s: String = s.as_bytes().to_hex();
        format!("0x{}", s)
    }

    /// Converts hex data into text data
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!("Hello, World!", Seth::ascii("48656c6c6f2c20576f726c6421")?);
    ///     assert_eq!("TurboDappTools", Seth::ascii("0x547572626f44617070546f6f6c73")?);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn ascii(hex: &str) -> Result<String> {
        let hex_trimmed = hex.trim_start_matches("0x");
        let iter = FromHexIter::new(hex_trimmed);
        let mut ascii = String::new();
        for letter in iter.collect::<Vec<_>>() {
            ascii.push(letter.unwrap() as char);
        }
        Ok(ascii)
    }

    /// Converts hex input to decimal
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    /// use ethers_core::types::U256;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(U256::from_dec_str("424242")?, Seth::to_dec("0x67932")?);
    ///     assert_eq!(U256::from_dec_str("1234")?, Seth::to_dec("0x4d2")?);
    ///
    ///     Ok(())
    /// }
    pub fn to_dec(hex: &str) -> Result<U256> {
        Ok(U256::from_str(hex)?)
    }

    /// Converts integers with specified decimals into fixed point numbers
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    /// use ethers_core::types::U256;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::to_fix(0, 10.into())?, "10.");
    ///     assert_eq!(Seth::to_fix(1, 10.into())?, "1.0");
    ///     assert_eq!(Seth::to_fix(2, 10.into())?, "0.10");
    ///     assert_eq!(Seth::to_fix(3, 10.into())?, "0.010");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_fix(decimals: u128, value: U256) -> Result<String> {
        let mut value: String = value.to_string();
        let decimals = decimals as usize;

        if decimals >= value.len() {
            // {0}.{0 * (number_of_decimals - value.len())}{value}
            Ok(format!("0.{:0>1$}", value, decimals))
        } else {
            // Insert decimal at -idx (i.e 1 => decimal idx = -1)
            value.insert(value.len() - decimals, '.');
            Ok(value)
        }
    }

    /// Converts decimal input to hex
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    /// use ethers_core::types::U256;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::hex(U256::from_dec_str("424242")?), "0x67932");
    ///     assert_eq!(Seth::hex(U256::from_dec_str("1234")?), "0x4d2");
    ///     assert_eq!(Seth::hex(U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935")?), "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn hex(u: U256) -> String {
        format!("{:#x}", u)
    }

    /// Converts a number into uint256 hex string with 0x prefix
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::to_uint256("100".to_string())?, "0x0000000000000000000000000000000000000000000000000000000000000064");
    ///     assert_eq!(Seth::to_uint256("192038293923".to_string())?, "0x0000000000000000000000000000000000000000000000000000002cb65fd1a3");
    ///     assert_eq!(
    ///         Seth::to_uint256("115792089237316195423570985008687907853269984665640564039457584007913129639935".to_string())?,
    ///         "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    ///     );
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_uint256(value: String) -> Result<String> {
        let num_u256 = U256::from_str_radix(&value, 10)?;
        let num_hex = format!("{:x}", num_u256);
        Ok(format!("0x{}{}", "0".repeat(64 - num_hex.len()), num_hex))
    }

    /// Converts an eth amount into wei
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::to_wei(1.into(), "".to_string())?, "1");
    ///     assert_eq!(Seth::to_wei(100.into(), "gwei".to_string())?, "100000000000");
    ///     assert_eq!(Seth::to_wei(100.into(), "eth".to_string())?, "100000000000000000000");
    ///     assert_eq!(Seth::to_wei(1000.into(), "ether".to_string())?, "1000000000000000000000");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to_wei(value: U256, unit: String) -> Result<String> {
        let value = value.to_string();
        Ok(match &unit[..] {
            "gwei" => format!("{:0<1$}", value, 9 + value.len()),
            "eth" | "ether" => format!("{:0<1$}", value, 18 + value.len()),
            _ => value,
        })
    }

    /// Converts an Ethereum address to its checksum format
    /// according to [EIP-55](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md)
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    /// use ethers_core::types::Address;
    /// use std::str::FromStr;
    ///
    /// # fn main() -> eyre::Result<()> {
    /// let addr = Address::from_str("0xb7e390864a90b7b923c9f9310c6f98aafe43f707")?;
    /// let addr = Seth::checksum_address(&addr)?;
    /// assert_eq!(addr, "0xB7e390864a90b7b923C9f9310C6F98aafE43F707");
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn checksum_address(address: &Address) -> Result<String> {
        Ok(utils::to_checksum(address, None))
    }

    /// Converts hexdata into bytes32 value
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// # fn main() -> eyre::Result<()> {
    /// let bytes = Seth::bytes32("1234")?;
    /// assert_eq!(bytes, "0x1234000000000000000000000000000000000000000000000000000000000000");
    ///
    /// let bytes = Seth::bytes32("0x1234")?;
    /// assert_eq!(bytes, "0x1234000000000000000000000000000000000000000000000000000000000000");
    ///
    /// let err = Seth::bytes32("0x123400000000000000000000000000000000000000000000000000000000000011").unwrap_err();
    /// assert_eq!(err.to_string(), "string >32 bytes");
    ///
    /// # Ok(())
    /// # }
    pub fn bytes32(s: &str) -> Result<String> {
        let s = strip_0x(s);
        if s.len() > 64 {
            eyre::bail!("string >32 bytes");
        }

        let padded = format!("0x{:0<64}", s);
        // need to use the Debug implementation
        Ok(format!("{:?}", H256::from_str(&padded)?))
    }

    /// Keccak-256 hashes arbitrary data
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::keccak("foo")?, "0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d");
    ///     assert_eq!(Seth::keccak("123abc")?, "0xb1f1c74a1ba56f07a892ea1110a39349d40f66ca01d245e704621033cb7046a4");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn keccak(data: &str) -> Result<String> {
        let hash: String = keccak256(data.as_bytes()).to_hex();
        Ok(format!("0x{}", hash))
    }

    /// Converts ENS names to their namehash representation
    /// [Namehash reference](https://docs.ens.domains/contract-api-reference/name-processing#hashing-names)
    /// [namehash-rust reference](https://github.com/InstateDev/namehash-rust/blob/master/src/lib.rs)
    ///
    /// ```
    /// use seth::SimpleSeth as Seth;
    ///
    /// fn main() -> eyre::Result<()> {
    ///     assert_eq!(Seth::namehash("")?, "0x0000000000000000000000000000000000000000000000000000000000000000");
    ///     assert_eq!(Seth::namehash("eth")?, "0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae");
    ///     assert_eq!(Seth::namehash("foo.eth")?, "0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f");
    ///     assert_eq!(Seth::namehash("sub.foo.eth")?, "0x500d86f9e663479e5aaa6e99276e55fc139c597211ee47d17e1e92da16a83402");
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
        Ok(format!("0x{}", namehash))
    }

    /// Parses string input as Token against the expected ParamType
    pub fn parse_tokens(params: &[(ParamType, &str)], lenient: bool) -> eyre::Result<Vec<Token>> {
        params
            .iter()
            .map(|&(ref param, value)| {
                if lenient {
                    LenientTokenizer::tokenize(param, value)
                } else {
                    StrictTokenizer::tokenize(param, value)
                }
            })
            .collect::<Result<_, _>>()
            .wrap_err("Failed to parse tokens")
    }

    /// Performs ABI encoding to produce the hexadecimal calldata with the given arguments.
    ///
    /// ```
    /// # use seth::SimpleSeth as Seth;
    ///
    /// # fn main() -> eyre::Result<()> {
    ///     assert_eq!(
    ///         "0xb3de648b0000000000000000000000000000000000000000000000000000000000000001",
    ///         Seth::calldata("f(uint a)", &["1"]).unwrap().as_str()
    ///     );
    /// #    Ok(())
    /// # }
    /// ```
    pub fn calldata(sig: impl AsRef<str>, args: &[impl AsRef<str>]) -> Result<String> {
        let fun = AbiParser::default().parse_function(sig.as_ref())?;
        let params: Vec<_> = fun
            .inputs
            .iter()
            .map(|param| param.kind.clone())
            .zip(args.iter().map(AsRef::as_ref))
            .collect();
        let tokens = SimpleSeth::parse_tokens(&params, true)?;
        let calldata = fun.encode_input(&tokens)?;
        Ok(format!("0x{}", calldata.to_hex::<String>()))
    }
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::SimpleSeth as Seth;

    #[test]
    fn calldata_uint() {
        assert_eq!(
            "0xb3de648b0000000000000000000000000000000000000000000000000000000000000001",
            Seth::calldata("f(uint a)", &["1"]).unwrap().as_str()
        );
    }

    #[test]
    fn calldata_bool() {
        assert_eq!(
            "0x6fae94120000000000000000000000000000000000000000000000000000000000000000",
            Seth::calldata("bar(bool)", &["false"]).unwrap().as_str()
        );
    }
}
