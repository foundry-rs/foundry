use super::{
    DictionaryRead, EvmFuzzState, FuzzState, fuzz_calldata, fuzz_calldata_from_state,
    fuzz_msg_value, fuzz_param, fuzz_param_from_state,
    lifecycle::{LifecycleStep, lifecycle_scenarios},
};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Selector, U256};
use eyre::{Result, eyre};
use foundry_config::InvariantConfig;
use proptest::{prelude::*, test_runner::TestRunner};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Arc};

const LIFECYCLE_TRANSACTION_WEIGHT: u32 = 20;
const MAX_LIFECYCLE_SEQUENCES: usize = 256;

#[derive(Default)]
struct PlannedCalls {
    generation: u64,
    calls: Vec<BoxedStrategy<CallDetails>>,
}

/// Concrete generator for stateless and invariant transactions.
#[derive(Clone)]
pub struct TxGenerator {
    strategy: BoxedStrategy<BasicTxDetails>,
    lifecycle: Option<Rc<RefCell<LifecycleBootstrap>>>,
}

struct LifecycleBootstrap {
    generation: Option<u64>,
    declared: Arc<[Vec<(Address, Selector)>]>,
    scenarios: Vec<Vec<LifecycleStep>>,
    eager_sequences: usize,
    cursor: usize,
    sequences: usize,
    pending: VecDeque<LifecycleStep>,
    calls: Vec<[BoxedStrategy<BasicTxDetails>; 2]>,
    state: FuzzState,
    fixtures: FuzzFixtures,
    contracts: FuzzRunIdentifiedContracts,
    actor: Address,
    max_time_delay: Option<u32>,
    max_block_delay: Option<u32>,
    payable_value_weight: u32,
}

impl LifecycleBootstrap {
    fn next_tx(&mut self, runner: &mut TestRunner) -> Result<Option<BasicTxDetails>> {
        let generation = self.contracts.fuzzed_functions_generation();
        if self.generation != Some(generation) {
            let functions = self.contracts.fuzzed_functions();
            (self.scenarios, self.eager_sequences) =
                lifecycle_scenarios(&functions, &self.declared);
            let actor = self.actor;
            self.calls = functions
                .iter()
                .map(|(target, function)| {
                    [0, 100].map(|dictionary_weight| {
                        let call = TxGenerator::call_strategy(
                            &self.state,
                            &self.fixtures,
                            *target,
                            function.clone(),
                            dictionary_weight,
                            self.payable_value_weight,
                        );
                        (
                            optional_delay(self.max_time_delay),
                            optional_delay(self.max_block_delay),
                            call,
                        )
                            .prop_map(move |(warp, roll, call_details)| BasicTxDetails {
                                warp,
                                roll,
                                sender: actor,
                                call_details,
                            })
                            .boxed()
                    })
                })
                .collect();
            self.pending.clear();
            self.cursor = 0;
            self.generation = Some(generation);
        }

        if self.pending.is_empty() {
            if self.sequences == MAX_LIFECYCLE_SEQUENCES
                || self.scenarios.is_empty()
                || (self.sequences >= self.eager_sequences
                    && !runner.rng().random_ratio(LIFECYCLE_TRANSACTION_WEIGHT, 100))
            {
                return Ok(None);
            }
            self.pending.extend(self.scenarios[self.cursor].iter().copied());
            self.cursor = (self.cursor + 1) % self.scenarios.len();
            self.sequences += 1;
        }

        let step = self.pending.pop_front().expect("checked non-empty");
        let strategy = &self.calls[step.function][usize::from(step.dictionary)];
        let tx = strategy
            .new_tree(runner)
            .map_err(|_| eyre!("Could not generate lifecycle case"))?
            .current();
        Ok(Some(tx))
    }
}

impl TxGenerator {
    /// Wraps a prebuilt strategy, primarily for deterministic tests.
    pub const fn from_strategy(strategy: BoxedStrategy<BasicTxDetails>) -> Self {
        Self { strategy, lifecycle: None }
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
            lifecycle: None,
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
        call_sequences: Arc<[Vec<(Address, Selector)>]>,
        config: InvariantConfig,
        fixtures: FuzzFixtures,
    ) -> Self {
        let dictionary_weight = config.dictionary.dictionary_weight;
        let payable_value_weight = config.corpus.payable_value_weight;
        let lifecycle = (config.lifecycle_bootstrap || !call_sequences.is_empty()).then(|| {
            let actor = senders.targeted.first().copied().unwrap_or_else(|| {
                let mut suffix = 1u8;
                loop {
                    let actor = Address::with_last_byte(suffix);
                    if senders.allows(actor) {
                        break actor;
                    }
                    suffix = suffix.wrapping_add(1);
                }
            });
            Rc::new(RefCell::new(LifecycleBootstrap {
                generation: None,
                declared: call_sequences,
                scenarios: Vec::new(),
                eager_sequences: 0,
                cursor: 0,
                sequences: 0,
                pending: VecDeque::new(),
                calls: Vec::new(),
                state: state.clone(),
                fixtures: fixtures.clone(),
                contracts: contracts.clone(),
                actor,
                max_time_delay: config.max_time_delay,
                max_block_delay: config.max_block_delay,
                payable_value_weight,
            }))
        });
        let senders = Rc::new(senders);
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
        Self { strategy, lifecycle }
    }

    /// Draws the next transaction from this generator.
    pub fn next_tx(&self, runner: &mut TestRunner) -> Result<BasicTxDetails> {
        if let Some(lifecycle) = &self.lifecycle
            && let Some(tx) = lifecycle.borrow_mut().next_tx(runner)?
        {
            return Ok(tx);
        }
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
    use crate::invariant::{TargetedContract, TargetedContracts};
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
            Arc::from([]),
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
    fn invariant_generator_emits_lifecycle_sequences() {
        let target = Address::with_last_byte(1);
        let functions = [
            "switchActor(uint256)",
            "switch_asset(uint256)",
            "supply(uint256,uint256)",
            "setUsingAsCollateral(uint256,bool)",
            "borrow(uint256,uint256)",
            "liquidationCall(uint256,uint256)",
        ]
        .into_iter()
        .map(|signature| Function::parse(signature).unwrap())
        .collect::<Vec<_>>();
        let mut abi = JsonAbi::new();
        for function in &functions {
            abi.functions.entry(function.name.clone()).or_default().push(function.clone());
        }
        let mut targets = TargetedContracts::new();
        targets.insert(target, TargetedContract::new("Target".into(), abi));
        let state = EvmFuzzState::new(
            &[],
            &CacheDB::<EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
        .into_invariant();
        let config = InvariantConfig { lifecycle_bootstrap: true, ..Default::default() };
        let generator = TxGenerator::invariant(
            state,
            SenderFilters::default(),
            FuzzRunIdentifiedContracts::new(targets, false),
            Arc::from([]),
            config,
            FuzzFixtures::default(),
        );
        let expected = [0, 1, 3, 2, 4, 0, 5].map(|index| functions[index].selector());
        let mut matched = 0;
        let mut runner = TestRunner::deterministic();

        for _ in 0..256 {
            let tx = generator.next_tx(&mut runner).unwrap();
            let selector = &tx.call_details.calldata[..4];
            if selector == expected[matched].as_slice() {
                matched += 1;
                if matched == expected.len() {
                    return;
                }
            } else {
                matched = usize::from(selector == expected[0].as_slice());
            }
        }
        panic!("lifecycle sequence was not emitted");
    }

    #[test]
    fn invariant_generator_eagerly_emits_declared_sequence() {
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);
        let prepare = Function::parse("prepare()").unwrap();
        let exercise = Function::parse("exercise()").unwrap();
        let mut targets = TargetedContracts::new();
        for (target, function) in [(first, &prepare), (second, &exercise)] {
            let mut abi = JsonAbi::new();
            abi.functions.entry(function.name.clone()).or_default().push(function.clone());
            targets.insert(target, TargetedContract::new("Target".into(), abi));
        }
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
            FuzzRunIdentifiedContracts::new(targets, false),
            Arc::from([vec![(first, prepare.selector()), (second, exercise.selector())]]),
            InvariantConfig::default(),
            FuzzFixtures::default(),
        );
        let mut runner = TestRunner::deterministic();

        for (target, selector) in [(first, prepare.selector()), (second, exercise.selector())] {
            let tx = generator.next_tx(&mut runner).unwrap();
            assert_eq!(tx.call_details.target, target);
            assert_eq!(&tx.call_details.calldata[..4], selector.as_slice());
        }
    }
}
