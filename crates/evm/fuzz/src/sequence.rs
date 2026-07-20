//! Pure transaction-sequence generation and mutation primitives.

use crate::{
    BasicTxDetails,
    invariant::FuzzRunIdentifiedContracts,
    strategies::{FuzzState, TxGenerator, generate_msg_value, mutate_param_value},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::U256;
use eyre::{Result, eyre};
use foundry_config::{FuzzCorpusConfig, FuzzCorpusMutationWeights};
use proptest::test_runner::TestRunner;
use rand::{
    Rng,
    distr::{Distribution, weighted::WeightedIndex},
};

/// A neutral borrowed view of one corpus entry.
#[derive(Clone, Copy)]
pub struct CorpusEntryView<'a> {
    transactions: &'a [BasicTxDetails],
    comparisons: &'a [Vec<ComparisonHint>],
}

impl<'a> CorpusEntryView<'a> {
    pub fn new(
        transactions: &'a [BasicTxDetails],
        comparisons: &'a [Vec<ComparisonHint>],
    ) -> Result<Self> {
        if transactions.is_empty() {
            return Err(eyre!("corpus entry has no transactions"));
        }
        if comparisons.len() > transactions.len() {
            return Err(eyre!("corpus entry has more comparison sets than transactions"));
        }
        Ok(Self { transactions, comparisons })
    }
}

#[derive(Clone)]
enum SequenceMode {
    Stateless(Function),
    Invariant(FuzzRunIdentifiedContracts),
}

/// Generates initial sequences and their lazy continuations.
#[derive(Clone)]
pub struct SequenceGenerator {
    tx: TxGenerator,
    state: FuzzState,
    mode: SequenceMode,
    weights: FuzzCorpusMutationWeights,
    mutations: WeightedIndex<u32>,
    arg_mutations: Option<WeightedIndex<u32>>,
    fresh_weight: u32,
    payable_weight: u32,
    has_corpus_dir: bool,
}

/// An initial sequence and the generator used to lazily continue it.
pub struct SequencePlan {
    initial: InitialSequence,
    tx: TxGenerator,
    fresh_weight: u32,
    has_corpus_dir: bool,
    stateless: bool,
    source: Option<usize>,
}

enum InitialSequence {
    Single(BasicTxDetails),
    Multiple(Vec<BasicTxDetails>),
}

impl InitialSequence {
    pub fn into_first(self) -> BasicTxDetails {
        match self {
            Self::Single(tx) => tx,
            Self::Multiple(mut txs) => txs.remove(0),
        }
    }

    pub fn as_slice(&self) -> &[BasicTxDetails] {
        match self {
            Self::Single(tx) => std::slice::from_ref(tx),
            Self::Multiple(txs) => txs,
        }
    }
}

impl SequencePlan {
    pub fn initial(&self) -> &[BasicTxDetails] {
        self.initial.as_slice()
    }
    pub fn into_first(self) -> BasicTxDetails {
        self.initial.into_first()
    }
    pub const fn source(&self) -> Option<usize> {
        self.source
    }
    pub fn next(
        &self,
        runner: &mut TestRunner,
        discarded: bool,
        depth: usize,
    ) -> Result<BasicTxDetails> {
        if self.stateless {
            return Err(eyre!("stateless sequence is limited to one transaction"));
        }
        if !self.has_corpus_dir || discarded {
            return self.tx.next_tx(runner);
        }
        let fresh = self.fresh_weight > 0 && runner.rng().random_ratio(self.fresh_weight, 100);
        if depth >= self.initial.as_slice().len() || fresh {
            self.tx.next_tx(runner)
        } else {
            Ok(self.initial.as_slice()[depth].clone())
        }
    }
}

#[derive(Clone, Copy)]
enum MutationType {
    Splice,
    Repeat,
    Interleave,
    Prefix,
    Suffix,
    Abi,
    Cmp,
}

impl SequenceGenerator {
    pub fn stateless(
        tx: TxGenerator,
        state: FuzzState,
        function: Function,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::new(tx, state, SequenceMode::Stateless(function), config)
    }
    pub fn invariant(
        tx: TxGenerator,
        state: FuzzState,
        targets: FuzzRunIdentifiedContracts,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::new(tx, state, SequenceMode::Invariant(targets), config)
    }
    fn new(
        tx: TxGenerator,
        state: FuzzState,
        mode: SequenceMode,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        let weights = config.mutation_weights.effective();
        if weights.total() > u64::from(u32::MAX) {
            return Err(eyre!(
                "effective mutation weights sum to {}, which exceeds the maximum supported total {}",
                weights.total(),
                u32::MAX
            ));
        }
        let all = [
            weights.mutation_weight_splice,
            weights.mutation_weight_repeat,
            weights.mutation_weight_interleave,
            weights.mutation_weight_prefix,
            weights.mutation_weight_suffix,
            weights.mutation_weight_abi,
            weights.mutation_weight_cmp,
        ];
        let mutations =
            WeightedIndex::new(all).map_err(|e| eyre!("invalid corpus mutation weights: {e}"))?;
        let arg_mutations = if weights.mutation_weight_abi == 0 && weights.mutation_weight_cmp == 0
        {
            None
        } else {
            Some(
                WeightedIndex::new([weights.mutation_weight_abi, weights.mutation_weight_cmp])
                    .map_err(|e| eyre!("invalid argument mutation weights: {e}"))?,
            )
        };
        Ok(Self {
            tx,
            state,
            mode,
            weights,
            mutations,
            arg_mutations,
            fresh_weight: config.corpus_random_sequence_weight.min(100),
            payable_weight: config.payable_value_weight,
            has_corpus_dir: config.corpus_dir.is_some(),
        })
    }

    pub fn start<'a, F>(
        &self,
        runner: &mut TestRunner,
        corpus_len: usize,
        mut entry_at: F,
        coverage: bool,
    ) -> Result<SequencePlan>
    where
        F: FnMut(usize) -> Result<CorpusEntryView<'a>>,
    {
        let (initial, source) = match &self.mode {
            SequenceMode::Stateless(function) => {
                self.start_stateless(runner, corpus_len, &mut entry_at, coverage, function)?
            }
            SequenceMode::Invariant(targets) => {
                self.start_invariant(runner, corpus_len, &mut entry_at, coverage, targets)?
            }
        };
        Ok(SequencePlan {
            initial,
            tx: self.tx.clone(),
            fresh_weight: self.fresh_weight,
            has_corpus_dir: self.has_corpus_dir,
            stateless: matches!(self.mode, SequenceMode::Stateless(_)),
            source,
        })
    }

    fn start_stateless<'a>(
        &self,
        runner: &mut TestRunner,
        corpus_len: usize,
        entry_at: &mut impl FnMut(usize) -> Result<CorpusEntryView<'a>>,
        coverage: bool,
        function: &Function,
    ) -> Result<(InitialSequence, Option<usize>)> {
        if !coverage
            || corpus_len == 0
            || (self.fresh_weight > 0 && runner.rng().random_ratio(self.fresh_weight, 100))
        {
            return Ok((InitialSequence::Single(self.tx.next_tx(runner)?), None));
        }
        let index = runner.rng().random_range(0..corpus_len);
        let entry = entry_at(index)?;
        let mut tx = entry.transactions[0].clone();
        let hints = entry.comparisons.first().map_or(&[][..], Vec::as_slice);
        match self.arg_mutations.as_ref().map(|d| d.sample(runner.rng()) == 1) {
            Some(true)
                if !SequenceMutator::cmp_mutate(&mut tx, function, hints, runner)?
                    && self.weights.mutation_weight_abi > 0
                    && !function.inputs.is_empty() =>
            {
                SequenceMutator::abi_mutate(
                    &mut tx,
                    function,
                    runner,
                    &self.state,
                    self.payable_weight,
                )?
            }
            Some(true) => {}
            Some(false) if self.weights.mutation_weight_abi > 0 && !function.inputs.is_empty() => {
                SequenceMutator::abi_mutate(
                    &mut tx,
                    function,
                    runner,
                    &self.state,
                    self.payable_weight,
                )?
            }
            Some(false) if self.weights.mutation_weight_cmp > 0 => {
                let _ = SequenceMutator::cmp_mutate(&mut tx, function, hints, runner)?;
            }
            None => return Ok((InitialSequence::Single(self.tx.next_tx(runner)?), None)),
            _ => {}
        }
        Ok((InitialSequence::Single(tx), Some(index)))
    }

    fn start_invariant<'a>(
        &self,
        runner: &mut TestRunner,
        corpus_len: usize,
        entry_at: &mut impl FnMut(usize) -> Result<CorpusEntryView<'a>>,
        coverage: bool,
        targets: &FuzzRunIdentifiedContracts,
    ) -> Result<(InitialSequence, Option<usize>)> {
        if !coverage || corpus_len == 0 {
            return Ok((InitialSequence::Multiple(vec![self.tx.next_tx(runner)?]), None));
        }
        let kind = match self.mutations.sample(runner.rng()) {
            0 => MutationType::Splice,
            1 => MutationType::Repeat,
            2 => MutationType::Interleave,
            3 => MutationType::Prefix,
            4 => MutationType::Suffix,
            5 => MutationType::Abi,
            _ => MutationType::Cmp,
        };
        let a = runner.rng().random_range(0..corpus_len);
        let b = runner.rng().random_range(0..corpus_len);
        let primary = entry_at(a)?;
        let secondary = entry_at(b)?;
        let (mut seq, source) = match kind {
            MutationType::Splice => {
                (SequenceMutator::splice(primary.transactions, secondary.transactions, runner), a)
            }
            MutationType::Interleave => (
                SequenceMutator::interleave(primary.transactions, secondary.transactions, runner),
                a,
            ),
            MutationType::Repeat => {
                let i = if runner.rng().random() { a } else { b };
                let entry = if i == a { primary } else { secondary };
                (SequenceMutator::repeat(entry.transactions, runner), i)
            }
            MutationType::Prefix | MutationType::Suffix => {
                let i = if runner.rng().random() { a } else { b };
                let base = if i == a { primary.transactions } else { secondary.transactions };
                let len = if matches!(kind, MutationType::Prefix) {
                    runner.rng().random_range(0..=base.len())
                } else {
                    runner.rng().random_range(0..base.len())
                };
                let mut r = Vec::with_capacity(len);
                for _ in 0..len {
                    r.push(self.tx.next_tx(runner)?)
                }
                (
                    if matches!(kind, MutationType::Prefix) {
                        SequenceMutator::prefix(base, r)
                    } else {
                        SequenceMutator::suffix(base, r)
                    },
                    i,
                )
            }
            MutationType::Abi | MutationType::Cmp => {
                let i = if runner.rng().random() { a } else { b };
                let entry = if i == a { primary } else { secondary };
                let mut seq = entry.transactions.to_vec();
                let fallback = runner.rng().random_range(0..seq.len());
                if matches!(kind, MutationType::Abi) {
                    let tx = &mut seq[fallback];
                    if let (_, Some(f)) = targets.targets().fuzzed_artifacts(tx)
                        && !f.inputs.is_empty()
                    {
                        SequenceMutator::abi_mutate(
                            tx,
                            f,
                            runner,
                            &self.state,
                            self.payable_weight,
                        )?;
                    }
                } else {
                    let candidates =
                        entry.comparisons.iter().enumerate().filter(|(_, h)| !h.is_empty());
                    let count = candidates.clone().count();
                    let mut mutated = false;
                    if count > 0 {
                        let start = runner.rng().random_range(0..count);
                        for (idx, h) in candidates.cycle().skip(start).take(count) {
                            let tx = &mut seq[idx];
                            if let (_, Some(f)) = targets.targets().fuzzed_artifacts(tx) {
                                mutated = SequenceMutator::cmp_mutate(tx, f, h, runner)?;
                                if mutated {
                                    break;
                                }
                            }
                        }
                    }
                    if !mutated && self.weights.mutation_weight_abi > 0 {
                        let tx = &mut seq[fallback];
                        if let (_, Some(f)) = targets.targets().fuzzed_artifacts(tx)
                            && !f.inputs.is_empty()
                        {
                            SequenceMutator::abi_mutate(
                                tx,
                                f,
                                runner,
                                &self.state,
                                self.payable_weight,
                            )?
                        }
                    }
                }
                (seq, i)
            }
        };
        if seq.is_empty() {
            seq.push(self.tx.next_tx(runner)?)
        }
        Ok((InitialSequence::Multiple(seq), Some(source)))
    }
}

/// An EVM comparison observed while executing an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComparisonHint {
    pub lhs: U256,
    pub rhs: U256,
}

/// Pure mutations shared by stateless and invariant sequence producers.
struct SequenceMutator;

impl SequenceMutator {
    fn splice(
        first: &[BasicTxDetails],
        second: &[BasicTxDetails],
        runner: &mut TestRunner,
    ) -> Vec<BasicTxDetails> {
        let rng = runner.rng();
        let start1 = rng.random_range(0..first.len());
        let end1 = rng.random_range(start1..first.len());
        let start2 = rng.random_range(0..second.len());
        let end2 = rng.random_range(start2..second.len());
        first[start1..end1].iter().chain(&second[start2..end2]).cloned().collect()
    }

    fn repeat(sequence: &[BasicTxDetails], runner: &mut TestRunner) -> Vec<BasicTxDetails> {
        let rng = runner.rng();
        let start = rng.random_range(0..sequence.len());
        let end = rng.random_range(start..sequence.len());
        let repeated = sequence[rng.random_range(0..sequence.len())].clone();
        let mut result = Vec::with_capacity(sequence.len());
        result.extend_from_slice(&sequence[..start]);
        result.extend((start..end).map(|_| repeated.clone()));
        result.extend_from_slice(&sequence[end..]);
        result
    }

    fn interleave(
        first: &[BasicTxDetails],
        second: &[BasicTxDetails],
        runner: &mut TestRunner,
    ) -> Vec<BasicTxDetails> {
        first
            .iter()
            .zip(second)
            .map(
                |(first, second)| {
                    if runner.rng().random() { first.clone() } else { second.clone() }
                },
            )
            .collect()
    }

    fn prefix(
        sequence: &[BasicTxDetails],
        mut replacements: Vec<BasicTxDetails>,
    ) -> Vec<BasicTxDetails> {
        replacements.truncate(sequence.len());
        let mut result = sequence.to_vec();
        result[..replacements.len()].clone_from_slice(&replacements);
        result
    }

    fn suffix(
        sequence: &[BasicTxDetails],
        mut replacements: Vec<BasicTxDetails>,
    ) -> Vec<BasicTxDetails> {
        replacements.truncate(sequence.len());
        let mut result = sequence.to_vec();
        let retained = result.len() - replacements.len();
        result[retained..].clone_from_slice(&replacements);
        result
    }

    /// Mutates ABI arguments while retaining transaction metadata and optionally changing value.
    fn abi_mutate(
        tx: &mut BasicTxDetails,
        function: &Function,
        runner: &mut TestRunner,
        state: &FuzzState,
        payable_value_weight: u32,
    ) -> Result<()> {
        if function.inputs.is_empty() || tx.call_details.calldata.len() < 4 {
            return Ok(());
        }
        if function.state_mutability == alloy_json_abi::StateMutability::Payable
            && runner.rng().random_ratio(payable_value_weight.min(100), 100)
        {
            tx.call_details.value = Some(generate_msg_value(runner));
        }
        let mut rounds = runner.rng().random_range(0..=function.inputs.len()).max(1);
        let indices = if function.inputs.len() <= 1 {
            vec![0]
        } else {
            (0..rounds).map(|_| runner.rng().random_range(0..function.inputs.len())).collect()
        };
        let mut inputs = function
            .abi_decode_input(&tx.call_details.calldata[4..])
            .map_err(|err| eyre!("failed to load previous inputs: {err}"))?;
        while rounds > 0 {
            let index = indices[rounds - 1];
            inputs[index] = mutate_param_value(
                &function.inputs[index].selector_type().parse()?,
                inputs[index].clone(),
                runner,
                state,
            );
            rounds -= 1;
        }
        tx.call_details.calldata =
            function.abi_encode_input(&inputs).map_err(|err| eyre!(err.to_string()))?.into();
        Ok(())
    }

    fn cmp_mutate(
        tx: &mut BasicTxDetails,
        function: &Function,
        hints: &[ComparisonHint],
        runner: &mut TestRunner,
    ) -> Result<bool> {
        if hints.is_empty() || tx.call_details.calldata.len() <= 4 {
            return Ok(false);
        }
        let start = runner.rng().random_range(0..hints.len());
        for offset in 0..hints.len() {
            if let Some(calldata) = cmp_mutated_calldata(
                tx.call_details.calldata.as_ref(),
                hints[(start + offset) % hints.len()],
                runner,
            ) && function.abi_decode_input(&calldata[4..]).is_ok()
            {
                tx.call_details.calldata = calldata.into();
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CallDetails,
        invariant::{TargetedContract, TargetedContracts},
        strategies::EvmFuzzState,
    };
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::JsonAbi;
    use alloy_primitives::{Address, Bytes};
    use foundry_config::FuzzDictionaryConfig;
    use proptest::{prelude::Just, strategy::Strategy};
    use revm::database::{CacheDB, EmptyDB};
    use std::path::PathBuf;

    fn sentinel(runner: &mut TestRunner) -> u64 {
        runner.rng().random()
    }

    fn forced_weights(kind: usize) -> FuzzCorpusMutationWeights {
        let mut weights = FuzzCorpusMutationWeights {
            mutation_weight_splice: 0,
            mutation_weight_repeat: 0,
            mutation_weight_interleave: 0,
            mutation_weight_prefix: 0,
            mutation_weight_suffix: 0,
            mutation_weight_abi: 0,
            mutation_weight_cmp: 0,
        };
        match kind {
            0 => weights.mutation_weight_splice = 1,
            1 => weights.mutation_weight_repeat = 1,
            2 => weights.mutation_weight_interleave = 1,
            3 => weights.mutation_weight_prefix = 1,
            4 => weights.mutation_weight_suffix = 1,
            5 => weights.mutation_weight_abi = 1,
            _ => weights.mutation_weight_cmp = 1,
        }
        weights
    }

    fn tx(sender: u8) -> BasicTxDetails {
        BasicTxDetails {
            warp: Some(U256::from(sender)),
            roll: Some(U256::from(sender + 1)),
            sender: Address::with_last_byte(sender),
            call_details: CallDetails {
                target: Address::with_last_byte(10),
                calldata: Bytes::from(vec![sender]),
                value: Some(U256::from(sender)),
            },
        }
    }

    fn state() -> FuzzState {
        EvmFuzzState::new(
            &[],
            &CacheDB::<EmptyDB>::default(),
            FuzzDictionaryConfig::default(),
            None,
        )
        .stateless_worker()
    }

    fn generator_tx(sender: u8) -> TxGenerator {
        TxGenerator::from_strategy(Just(tx(sender)).boxed())
    }

    fn config() -> FuzzCorpusConfig {
        FuzzCorpusConfig { corpus_dir: Some(PathBuf::from("corpus")), ..Default::default() }
    }

    #[test]
    fn prefix_and_suffix_handle_sequence_edges() {
        let sequence = vec![tx(1), tx(2), tx(3)];
        assert_eq!(SequenceMutator::prefix(&sequence, Vec::new())[0].sender, sequence[0].sender);
        assert_eq!(SequenceMutator::suffix(&sequence, Vec::new())[2].sender, sequence[2].sender);
        assert_eq!(
            SequenceMutator::prefix(&sequence, vec![tx(9), tx(8), tx(7)])[2].sender,
            tx(7).sender
        );
        assert_eq!(SequenceMutator::suffix(&sequence, vec![tx(9)])[0].warp, sequence[0].warp);
    }

    #[test]
    fn cmp_mutation_replaces_operand_and_retains_metadata() {
        let function = Function::parse("testCmp(uint256)").unwrap();
        let mut input = tx(7);
        input.call_details.calldata =
            function.abi_encode_input(&[DynSolValue::Uint(U256::from(7), 256)]).unwrap().into();
        let metadata = (input.warp, input.roll, input.sender, input.call_details.value);
        let mut runner = TestRunner::default();

        assert!(
            SequenceMutator::cmp_mutate(
                &mut input,
                &function,
                &[ComparisonHint { lhs: U256::from(7), rhs: U256::from(42) }],
                &mut runner,
            )
            .unwrap()
        );
        let decoded = function.abi_decode_input(&input.call_details.calldata[4..]).unwrap();
        assert_eq!(decoded[0].as_uint().unwrap().0, U256::from(42));
        assert_eq!((input.warp, input.roll, input.sender, input.call_details.value), metadata);
    }

    #[test]
    fn stateless_fresh_and_disabled_mutators_generate_one_transaction() {
        let function = Function::parse("test(uint256)").unwrap();
        let corpus_tx = tx(1);
        let corpus = [CorpusEntryView::new(std::slice::from_ref(&corpus_tx), &[]).unwrap()];
        for disable_mutators in [false, true] {
            let mut config = config();
            config.corpus_random_sequence_weight = if disable_mutators { 0 } else { 100 };
            if disable_mutators {
                config.mutation_weights.mutation_weight_abi = 0;
                config.mutation_weights.mutation_weight_cmp = 0;
            }
            let generator =
                SequenceGenerator::stateless(generator_tx(9), state(), function.clone(), &config)
                    .unwrap();
            let plan = generator
                .start(&mut TestRunner::default(), corpus.len(), |i| Ok(corpus[i]), true)
                .unwrap();
            assert_eq!(plan.initial().len(), 1);
            assert_eq!(plan.initial()[0].sender, tx(9).sender);
            assert_eq!(plan.initial()[0].call_details.value, tx(9).call_details.value);
            assert_eq!(plan.source(), None);
        }
    }

    #[test]
    fn sequence_plan_continues_corpus_or_generates_when_unavailable() {
        let mut config = config();
        config.corpus_random_sequence_weight = 0;
        config.mutation_weights.mutation_weight_splice = 1;
        config.mutation_weights.mutation_weight_repeat = 0;
        config.mutation_weights.mutation_weight_interleave = 0;
        config.mutation_weights.mutation_weight_prefix = 0;
        config.mutation_weights.mutation_weight_suffix = 0;
        config.mutation_weights.mutation_weight_abi = 0;
        config.mutation_weights.mutation_weight_cmp = 0;
        let targets = FuzzRunIdentifiedContracts::new(TargetedContracts::new(), false);
        let generator =
            SequenceGenerator::invariant(generator_tx(9), state(), targets, &config).unwrap();
        let mut runner = TestRunner::default();
        let plan = SequencePlan {
            initial: InitialSequence::Multiple(vec![tx(1), tx(2)]),
            tx: generator.tx,
            fresh_weight: generator.fresh_weight,
            has_corpus_dir: generator.has_corpus_dir,
            stateless: false,
            source: Some(0),
        };

        assert_eq!(plan.next(&mut runner, false, 1).unwrap().sender, tx(2).sender);
        assert_eq!(plan.next(&mut runner, true, 1).unwrap().sender, tx(9).sender);
        assert_eq!(plan.next(&mut runner, false, 2).unwrap().sender, tx(9).sender);
    }

    #[test]
    fn continuation_preserves_gate_draw_order() {
        for (fresh_weight, discarded, depth, expected_sender, draws_gate) in [
            (0, false, 1, 2, false),
            (100, false, 1, 9, true),
            (50, false, 2, 9, true),
            (50, true, 1, 9, false),
        ] {
            let plan = SequencePlan {
                initial: InitialSequence::Multiple(vec![tx(1), tx(2)]),
                tx: generator_tx(9),
                fresh_weight,
                has_corpus_dir: true,
                stateless: false,
                source: Some(0),
            };
            let mut actual = TestRunner::deterministic();
            let mut reference = TestRunner::deterministic();
            if draws_gate {
                let _ = reference.rng().random_ratio(fresh_weight, 100);
            }
            assert_eq!(
                plan.next(&mut actual, discarded, depth).unwrap().sender,
                tx(expected_sender).sender
            );
            assert_eq!(sentinel(&mut actual), sentinel(&mut reference));
        }
    }

    #[test]
    fn forced_invariant_mutations_preserve_old_selection_draws() {
        let sequences = [vec![tx(1), tx(2), tx(3)], vec![tx(4), tx(5), tx(6)]];
        let entries = sequences
            .iter()
            .map(|sequence| CorpusEntryView::new(sequence, &[]).unwrap())
            .collect::<Vec<_>>();
        for kind in 0..7 {
            let mut config = config();
            config.mutation_weights = forced_weights(kind);
            let generator = SequenceGenerator::invariant(
                generator_tx(9),
                state(),
                FuzzRunIdentifiedContracts::new(TargetedContracts::new(), false),
                &config,
            )
            .unwrap();
            let mut actual = TestRunner::deterministic();
            let mut reference = TestRunner::deterministic();

            // The legacy producer selected the mutation, both corpus entries, and then the source
            // before making operation-specific draws. Keep these draws explicit: this test is
            // intended to catch seemingly harmless reordering during further extraction work.
            let distribution = WeightedIndex::new(
                [kind == 0, kind == 1, kind == 2, kind == 3, kind == 4, kind == 5, kind == 6]
                    .map(u32::from),
            )
            .unwrap();
            assert_eq!(distribution.sample(reference.rng()), kind);
            let a = reference.rng().random_range(0..entries.len());
            let b = reference.rng().random_range(0..entries.len());
            let source = match kind {
                0 | 2 => a,
                _ => {
                    if reference.rng().random() {
                        a
                    } else {
                        b
                    }
                }
            };
            // ABI/CMP always selected a fallback transaction after choosing the source.
            if kind >= 5 {
                let _ = reference.rng().random_range(0..entries[source].transactions.len());
            }

            let plan =
                generator.start(&mut actual, entries.len(), |i| Ok(entries[i]), true).unwrap();
            assert_eq!(plan.source(), Some(source), "mutation family {kind}");
            // For argument mutation there are no matching artifacts, so the fallback index is the
            // final draw. Structural mutations make additional operation-specific draws and are
            // covered independently by the mutator unit tests.
            if kind >= 5 {
                assert_eq!(
                    sentinel(&mut actual),
                    sentinel(&mut reference),
                    "mutation family {kind}"
                );
            }
        }
    }

    #[test]
    fn stateless_plan_never_exposes_continuation() {
        let generator = SequenceGenerator::stateless(
            generator_tx(9),
            state(),
            Function::parse("test()").unwrap(),
            &config(),
        )
        .unwrap();
        let plan = generator
            .start(&mut TestRunner::deterministic(), 0, |_| unreachable!(), false)
            .unwrap();
        assert!(plan.next(&mut TestRunner::deterministic(), false, 0).is_err());
    }

    #[test]
    fn overflowing_effective_mutation_weight_is_rejected() {
        let mut config = config();
        config.mutation_weights.mutation_weight_splice = u32::MAX;
        config.mutation_weights.mutation_weight_repeat = 1;
        assert!(
            SequenceGenerator::stateless(
                generator_tx(1),
                state(),
                Function::parse("test()").unwrap(),
                &config,
            )
            .is_err()
        );
    }

    #[test]
    fn invariant_cmp_only_does_not_fallback_to_abi() {
        let target = Address::with_last_byte(42);
        let function = Function::parse("test(uint256)").unwrap();
        let mut abi = JsonAbi::new();
        abi.functions.entry(function.name.clone()).or_default().push(function.clone());
        let mut contracts = TargetedContracts::new();
        contracts.insert(target, TargetedContract::new("Target".into(), abi));
        let mut original = tx(1);
        original.call_details.target = target;
        original.call_details.calldata =
            function.abi_encode_input(&[DynSolValue::Uint(U256::from(7), 256)]).unwrap().into();
        let sequence = [original.clone()];
        let corpus = [CorpusEntryView::new(&sequence, &[]).unwrap()];
        let mut config = config();
        config.mutation_weights = FuzzCorpusMutationWeights {
            mutation_weight_splice: 0,
            mutation_weight_repeat: 0,
            mutation_weight_interleave: 0,
            mutation_weight_prefix: 0,
            mutation_weight_suffix: 0,
            mutation_weight_abi: 0,
            mutation_weight_cmp: 1,
        };
        let generator = SequenceGenerator::invariant(
            generator_tx(9),
            state(),
            FuzzRunIdentifiedContracts::new(contracts, false),
            &config,
        )
        .unwrap();
        let plan = generator
            .start(&mut TestRunner::default(), corpus.len(), |i| Ok(corpus[i]), true)
            .unwrap();
        assert_eq!(plan.initial()[0].call_details.calldata, original.call_details.calldata);
    }

    #[test]
    fn malformed_corpus_views_are_rejected_without_panicking() {
        assert!(CorpusEntryView::new(&[], &[]).is_err());
        let sequence = [tx(1)];
        let comparisons = [Vec::new(), Vec::new()];
        assert!(CorpusEntryView::new(&sequence, &comparisons).is_err());

        let generator = SequenceGenerator::stateless(
            generator_tx(9),
            state(),
            Function::parse("test(uint256)").unwrap(),
            &config(),
        )
        .unwrap();
        let result = generator.start(
            &mut TestRunner::deterministic(),
            1,
            |_| CorpusEntryView::new(&[], &[]),
            true,
        );
        assert!(result.is_err());
    }

    #[test]
    fn start_accesses_at_most_two_corpus_entries() {
        let mut config = config();
        config.mutation_weights.mutation_weight_splice = 1;
        config.mutation_weights.mutation_weight_repeat = 0;
        config.mutation_weights.mutation_weight_interleave = 0;
        config.mutation_weights.mutation_weight_prefix = 0;
        config.mutation_weights.mutation_weight_suffix = 0;
        config.mutation_weights.mutation_weight_abi = 0;
        config.mutation_weights.mutation_weight_cmp = 0;
        let generator = SequenceGenerator::invariant(
            generator_tx(9),
            state(),
            FuzzRunIdentifiedContracts::new(TargetedContracts::new(), false),
            &config,
        )
        .unwrap();
        let sequence = [tx(1)];
        let mut accesses = 0;
        generator
            .start(
                &mut TestRunner::deterministic(),
                10_000,
                |_| {
                    accesses += 1;
                    CorpusEntryView::new(&sequence, &[])
                },
                true,
            )
            .unwrap();
        assert_eq!(accesses, 2);
    }
}

fn cmp_mutated_calldata(
    calldata: &[u8],
    hint: ComparisonHint,
    runner: &mut TestRunner,
) -> Option<Vec<u8>> {
    const WIDTHS: [usize; 6] = [32, 16, 8, 4, 2, 1];
    let lhs = hint.lhs.to_be_bytes::<32>();
    let rhs = hint.rhs.to_be_bytes::<32>();
    let start = runner.rng().random_range(0..WIDTHS.len());
    for offset in 0..WIDTHS.len() {
        let width = WIDTHS[(start + offset) % WIDTHS.len()];
        let lhs = &lhs[32 - width..];
        let rhs = &rhs[32 - width..];
        if lhs == rhs {
            continue;
        }
        let pairs =
            if runner.rng().random() { [(lhs, rhs), (rhs, lhs)] } else { [(rhs, lhs), (lhs, rhs)] };
        for (pattern, replacement) in pairs {
            if let Some(mutated) = replace_operand(calldata, pattern, replacement, runner) {
                return Some(mutated);
            }
        }
    }
    None
}

fn replace_operand(
    calldata: &[u8],
    pattern: &[u8],
    replacement: &[u8],
    runner: &mut TestRunner,
) -> Option<Vec<u8>> {
    const SELECTOR_LEN: usize = 4;
    if pattern.is_empty()
        || pattern.len() != replacement.len()
        || calldata.len() < SELECTOR_LEN + pattern.len()
        || (pattern.len() < 32 && pattern.iter().all(|byte| *byte == 0))
    {
        return None;
    }
    let search_len = calldata.len() - SELECTOR_LEN - pattern.len() + 1;
    let start = runner.rng().random_range(0..search_len);
    for offset in 0..search_len {
        let index = SELECTOR_LEN + ((start + offset) % search_len);
        if &calldata[index..index + pattern.len()] == pattern {
            let mut mutated = calldata.to_vec();
            mutated[index..index + replacement.len()].copy_from_slice(replacement);
            return Some(mutated);
        }
    }
    None
}
