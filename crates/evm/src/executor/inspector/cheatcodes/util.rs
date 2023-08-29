use super::{ensure, fmt_err, Cheatcodes, Error, Result};
use crate::{
    abi::HEVMCalls,
    executor::backend::{
        error::{DatabaseError, DatabaseResult},
        DatabaseExt,
    },
    utils::{h160_to_b160, h256_to_u256_be, ru256_to_u256, u256_to_ru256},
};
use bytes::{BufMut, Bytes, BytesMut};
use ethers::{
    abi::{AbiEncode, Address, ParamType, Token},
    core::k256::elliptic_curve::Curve,
    prelude::{
        k256::{
            ecdsa::SigningKey,
            elliptic_curve::{bigint::Encoding, sec1::ToEncodedPoint},
            Secp256k1,
        },
        LocalWallet, Signer, H160, *,
    },
    signers::{
        coins_bip39::{
            ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese,
            Korean, Portuguese, Spanish, Wordlist,
        },
        MnemonicBuilder,
    },
    types::{transaction::eip2718::TypedTransaction, NameOrAddress, H256, U256},
    utils::{self, keccak256},
};
use foundry_common::{fmt::*, RpcUrl};
use revm::{
    interpreter::CreateInputs,
    primitives::{Account, TransactTo},
    Database, EVMData, JournaledState,
};
use std::{collections::VecDeque, str::FromStr, path::Path};

const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";

/// Address of the default CREATE2 deployer 0x4e59b44847b379578588920ca78fbf26c0b4956c
pub const DEFAULT_CREATE2_DEPLOYER: H160 = H160([
    78, 89, 180, 72, 71, 179, 121, 87, 133, 136, 146, 12, 167, 143, 191, 38, 192, 180, 149, 108,
]);

pub const MAGIC_SKIP_BYTES: &[u8] = b"FOUNDRY::SKIP";

/// Helps collecting transactions from different forks.
#[derive(Debug, Clone, Default)]
pub struct BroadcastableTransaction {
    pub rpc: Option<RpcUrl>,
    pub transaction: TypedTransaction,
}

pub type BroadcastableTransactions = VecDeque<BroadcastableTransaction>;

/// Configures the env for the transaction
pub fn configure_tx_env(env: &mut revm::primitives::Env, tx: &Transaction) {
    env.tx.caller = h160_to_b160(tx.from);
    env.tx.gas_limit = tx.gas.as_u64();
    env.tx.gas_price = tx.gas_price.unwrap_or_default().into();
    env.tx.gas_priority_fee = tx.max_priority_fee_per_gas.map(Into::into);
    env.tx.nonce = Some(tx.nonce.as_u64());
    env.tx.access_list = tx
        .access_list
        .clone()
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|item| {
            (
                h160_to_b160(item.address),
                item.storage_keys.into_iter().map(h256_to_u256_be).map(u256_to_ru256).collect(),
            )
        })
        .collect();
    env.tx.value = tx.value.into();
    env.tx.data = tx.input.0.clone();
    env.tx.transact_to =
        tx.to.map(h160_to_b160).map(TransactTo::Call).unwrap_or_else(TransactTo::create)
}

/// Applies the given function `f` to the `revm::Account` belonging to the `addr`
///
/// This will ensure the `Account` is loaded and `touched`, see [`JournaledState::touch`]
pub fn with_journaled_account<F, R, DB: Database>(
    journaled_state: &mut JournaledState,
    db: &mut DB,
    addr: Address,
    mut f: F,
) -> Result<R, DB::Error>
where
    F: FnMut(&mut Account) -> R,
{
    let addr = h160_to_b160(addr);
    journaled_state.load_account(addr, db)?;
    journaled_state.touch(&addr);
    let account = journaled_state.state.get_mut(&addr).expect("account loaded;");
    Ok(f(account))
}

fn addr(private_key: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = utils::secret_key_to_address(&key);
    Ok(addr.encode().into())
}

fn sign(private_key: U256, digest: H256, chain_id: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());

    // The `ecrecover` precompile does not use EIP-155
    let sig = wallet.sign_hash(digest)?;
    let recovered = sig.recover(digest)?;

    assert_eq!(recovered, wallet.address());

    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    sig.r.to_big_endian(&mut r_bytes);
    sig.s.to_big_endian(&mut s_bytes);

    Ok((sig.v, r_bytes, s_bytes).encode().into())
}

/// Using a given private key, return its public ETH address, its public key affine x and y
/// coodinates, and its private key (see the 'Wallet' struct)
///
/// If 'label' is set to 'Some()', assign that label to the associated ETH address in state
fn create_wallet(private_key: U256, label: Option<String>, state: &mut Cheatcodes) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = utils::secret_key_to_address(&key);

    let pub_key = key.verifying_key().as_affine().to_encoded_point(false);
    let pub_key_x = pub_key.x().ok_or("No x coordinate was found")?;
    let pub_key_y = pub_key.y().ok_or("No y coordinate was found")?;

    let pub_key_x = U256::from(pub_key_x.as_slice());
    let pub_key_y = U256::from(pub_key_y.as_slice());

    if let Some(label) = label {
        state.labels.insert(addr, label);
    }

    Ok((addr, pub_key_x, pub_key_y, private_key).encode().into())
}

enum WordlistLang {
    ChineseSimplified,
    ChineseTraditional,
    Czech,
    English,
    French,
    Italian,
    Japanese,
    Korean,
    Portuguese,
    Spanish,
}

impl FromStr for WordlistLang {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "chinese_simplified" => Ok(Self::ChineseSimplified),
            "chinese_traditional" => Ok(Self::ChineseTraditional),
            "czech" => Ok(Self::Czech),
            "english" => Ok(Self::English),
            "french" => Ok(Self::French),
            "italian" => Ok(Self::Italian),
            "japanese" => Ok(Self::Japanese),
            "korean" => Ok(Self::Korean),
            "portuguese" => Ok(Self::Portuguese),
            "spanish" => Ok(Self::Spanish),
            _ => Err(format!("the language `{}` has no wordlist", input)),
        }
    }
}

fn derive_key<W: Wordlist>(mnemonic: &str, path: &str, index: u32) -> Result {
    let derivation_path =
        if path.ends_with('/') { format!("{path}{index}") } else { format!("{path}/{index}") };

    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(&derivation_path)?
        .build()?;

    let private_key = U256::from_big_endian(wallet.signer().to_bytes().as_slice());

    Ok(private_key.encode().into())
}

fn derive_key_with_wordlist(mnemonic: &str, path: &str, index: u32, lang: &str) -> Result {
    let lang = WordlistLang::from_str(lang)?;
    match lang {
        WordlistLang::ChineseSimplified => derive_key::<ChineseSimplified>(mnemonic, path, index),
        WordlistLang::ChineseTraditional => derive_key::<ChineseTraditional>(mnemonic, path, index),
        WordlistLang::Czech => derive_key::<Czech>(mnemonic, path, index),
        WordlistLang::English => derive_key::<English>(mnemonic, path, index),
        WordlistLang::French => derive_key::<French>(mnemonic, path, index),
        WordlistLang::Italian => derive_key::<Italian>(mnemonic, path, index),
        WordlistLang::Japanese => derive_key::<Japanese>(mnemonic, path, index),
        WordlistLang::Korean => derive_key::<Korean>(mnemonic, path, index),
        WordlistLang::Portuguese => derive_key::<Portuguese>(mnemonic, path, index),
        WordlistLang::Spanish => derive_key::<Spanish>(mnemonic, path, index),
    }
}

fn remember_key(state: &mut Cheatcodes, private_key: U256, chain_id: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());
    let address = wallet.address();

    state.script_wallets.push(wallet);

    Ok(address.encode().into())
}

pub fn parse(s: &str, ty: &ParamType) -> Result {
    parse_token(s, ty)
        .map(|token| abi::encode(&[token]).into())
        .map_err(|e| fmt_err!("Failed to parse `{s}` as type `{ty}`: {e}"))
}

pub fn skip(state: &mut Cheatcodes, depth: u64, skip: bool) -> Result {
    if !skip {
        return Ok(b"".into())
    }

    // Skip should not work if called deeper than at test level.
    // As we're not returning the magic skip bytes, this will cause a test failure.
    if depth > 1 {
        return Err(Error::custom("The skip cheatcode can only be used at test level"))
    }

    state.skip = true;
    Err(Error::custom_bytes(MAGIC_SKIP_BYTES))
}

#[instrument(level = "error", name = "util", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result> {
    Some(match call {
        HEVMCalls::Addr(inner) => addr(inner.0),
        // [function sign(uint256,bytes32)] Used to sign bytes32 digests using the given private key
        HEVMCalls::Sign0(inner) => sign(inner.0, inner.1.into(), data.env.cfg.chain_id.into()),
        // [function createWallet(string)] Used to derive private key and label the wallet with the
        // same string
        HEVMCalls::CreateWallet0(inner) => {
            create_wallet(U256::from(keccak256(&inner.0)), Some(inner.0.clone()), state)
        }
        // [function createWallet(uint256)] creates a new wallet with the given private key
        HEVMCalls::CreateWallet1(inner) => create_wallet(inner.0, None, state),
        // [function createWallet(uint256,string)] creates a new wallet with the given private key
        // and labels it with the given string
        HEVMCalls::CreateWallet2(inner) => create_wallet(inner.0, Some(inner.1.clone()), state),
        // [function sign(uint256,bytes32)] Used to sign bytes32 digests using the given Wallet's
        // private key
        HEVMCalls::Sign1(inner) => {
            sign(inner.0.private_key, inner.1.into(), data.env.cfg.chain_id.into())
        }
        HEVMCalls::DeriveKey0(inner) => {
            derive_key::<English>(&inner.0, DEFAULT_DERIVATION_PATH_PREFIX, inner.1)
        }
        HEVMCalls::DeriveKey1(inner) => derive_key::<English>(&inner.0, &inner.1, inner.2),
        HEVMCalls::DeriveKey2(inner) => {
            derive_key_with_wordlist(&inner.0, DEFAULT_DERIVATION_PATH_PREFIX, inner.1, &inner.2)
        }
        HEVMCalls::DeriveKey3(inner) => {
            derive_key_with_wordlist(&inner.0, &inner.1, inner.2, &inner.3)
        }
        HEVMCalls::RememberKey(inner) => remember_key(state, inner.0, data.env.cfg.chain_id.into()),
        HEVMCalls::Label(inner) => {
            state.labels.insert(inner.0, inner.1.clone());
            Ok(Default::default())
        }
        HEVMCalls::GetLabel(inner) => {
            let label = state
                .labels
                .get(&inner.0)
                .cloned()
                .unwrap_or_else(|| format!("unlabeled:{:?}", inner.0));
            Ok(ethers::abi::encode(&[Token::String(label)]).into())
        }
        HEVMCalls::ToString0(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString1(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString2(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString3(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString4(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString5(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ParseBytes(inner) => parse(&inner.0, &ParamType::Bytes),
        HEVMCalls::ParseAddress(inner) => parse(&inner.0, &ParamType::Address),
        HEVMCalls::ParseUint(inner) => parse(&inner.0, &ParamType::Uint(256)),
        HEVMCalls::ParseInt(inner) => parse(&inner.0, &ParamType::Int(256)),
        HEVMCalls::ParseBytes32(inner) => parse(&inner.0, &ParamType::FixedBytes(32)),
        HEVMCalls::ParseBool(inner) => parse(&inner.0, &ParamType::Bool),
        HEVMCalls::Skip(inner) => skip(state, data.journaled_state.depth(), inner.0),
        _ => return None,
    })
}

pub fn process_create<DB>(
    broadcast_sender: Address,
    bytecode: Bytes,
    data: &mut EVMData<'_, DB>,
    call: &mut CreateInputs,
) -> DatabaseResult<(Bytes, Option<NameOrAddress>, u64)>
where
    DB: Database<Error = DatabaseError>,
{
    let broadcast_sender = h160_to_b160(broadcast_sender);
    match call.scheme {
        revm::primitives::CreateScheme::Create => {
            call.caller = broadcast_sender;

            Ok((bytecode, None, data.journaled_state.account(broadcast_sender).info.nonce))
        }
        revm::primitives::CreateScheme::Create2 { salt } => {
            // Sanity checks for our CREATE2 deployer
            data.journaled_state.load_account(h160_to_b160(DEFAULT_CREATE2_DEPLOYER), data.db)?;

            let info = &data.journaled_state.account(h160_to_b160(DEFAULT_CREATE2_DEPLOYER)).info;
            match &info.code {
                Some(code) => {
                    if code.is_empty() {
                        trace!(create2=?DEFAULT_CREATE2_DEPLOYER, "Empty Create 2 deployer code");
                        return Err(DatabaseError::MissingCreate2Deployer)
                    }
                }
                None => {
                    // forked db
                    trace!(create2=?DEFAULT_CREATE2_DEPLOYER, "Missing Create 2 deployer code");
                    if data.db.code_by_hash(info.code_hash)?.is_empty() {
                        return Err(DatabaseError::MissingCreate2Deployer)
                    }
                }
            }

            call.caller = h160_to_b160(DEFAULT_CREATE2_DEPLOYER);

            // We have to increment the nonce of the user address, since this create2 will be done
            // by the create2_deployer
            let account = data.journaled_state.state().get_mut(&broadcast_sender).unwrap();
            let nonce = account.info.nonce;
            account.info.nonce += 1;

            // Proxy deployer requires the data to be on the following format `salt.init_code`
            let mut calldata = BytesMut::with_capacity(32 + bytecode.len());
            let salt = ru256_to_u256(salt);
            let mut salt_bytes = [0u8; 32];
            salt.to_big_endian(&mut salt_bytes);
            calldata.put_slice(&salt_bytes);
            calldata.put(bytecode);

            Ok((calldata.freeze(), Some(NameOrAddress::Address(DEFAULT_CREATE2_DEPLOYER)), nonce))
        }
    }
}

pub fn parse_array<I, T>(values: I, ty: &ParamType) -> Result
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut values = values.into_iter();
    match values.next() {
        Some(first) if !first.as_ref().is_empty() => {
            let tokens = std::iter::once(first)
                .chain(values)
                .map(|v| parse_token(v.as_ref(), ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(abi::encode(&[Token::Array(tokens)]).into())
        }
        // return the empty encoded Bytes when values is empty or the first element is empty
        _ => Ok(abi::encode(&[Token::String(String::new())]).into()),
    }
}

fn parse_token(s: &str, ty: &ParamType) -> Result<Token, String> {
    match ty {
        ParamType::Bool => {
            s.to_ascii_lowercase().parse().map(Token::Bool).map_err(|e| e.to_string())
        }
        ParamType::Uint(256) => parse_uint(s).map(Token::Uint),
        ParamType::Int(256) => parse_int(s).map(Token::Int),
        ParamType::Address => s.parse().map(Token::Address).map_err(|e| e.to_string()),
        ParamType::FixedBytes(32) => parse_bytes(s).map(Token::FixedBytes),
        ParamType::Bytes => parse_bytes(s).map(Token::Bytes),
        ParamType::String => Ok(Token::String(s.to_string())),
        _ => Err("unsupported type".into()),
    }
}

fn parse_int(s: &str) -> Result<U256, String> {
    // hex string may start with "0x", "+0x", or "-0x" which needs to be stripped for
    // `I256::from_hex_str`
    if s.starts_with("0x") || s.starts_with("+0x") || s.starts_with("-0x") {
        s.replacen("0x", "", 1).parse::<I256>().map_err(|err| err.to_string())
    } else {
        match I256::from_dec_str(s) {
            Ok(val) => Ok(val),
            Err(dec_err) => s.parse::<I256>().map_err(|hex_err| {
                format!("could not parse value as decimal or hex: {dec_err}, {hex_err}")
            }),
        }
    }
    .map(|v| v.into_raw())
}

fn parse_uint(s: &str) -> Result<U256, String> {
    if s.starts_with("0x") {
        s.parse::<U256>().map_err(|err| err.to_string())
    } else {
        match U256::from_dec_str(s) {
            Ok(val) => Ok(val),
            Err(dec_err) => s.parse::<U256>().map_err(|hex_err| {
                format!("could not parse value as decimal or hex: {dec_err}, {hex_err}")
            }),
        }
    }
}

fn parse_bytes(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).map_err(|e| e.to_string())
}

pub fn parse_private_key(private_key: U256) -> Result<SigningKey> {
    ensure!(!private_key.is_zero(), "Private key cannot be 0.");
    ensure!(
        private_key < U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()),
        "Private key must be less than the secp256k1 curve order \
        (115792089237316195423570985008687907852837564279074904382605163141518161494337).",
    );
    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);
    SigningKey::from_bytes((&bytes).into()).map_err(Into::into)
}

// Determines if the gas limit on a given call was manually set in the script and should therefore
// not be overwritten by later estimations
pub fn check_if_fixed_gas_limit<DB: DatabaseExt>(
    data: &EVMData<'_, DB>,
    call_gas_limit: u64,
) -> bool {
    // If the gas limit was not set in the source code it is set to the estimated gas left at the
    // time of the call, which should be rather close to configured gas limit.
    // TODO: Find a way to reliably make this determination. (for example by
    // generating it in the compilation or evm simulation process)
    U256::from(data.env.tx.gas_limit) > data.env.block.gas_limit.into() &&
        U256::from(call_gas_limit) <= data.env.block.gas_limit.into()
        // Transfers in forge scripts seem to be estimated at 2300 by revm leading to "Intrinsic
        // gas too low" failure when simulated on chain
        && call_gas_limit > 2300
}

/// Small utility function that checks if an address is a potential precompile.
pub fn is_potential_precompile(address: H160) -> bool {
    address < H160::from_low_u64_be(10) && address != H160::zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::abi::AbiDecode;

    #[test]
    fn test_uint_env() {
        let pk = "0x10532cc9d0d992825c3f709c62c969748e317a549634fb2a9fa949326022e81f";
        let val: U256 = pk.parse().unwrap();
        let parsed = parse(pk, &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(val, decoded);

        let parsed = parse(pk.strip_prefix("0x").unwrap(), &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(val, decoded);

        let parsed = parse("1337", &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(U256::from(1337u64), decoded);
    }

    #[test]
    fn test_int_env() {
        let val = U256::from(100u64);
        let parsed = parse(&val.to_string(), &ParamType::Int(256)).unwrap();
        let decoded = I256::decode(parsed).unwrap();
        assert_eq!(val, decoded.try_into().unwrap());

        let parsed = parse("100", &ParamType::Int(256)).unwrap();
        let decoded = I256::decode(parsed).unwrap();
        assert_eq!(U256::from(100u64), decoded.try_into().unwrap());
    }
}
