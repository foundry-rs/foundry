use super::{
    DictionaryRead, EvmFuzzState, FuzzState, fuzz_calldata, fuzz_calldata_from_state,
    fuzz_msg_value, fuzz_param, fuzz_param_from_state,
};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures, FuzzGuidance,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters, TargetedContracts},
};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use eyre::{Result, eyre};
use foundry_config::InvariantConfig;
use proptest::{prelude::*, test_runner::TestRunner};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Default)]
struct PlannedCalls {
    generation: u64,
    calls: Vec<BoxedStrategy<CallDetails>>,
    /// Cumulative guidance selector weights of `calls`, empty without selector guidance.
    cumulative_weights: Vec<u64>,
}

impl PlannedCalls {
    fn rebuild(
        &mut self,
        generation: u64,
        fuzzed_functions: &[(Address, Function)],
        targets: &TargetedContracts,
        guidance: &FuzzGuidance,
        mut build: impl FnMut(Address, Function) -> BoxedStrategy<CallDetails>,
    ) {
        self.calls.clear();
        self.calls.reserve(fuzzed_functions.len());
        self.cumulative_weights.clear();
        if guidance.has_selector_weights() {
            self.cumulative_weights.reserve(fuzzed_functions.len());
        }

        let mut total = 0u64;
        for (target, function) in fuzzed_functions {
            self.calls.push(build(*target, function.clone()));
            if guidance.has_selector_weights() {
                let weight = targets
                    .get(target)
                    .and_then(|contract| guidance.selector_weight(&contract.identifier, function))
                    .unwrap_or(1);
                total += u64::from(weight);
                self.cumulative_weights.push(total);
            }
        }
        self.generation = generation;
    }

    /// Picks a call using the drawn random choice.
    fn select(&self, choice: CallChoice) -> &BoxedStrategy<CallDetails> {
        match choice {
            CallChoice::Uniform(selector) => selector.select(self.calls.iter()),
            CallChoice::Weighted(index) => match self.cumulative_weights.last() {
                Some(&total) if total > 0 => {
                    let point = index.index(total as usize) as u64;
                    let call = self.cumulative_weights.partition_point(|&weight| weight <= point);
                    &self.calls[call]
                }
                // Every function has weight 0: ignore the weights rather than generating nothing.
                _ => index.get(&self.calls),
            },
        }
    }
}

/// Random choice used to pick the next invariant call.
///
/// Values are drawn and consumed once per generated call, so boxing the selector would only add
/// an allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum CallChoice {
    Uniform(prop::sample::Selector),
    Weighted(prop::sample::Index),
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
        let guidance = state.with_dictionary(|dict| Arc::clone(dict.guidance()));
        // Only draw a weighted index when selector guidance is present, so unguided runs stay
        // reproducible.
        let choice = if guidance.has_selector_weights() {
            any::<prop::sample::Index>().prop_map(CallChoice::Weighted).boxed()
        } else {
            any::<prop::sample::Selector>().prop_map(CallChoice::Uniform).boxed()
        };
        let strategy = choice
            .prop_flat_map(move |choice| {
                let sender = select_sender(&state, senders.clone(), dictionary_weight);
                let call = {
                    let generation = contracts.fuzzed_functions_generation();
                    let mut planned = planned.borrow_mut();
                    if planned.generation != generation || planned.calls.is_empty() {
                        let fuzzed_functions = contracts.fuzzed_functions();
                        let targets = contracts.targets();
                        planned.rebuild(
                            generation,
                            &fuzzed_functions,
                            &targets,
                            &guidance,
                            |target, function| {
                                Self::call_strategy(
                                    &state,
                                    &fixtures,
                                    target,
                                    function,
                                    dictionary_weight,
                                    payable_value_weight,
                                )
                            },
                        );
                    }
                    planned.select(choice).clone()
                };
                let warp = optional_delay(config.max_time_delay);
                let roll = optional_delay(config.max_block_delay);
                (warp, roll, sender, call)
            })
            .prop_map(move |(warp, roll, sender, call_details)| BasicTxDetails {
                warp,
                roll,
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

fn optional_delay(max: Option<u32>) -> BoxedStrategy<Option<U256>> {
    if let Some(max) = max.filter(|max| *max > 0) {
        any::<U256>().prop_map(move |value| Some(value % U256::from(max))).boxed()
    } else {
        Just(None).boxed()
    }
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
    use crate::invariant::TargetedContract;
    use alloy_json_abi::JsonAbi;
    use foundry_config::FuzzDictionaryConfig;
    use revm::database::{CacheDB, EmptyDB};

    #[test]
    fn zero_delay_is_disabled() {
        let mut runner = TestRunner::deterministic();
        assert_eq!(optional_delay(Some(0)).new_tree(&mut runner).unwrap().current(), None);
        assert_eq!(
            optional_delay(Some(1)).new_tree(&mut runner).unwrap().current(),
            Some(U256::ZERO)
        );
    }

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

    #[test]
    fn invariant_generator_skips_zero_weight_functions() {
        let target = Address::with_last_byte(1);
        let keep = Function::parse("keep(uint256)").unwrap();
        let skip = Function::parse("skip(uint256)").unwrap();
        let keep_selector = keep.selector();
        let mut abi = JsonAbi::new();
        for function in [keep, skip] {
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }
        let mut targets = TargetedContracts::new();
        targets.insert(target, TargetedContract::new("src/Target.sol:Target".into(), abi));
        let mut state = EvmFuzzState::test();
        state.set_guidance(Arc::new(
            FuzzGuidance::from_json(
                r#"{"version": 1, "selector_weights": {"Target.skip(uint256)": 0}}"#,
            )
            .unwrap(),
        ));
        let generator = TxGenerator::invariant(
            state.into_invariant(),
            SenderFilters::default(),
            FuzzRunIdentifiedContracts::new(targets, false),
            InvariantConfig::default(),
            FuzzFixtures::default(),
        );
        let mut runner = TestRunner::deterministic();

        for _ in 0..64 {
            let tx = generator.next_tx(&mut runner).unwrap();
            assert_eq!(tx.call_details.calldata[..4], keep_selector[..]);
        }
    }
}
