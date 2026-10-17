//! Minimal Arc native USDC transfer compatibility.
//!
//! Implements Native Coin Authority transfers and Native Coin Control blocklist queries used by
//! the deployed ERC-20 USDC contract.
//! This does not implement Arc's transaction validation, native CALL/CREATE/SELFDESTRUCT rules,
//! minting, burning, or block execution. Rules follow `circlefin/arc-node` v0.8.0.
//!
//! <https://github.com/circlefin/arc-node/blob/v0.8.0/crates/precompiles/src/native_coin_authority.rs>

use alloy_evm::{
    EvmInternals,
    precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap},
};
use alloy_primitives::{Address, Bytes, Log, U256, address, keccak256};
use alloy_sol_types::{SolCall, SolEvent, SolValue, sol};
use revm::{
    context_interface::journaled_state::TransferError,
    interpreter::{
        Gas,
        gas::{COLD_ACCOUNT_ACCESS_COST, COLD_SLOAD_COST, WARM_STORAGE_READ_COST},
    },
    precompile::{
        PrecompileError, PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult,
    },
};

/// Arc's Native Coin Authority precompile.
pub const NATIVE_COIN_AUTHORITY: Address = address!("1800000000000000000000000000000000000000");

/// Native Coin Control stores Arc's native-currency blocklist.
pub const NATIVE_COIN_CONTROL: Address = address!("1800000000000000000000000000000000000001");

/// Only the native USDC proxy may transfer another account's native balance.
const NATIVE_USDC: Address = address!("3600000000000000000000000000000000000000");

/// Arc emits native transfer logs at the system address, in native (18-decimal) units.
const SYSTEM_ADDRESS: Address = address!("fffffffffffffffffffffffffffffffffffffffe");

sol! {
    function transfer(address from, address to, uint256 amount) external returns (bool);
    function isBlocklisted(address account) external view returns (bool);
    event Transfer(address indexed from, address indexed to, uint256 value);
}

/// Installs native USDC transfer compatibility on Arc mainnet and testnet.
pub fn apply_precompiles(precompiles: &mut PrecompilesMap, chain_id: u64) {
    if !matches!(chain_id, 5_042 | 5_042_002) {
        return;
    }
    // Match Arc's provider: lookup-only precompiles are cold until accessed, unlike Ethereum's
    // always-warm precompile set. Preserve any previously installed lookup for other addresses.
    precompiles.map_precompile_lookup(move |address, previous| {
        precompile(chain_id, address).or_else(|| previous.and_then(|lookup| lookup.lookup(address)))
    });
}

/// Resolves a cold native USDC precompile using the active chain ID.
pub fn precompile(chain_id: u64, address: &Address) -> Option<DynPrecompile> {
    if !matches!(chain_id, 5_042 | 5_042_002) {
        return None;
    }
    match *address {
        NATIVE_COIN_AUTHORITY => Some(DynPrecompile::new_stateful(
            PrecompileId::Custom("ArcNativeCoinAuthority".into()),
            native_coin_authority,
        )),
        NATIVE_COIN_CONTROL => Some(DynPrecompile::new_stateful(
            PrecompileId::Custom("ArcNativeCoinControl".into()),
            native_coin_control,
        )),
        _ => None,
    }
}

fn native_coin_authority(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let mut gas = Gas::new(input.gas());
    let result = transfer_native(&mut input, &mut gas).map(|()| true.abi_encode().into());
    precompile_output(gas, input.reservoir, result)
}

fn native_coin_control(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let mut gas = Gas::new(input.gas());
    let result = isBlocklistedCall::abi_decode_validate(input.data())
        .map_err(|_| TransferFailure::EarlyRevert("Execution reverted"))
        .and_then(|args| {
            if gas.remaining() < COLD_SLOAD_COST {
                return Err(TransferFailure::OutOfGas);
            }
            is_blocklisted(input.internals_mut(), args.account, &mut gas)
        })
        .map(|blocked| blocked.abi_encode().into());
    precompile_output(gas, input.reservoir, result)
}

fn precompile_output(
    gas: Gas,
    reservoir: u64,
    result: Result<Bytes, TransferFailure>,
) -> PrecompileResult {
    let mut gas = gas;
    let output = match result {
        Ok(output) => PrecompileOutput::new(gas.used(), output, reservoir),
        Err(TransferFailure::Revert(message)) => {
            PrecompileOutput::revert(gas.used(), revert_data(message), reservoir)
        }
        Err(TransferFailure::EarlyRevert(message)) => {
            if gas.record_regular_cost(200) {
                PrecompileOutput::revert(gas.used(), revert_data(message), reservoir)
            } else {
                PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir)
            }
        }
        Err(TransferFailure::OutOfGas) => {
            PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir)
        }
        Err(TransferFailure::Fatal(message)) => return Err(PrecompileError::Fatal(message)),
    };
    Ok(output)
}

fn transfer_native(input: &mut PrecompileInput<'_>, gas: &mut Gas) -> Result<(), TransferFailure> {
    if input.is_static_call() {
        gas.spend_all();
        return Err(TransferFailure::Revert("State change during static call"));
    }
    let args = transferCall::abi_decode_validate(input.data())
        .map_err(|_| TransferFailure::EarlyRevert("Execution reverted"))?;
    if input.caller != NATIVE_USDC {
        return Err(TransferFailure::EarlyRevert("Not enabled for native coin transfers"));
    }
    if !input.is_direct_call() {
        return Err(TransferFailure::EarlyRevert("Delegate call not allowed"));
    }
    if args.from.is_zero() || args.to.is_zero() {
        return Err(TransferFailure::EarlyRevert("Zero address not allowed"));
    }
    for address in [args.from, args.to] {
        if is_blocklisted(input.internals_mut(), address, gas)? {
            return Err(TransferFailure::EarlyRevert("Blocked address"));
        }
    }
    if args.amount.is_zero() {
        return Ok(());
    }

    let from =
        input.internals_mut().load_account(args.from).map_err(TransferFailure::from_error)?;
    charge(gas, if from.is_cold { COLD_ACCOUNT_ACCESS_COST } else { WARM_STORAGE_READ_COST })?;
    let remaining = from
        .info
        .balance
        .checked_sub(args.amount)
        .ok_or(TransferFailure::Revert("Insufficient funds"))?;
    let empty_sender = from.info.nonce == 0 && from.info.is_code_hash_empty_or_zero();
    // Zero8 permits draining an otherwise empty account. Earlier Arc revisions reject it.
    let zero8 = match input.internals.chain_id() {
        5_042 => 1_789_052_400u64,
        _ => 1_788_447_600u64,
    };
    if input.internals.block_timestamp() < U256::from(zero8) && remaining.is_zero() && empty_sender
    {
        charge(gas, 2900)?;
        return Err(TransferFailure::Revert("Cannot clear balance of empty account"));
    }
    charge(gas, 2900)?;
    let to = input.internals_mut().load_account(args.to).map_err(TransferFailure::from_error)?;
    charge(gas, if to.is_cold { COLD_ACCOUNT_ACCESS_COST } else { WARM_STORAGE_READ_COST })?;
    charge(gas, 2900)?;
    if to.is_selfdestructed() {
        return Err(TransferFailure::Revert(
            "Cannot increase the balance of selfdestructed account",
        ));
    }
    if to.info.is_empty() {
        charge(gas, 25_000)?;
    }
    // Reserve log gas before changing balances. The EVM's precompile checkpoint owns rollback.
    if args.from != args.to {
        charge(gas, 375 + 3 * 375 + 32 * 8)?;
    }
    let internals = input.internals_mut();
    if let Some(error) =
        internals.transfer(args.from, args.to, args.amount).map_err(TransferFailure::from_error)?
    {
        return Err(TransferFailure::Revert(match error {
            TransferError::OutOfFunds => "Insufficient funds",
            TransferError::OverflowPayment => "Arithmetic overflow",
            TransferError::CreateCollision => "Execution reverted",
        }));
    }
    if args.from != args.to {
        internals.log(Log {
            address: SYSTEM_ADDRESS,
            data: Transfer { from: args.from, to: args.to, value: args.amount }.encode_log_data(),
        });
    }
    Ok(())
}

enum TransferFailure {
    Revert(&'static str),
    EarlyRevert(&'static str),
    OutOfGas,
    Fatal(String),
}

impl TransferFailure {
    fn from_error(error: impl std::fmt::Debug) -> Self {
        Self::Fatal(format!("Arc native transfer failed: {error:?}"))
    }
}

fn charge(gas: &mut Gas, cost: u64) -> Result<(), TransferFailure> {
    gas.record_regular_cost(cost).then_some(()).ok_or(TransferFailure::OutOfGas)
}

fn revert_data(message: &str) -> Bytes {
    let mut data = vec![0x08, 0xc3, 0x79, 0xa0];
    data.extend(message.abi_encode());
    data.into()
}

fn is_blocklisted(
    internals: &mut EvmInternals<'_>,
    address: Address,
    gas: &mut Gas,
) -> Result<bool, TransferFailure> {
    let mut key = [0; 64];
    key[12..32].copy_from_slice(address.as_slice());
    key[63] = 2;
    internals.load_account(NATIVE_COIN_CONTROL).map_err(TransferFailure::from_error)?;
    let status = internals
        .sload(NATIVE_COIN_CONTROL, keccak256(key).into())
        .map_err(TransferFailure::from_error)?;
    charge(gas, if status.is_cold { COLD_SLOAD_COST } else { WARM_STORAGE_READ_COST })?;
    Ok(!status.data.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_evm::{EthEvmFactory, Evm, EvmEnv, EvmFactory};
    use alloy_primitives::TxKind;
    use revm::{
        context::{
            BlockEnv, CfgEnv, TxEnv,
            result::{ExecutionResult, ResultAndState},
        },
        database::InMemoryDB,
        primitives::hardfork::SpecId,
        state::AccountInfo,
    };

    const SENDER: Address = Address::with_last_byte(0x42);
    const RECIPIENT: Address = Address::with_last_byte(0x43);

    fn execute_transfer(caller: Address, amount: u64, gas_limit: u64) -> ResultAndState {
        execute_transfer_at(caller, amount, gas_limit, 1_800_000_000)
    }

    fn execute_transfer_at(
        caller: Address,
        amount: u64,
        gas_limit: u64,
        timestamp: u64,
    ) -> ResultAndState {
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            SENDER,
            AccountInfo { balance: U256::from(100), ..Default::default() },
        );
        let env = EvmEnv::new(
            CfgEnv::new().with_chain_id(5_042_002).with_spec_and_mainnet_gas_params(SpecId::OSAKA),
            BlockEnv { timestamp: U256::from(timestamp), ..Default::default() },
        );
        let mut evm = EthEvmFactory::default().create_evm(db, env);
        apply_precompiles(evm.precompiles_mut(), 5_042_002);
        evm.transact(TxEnv {
            caller,
            chain_id: Some(5_042_002),
            kind: TxKind::Call(NATIVE_COIN_AUTHORITY),
            data: transferCall { from: SENDER, to: RECIPIENT, amount: U256::from(amount) }
                .abi_encode()
                .into(),
            gas_limit,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn registration_is_chain_scoped() {
        for chain_id in [1, 31_337, 5_042, 5_042_002] {
            let mut precompiles =
                PrecompilesMap::from_static(revm::precompile::Precompiles::osaka());
            apply_precompiles(&mut precompiles, chain_id);
            assert_eq!(
                precompiles.get(&NATIVE_COIN_AUTHORITY).is_some(),
                matches!(chain_id, 5_042 | 5_042_002)
            );
            assert_eq!(
                precompiles.get(&NATIVE_COIN_CONTROL).is_some(),
                matches!(chain_id, 5_042 | 5_042_002)
            );
            assert!(
                !precompiles
                    .addresses()
                    .any(|address| matches!(*address, NATIVE_COIN_AUTHORITY | NATIVE_COIN_CONTROL))
            );
        }
    }

    #[test]
    fn transfer_updates_native_balances_and_emits_log() {
        let output = execute_transfer(NATIVE_USDC, 30, 100_000);
        assert!(output.result.is_success());
        assert_eq!(output.state[&SENDER].info.balance, U256::from(70));
        assert_eq!(output.state[&RECIPIENT].info.balance, U256::from(30));
        assert_eq!(
            output.result.logs(),
            &[Log {
                address: SYSTEM_ADDRESS,
                data: Transfer { from: SENDER, to: RECIPIENT, value: U256::from(30) }
                    .encode_log_data(),
            }]
        );
    }

    #[test]
    fn unauthorized_transfer_reverts_without_moving_funds() {
        let output = execute_transfer(SENDER, 30, 100_000);
        assert!(matches!(&output.result, ExecutionResult::Revert { .. }));
        assert_eq!(
            output.result.output(),
            Some(&revert_data("Not enabled for native coin transfers"))
        );
        assert_eq!(output.state[&SENDER].info.balance, U256::from(100));
        assert!(output.result.logs().is_empty());
    }

    #[test]
    fn out_of_gas_transfer_does_not_move_funds() {
        let output = execute_transfer(NATIVE_USDC, 30, 23_000);
        assert!(output.result.is_halt());
        assert_eq!(
            output.state.get(&SENDER).map_or(U256::from(100), |account| account.info.balance),
            U256::from(100)
        );
        assert!(output.result.logs().is_empty());
    }

    #[test]
    fn zero8_allows_draining_an_empty_account() {
        let before = execute_transfer_at(NATIVE_USDC, 100, 100_000, 1_788_447_599);
        assert!(matches!(&before.result, ExecutionResult::Revert { .. }));
        assert_eq!(before.state[&SENDER].info.balance, U256::from(100));
        let after = execute_transfer_at(NATIVE_USDC, 100, 100_000, 1_788_447_600);
        assert!(after.result.is_success());
        assert_eq!(after.state[&SENDER].info.balance, U256::ZERO);
        assert_eq!(after.state[&RECIPIENT].info.balance, U256::from(100));
    }

    #[test]
    fn registration_preserves_previous_lookup() {
        let mut precompiles = PrecompilesMap::from_static(revm::precompile::Precompiles::osaka());
        precompiles.set_precompile_lookup(|address: &Address| {
            (*address == SENDER).then(|| {
                DynPrecompile::new(PrecompileId::Custom("existing".into()), |input| {
                    Ok(PrecompileOutput::new(0, Bytes::new(), input.reservoir))
                })
            })
        });
        apply_precompiles(&mut precompiles, 5_042_002);
        assert!(precompiles.get(&SENDER).is_some());
        assert!(precompiles.get(&NATIVE_COIN_AUTHORITY).is_some());
    }
}
