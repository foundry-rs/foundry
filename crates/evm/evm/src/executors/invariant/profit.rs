//! Built-in attacker-profit oracle for invariant campaigns.
//!
//! The oracle needs no user invariant. It tracks the actors of a sequence and their assets. An
//! actor profits when it ends with more of some asset than its baseline and with no less of any
//! other tracked asset. Without a price oracle, each asset counts on its own.
//!
//! In the default `victim` mode, a profit fails the sequence only if other tracked parties paid
//! for it over the same sequence. They can be other actors whose value went down, so some asset is
//! lower than its baseline and no asset is higher, and who together lost at least the gain in the
//! gained asset. For this check, ERC-4626 vault shares count as
//! the assets they redeem for, and value that the actor sent away in its own calls is no loss,
//! except deposits into an ERC-4626 vault of that token. It can also be one contract that the
//! sequence calls, if its holdings of the gained asset dropped by at least the gain. So yield
//! minted from nothing, an index raise, or a reward emission does not fail the sequence. In the
//! `any` mode, every profit fails the sequence.
//!
//! Actors are the senders of the sequence calls that have no code, plus
//! `invariant.profit_actors`. Assets are native ETH, the tokens that emit ERC-20 `Transfer` events
//! during the sequence, and `invariant.profit_tokens`.
//!
//! The baseline of an asset is the actor balance at sequence start plus what the actor received
//! from outside. An actor receives from outside when it gets value in a call that it neither
//! sends nor receives, or when another actor transfers tokens straight to it. Configured actors
//! act in every call, because a handler can act for them. A direct call to a token that mints
//! without payment is a faucet: the caller receives the minted value from outside if the mint
//! goes to the caller or to a contract. Balance changes that emit no event, for example `deal`,
//! count as the actor's own gains.

use super::{InvariantFuzzError, error::FailedInvariantCaseData};
use crate::executors::Executor;
use alloy_primitives::{
    Address, Log, U256,
    map::{AddressMap, AddressSet, HashMap},
};
use alloy_sol_types::{SolEvent, sol};
use foundry_config::{InvariantConfig, ProfitOracleConfig, ProfitOracleMode};
use foundry_evm_core::{constants::CALLER, evm::FoundryEvmNetwork, utils::StateChangeset};
use foundry_evm_fuzz::{BasicTxDetails, invariant::InvariantContract};
use proptest::test_runner::TestError;
use std::fmt;

/// Name of the attacker-profit oracle in invariant results.
pub const ATTACKER_PROFIT: &str = "attacker_profit";

/// Asset key for native ETH. `invariant.profit_tolerance` uses the same key.
const NATIVE: Address = Address::ZERO;

sol! {
    interface IProfitToken {
        event Transfer(address indexed from, address indexed to, uint256 value);

        function balanceOf(address account) external view returns (uint256);

        function asset() external view returns (address);

        function convertToAssets(uint256 shares) external view returns (uint256);
    }
}

/// Balance change of one actor asset over a sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetDelta {
    /// Token address, or the zero address for native ETH.
    pub asset: Address,
    /// Balance at sequence start.
    pub start: U256,
    /// Balance at the check.
    pub end: U256,
    /// Value received from outside during the sequence.
    pub outside: U256,
}

/// Change of an asset against its baseline, after the tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Gain(U256),
    Loss(U256),
    Flat,
}

impl AssetDelta {
    /// Returns the change against the baseline `start + outside`. A change of at most
    /// `tolerance` is flat.
    fn change(&self, tolerance: U256) -> Change {
        let baseline = self.start.saturating_add(self.outside);
        if self.end > baseline {
            let gain = self.end - baseline;
            if gain > tolerance { Change::Gain(gain) } else { Change::Flat }
        } else {
            let loss = baseline - self.end;
            if loss > tolerance { Change::Loss(loss) } else { Change::Flat }
        }
    }
}

/// An actor that ended a sequence with a profit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfitViolation {
    /// The actor that profited.
    pub actor: Address,
    /// Changes of the assets that moved, in tracking order. Native ETH is first.
    pub deltas: Vec<AssetDelta>,
    /// The party that lost over the same sequence. `None` in the `any` mode.
    pub victim: Option<ProfitVictim>,
}

/// A tracked party that lost over the sequence in which an actor profited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfitVictim {
    /// The actor that lost most of the gained asset, when the other actors whose value went down
    /// together lost at least the gain. Vault shares count as the assets they redeem for.
    Actor {
        /// The actor that lost.
        address: Address,
        /// Changes of the assets that moved, after vault shares were counted as assets.
        deltas: Vec<AssetDelta>,
    },
    /// A contract that the sequence calls and whose holdings of the gained asset dropped by at
    /// least the gain.
    Contract {
        /// The contract that lost.
        address: Address,
        /// The gained asset.
        asset: Address,
        /// Holdings at sequence start.
        start: U256,
        /// Holdings at the check.
        end: U256,
    },
}

impl fmt::Display for ProfitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ended with more value than it put in:", self.actor)?;
        write_deltas(f, &self.deltas)?;
        match &self.victim {
            Some(ProfitVictim::Actor { address, deltas }) => {
                write!(f, "; victim {address} lost value:")?;
                write_deltas(f, deltas)
            }
            Some(ProfitVictim::Contract { address, asset, start, end }) => {
                write!(f, "; victim {address} holds {} less", start.saturating_sub(*end))?;
                write_asset(f, *asset)?;
                write!(f, " (start {start}, end {end})")
            }
            None => Ok(()),
        }
    }
}

/// Per-sequence state of the attacker-profit oracle.
#[derive(Debug)]
pub(crate) struct ProfitLedger {
    /// Rule that decides when a profit fails the sequence.
    mode: ProfitOracleMode,
    /// Actors from `invariant.profit_actors`. They are tracked even if they have code.
    configured_actors: AddressSet,
    /// Tracked actors, in the order they were found.
    actors: Vec<Address>,
    actor_set: AddressSet,
    /// Contracts that the sequence calls, in the order they were found. They can be victims.
    targets: Vec<Address>,
    /// Actors that profited at their last check, but with no victim.
    pending: AddressSet,
    /// Tracked tokens, in the order they were found.
    tokens: Vec<Address>,
    /// Underlying asset of each address that is an ERC-4626 vault, or `None` for other addresses.
    vaults: AddressMap<Option<Address>>,
    /// Per-asset tolerance.
    tolerance: AddressMap<U256>,
    /// Cached start balances per `(actor, asset)`. `None` if the balance query failed.
    start: HashMap<(Address, Address), Option<U256>>,
    /// Value received from outside per `(address, asset)`, kept for all addresses because an
    /// address can become an actor later in the sequence.
    outside: HashMap<(Address, Address), U256>,
    /// Value that each address sent away in its own calls, per `(address, asset)`, except deposits
    /// into ERC-4626 vaults of that asset. The sender chose to move it, so it does not make the
    /// sender a victim. Only the `victim` mode records it.
    given: HashMap<(Address, Address), U256>,
    /// Last observed ETH balance of each address that a call touched.
    eth: AddressMap<U256>,
    /// Addresses whose balances can have changed since the last check.
    dirty: AddressSet,
    /// Whether a token was found since the last check, so every actor needs a new check.
    new_token: bool,
}

impl ProfitLedger {
    /// Creates the ledger for one sequence.
    pub(crate) fn new(config: &ProfitOracleConfig) -> Self {
        let mut ledger = Self {
            mode: config.profit_oracle_mode,
            configured_actors: config.profit_actors.iter().copied().collect(),
            actors: Vec::new(),
            actor_set: AddressSet::default(),
            targets: Vec::new(),
            pending: AddressSet::default(),
            tokens: Vec::new(),
            vaults: AddressMap::default(),
            tolerance: config
                .profit_tolerance
                .iter()
                .map(|(asset, amount)| (*asset, *amount))
                .collect(),
            start: HashMap::default(),
            outside: HashMap::default(),
            given: HashMap::default(),
            eth: AddressMap::default(),
            dirty: AddressSet::default(),
            new_token: false,
        };
        for &actor in &config.profit_actors {
            ledger.add_actor(actor);
        }
        for &token in &config.profit_tokens {
            ledger.add_token(token);
        }
        ledger
    }

    /// Records the balance movements of one accepted call.
    ///
    /// `start` is the executor at sequence start. Call this after the call was committed.
    pub(crate) fn observe<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        tx: &BasicTxDetails,
        reverted: bool,
        logs: &[Log],
        state_changeset: &StateChangeset,
    ) {
        // A reverted call moves no value.
        if reverted {
            return;
        }
        let sender = tx.sender;
        let target = tx.call_details.target;
        self.add_actor(sender);
        if !self.targets.contains(&target) {
            self.targets.push(target);
        }

        for (&address, account) in state_changeset {
            let balance = account.info.balance;
            let previous = self
                .eth
                .insert(address, balance)
                .unwrap_or_else(|| start.get_balance(address).unwrap_or_default());
            if balance != previous {
                self.dirty.insert(address);
            }
            if balance > previous && !self.acts(tx, address) {
                let outside = self.outside.entry((address, NATIVE)).or_default();
                *outside = outside.saturating_add(balance - previous);
            }
            // ETH that the sender pays in its own call is a gift or a deposit with no price.
            if balance < previous && address == sender && self.mode == ProfitOracleMode::Victim {
                let given = self.given.entry((sender, NATIVE)).or_default();
                *given = given.saturating_add(previous - balance);
            }
        }

        let transfers = logs
            .iter()
            .filter_map(|log| {
                IProfitToken::Transfer::decode_log(log).ok().map(|transfer| (log.address, transfer))
            })
            .collect::<Vec<_>>();
        let sender_paid = transfers.iter().any(|(_, transfer)| transfer.from == sender);
        for (token, transfer) in transfers {
            // A direct call to a token that mints without payment is a faucet.
            let faucet = token == target && transfer.from.is_zero() && !sender_paid;
            let from_actor = transfer.from != transfer.to
                && transfer.to != target
                && self.actor_set.contains(&transfer.from);
            let funded = match (self.acts(tx, transfer.to), faucet) {
                // A transfer straight from another actor moves value between actors.
                _ if from_actor => Some(transfer.to),
                // The caller adds a free mint to itself, or to a contract such as a vault.
                (true, true) => Some(sender),
                (false, true) if !start.is_empty_code(transfer.to).unwrap_or(true) => Some(sender),
                // Value that arrives in a call the receiver neither sends nor receives.
                (false, _) => Some(transfer.to),
                (true, false) => None,
            };
            self.add_token(token);
            self.dirty.insert(transfer.from);
            self.dirty.insert(transfer.to);
            if let Some(funded) = funded {
                let outside = self.outside.entry((funded, token)).or_default();
                *outside = outside.saturating_add(transfer.value);
            }
            // Value that the sender moves away from itself is a gift, or a deposit whose claim
            // has no price: its own tokens or a free mint with a direct token call, or its own
            // tokens into a contract that is not an ERC-4626 vault of that token.
            let gift = if self.mode == ProfitOracleMode::Any || transfer.to == sender {
                false
            } else if token == target {
                transfer.from == sender || funded == Some(sender)
            } else {
                transfer.from == sender && self.vault_asset(start, transfer.to) != Some(token)
            };
            if gift {
                let given = self.given.entry((sender, token)).or_default();
                *given = given.saturating_add(transfer.value);
            }
        }
    }

    /// Returns the first actor that profited, if any. In the `victim` mode, the profit counts only
    /// with a victim.
    ///
    /// `start` is the executor at sequence start and `end` is the executor at the check. Only
    /// actors with balance movements since the last check are checked again. An actor that
    /// profited with no victim is checked again after any balance movement, because a victim can
    /// lose later.
    pub(crate) fn check<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
    ) -> Option<ProfitViolation> {
        let check_all = std::mem::take(&mut self.new_token);
        let dirty = std::mem::take(&mut self.dirty);
        let assets = std::iter::once(NATIVE).chain(self.tokens.iter().copied()).collect::<Vec<_>>();
        for actor in self.actors.clone() {
            let recheck = !dirty.is_empty() && self.pending.contains(&actor);
            if !check_all && !recheck && !dirty.contains(&actor) {
                continue;
            }
            if !self.is_tracked(end, actor) {
                continue;
            }
            let deltas = self.deltas(start, end, actor, &assets);
            if !is_profit(&deltas, &self.tolerance) {
                self.pending.remove(&actor);
                continue;
            }
            let victim = match self.mode {
                ProfitOracleMode::Any => None,
                ProfitOracleMode::Victim => {
                    let Some(victim) = self.victim(start, end, actor, &deltas, &assets) else {
                        self.pending.insert(actor);
                        continue;
                    };
                    Some(victim)
                }
            };
            return Some(ProfitViolation { actor, deltas: moved(deltas), victim });
        }
        None
    }

    /// Returns a party that paid for the gains in `deltas` of `actor`: the actor that lost most
    /// of a gained asset if the other actors together lost at least the gain, or a contract that
    /// the sequence calls and that holds at least the gain less.
    fn victim<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
        actor: Address,
        deltas: &[AssetDelta],
        assets: &[Address],
    ) -> Option<ProfitVictim> {
        // Other actors whose value went down. Vault shares count as the assets they redeem for,
        // and value that an actor gave away itself is added back.
        let mut losers = Vec::new();
        for address in self.actors.clone() {
            if address == actor || !self.is_tracked(end, address) {
                continue;
            }
            let actor_deltas = self.deltas(start, end, address, assets);
            let actor_deltas = self.redeem_value(start, end, address, actor_deltas);
            let kept = actor_deltas
                .iter()
                .map(|delta| AssetDelta {
                    end: delta.end.saturating_add(
                        self.given.get(&(address, delta.asset)).copied().unwrap_or_default(),
                    ),
                    ..*delta
                })
                .collect::<Vec<_>>();
            if is_loss(&kept, &self.tolerance) {
                losers.push((address, actor_deltas, kept));
            }
        }

        for delta in deltas {
            let tolerance = self.tolerance.get(&delta.asset).copied().unwrap_or_default();
            let Change::Gain(gain) = delta.change(tolerance) else { continue };

            // The other actors together lost at least the gain. A gain of vault shares counts as
            // the assets that it redeems for.
            let (asset, value) = self
                .vault_asset(start, delta.asset)
                .zip(convert_to_assets(end, delta.asset, gain))
                .unwrap_or((delta.asset, gain));
            let lost = |kept: &[AssetDelta]| {
                kept.iter().find(|kept| kept.asset == asset).map_or(U256::ZERO, |kept| {
                    kept.start.saturating_add(kept.outside).saturating_sub(kept.end)
                })
            };
            let total = losers
                .iter()
                .fold(U256::ZERO, |total, (_, _, kept)| total.saturating_add(lost(kept)));
            if !value.is_zero()
                && total >= value
                && let Some((address, actor_deltas, _)) =
                    losers.iter().max_by_key(|(_, _, kept)| lost(kept))
            {
                return Some(ProfitVictim::Actor {
                    address: *address,
                    deltas: moved(actor_deltas.clone()),
                });
            }

            // A contract that the sequence calls and that holds at least the gain less.
            for &address in &self.targets {
                if address != actor
                    && let Some(held_start) = balance_of(start, address, delta.asset)
                    && let Some(held_end) = balance_of(end, address, delta.asset)
                    && held_start.saturating_sub(held_end) >= gain
                {
                    return Some(ProfitVictim::Contract {
                        address,
                        asset: delta.asset,
                        start: held_start,
                        end: held_end,
                    });
                }
            }
        }
        None
    }

    /// Returns the changes of `assets` held by `address`. Assets whose balance query fails are
    /// left out.
    fn deltas<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
        address: Address,
        assets: &[Address],
    ) -> Vec<AssetDelta> {
        assets.iter().filter_map(|&asset| self.delta(start, end, address, asset)).collect()
    }

    /// Returns the change of `asset` held by `address`, or `None` if a balance query fails.
    fn delta<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
        address: Address,
        asset: Address,
    ) -> Option<AssetDelta> {
        let start_balance = *self
            .start
            .entry((address, asset))
            .or_insert_with(|| balance_of(start, address, asset));
        Some(AssetDelta {
            asset,
            start: start_balance?,
            end: balance_of(end, address, asset)?,
            outside: self.outside.get(&(address, asset)).copied().unwrap_or_default(),
        })
    }

    /// Counts the ERC-4626 vault shares in `deltas` as the underlying assets that they redeem
    /// for, at the start and at the check. Shares that cannot be converted stay as they are.
    fn redeem_value<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
        address: Address,
        deltas: Vec<AssetDelta>,
    ) -> Vec<AssetDelta> {
        let mut valued = Vec::with_capacity(deltas.len());
        let mut shares = Vec::new();
        for delta in deltas {
            match self.vault_asset(start, delta.asset) {
                Some(underlying) => shares.push((delta, underlying)),
                None => valued.push(delta),
            }
        }
        for (share, underlying) in shares {
            let (Some(start_value), Some(end_value), Some(outside_value)) = (
                convert_to_assets(start, share.asset, share.start),
                convert_to_assets(end, share.asset, share.end),
                convert_to_assets(end, share.asset, share.outside),
            ) else {
                valued.push(share);
                continue;
            };
            let index = match valued.iter().position(|delta| delta.asset == underlying) {
                Some(index) => index,
                None => {
                    let Some(delta) = self.delta(start, end, address, underlying) else {
                        valued.push(share);
                        continue;
                    };
                    valued.push(delta);
                    valued.len() - 1
                }
            };
            let delta = &mut valued[index];
            delta.start = delta.start.saturating_add(start_value);
            delta.end = delta.end.saturating_add(end_value);
            delta.outside = delta.outside.saturating_add(outside_value);
        }
        valued
    }

    /// Returns the underlying asset of `address` if it is an ERC-4626 vault at sequence start.
    fn vault_asset<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        address: Address,
    ) -> Option<Address> {
        if address == NATIVE {
            return None;
        }
        *self.vaults.entry(address).or_insert_with(|| {
            start
                .call_sol(CALLER, address, &IProfitToken::assetCall {}, U256::ZERO, None)
                .ok()
                .map(|result| result.decoded_result)
                .filter(|asset| !asset.is_zero() && *asset != address)
        })
    }

    /// Returns whether `address` can profit or lose: a configured actor, or an address without
    /// code. Fuzzer senders with code are protocol or test contracts.
    fn is_tracked<FEN: FoundryEvmNetwork>(&self, end: &Executor<FEN>, address: Address) -> bool {
        self.configured_actors.contains(&address) || end.is_empty_code(address).unwrap_or(false)
    }

    /// Returns whether `address` acts in `tx`: it sends or receives the call, or it is a
    /// configured actor, which a handler can act for in any call.
    fn acts(&self, tx: &BasicTxDetails, address: Address) -> bool {
        address == tx.sender
            || address == tx.call_details.target
            || self.configured_actors.contains(&address)
    }

    fn add_actor(&mut self, actor: Address) {
        if self.actor_set.insert(actor) {
            self.actors.push(actor);
            self.dirty.insert(actor);
        }
    }

    fn add_token(&mut self, token: Address) {
        if !self.tokens.contains(&token) {
            self.tokens.push(token);
            self.new_token = true;
        }
    }
}

/// Returns whether the attacker-profit oracle runs for this campaign.
///
/// Optimization campaigns never fail, so the oracle does not run for them.
pub fn profit_oracle_enabled(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
) -> bool {
    config.profit.profit_oracle && !invariant_contract.is_optimization()
}

/// Builds the invariant failure that reports `violation` for the `calls` sequence.
pub(crate) fn profit_failure(
    invariant_contract: &InvariantContract<'_>,
    config: &InvariantConfig,
    calls: &[BasicTxDetails],
    violation: &ProfitViolation,
) -> InvariantFuzzError {
    let revert_reason = violation.to_string();
    InvariantFuzzError::BrokenInvariant(FailedInvariantCaseData {
        test_error: TestError::Fail(
            format!("{ATTACKER_PROFIT}, reason: {revert_reason}").into(),
            calls.to_vec(),
        ),
        return_reason: "".into(),
        revert_reason,
        addr: invariant_contract.address,
        calldata: Default::default(),
        inner_sequence: Vec::new(),
        shrink_run_limit: config.shrink_run_limit,
        fail_on_revert: false,
        assertion_failure: false,
        fork_block_number: None,
    })
}

/// Returns whether the deltas show a profit: one asset gained and no asset lost.
fn is_profit(deltas: &[AssetDelta], tolerance: &AddressMap<U256>) -> bool {
    let mut gained = false;
    for delta in deltas {
        match delta.change(tolerance.get(&delta.asset).copied().unwrap_or_default()) {
            Change::Gain(_) => gained = true,
            Change::Loss(_) => return false,
            Change::Flat => {}
        }
    }
    gained
}

/// Returns whether the deltas show a loss: one asset lost and no asset gained.
fn is_loss(deltas: &[AssetDelta], tolerance: &AddressMap<U256>) -> bool {
    let mut lost = false;
    for delta in deltas {
        match delta.change(tolerance.get(&delta.asset).copied().unwrap_or_default()) {
            Change::Loss(_) => lost = true,
            Change::Gain(_) => return false,
            Change::Flat => {}
        }
    }
    lost
}

/// Keeps the deltas of the assets that moved or that received value from outside.
fn moved(deltas: Vec<AssetDelta>) -> Vec<AssetDelta> {
    deltas
        .into_iter()
        .filter(|delta| delta.start != delta.end || !delta.outside.is_zero())
        .collect()
}

/// Returns the assets that `shares` of the ERC-4626 `vault` redeem for, or `None` if the query
/// fails.
fn convert_to_assets<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    vault: Address,
    shares: U256,
) -> Option<U256> {
    if shares.is_zero() {
        return Some(U256::ZERO);
    }
    executor
        .call_sol(CALLER, vault, &IProfitToken::convertToAssetsCall { shares }, U256::ZERO, None)
        .ok()
        .map(|result| result.decoded_result)
}

fn write_deltas(f: &mut fmt::Formatter<'_>, deltas: &[AssetDelta]) -> fmt::Result {
    for (i, delta) in deltas.iter().enumerate() {
        let baseline = delta.start.saturating_add(delta.outside);
        let sign = if delta.end >= baseline { '+' } else { '-' };
        let amount = delta.end.abs_diff(baseline);
        if i > 0 {
            f.write_str(";")?;
        }
        write_asset(f, delta.asset)?;
        write!(
            f,
            " {sign}{amount} (start {}, end {}, outside {})",
            delta.start, delta.end, delta.outside
        )?;
    }
    Ok(())
}

fn write_asset(f: &mut fmt::Formatter<'_>, asset: Address) -> fmt::Result {
    if asset == NATIVE { f.write_str(" ETH") } else { write!(f, " {asset}") }
}

/// Returns the balance of `asset` held by `actor`, or `None` if the query fails.
fn balance_of<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    actor: Address,
    asset: Address,
) -> Option<U256> {
    if asset == NATIVE {
        return executor.get_balance(actor).ok();
    }
    executor
        .call_sol(CALLER, asset, &IProfitToken::balanceOfCall { account: actor }, U256::ZERO, None)
        .ok()
        .map(|result| result.decoded_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::ExecutorBuilder;
    use alloy_evm::EvmEnv;
    use alloy_primitives::Bytes;
    use foundry_evm_core::{backend::Backend, evm::EthEvmNetwork};
    use foundry_evm_fuzz::CallDetails;
    use revm::{
        bytecode::Bytecode,
        context::TxEnv,
        state::{Account, AccountInfo},
    };

    const TOKEN: Address = Address::repeat_byte(0x70);
    const SHARE: Address = Address::repeat_byte(0x51);
    const ALICE: Address = Address::repeat_byte(0xa1);
    const BOB: Address = Address::repeat_byte(0xb0);

    fn executor() -> Executor<EthEvmNetwork> {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        ExecutorBuilder::default().gas_limit(1 << 24).build(
            EvmEnv::default(),
            TxEnv::default(),
            backend,
            Default::default(),
        )
    }

    fn tx(sender: Address, target: Address) -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender,
            call_details: CallDetails { target, calldata: Default::default(), value: None },
        }
    }

    fn transfer(token: Address, from: Address, to: Address, value: u64) -> Log {
        let event = IProfitToken::Transfer { from, to, value: U256::from(value) };
        Log { address: token, data: event.encode_log_data() }
    }

    fn outside(ledger: &ProfitLedger, address: Address, asset: Address) -> U256 {
        ledger.outside.get(&(address, asset)).copied().unwrap_or_default()
    }

    #[test]
    fn observe_counts_value_from_outside() {
        let vault = SHARE;
        let mut start = executor();
        start.set_code(vault, Bytecode::new_raw(Bytes::from_static(&[0x00]))).unwrap();
        let mut ledger = ProfitLedger::new(&ProfitOracleConfig::default());

        // Alice deposits: she pays assets and the vault mints shares to her.
        ledger.observe(
            &start,
            &tx(ALICE, vault),
            false,
            &[transfer(TOKEN, ALICE, vault, 100), transfer(SHARE, Address::ZERO, ALICE, 100)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, ALICE, SHARE), U256::ZERO);

        // A payout to the caller is the caller's own gain.
        ledger.observe(
            &start,
            &tx(ALICE, vault),
            false,
            &[transfer(TOKEN, vault, ALICE, 150)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, ALICE, TOKEN), U256::ZERO);

        // A direct transfer from Alice is outside value for Bob.
        ledger.observe(
            &start,
            &tx(ALICE, TOKEN),
            false,
            &[transfer(TOKEN, ALICE, BOB, 7)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, BOB, TOKEN), U256::from(7));

        // A free mint from a direct token call is a faucet.
        ledger.observe(
            &start,
            &tx(BOB, TOKEN),
            false,
            &[transfer(TOKEN, Address::ZERO, BOB, 5)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, BOB, TOKEN), U256::from(12));

        // A free mint to a contract is a donation that the caller received from outside.
        ledger.observe(
            &start,
            &tx(BOB, TOKEN),
            false,
            &[transfer(TOKEN, Address::ZERO, vault, 9)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, BOB, TOKEN), U256::from(21));
        assert_eq!(outside(&ledger, vault, TOKEN), U256::ZERO);

        // A reverted call moves nothing and adds no actor.
        let carol = Address::repeat_byte(0xc0);
        ledger.observe(
            &start,
            &tx(carol, TOKEN),
            true,
            &[transfer(TOKEN, Address::ZERO, carol, 5)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, carol, TOKEN), U256::ZERO);
        assert_eq!(ledger.actors, [ALICE, BOB]);
        assert_eq!(ledger.tokens, [TOKEN, SHARE]);
    }

    #[test]
    fn observe_lets_configured_actors_act_in_every_call() {
        let start = executor();
        let config = ProfitOracleConfig { profit_actors: vec![ALICE, BOB], ..Default::default() };
        let mut ledger = ProfitLedger::new(&config);
        let (sender, handler) = (Address::repeat_byte(0x5e), Address::repeat_byte(0x4a));

        // The handler acts for Alice: a vault payout to her is her own gain.
        ledger.observe(
            &start,
            &tx(sender, handler),
            false,
            &[transfer(TOKEN, SHARE, ALICE, 10)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, ALICE, TOKEN), U256::ZERO);

        // A transfer straight from Bob to Alice moves value between actors.
        ledger.observe(
            &start,
            &tx(sender, handler),
            false,
            &[transfer(TOKEN, BOB, ALICE, 4)],
            &StateChangeset::default(),
        );
        assert_eq!(outside(&ledger, ALICE, TOKEN), U256::from(4));
        assert_eq!(ledger.actors, [ALICE, BOB, sender]);
    }

    #[test]
    fn observe_counts_eth_from_outside() {
        let start = executor();
        let mut ledger = ProfitLedger::new(&ProfitOracleConfig::default());
        let account = |balance: u64| {
            Account::from(AccountInfo { balance: U256::from(balance), ..Default::default() })
        };

        // Bob receives ETH in Alice's call, then Alice receives ETH in her own call.
        let changeset = StateChangeset::from_iter([(BOB, account(30)), (ALICE, account(0))]);
        ledger.observe(&start, &tx(ALICE, SHARE), false, &[], &changeset);
        let changeset = StateChangeset::from_iter([(ALICE, account(40))]);
        ledger.observe(&start, &tx(ALICE, SHARE), false, &[], &changeset);

        assert_eq!(outside(&ledger, BOB, NATIVE), U256::from(30));
        assert_eq!(outside(&ledger, ALICE, NATIVE), U256::ZERO);

        // Alice pays ETH in her own call, and Bob loses ETH in her call. Only Alice gave it.
        let changeset = StateChangeset::from_iter([(ALICE, account(15)), (BOB, account(20))]);
        ledger.observe(&start, &tx(ALICE, SHARE), false, &[], &changeset);
        assert_eq!(ledger.given.get(&(ALICE, NATIVE)), Some(&U256::from(25)));
        assert_eq!(ledger.given.get(&(BOB, NATIVE)), None);
    }

    fn delta(asset: Address, start: u64, end: u64, outside: u64) -> AssetDelta {
        AssetDelta {
            asset,
            start: U256::from(start),
            end: U256::from(end),
            outside: U256::from(outside),
        }
    }

    #[test]
    fn gain_without_loss_is_profit() {
        let deltas = [delta(NATIVE, 0, 0, 0), delta(TOKEN, 100, 150, 0)];
        assert!(is_profit(&deltas, &AddressMap::default()));
    }

    #[test]
    fn gain_with_loss_is_not_profit() {
        // Deposit: assets go down, shares go up.
        let deltas = [delta(TOKEN, 100, 0, 0), delta(SHARE, 0, 100, 0)];
        assert!(!is_profit(&deltas, &AddressMap::default()));
    }

    #[test]
    fn no_change_is_not_profit() {
        let deltas = [delta(NATIVE, 5, 5, 0), delta(TOKEN, 100, 100, 0)];
        assert!(!is_profit(&deltas, &AddressMap::default()));
        assert!(!is_profit(&[], &AddressMap::default()));
    }

    #[test]
    fn outside_value_raises_the_baseline() {
        // The actor received 50 from outside, so ending 50 higher is no profit.
        assert!(!is_profit(&[delta(TOKEN, 100, 150, 50)], &AddressMap::default()));
        // Spending part of the outside value is a loss, not a gain.
        assert_eq!(delta(TOKEN, 100, 120, 50).change(U256::ZERO), Change::Loss(U256::from(30)));
        // A gain on top of the outside value is still profit.
        assert!(is_profit(&[delta(TOKEN, 100, 151, 50)], &AddressMap::default()));
    }

    #[test]
    fn tolerance_flattens_small_changes() {
        let tolerance = AddressMap::from_iter([(TOKEN, U256::from(10)), (SHARE, U256::from(1))]);
        // A fee refund or reward of at most the tolerance is no profit.
        assert!(!is_profit(&[delta(TOKEN, 100, 110, 0)], &tolerance));
        assert!(is_profit(&[delta(TOKEN, 100, 111, 0)], &tolerance));
        // A loss within its tolerance does not hide a gain in another asset.
        assert!(is_profit(&[delta(TOKEN, 100, 200, 0), delta(SHARE, 10, 9, 0)], &tolerance));
        assert!(!is_profit(&[delta(TOKEN, 100, 200, 0), delta(SHARE, 10, 8, 0)], &tolerance));
    }

    #[test]
    fn baseline_saturates() {
        let max = AssetDelta { asset: TOKEN, start: U256::MAX, end: U256::MAX, outside: U256::MAX };
        assert_eq!(max.change(U256::ZERO), Change::Flat);
    }

    #[test]
    fn violation_lists_signed_deltas() {
        let mut violation = ProfitViolation {
            actor: Address::repeat_byte(0x11),
            deltas: vec![delta(NATIVE, 10, 10, 2), delta(TOKEN, 100, 180, 30)],
            victim: None,
        };
        assert_eq!(
            violation.to_string(),
            "0x1111111111111111111111111111111111111111 ended with more value than it put in: \
             ETH -2 (start 10, end 10, outside 2); 0x7070707070707070707070707070707070707070 +50 \
             (start 100, end 180, outside 30)"
        );

        violation.deltas.truncate(1);
        violation.victim =
            Some(ProfitVictim::Actor { address: BOB, deltas: vec![delta(TOKEN, 100, 40, 0)] });
        assert_eq!(
            violation.to_string(),
            "0x1111111111111111111111111111111111111111 ended with more value than it put in: \
             ETH -2 (start 10, end 10, outside 2); victim \
             0xB0B0b0B0B0B0B0b0B0B0B0b0b0b0b0B0b0b0B0B0 lost value: \
             0x7070707070707070707070707070707070707070 -60 (start 100, end 40, outside 0)"
        );

        violation.victim = Some(ProfitVictim::Contract {
            address: SHARE,
            asset: NATIVE,
            start: U256::from(9),
            end: U256::from(2),
        });
        assert_eq!(
            violation.to_string(),
            "0x1111111111111111111111111111111111111111 ended with more value than it put in: \
             ETH -2 (start 10, end 10, outside 2); victim \
             0x5151515151515151515151515151515151515151 holds 7 less ETH (start 9, end 2)"
        );
    }

    #[test]
    fn loss_needs_a_lost_asset_and_no_gain() {
        let tolerance = AddressMap::from_iter([(TOKEN, U256::from(10))]);
        assert!(is_loss(&[delta(NATIVE, 5, 5, 0), delta(TOKEN, 100, 50, 0)], &tolerance));
        // A deposit trades one asset for another, so it is no loss.
        assert!(!is_loss(&[delta(TOKEN, 100, 50, 0), delta(SHARE, 0, 50, 0)], &tolerance));
        // A loss within the tolerance, or of value received from outside, is no loss.
        assert!(!is_loss(&[delta(TOKEN, 100, 90, 0)], &tolerance));
        assert!(!is_loss(&[delta(TOKEN, 100, 150, 50)], &tolerance));
        assert!(!is_loss(&[], &tolerance));
    }

    /// Runtime code of a token that is also an ERC-4626 vault when storage slot 0 holds an asset.
    /// `asset()` returns slot 0, `convertToAssets(x)` returns `x` times slot 1, and every other
    /// call returns the slot keyed by its first argument, so `balanceOf(a)` reads slot `a`.
    const TOKEN_CODE: &[u8] = &[
        0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c, // selector
        0x80, 0x63, 0x38, 0xd5, 0x2e, 0x0f, 0x14, 0x60, 0x20, 0x57, // asset()
        0x63, 0x07, 0xa2, 0xd1, 0x3a, 0x14, 0x60, 0x28, 0x57, // convertToAssets(uint256)
        0x60, 0x04, 0x35, 0x54, 0x60, 0x30, 0x56, // balanceOf(address)
        0x5b, 0x50, 0x60, 0x00, 0x54, 0x60, 0x30, 0x56, // asset: slot 0
        0x5b, 0x60, 0x01, 0x54, 0x60, 0x04, 0x35, 0x02, // convertToAssets: x * slot 1
        0x5b, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3, // return one word
    ];

    const DISTRIBUTOR: Address = Address::repeat_byte(0xd1);

    /// Returns executors at sequence start and at the check, with `TOKEN`, `SHARE`, and the
    /// distributor contract deployed. Each entry is `(token, holder, start value, end value)`.
    fn states(
        entries: &[(Address, Address, u64, u64)],
    ) -> (Executor<EthEvmNetwork>, Executor<EthEvmNetwork>) {
        let mut start = executor();
        for address in [TOKEN, SHARE, DISTRIBUTOR] {
            start.set_code(address, Bytecode::new_raw(Bytes::from_static(TOKEN_CODE))).unwrap();
        }
        let mut end = start.clone();
        for &(token, holder, start_value, end_value) in entries {
            let slot = U256::from_be_slice(holder.as_slice());
            start.set_storage_slot(token, slot, U256::from(start_value)).unwrap();
            end.set_storage_slot(token, slot, U256::from(end_value)).unwrap();
        }
        (start, end)
    }

    fn victim_ledger(mode: ProfitOracleMode) -> ProfitLedger {
        ProfitLedger::new(&ProfitOracleConfig { profit_oracle_mode: mode, ..Default::default() })
    }

    #[test]
    fn victim_mode_ignores_profit_without_a_loser() {
        // Alice claims a reward that the distributor mints for her.
        let (start, end) = states(&[(TOKEN, ALICE, 0, 100)]);
        let logs = [transfer(TOKEN, Address::ZERO, ALICE, 100)];
        for mode in [ProfitOracleMode::Victim, ProfitOracleMode::Any] {
            let mut ledger = victim_ledger(mode);
            ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
            let violation = ledger.check(&start, &end);
            assert_eq!(violation.is_some(), mode == ProfitOracleMode::Any);
            assert_eq!(violation.and_then(|violation| violation.victim), None);
        }
    }

    #[test]
    fn victim_mode_reports_an_actor_that_lost() {
        // Alice pulls Bob's tokens into the distributor, which pays her. Bob counts only once he
        // is an actor.
        let (start, end) = states(&[(TOKEN, ALICE, 0, 100), (TOKEN, BOB, 100, 0)]);
        let logs =
            [transfer(TOKEN, BOB, DISTRIBUTOR, 100), transfer(TOKEN, DISTRIBUTOR, ALICE, 100)];
        for bob_acts in [false, true] {
            let mut ledger = victim_ledger(ProfitOracleMode::Victim);
            if bob_acts {
                ledger.observe(&start, &tx(BOB, DISTRIBUTOR), false, &[], &Default::default());
            }
            ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
            let violation = ledger.check(&start, &end);
            assert_eq!(
                violation.map(|violation| (violation.actor, violation.victim)),
                bob_acts.then(|| (
                    ALICE,
                    Some(ProfitVictim::Actor {
                        address: BOB,
                        deltas: vec![delta(TOKEN, 100, 0, 0)]
                    })
                ))
            );
        }
    }

    #[test]
    fn victim_mode_needs_actor_losses_of_at_least_the_gain() {
        // Bob loses 2 tokens to rounding while Alice claims 100, then Carol loses the other 98.
        let carol = Address::repeat_byte(0xc0);
        let logs = [transfer(TOKEN, DISTRIBUTOR, ALICE, 100)];
        for (carol_end, caught) in [(100, false), (2, true)] {
            let (start, end) = states(&[
                (TOKEN, ALICE, 0, 100),
                (TOKEN, BOB, 100, 98),
                (TOKEN, carol, 100, carol_end),
            ]);
            let mut ledger = victim_ledger(ProfitOracleMode::Victim);
            for sender in [BOB, carol] {
                ledger.observe(&start, &tx(sender, DISTRIBUTOR), false, &[], &Default::default());
            }
            ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
            let victim = ledger.check(&start, &end).and_then(|violation| violation.victim);
            assert_eq!(
                victim,
                caught.then(|| ProfitVictim::Actor {
                    address: carol,
                    deltas: vec![delta(TOKEN, 100, 2, 0)],
                })
            );
        }
    }

    #[test]
    fn victim_mode_checks_a_pending_profit_again() {
        // Alice gains first with no loser, then the distributor pulls Bob's tokens in a later call.
        let (start, mut end) = states(&[(TOKEN, ALICE, 0, 100), (TOKEN, BOB, 100, 100)]);
        let mut ledger = victim_ledger(ProfitOracleMode::Victim);
        ledger.observe(&start, &tx(BOB, DISTRIBUTOR), false, &[], &Default::default());
        let logs = [transfer(TOKEN, Address::ZERO, ALICE, 100)];
        ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
        assert_eq!(ledger.check(&start, &end), None);

        end.set_storage_slot(TOKEN, U256::from_be_slice(BOB.as_slice()), U256::ZERO).unwrap();
        let logs = [transfer(TOKEN, BOB, DISTRIBUTOR, 100)];
        ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
        let violation = ledger.check(&start, &end).unwrap();
        assert_eq!((violation.actor, violation.victim.is_some()), (ALICE, true));
    }

    #[test]
    fn victim_mode_ignores_a_gift() {
        // Bob sends tokens away with a direct token call while Alice claims a minted reward.
        let (start, end) = states(&[(TOKEN, ALICE, 0, 100), (TOKEN, BOB, 100, 0)]);
        let mut ledger = victim_ledger(ProfitOracleMode::Victim);
        let logs = [transfer(TOKEN, BOB, DISTRIBUTOR, 100)];
        ledger.observe(&start, &tx(BOB, TOKEN), false, &logs, &Default::default());
        let logs = [transfer(TOKEN, Address::ZERO, ALICE, 100)];
        ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
        assert_eq!(ledger.check(&start, &end), None);
    }

    #[test]
    fn victim_mode_needs_a_contract_drop_of_at_least_the_gain() {
        let logs = [transfer(TOKEN, DISTRIBUTOR, ALICE, 100)];
        for (held_end, caught) in [(50, true), (100, false)] {
            let (start, end) =
                states(&[(TOKEN, ALICE, 0, 100), (TOKEN, DISTRIBUTOR, 150, held_end)]);
            let mut ledger = victim_ledger(ProfitOracleMode::Victim);
            ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
            let violation = ledger.check(&start, &end);
            assert_eq!(
                violation.and_then(|violation| violation.victim),
                caught.then_some(ProfitVictim::Contract {
                    address: DISTRIBUTOR,
                    asset: TOKEN,
                    start: U256::from(150),
                    end: U256::from(held_end),
                })
            );
        }
    }

    #[test]
    fn victim_mode_counts_vault_shares_as_redeem_value() {
        // Bob holds 10 shares of the `SHARE` vault over `TOKEN`. A share redeems for 10 tokens at
        // the start and for `rate_end` tokens at the check.
        let asset = U256::from_be_slice(TOKEN.as_slice());
        let config = ProfitOracleConfig { profit_tokens: vec![TOKEN, SHARE], ..Default::default() };
        let logs = [transfer(TOKEN, DISTRIBUTOR, ALICE, 50)];
        for (rate_end, caught) in [(5, true), (10, false)] {
            let (mut start, mut end) = states(&[(TOKEN, ALICE, 0, 50), (SHARE, BOB, 10, 10)]);
            for (executor, rate) in [(&mut start, 10), (&mut end, rate_end)] {
                executor.set_storage_slot(SHARE, U256::ZERO, asset).unwrap();
                executor.set_storage_slot(SHARE, U256::from(1), U256::from(rate)).unwrap();
            }
            let mut ledger = ProfitLedger::new(&config);
            ledger.observe(&start, &tx(BOB, SHARE), false, &[], &Default::default());
            ledger.observe(&start, &tx(ALICE, DISTRIBUTOR), false, &logs, &Default::default());
            let violation = ledger.check(&start, &end);
            assert_eq!(
                violation.and_then(|violation| violation.victim),
                caught.then(|| ProfitVictim::Actor {
                    address: BOB,
                    deltas: vec![delta(TOKEN, 100, 50, 0)],
                })
            );
        }
    }
}
