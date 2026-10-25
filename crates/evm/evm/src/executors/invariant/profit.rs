//! Built-in attacker-profit oracle for invariant campaigns.
//!
//! The oracle needs no user invariant. It tracks the actors of a sequence and their assets, and it
//! fails the sequence when an actor ends with more of some asset than its baseline and with no
//! less of any other tracked asset. Without a price oracle, each asset counts on its own.
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
use foundry_config::{InvariantConfig, ProfitOracleConfig};
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
}

impl fmt::Display for ProfitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ended with more value than it put in:", self.actor)?;
        for (i, delta) in self.deltas.iter().enumerate() {
            let baseline = delta.start.saturating_add(delta.outside);
            let sign = if delta.end >= baseline { '+' } else { '-' };
            let amount = delta.end.abs_diff(baseline);
            let separator = if i == 0 { "" } else { ";" };
            if delta.asset == NATIVE {
                write!(f, "{separator} ETH")?;
            } else {
                write!(f, "{separator} {}", delta.asset)?;
            }
            write!(
                f,
                " {sign}{amount} (start {}, end {}, outside {})",
                delta.start, delta.end, delta.outside
            )?;
        }
        Ok(())
    }
}

/// Per-sequence state of the attacker-profit oracle.
#[derive(Debug)]
pub(crate) struct ProfitLedger {
    /// Actors from `invariant.profit_actors`. They are tracked even if they have code.
    configured_actors: AddressSet,
    /// Tracked actors, in the order they were found.
    actors: Vec<Address>,
    actor_set: AddressSet,
    /// Tracked tokens, in the order they were found.
    tokens: Vec<Address>,
    /// Per-asset tolerance.
    tolerance: AddressMap<U256>,
    /// Cached start balances per `(actor, asset)`. `None` if the balance query failed.
    start: HashMap<(Address, Address), Option<U256>>,
    /// Value received from outside per `(address, asset)`, kept for all addresses because an
    /// address can become an actor later in the sequence.
    outside: HashMap<(Address, Address), U256>,
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
            configured_actors: config.profit_actors.iter().copied().collect(),
            actors: Vec::new(),
            actor_set: AddressSet::default(),
            tokens: Vec::new(),
            tolerance: config
                .profit_tolerance
                .iter()
                .map(|(asset, amount)| (*asset, *amount))
                .collect(),
            start: HashMap::default(),
            outside: HashMap::default(),
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
        }
    }

    /// Returns the first actor that profited, if any.
    ///
    /// `start` is the executor at sequence start and `end` is the executor at the check. Only
    /// actors with balance movements since the last check are checked again.
    pub(crate) fn check<FEN: FoundryEvmNetwork>(
        &mut self,
        start: &Executor<FEN>,
        end: &Executor<FEN>,
    ) -> Option<ProfitViolation> {
        let check_all = std::mem::take(&mut self.new_token);
        let dirty = std::mem::take(&mut self.dirty);
        let assets = std::iter::once(NATIVE).chain(self.tokens.iter().copied()).collect::<Vec<_>>();
        for actor in self.actors.clone() {
            if !check_all && !dirty.contains(&actor) {
                continue;
            }
            // Fuzzer senders with code are protocol or test contracts, not actors.
            if !self.configured_actors.contains(&actor)
                && !end.is_empty_code(actor).unwrap_or(false)
            {
                continue;
            }
            let deltas = assets
                .iter()
                .filter_map(|&asset| {
                    let start_balance = *self
                        .start
                        .entry((actor, asset))
                        .or_insert_with(|| balance_of(start, actor, asset));
                    Some(AssetDelta {
                        asset,
                        start: start_balance?,
                        end: balance_of(end, actor, asset)?,
                        outside: self.outside.get(&(actor, asset)).copied().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>();
            if is_profit(&deltas, &self.tolerance) {
                let deltas = deltas
                    .into_iter()
                    .filter(|delta| delta.start != delta.end || !delta.outside.is_zero())
                    .collect();
                return Some(ProfitViolation { actor, deltas });
            }
        }
        None
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
        ExecutorBuilder::default().build(
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
        let violation = ProfitViolation {
            actor: Address::repeat_byte(0x11),
            deltas: vec![delta(NATIVE, 10, 10, 2), delta(TOKEN, 100, 180, 30)],
        };
        assert_eq!(
            violation.to_string(),
            "0x1111111111111111111111111111111111111111 ended with more value than it put in: \
             ETH -2 (start 10, end 10, outside 2); 0x7070707070707070707070707070707070707070 +50 \
             (start 100, end 180, outside 30)"
        );
    }
}
