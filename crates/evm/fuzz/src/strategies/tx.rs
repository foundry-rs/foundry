use super::{
    DictionaryRead, EvmFuzzState, FuzzState, fuzz_calldata, fuzz_calldata_from_state,
    fuzz_msg_value, fuzz_param, fuzz_param_from_state,
};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use eyre::{Result, eyre};
use foundry_config::InvariantConfig;
use proptest::{prelude::*, test_runner::TestRunner};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct PlannedCalls {
    generation: u64,
    calls: Vec<BoxedStrategy<CallDetails>>,
}

/// Concrete generator for stateless and invariant transactions.
#[derive(Clone)]
pub struct TxGenerator {
    strategy: BoxedStrategy<BasicTxDetails>,
}

impl TxGenerator {
    /// Wraps a prebuilt strategy, primarily for deterministic tests.
    pub const fn from_strategy(strategy: BoxedStrategy<BasicTxDetails>) -> Self {
        Self { strategy }
    }
    /// Creates a fixed-target, fixed-sender stateless generator.
    pub fn stateless(
        state: EvmFuzzState,
        fixtures: FuzzFixtures,
        target: Address,
        sender: Address,
        function: Function,
        dictionary_weight: u32,
        payable_value_weight: u32,
    ) -> Self {
        let call = Self::call_strategy(
            &state,
            &fixtures,
            target,
            function,
            dictionary_weight,
            payable_value_weight,
        );
        Self {
            strategy: call
                .prop_map(move |call_details| BasicTxDetails {
                    warp: None,
                    roll: None,
                    sender,
                    call_details,
                })
                .boxed(),
        }
    }

    /// Creates a lazy invariant generator whose target list follows deployed contracts.
    pub fn invariant(
        state: FuzzState,
        senders: SenderFilters,
        contracts: FuzzRunIdentifiedContracts,
        config: InvariantConfig,
        fixtures: FuzzFixtures,
    ) -> Self {
        let senders = Rc::new(senders);
        let dictionary_weight = config.dictionary.dictionary_weight;
        let payable_value_weight = config.corpus.payable_value_weight;
        let planned = Rc::new(RefCell::new(PlannedCalls::default()));
        let strategy = any::<prop::sample::Selector>()
            .prop_flat_map(move |selector| {
                let sender = select_sender(&state, senders.clone(), dictionary_weight);
                let call = {
                    let generation = contracts.fuzzed_functions_generation();
                    let mut planned = planned.borrow_mut();
                    if planned.generation != generation || planned.calls.is_empty() {
                        planned.calls = contracts
                            .fuzzed_functions()
                            .iter()
                            .map(|(target, function)| {
                                Self::call_strategy(
                                    &state,
                                    &fixtures,
                                    *target,
                                    function.clone(),
                                    dictionary_weight,
                                    payable_value_weight,
                                )
                            })
                            .collect();
                        planned.generation = generation;
                    }
                    selector.select(planned.calls.iter()).clone()
                };
                let warp = optional_delay(config.max_time_delay.is_some());
                let roll = optional_delay(config.max_block_delay.is_some());
                (warp, roll, sender, call)
            })
            .prop_map(move |(warp, roll, sender, call_details)| BasicTxDetails {
                warp: warp
                    .map(|value| value % U256::from(config.max_time_delay.unwrap_or_default())),
                roll: roll
                    .map(|value| value % U256::from(config.max_block_delay.unwrap_or_default())),
                sender,
                call_details,
            })
            .boxed();
        Self { strategy }
    }

    /// Draws the next transaction from this generator.
    pub fn next_tx(&self, runner: &mut TestRunner) -> Result<BasicTxDetails> {
        Ok(self.strategy.new_tree(runner).map_err(|_| eyre!("Could not generate case"))?.current())
    }

    /// Generates calldata and payable value for one contract call.
    pub(crate) fn call_strategy<S: DictionaryRead>(
        state: &S,
        fixtures: &FuzzFixtures,
        target: Address,
        function: Function,
        dictionary_weight: u32,
        payable_value_weight: u32,
    ) -> BoxedStrategy<CallDetails> {
        let payable = function.state_mutability == alloy_json_abi::StateMutability::Payable;
        let dictionary_weight = dictionary_weight.min(100);
        let calldata = prop_oneof![
            100 - dictionary_weight => fuzz_calldata(function.clone(), fixtures),
            dictionary_weight => fuzz_calldata_from_state(function, state, fixtures),
        ];
        let value =
            if payable { fuzz_msg_value(payable_value_weight).boxed() } else { Just(None).boxed() };
        (calldata, value)
            .prop_map(move |(calldata, value)| CallDetails { target, calldata, value })
            .boxed()
    }
}

fn optional_delay(enabled: bool) -> BoxedStrategy<Option<U256>> {
    if enabled { any::<U256>().prop_map(Some).boxed() } else { Just(None).boxed() }
}

fn select_sender(
    state: &FuzzState,
    senders: Rc<SenderFilters>,
    dictionary_weight: u32,
) -> BoxedStrategy<Address> {
    if senders.targeted.is_empty() {
        let dictionary_weight = dictionary_weight.min(100);
        prop_oneof![
            100 - dictionary_weight => fuzz_param(&DynSolType::Address),
            dictionary_weight => fuzz_param_from_state(&DynSolType::Address, state),
        ]
        .prop_map(move |value| {
            let mut sender = value.as_address().unwrap();
            while senders.excluded.contains(&sender) {
                sender = Address::random();
            }
            sender
        })
        .boxed()
    } else {
        any::<prop::sample::Index>().prop_map(move |index| *index.get(&senders.targeted)).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invariant::{TargetedContract, TargetedContracts};
    use alloy_json_abi::JsonAbi;
    use foundry_config::FuzzDictionaryConfig;
    use revm::database::{CacheDB, EmptyDB};
    #[test]
    fn stateless_generator_has_fixed_metadata() {
        let target = Address::with_last_byte(1);
        let sender = Address::with_last_byte(2);
        let function = Function::parse("fuzz(uint256)").unwrap();
        let generator = TxGenerator::stateless(
            EvmFuzzState::test(),
            FuzzFixtures::default(),
            target,
            sender,
            function,
            40,
            10,
        );
        let mut runner = TestRunner::deterministic();
        let tx = generator.next_tx(&mut runner).unwrap();
        assert_eq!(tx.sender, sender);
        assert_eq!(tx.call_details.target, target);
        assert_eq!(tx.warp, None);
        assert_eq!(tx.roll, None);
    }

    #[test]
    fn invariant_generator_refreshes_removed_targets_lazily() {
        let retained = Address::with_last_byte(1);
        let removed = Address::with_last_byte(2);
        let function = Function::parse("fuzz(uint256)").unwrap();
        let mut abi = JsonAbi::new();
        abi.functions.entry(function.name.clone()).or_default().push(function);
        let mut targets = TargetedContracts::new();
        targets.insert(retained, TargetedContract::new("Retained".into(), abi.clone()));
        targets.insert(removed, TargetedContract::new("Removed".into(), abi));
        let identified = FuzzRunIdentifiedContracts::new(targets, false);
        let state = EvmFuzzState::new(
            &[],
            &CacheDB::<EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
        .into_invariant();
        let generator = TxGenerator::invariant(
            state,
            SenderFilters::default(),
            identified.clone(),
            InvariantConfig::default(),
            FuzzFixtures::default(),
        );
        let mut runner = TestRunner::deterministic();

        // Populate the lazy cache while both calls are available, then invalidate it solely via
        // the public lifecycle API used after invariant runs.
        let _ = generator.next_tx(&mut runner).unwrap();
        identified.clear_created_contracts(vec![removed]);
        for _ in 0..32 {
            assert_eq!(generator.next_tx(&mut runner).unwrap().call_details.target, retained);
        }
    }
}
