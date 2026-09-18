use super::{
    DictionaryRead, EvmFuzzState, FuzzState, fuzz_calldata, fuzz_calldata_from_state,
    fuzz_msg_value, fuzz_param, fuzz_param_from_state,
    jev::{ArgumentSource, Jev, ViewRelation},
};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
};
use alloy_dyn_abi::{DynSolType, DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256};
use eyre::{Result, eyre};
use foundry_config::{InvariantConfig, InvariantTxGenerator};
use proptest::{prelude::*, test_runner::TestRunner};
use std::{cell::RefCell, rc::Rc};

const JEV_TRANSACTION_WEIGHT: u32 = 20;

/// A non-committing read selected from the ABI grammar and evaluated by the executor immediately
/// before its associated transaction.
pub struct ViewCall {
    pub target: Address,
    pub sender: Address,
    pub calldata: Bytes,
}

struct PendingView {
    tx: BasicTxDetails,
    target_function: Function,
    relation: ViewRelation,
}

#[derive(Default)]
struct PlannedCalls {
    generation: u64,
    calls: Vec<BoxedStrategy<CallDetails>>,
}

/// Concrete generator for stateless and invariant transactions.
#[derive(Clone)]
pub struct TxGenerator {
    strategy: BoxedStrategy<BasicTxDetails>,
    guided: Option<Rc<RefCell<GuidedCalls>>>,
}

struct GuidedCalls {
    jev: Jev,
    generation: Option<u64>,
    calls: Vec<BoxedStrategy<BasicTxDetails>>,
    state: FuzzState,
    fixtures: FuzzFixtures,
    contracts: FuzzRunIdentifiedContracts,
    sender: BoxedStrategy<Address>,
    config: InvariantConfig,
    pending_views: Vec<PendingView>,
    pending_feedback: Vec<(BasicTxDetails, usize)>,
}

impl GuidedCalls {
    fn next_strategy(
        &mut self,
    ) -> Option<(BoxedStrategy<BasicTxDetails>, Function, ArgumentSource, usize)> {
        let generation = self.contracts.fuzzed_functions_generation();
        if self.generation != Some(generation) {
            let functions = self.contracts.fuzzed_functions();
            self.jev.refresh(&functions, &self.contracts.view_functions());
            self.calls = functions
                .iter()
                .flat_map(|(target, function)| {
                    [0, 100].map(|dictionary_weight| {
                        let call = TxGenerator::call_strategy(
                            &self.state,
                            &self.fixtures,
                            *target,
                            function.clone(),
                            dictionary_weight,
                            self.config.corpus.payable_value_weight,
                        );
                        (
                            optional_delay(self.config.max_time_delay),
                            optional_delay(self.config.max_block_delay),
                            self.sender.clone(),
                            call,
                        )
                            .prop_map(|(warp, roll, sender, call_details)| BasicTxDetails {
                                warp,
                                roll,
                                sender,
                                call_details,
                            })
                            .boxed()
                    })
                })
                .collect();
            self.generation = Some(generation);
        }
        let production = self.jev.next()?;
        let strategy_index = production.function * 2
            + usize::from(matches!(production.source, ArgumentSource::Dictionary));
        tracing::debug!(target: "forge::jev", production = production.id, "model-selected transaction");
        let function = self.contracts.fuzzed_functions().get(production.function)?.1.clone();
        self.calls
            .get(strategy_index)
            .cloned()
            .map(|strategy| (strategy, function, production.source, production.id))
    }
}

impl TxGenerator {
    /// Wraps a prebuilt strategy, primarily for deterministic tests.
    pub const fn from_strategy(strategy: BoxedStrategy<BasicTxDetails>) -> Self {
        Self { strategy, guided: None }
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
            guided: None,
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
        let guided = (config.tx_generator == InvariantTxGenerator::Jev).then(|| {
            Rc::new(RefCell::new(GuidedCalls {
                jev: Jev::new(),
                generation: None,
                calls: Vec::new(),
                state: state.clone(),
                fixtures: fixtures.clone(),
                contracts: contracts.clone(),
                sender: select_sender(&state, senders.clone(), dictionary_weight),
                config: config.clone(),
                pending_views: Vec::new(),
                pending_feedback: Vec::new(),
            }))
        });
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
        Self { strategy, guided }
    }

    /// Draws the next transaction from this generator.
    pub fn next_tx(&self, runner: &mut TestRunner) -> Result<BasicTxDetails> {
        if let Some(guided) = &self.guided
            && runner.rng().random_ratio(JEV_TRANSACTION_WEIGHT, 100)
        {
            let next = guided.borrow_mut().next_strategy();
            if let Some((strategy, target_function, source, production)) = next {
                let tx = strategy
                    .new_tree(runner)
                    .map_err(|_| eyre!("Could not generate guided case"))?
                    .current();
                if let ArgumentSource::View(relation) = source {
                    guided.borrow_mut().pending_views.push(PendingView {
                        tx: tx.clone(),
                        target_function,
                        relation,
                    });
                }
                guided.borrow_mut().pending_feedback.push((tx.clone(), production));
                return Ok(tx);
            }
        }
        Ok(self.strategy.new_tree(runner).map_err(|_| eyre!("Could not generate case"))?.current())
    }

    /// Resolves a pending compatible-view production against the caller's current EVM state. A
    /// failed read or decode leaves the original random argument unchanged.
    pub fn resolve_view(
        &self,
        tx: &mut BasicTxDetails,
        mut call: impl FnMut(ViewCall) -> Option<Bytes>,
    ) {
        let Some(guided) = &self.guided else { return };
        let pending = {
            let mut guided = guided.borrow_mut();
            let Some(index) = guided.pending_views.iter().position(|pending| pending.tx == *tx)
            else {
                return;
            };
            guided.pending_views.swap_remove(index)
        };
        let original_tx = tx.clone();
        let view_args = pending
            .relation
            .function
            .inputs
            .iter()
            .map(|_| DynSolValue::Address(tx.sender))
            .collect::<Vec<_>>();
        let Ok(view_calldata) = pending.relation.function.abi_encode_input(&view_args) else {
            return;
        };
        let Some(result) = call(ViewCall {
            target: tx.call_details.target,
            sender: tx.sender,
            calldata: view_calldata.into(),
        }) else {
            return;
        };
        let Ok(mut view_values) = pending.relation.function.abi_decode_output(&result) else {
            return;
        };
        let Some(value) = view_values.pop() else { return };
        let Some(encoded_args) = tx.call_details.calldata.get(4..) else { return };
        let Ok(mut args) = pending.target_function.abi_decode_input(encoded_args) else { return };
        let Some(argument) = args.get_mut(pending.relation.argument) else { return };
        *argument = value;
        if let Ok(calldata) = pending.target_function.abi_encode_input(&args) {
            tx.call_details.calldata = calldata.into();
            if let Some(feedback) = guided
                .borrow_mut()
                .pending_feedback
                .iter_mut()
                .find(|(candidate, _)| *candidate == original_tx)
            {
                feedback.0 = tx.clone();
            }
            tracing::debug!(target: "forge::jev", view = %pending.relation.function.signature(),
                argument = pending.relation.argument, "resolved Jev view argument");
        }
    }

    /// Reset observed execution history at the beginning of each EVM-reset run.
    pub fn begin_run(&self) {
        if let Some(guided) = &self.guided {
            let mut guided = guided.borrow_mut();
            guided.jev.begin_run();
            guided.pending_views.clear();
            guided.pending_feedback.clear();
        }
    }

    /// Supply execution feedback for future online choices, including corpus-generated calls.
    pub fn observe(
        &self,
        tx: &BasicTxDetails,
        reverted: bool,
        discarded: bool,
        new_coverage: bool,
    ) {
        if let Some(guided) = &self.guided {
            let mut guided = guided.borrow_mut();
            let selected = guided
                .pending_feedback
                .iter()
                .position(|(candidate, _)| candidate == tx)
                .map(|index| guided.pending_feedback.swap_remove(index).1);
            guided.jev.observe(tx, selected, reverted, discarded, new_coverage);
        }
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
    use serde_json::{Map, json};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn jev_decides_native_transactions_online_and_receives_execution_feedback() {
        let target = Address::with_last_byte(1);
        let deposit = Function::parse("deposit(uint256)").unwrap();
        let withdraw = Function::parse("withdraw(uint256)").unwrap();
        let mut abi = JsonAbi::new();
        abi.functions.insert(deposit.name.clone(), vec![deposit]);
        abi.functions.insert(withdraw.name.clone(), vec![withdraw.clone()]);
        let mut targets = TargetedContracts::new();
        targets.insert(target, TargetedContract::new("Ledger".into(), abi));
        let contracts = FuzzRunIdentifiedContracts::new(targets, false);
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
            contracts,
            InvariantConfig { tx_generator: InvariantTxGenerator::Jev, ..Default::default() },
            FuzzFixtures::default(),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        generator.guided.as_ref().unwrap().borrow_mut().jev = Jev::with_decider(move |request| {
            let batch = counter.fetch_add(1, Ordering::SeqCst);
            if batch > 0 {
                assert_eq!(request["state"]["recent_execution"][0]["new_coverage"], true);
                assert_eq!(request["state"]["recent_execution"][0]["reverted"], false);
                assert!(
                    request["state"]["recent_execution"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|execution| execution["function_productions"] == json!(["p3"]))
                );
            }
            let answers: Map<_, _> = request["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(slot, question)| {
                    let key = question["criteria"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .find(|(_, value)| value.as_str().unwrap().contains("withdraw(uint256)"))
                        .unwrap()
                        .0;
                    (slot.clone(), json!({ "type": "choice", "choice": key }))
                })
                .collect();
            Ok(json!({"model":"typesafe/jev-1.13","answers":answers}))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let mut runner = TestRunner::deterministic();
        let mut guided_calls = 0;
        for _ in 0..80 {
            let tx = generator.next_tx(&mut runner).unwrap();
            assert_eq!(tx.call_details.target, target);
            if tx.call_details.calldata.get(..4) == Some(withdraw.selector().as_slice()) {
                guided_calls += 1;
                assert_eq!(tx.call_details.calldata.len(), 36);
            }
            generator.observe(&tx, false, false, true);
        }
        assert!(guided_calls > 0);
        assert!(calls.load(Ordering::SeqCst) > 0);
        generator.begin_run();
    }

    #[test]
    fn jev_view_relationship_is_resolved_with_sender_and_current_result() {
        let target = Address::with_last_byte(1);
        let sender = Address::with_last_byte(2);
        let withdraw = Function::parse("withdraw(uint256)").unwrap();
        let balance =
            Function::parse("function balanceOf(address) external view returns (uint256)").unwrap();
        let mut abi = JsonAbi::new();
        abi.functions.insert(withdraw.name.clone(), vec![withdraw.clone()]);
        abi.functions.insert(balance.name.clone(), vec![balance.clone()]);
        let mut targets = TargetedContracts::new();
        targets.insert(target, TargetedContract::new("Ledger".into(), abi));
        let contracts = FuzzRunIdentifiedContracts::new(targets, false);
        let state = EvmFuzzState::new(
            &[],
            &CacheDB::<EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
        .into_invariant();
        let generator = TxGenerator::invariant(
            state,
            SenderFilters { targeted: vec![sender], ..Default::default() },
            contracts,
            InvariantConfig { tx_generator: InvariantTxGenerator::Jev, ..Default::default() },
            FuzzFixtures::default(),
        );
        generator.guided.as_ref().unwrap().borrow_mut().jev = Jev::with_decider(move |request| {
            let answers: Map<_, _> = request["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(slot, question)| {
                    let key = question["criteria"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .find(|(_, value)| value.as_str().unwrap().contains("taken from"))
                        .unwrap()
                        .0;
                    (slot.clone(), json!({ "type": "choice", "choice": key }))
                })
                .collect();
            Ok(json!({"model":"typesafe/jev-1.13","answers":answers}))
        });

        let mut runner = TestRunner::deterministic();
        let mut tx = (0..100)
            .find_map(|_| {
                let tx = generator.next_tx(&mut runner).unwrap();
                (!generator.guided.as_ref().unwrap().borrow().pending_views.is_empty())
                    .then_some(tx)
            })
            .expect("deterministic mixed strategy eventually selects Jev");
        generator.resolve_view(&mut tx, |view| {
            assert_eq!(view.target, target);
            assert_eq!(view.sender, sender);
            let args = balance.abi_decode_input(&view.calldata[4..]).unwrap();
            assert_eq!(args, vec![DynSolValue::Address(sender)]);
            Some(
                balance
                    .abi_encode_output(&[DynSolValue::Uint(U256::from(42), 256)])
                    .unwrap()
                    .into(),
            )
        });
        let args = withdraw.abi_decode_input(&tx.call_details.calldata[4..]).unwrap();
        assert_eq!(args, vec![DynSolValue::Uint(U256::from(42), 256)]);
        assert!(generator.guided.as_ref().unwrap().borrow().pending_views.is_empty());
    }

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
}
