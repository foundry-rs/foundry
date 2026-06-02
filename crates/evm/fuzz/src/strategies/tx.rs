use super::{
    FuzzStateReader, fuzz_calldata, fuzz_calldata_from_state, fuzz_msg_value, fuzz_param,
    fuzz_param_from_state,
};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::{Function, StateMutability};
use alloy_primitives::{Address, U256};
use foundry_config::InvariantConfig;
use proptest::prelude::*;
use std::rc::Rc;

/// Controls how generated transactions populate `msg.value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueKind {
    /// Never generate a value. Used by stateless fuzz tests to preserve legacy behavior.
    None,
    /// Generate a random value for payable target functions.
    Payable,
}

/// Configuration for generating [`BasicTxDetails`] sequences.
#[derive(Clone, Copy, Debug)]
struct TxConfig {
    /// Weight used when choosing calldata arguments from the fuzz dictionary.
    calldata_dictionary_weight: u32,
    /// Weight used when choosing senders from the fuzz dictionary.
    sender_dictionary_weight: u32,
    /// Optional maximum timestamp delay before each transaction.
    max_time_delay: Option<u32>,
    /// Optional maximum block delay before each transaction.
    max_block_delay: Option<u32>,
    /// How to generate `msg.value`.
    value_kind: ValueKind,
}

impl TxConfig {
    /// Stateless fuzzing is depth-1 tx generation with fixed sender/target and no value/delay.
    pub const fn stateless(dictionary_weight: u32) -> Self {
        Self {
            calldata_dictionary_weight: dictionary_weight,
            sender_dictionary_weight: 0,
            max_time_delay: None,
            max_block_delay: None,
            value_kind: ValueKind::None,
        }
    }

    /// Invariant fuzzing generates full tx details, including sender, value, warp, and roll.
    pub const fn invariant(config: &InvariantConfig) -> Self {
        Self {
            // Preserve the historical invariant calldata mix: sender selection honors
            // `dictionary_weight`, while calldata used a fixed 40% dictionary branch.
            calldata_dictionary_weight: 40,
            sender_dictionary_weight: config.dictionary.dictionary_weight,
            max_time_delay: config.max_time_delay,
            max_block_delay: config.max_block_delay,
            value_kind: ValueKind::Payable,
        }
    }
}

enum TxTargets {
    Stateless { sender: Address, target: Address, function: Function },
    Invariant { senders: SenderFilters, contracts: FuzzRunIdentifiedContracts },
}

/// Concrete transaction generator shared by stateless and invariant fuzz campaigns.
pub struct TxGenerator<S> {
    fuzz_state: S,
    fuzz_fixtures: FuzzFixtures,
    targets: TxTargets,
    config: TxConfig,
}

impl<S: FuzzStateReader> TxGenerator<S> {
    /// Creates a depth-1 stateless tx generator for one fuzz test function.
    pub const fn stateless(
        fuzz_state: S,
        sender: Address,
        target: Address,
        function: Function,
        fuzz_fixtures: FuzzFixtures,
        dictionary_weight: u32,
    ) -> Self {
        Self {
            fuzz_state,
            fuzz_fixtures,
            targets: TxTargets::Stateless { sender, target, function },
            config: TxConfig::stateless(dictionary_weight),
        }
    }

    /// Creates a sequence tx generator for invariant campaigns.
    pub fn invariant(
        fuzz_state: S,
        senders: SenderFilters,
        contracts: FuzzRunIdentifiedContracts,
        config: InvariantConfig,
        fuzz_fixtures: FuzzFixtures,
    ) -> Self {
        Self {
            fuzz_state,
            fuzz_fixtures,
            targets: TxTargets::Invariant { senders, contracts },
            config: TxConfig::invariant(&config),
        }
    }

    /// Builds the proptest strategy that produces one transaction at a time. Corpus code is
    /// responsible for composing these into depth-1 or deeper sequences.
    pub fn strategy(self) -> BoxedStrategy<BasicTxDetails> {
        match self.targets {
            TxTargets::Stateless { sender, target, function } => {
                fuzz_contract_with_calldata_config(
                    &self.fuzz_state,
                    &self.fuzz_fixtures,
                    target,
                    function,
                    self.config.calldata_dictionary_weight,
                    self.config.value_kind,
                )
                .prop_map(move |call_details| BasicTxDetails {
                    warp: None,
                    roll: None,
                    sender,
                    call_details,
                })
                .boxed()
            }
            TxTargets::Invariant { senders, contracts } => {
                let senders = Rc::new(senders);
                let config = self.config;
                let fuzz_state = self.fuzz_state;
                let fuzz_fixtures = self.fuzz_fixtures;

                any::<prop::sample::Selector>()
                    .prop_flat_map(move |selector| {
                        let contracts = contracts.targets();
                        let functions = contracts.fuzzed_functions();
                        let (target_address, target_function) = selector.select(functions);

                        let sender = select_random_sender(
                            &fuzz_state,
                            senders.clone(),
                            config.sender_dictionary_weight,
                        );

                        let call_details = fuzz_contract_with_calldata_config(
                            &fuzz_state,
                            &fuzz_fixtures,
                            *target_address,
                            target_function.clone(),
                            config.calldata_dictionary_weight,
                            config.value_kind,
                        );

                        let warp = warp_roll_strat(config.max_time_delay.is_some());
                        let roll = warp_roll_strat(config.max_block_delay.is_some());

                        (warp, roll, sender, call_details)
                    })
                    .prop_map(move |(warp, roll, sender, call_details)| {
                        let warp = warp.map(|time| {
                            time % U256::from(config.max_time_delay.unwrap_or_default())
                        });
                        let roll = roll.map(|block| {
                            block % U256::from(config.max_block_delay.unwrap_or_default())
                        });
                        BasicTxDetails { warp, roll, sender, call_details }
                    })
                    .boxed()
            }
        }
    }
}

// Strategy to generate values for tx warp and roll.
fn warp_roll_strat(cond: bool) -> BoxedStrategy<Option<U256>> {
    if cond { any::<U256>().prop_map(Some).boxed() } else { Just(None).boxed() }
}

/// Strategy to select a sender address:
/// * If `senders` is empty, then it's either a random address (10%) or from the dictionary (90%).
/// * If `senders` is not empty, a random address is chosen from the list of senders.
fn select_random_sender<S: FuzzStateReader>(
    fuzz_state: &S,
    senders: Rc<SenderFilters>,
    dictionary_weight: u32,
) -> impl Strategy<Value = Address> + use<S> {
    if senders.targeted.is_empty() {
        assert!(dictionary_weight <= 100, "dictionary_weight must be <= 100");
        proptest::prop_oneof![
            100 - dictionary_weight => fuzz_param(&DynSolType::Address),
            dictionary_weight => fuzz_param_from_state(&DynSolType::Address, fuzz_state),
        ]
        .prop_map(move |addr| {
            let mut addr = addr.as_address().unwrap();
            // Make sure the selected address is not in the list of excluded senders.
            // We don't use proptest's filter to avoid reaching the `PROPTEST_MAX_LOCAL_REJECTS`
            // max rejects and exiting test before all runs completes.
            // See <https://github.com/foundry-rs/foundry/issues/11369>.
            loop {
                if !senders.excluded.contains(&addr) {
                    break;
                }
                addr = Address::random();
            }
            addr
        })
        .boxed()
    } else {
        any::<prop::sample::Index>().prop_map(move |index| *index.get(&senders.targeted)).boxed()
    }
}

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_contract_with_calldata<S: FuzzStateReader>(
    fuzz_state: &S,
    fuzz_fixtures: &FuzzFixtures,
    target: Address,
    func: Function,
) -> impl Strategy<Value = CallDetails> + use<S> {
    fuzz_contract_with_calldata_config(
        fuzz_state,
        fuzz_fixtures,
        target,
        func,
        40,
        ValueKind::Payable,
    )
}

/// Given a function, it returns a proptest strategy which generates transaction call details.
fn fuzz_contract_with_calldata_config<S: FuzzStateReader>(
    fuzz_state: &S,
    fuzz_fixtures: &FuzzFixtures,
    target: Address,
    func: Function,
    dictionary_weight: u32,
    value_kind: ValueKind,
) -> impl Strategy<Value = CallDetails> + use<S> {
    let is_payable = func.state_mutability == StateMutability::Payable;
    let dictionary_weight = dictionary_weight.min(100);

    // We need to compose all the strategies generated for each parameter in all possible
    // combinations.
    // `prop_oneof!` / `TupleUnion` `Arc`s for cheap cloning.
    let calldata_strategy = prop_oneof![
        100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
        dictionary_weight => fuzz_calldata_from_state(func, fuzz_state),
    ];

    let value_strategy = if value_kind == ValueKind::Payable && is_payable {
        fuzz_msg_value().boxed()
    } else {
        Just(None).boxed()
    };

    (calldata_strategy, value_strategy).prop_map(move |(calldata, value)| {
        trace!(input=?calldata, ?value);
        CallDetails { target, calldata, value }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::EvmFuzzState;
    use proptest::{strategy::ValueTree, test_runner::TestRunner};

    #[test]
    fn stateless_tx_generator_preserves_fixed_call_context() {
        let sender = Address::with_last_byte(0x11);
        let target = Address::with_last_byte(0x22);
        let function = Function::parse("fuzz_me(uint256)").unwrap();
        let selector = function.selector();
        let fuzz_state = EvmFuzzState::test();

        let tx = TxGenerator::stateless(
            fuzz_state,
            sender,
            target,
            function,
            FuzzFixtures::default(),
            0,
        )
        .strategy()
        .new_tree(&mut TestRunner::default())
        .unwrap()
        .current();

        assert_eq!(tx.sender, sender);
        assert_eq!(tx.call_details.target, target);
        assert_eq!(tx.call_details.calldata.get(..4), Some(selector.as_slice()));
        assert_eq!(tx.call_details.value, None);
        assert_eq!(tx.warp, None);
        assert_eq!(tx.roll, None);
    }
}
