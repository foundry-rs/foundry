use super::{ensure, Cheatcodes, Error, Result};
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
    abi::Address,
    core::k256::elliptic_curve::Curve,
    prelude::{
        k256::{ecdsa::SigningKey, elliptic_curve::bigint::Encoding, Secp256k1},
        H160, *,
    },
    types::{transaction::eip2718::TypedTransaction, NameOrAddress, U256},
};
use foundry_common::RpcUrl;
use revm::{
    interpreter::CreateInputs,
    primitives::{Account, TransactTo},
    Database, EVMData, JournaledState,
};
use std::collections::VecDeque;

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

/// Skip the current test, by returning a magic value that will be checked by the test runner.
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
