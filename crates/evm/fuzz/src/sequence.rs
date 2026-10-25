//! Pure transaction-sequence generation and mutation primitives.

use crate::{
    BasicTxDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, TargetedContracts},
    strategies::{
        FuzzState, TxGenerator, constrain_enum_value, generate_msg_value, mutate_param_value,
    },
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt, abi::AbiDecoderConfig};
use alloy_json_abi::Function;
use alloy_primitives::{
    U256,
    map::{HashMap, HashSet},
};
use eyre::{Result, eyre};
use foundry_config::{FuzzCorpusConfig, FuzzCorpusMutationWeights};
use proptest::test_runner::TestRunner;
use rand::{
    Rng,
    distr::{Distribution, weighted::WeightedIndex},
};
use std::borrow::Cow;

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
    fixtures: FuzzFixtures,
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
    Reuse,
    Swap,
    Attacker,
}

impl SequenceGenerator {
    pub fn stateless(
        tx: TxGenerator,
        state: FuzzState,
        function: Function,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::stateless_with_fixtures(tx, state, FuzzFixtures::default(), function, config)
    }
    pub fn stateless_with_fixtures(
        tx: TxGenerator,
        state: FuzzState,
        fixtures: FuzzFixtures,
        function: Function,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::new(tx, state, fixtures, SequenceMode::Stateless(function), config)
    }
    pub fn invariant(
        tx: TxGenerator,
        state: FuzzState,
        targets: FuzzRunIdentifiedContracts,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::invariant_with_fixtures(tx, state, FuzzFixtures::default(), targets, config)
    }
    pub fn invariant_with_fixtures(
        tx: TxGenerator,
        state: FuzzState,
        fixtures: FuzzFixtures,
        targets: FuzzRunIdentifiedContracts,
        config: &FuzzCorpusConfig,
    ) -> Result<Self> {
        Self::new(tx, state, fixtures, SequenceMode::Invariant(targets), config)
    }
    fn new(
        tx: TxGenerator,
        state: FuzzState,
        fixtures: FuzzFixtures,
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
            weights.mutation_weight_reuse,
            weights.mutation_weight_swap,
            weights.mutation_weight_attacker,
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
            fixtures,
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
                if !SequenceMutator::cmp_mutate(
                    &mut tx,
                    function,
                    hints,
                    runner,
                    &self.fixtures,
                )? && self.weights.mutation_weight_abi > 0
                    && !function.inputs.is_empty() =>
            {
                SequenceMutator::abi_mutate(
                    &mut tx,
                    function,
                    runner,
                    &self.state,
                    &self.fixtures,
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
                    &self.fixtures,
                    self.payable_weight,
                )?
            }
            Some(false) if self.weights.mutation_weight_cmp > 0 => {
                let _ =
                    SequenceMutator::cmp_mutate(&mut tx, function, hints, runner, &self.fixtures)?;
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
            6 => MutationType::Cmp,
            7 => MutationType::Reuse,
            8 => MutationType::Swap,
            _ => MutationType::Attacker,
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
            MutationType::Abi
            | MutationType::Cmp
            | MutationType::Reuse
            | MutationType::Swap
            | MutationType::Attacker => {
                let i = if runner.rng().random() { a } else { b };
                let entry = if i == a { primary } else { secondary };
                let mut seq = entry.transactions.to_vec();
                let fallback = runner.rng().random_range(0..seq.len());
                let targets = targets.targets();
                let mutated = match kind {
                    MutationType::Cmp => {
                        let candidates =
                            entry.comparisons.iter().enumerate().filter(|(_, h)| !h.is_empty());
                        let count = candidates.clone().count();
                        let mut mutated = false;
                        if count > 0 {
                            let start = runner.rng().random_range(0..count);
                            for (idx, h) in candidates.cycle().skip(start).take(count) {
                                let tx = &mut seq[idx];
                                if let (_, Some(f)) = targets.fuzzed_artifacts(tx) {
                                    mutated = SequenceMutator::cmp_mutate(
                                        tx,
                                        f,
                                        h,
                                        runner,
                                        &self.fixtures,
                                    )?;
                                    if mutated {
                                        break;
                                    }
                                }
                            }
                        }
                        mutated
                    }
                    MutationType::Reuse => TypedArgs::decode(&seq, &targets).reuse(
                        &mut seq,
                        runner,
                        &self.fixtures,
                        |_| true,
                    )?,
                    MutationType::Swap => {
                        TypedArgs::decode(&seq, &targets).swap(&mut seq, runner, &self.fixtures)?
                    }
                    MutationType::Attacker => {
                        SequenceMutator::attacker(&mut seq, &targets, runner, &self.fixtures)?
                    }
                    _ => false,
                };
                // ABI mutation also runs when a comparison or typed mutation does not apply, so the
                // selected entry is not replayed unchanged.
                if !mutated && self.weights.mutation_weight_abi > 0 {
                    let tx = &mut seq[fallback];
                    if let (_, Some(f)) = targets.fuzzed_artifacts(tx)
                        && !f.inputs.is_empty()
                    {
                        SequenceMutator::abi_mutate(
                            tx,
                            f,
                            runner,
                            &self.state,
                            &self.fixtures,
                            self.payable_weight,
                        )?
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComparisonHint {
    pub lhs: U256,
    pub rhs: U256,
}

impl ComparisonHint {
    /// Collects the distinct `hints` that a cmp mutation can apply to `calldata`, in first-seen
    /// order.
    ///
    /// Cmp mutations only rewrite the calldata stored with the hints, so other hints can never
    /// produce a mutation.
    pub fn applicable(calldata: &[u8], hints: impl IntoIterator<Item = Self>) -> Vec<Self> {
        let mut seen = HashSet::<Self>::default();
        hints.into_iter().filter(|hint| seen.insert(*hint) && hint.applies_to(calldata)).collect()
    }

    /// Returns whether a cmp mutation can replace one of this comparison's operands in
    /// `calldata`.
    pub fn applies_to(&self, calldata: &[u8]) -> bool {
        let lhs = self.lhs.to_be_bytes::<32>();
        let rhs = self.rhs.to_be_bytes::<32>();
        CMP_WIDTHS.iter().any(|&width| {
            let lhs = &lhs[32 - width..];
            let rhs = &rhs[32 - width..];
            lhs != rhs && (contains_operand(calldata, lhs) || contains_operand(calldata, rhs))
        })
    }
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
        first[start1..=end1].iter().chain(&second[start2..=end2]).cloned().collect()
    }

    fn repeat(sequence: &[BasicTxDetails], runner: &mut TestRunner) -> Vec<BasicTxDetails> {
        let rng = runner.rng();
        let start = rng.random_range(0..sequence.len());
        let end = rng.random_range(start..=sequence.len());
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
        fixtures: &FuzzFixtures,
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
        let inputs = inputs
            .into_iter()
            .zip(&function.inputs)
            .map(|(value, input)| constrain_enum_value(value, input, fixtures))
            .collect::<Vec<_>>();
        tx.call_details.calldata =
            function.abi_encode_input(&inputs).map_err(|err| eyre!(err.to_string()))?.into();
        Ok(())
    }

    fn cmp_mutate(
        tx: &mut BasicTxDetails,
        function: &Function,
        hints: &[ComparisonHint],
        runner: &mut TestRunner,
        fixtures: &FuzzFixtures,
    ) -> Result<bool> {
        if hints.is_empty() || tx.call_details.calldata.len() <= 4 {
            return Ok(false);
        }
        let config = AbiDecoderConfig::new().validate_words(true);
        let start = runner.rng().random_range(0..hints.len());
        for offset in 0..hints.len() {
            if let Some(calldata) = cmp_mutated_calldata(
                tx.call_details.calldata.as_ref(),
                hints[(start + offset) % hints.len()],
                runner,
            ) && let Ok(inputs) = function.abi_decode_input_with_config(&calldata[4..], config)
                && inputs.iter().zip(&function.inputs).all(|(value, input)| {
                    constrain_enum_value(value.clone(), input, fixtures) == *value
                })
            {
                tx.call_details.calldata = calldata.into();
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Resends a call from another sender of the sequence, or reuses a `bytes` argument of one
    /// call in another call. Tries the other variant if the drawn one does not apply.
    fn attacker(
        sequence: &mut Vec<BasicTxDetails>,
        targets: &TargetedContracts,
        runner: &mut TestRunner,
        fixtures: &FuzzFixtures,
    ) -> Result<bool> {
        let resend_first = runner.rng().random_bool(0.5);
        for resend in [resend_first, !resend_first] {
            let mutated = if resend {
                Self::resend(sequence, runner)
            } else {
                TypedArgs::decode(sequence, targets)
                    .reuse(sequence, runner, fixtures, |ty| ty == "bytes")?
            };
            if mutated {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Inserts a copy of a call, sent by another sender of the sequence, after the original
    /// call. The copy does not advance time or block number.
    fn resend(sequence: &mut Vec<BasicTxDetails>, runner: &mut TestRunner) -> bool {
        let source = runner.rng().random_range(0..sequence.len());
        let original = sequence[source].sender;
        let senders = sequence.iter().map(|tx| tx.sender).filter(|s| *s != original);
        let count = senders.clone().count();
        if count == 0 {
            return false;
        }
        let sender = senders.clone().nth(runner.rng().random_range(0..count)).unwrap();
        let mut copy = sequence[source].clone();
        copy.sender = sender;
        copy.warp = None;
        copy.roll = None;
        let position = runner.rng().random_range(source + 1..=sequence.len());
        sequence.insert(position, copy);
        true
    }
}

/// Maximum number of draws a typed mutation makes before it gives up on equal values.
const TYPED_MUTATION_ATTEMPTS: usize = 8;

/// The position of one top-level argument in a sequence, as `(call, argument)`.
type ArgSlot = (usize, usize);

/// The top-level arguments of the calls in a sequence, grouped by ABI type.
struct TypedArgs<'a> {
    /// The target function and decoded arguments of each call, if the call decodes.
    calls: Vec<Option<(&'a Function, Vec<DynSolValue>)>>,
    /// The ABI type and argument slots of each argument group.
    groups: Vec<(Cow<'a, str>, Vec<ArgSlot>)>,
}

impl<'a> TypedArgs<'a> {
    fn decode(sequence: &[BasicTxDetails], targets: &'a TargetedContracts) -> Self {
        let mut index = HashMap::<Cow<'a, str>, usize>::default();
        let mut groups = Vec::<(Cow<'a, str>, Vec<_>)>::new();
        let calls = sequence
            .iter()
            .enumerate()
            .map(|(call, tx)| {
                let (_, Some(function)) = targets.fuzzed_artifacts(tx) else { return None };
                let values = function.abi_decode_input(tx.call_details.calldata.get(4..)?).ok()?;
                for (arg, input) in function.inputs.iter().enumerate() {
                    let ty = input.selector_type();
                    let group = *index.entry(ty.clone()).or_insert_with(|| {
                        groups.push((ty, Vec::new()));
                        groups.len() - 1
                    });
                    groups[group].1.push((call, arg));
                }
                Some((function, values))
            })
            .collect();
        Self { calls, groups }
    }

    fn value(&self, (call, arg): ArgSlot) -> &DynSolValue {
        &self.calls[call].as_ref().expect("slots only refer to decoded calls").1[arg]
    }

    /// Picks a random slot of a group accepted by `filter`, and a random other slot of the same
    /// group accepted by `peer`.
    fn draw(
        &self,
        runner: &mut TestRunner,
        filter: impl Fn(&str) -> bool,
        peer: impl Fn(ArgSlot, ArgSlot) -> bool,
    ) -> Option<(ArgSlot, ArgSlot)> {
        let candidates = self
            .groups
            .iter()
            .filter(|(ty, slots)| slots.len() > 1 && filter(ty))
            .flat_map(|(_, slots)| slots.iter().map(move |&slot| (slots, slot)))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        for _ in 0..TYPED_MUTATION_ATTEMPTS {
            let (slots, first) = candidates[runner.rng().random_range(0..candidates.len())];
            let peers = slots.iter().filter(|&&slot| slot != first && peer(first, slot));
            let count = peers.clone().count();
            if count == 0 {
                continue;
            }
            let second = *peers.clone().nth(runner.rng().random_range(0..count)).unwrap();
            if self.value(first) != self.value(second) {
                return Some((first, second));
            }
        }
        None
    }

    /// Replaces an argument with a same-typed argument of another call.
    fn reuse(
        mut self,
        sequence: &mut [BasicTxDetails],
        runner: &mut TestRunner,
        fixtures: &FuzzFixtures,
        filter: impl Fn(&str) -> bool,
    ) -> Result<bool> {
        let Some((target, source)) = self.draw(runner, filter, |a, b| a.0 != b.0) else {
            return Ok(false);
        };
        let value = self.value(source).clone();
        self.set(target, value);
        self.encode(sequence, target.0, fixtures)?;
        Ok(true)
    }

    /// Exchanges two same-typed arguments, within one call or across calls.
    fn swap(
        mut self,
        sequence: &mut [BasicTxDetails],
        runner: &mut TestRunner,
        fixtures: &FuzzFixtures,
    ) -> Result<bool> {
        let Some((first, second)) = self.draw(runner, |_| true, |_, _| true) else {
            return Ok(false);
        };
        let first_value = self.value(first).clone();
        let second_value = self.value(second).clone();
        self.set(first, second_value);
        self.set(second, first_value);
        self.encode(sequence, first.0, fixtures)?;
        if second.0 != first.0 {
            self.encode(sequence, second.0, fixtures)?;
        }
        Ok(true)
    }

    fn set(&mut self, (call, arg): ArgSlot, value: DynSolValue) {
        self.calls[call].as_mut().expect("slots only refer to decoded calls").1[arg] = value;
    }

    /// Writes the arguments of `call` back to its calldata.
    fn encode(
        &self,
        sequence: &mut [BasicTxDetails],
        call: usize,
        fixtures: &FuzzFixtures,
    ) -> Result<()> {
        let (function, values) =
            self.calls[call].as_ref().expect("slots only refer to decoded calls");
        let values = values
            .iter()
            .zip(&function.inputs)
            .map(|(value, input)| constrain_enum_value(value.clone(), input, fixtures))
            .collect::<Vec<_>>();
        sequence[call].call_details.calldata =
            function.abi_encode_input(&values).map_err(|err| eyre!(err.to_string()))?.into();
        Ok(())
    }
}

/// Operand widths, in bytes, that cmp mutations try to replace.
const CMP_WIDTHS: [usize; 6] = [32, 16, 8, 4, 2, 1];

fn cmp_mutated_calldata(
    calldata: &[u8],
    hint: ComparisonHint,
    runner: &mut TestRunner,
) -> Option<Vec<u8>> {
    let lhs = hint.lhs.to_be_bytes::<32>();
    let rhs = hint.rhs.to_be_bytes::<32>();
    let start = runner.rng().random_range(0..CMP_WIDTHS.len());
    for offset in 0..CMP_WIDTHS.len() {
        let width = CMP_WIDTHS[(start + offset) % CMP_WIDTHS.len()];
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

/// Length of the function selector that cmp mutations never rewrite.
const SELECTOR_LEN: usize = 4;

/// Returns whether `pattern` can be searched for in `calldata` arguments.
fn is_replaceable_operand(calldata: &[u8], pattern: &[u8]) -> bool {
    !pattern.is_empty()
        && calldata.len() >= SELECTOR_LEN + pattern.len()
        && (pattern.len() == 32 || pattern.iter().any(|byte| *byte != 0))
}

/// Returns whether `pattern` occurs in `calldata` arguments.
fn contains_operand(calldata: &[u8], pattern: &[u8]) -> bool {
    is_replaceable_operand(calldata, pattern)
        && calldata[SELECTOR_LEN..].windows(pattern.len()).any(|window| window == pattern)
}

fn replace_operand(
    calldata: &[u8],
    pattern: &[u8],
    replacement: &[u8],
    runner: &mut TestRunner,
) -> Option<Vec<u8>> {
    if pattern.len() != replacement.len() || !is_replaceable_operand(calldata, pattern) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallDetails, invariant::TargetedContract, strategies::EvmFuzzState};
    use alloy_json_abi::JsonAbi;
    use alloy_primitives::{Address, Bytes, I256};
    use foundry_config::FuzzDictionaryConfig;
    use proptest::{prelude::Just, strategy::Strategy};
    use revm::database::InMemoryDB;
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
            mutation_weight_reuse: 0,
            mutation_weight_swap: 0,
            mutation_weight_attacker: 0,
        };
        match kind {
            0 => weights.mutation_weight_splice = 1,
            1 => weights.mutation_weight_repeat = 1,
            2 => weights.mutation_weight_interleave = 1,
            3 => weights.mutation_weight_prefix = 1,
            4 => weights.mutation_weight_suffix = 1,
            5 => weights.mutation_weight_abi = 1,
            6 => weights.mutation_weight_cmp = 1,
            7 => weights.mutation_weight_reuse = 1,
            8 => weights.mutation_weight_swap = 1,
            _ => weights.mutation_weight_attacker = 1,
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
        EvmFuzzState::new(&[], &InMemoryDB::default(), FuzzDictionaryConfig::default(), None)
            .stateless_worker()
    }

    fn generator_tx(sender: u8) -> TxGenerator {
        TxGenerator::from_strategy(Just(tx(sender)).boxed())
    }

    fn config() -> FuzzCorpusConfig {
        FuzzCorpusConfig { corpus_dir: Some(PathBuf::from("corpus")), ..Default::default() }
    }

    /// Targets with `signatures` on one contract at `TYPED_TARGET`.
    fn typed_targets(signatures: &[&str]) -> TargetedContracts {
        let mut abi = JsonAbi::new();
        for signature in signatures {
            let function = Function::parse(signature).unwrap();
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }
        let mut contracts = TargetedContracts::new();
        contracts.insert(TYPED_TARGET, TargetedContract::new("Target".into(), abi));
        contracts
    }

    const TYPED_TARGET: Address = Address::with_last_byte(42);

    fn typed_call(sender: u8, signature: &str, args: &[DynSolValue]) -> BasicTxDetails {
        let mut call = tx(sender);
        call.call_details.target = TYPED_TARGET;
        call.call_details.calldata =
            Function::parse(signature).unwrap().abi_encode_input(args).unwrap().into();
        call
    }

    fn decoded(signature: &str, call: &BasicTxDetails) -> Vec<DynSolValue> {
        Function::parse(signature)
            .unwrap()
            .abi_decode_input(&call.call_details.calldata[4..])
            .unwrap()
    }

    fn uint(value: u64) -> DynSolValue {
        DynSolValue::Uint(U256::from(value), 256)
    }

    fn address(byte: u8) -> DynSolValue {
        DynSolValue::Address(Address::with_last_byte(byte))
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
    fn splice_retains_singleton_entries() {
        let mut runner = TestRunner::deterministic();
        let result = SequenceMutator::splice(&[tx(1)], &[tx(2)], &mut runner);

        assert_eq!(
            result.iter().map(|tx| tx.sender).collect::<Vec<_>>(),
            [tx(1).sender, tx(2).sender]
        );
    }

    #[test]
    fn splice_can_select_final_transactions() {
        let first = [tx(1), tx(2)];
        let second = [tx(3), tx(4)];
        let mut runner = TestRunner::deterministic();
        let mut selected_first_final = false;
        let mut selected_second_final = false;

        for _ in 0..1000 {
            let result = SequenceMutator::splice(&first, &second, &mut runner);
            selected_first_final |= result.iter().any(|tx| tx.sender == first[1].sender);
            selected_second_final |= result.iter().any(|tx| tx.sender == second[1].sender);
        }

        assert!(selected_first_final, "splice never selected the first entry's final transaction");
        assert!(
            selected_second_final,
            "splice never selected the second entry's final transaction"
        );
    }

    #[test]
    fn repeat_can_replace_final_transaction() {
        let sequence = [tx(1), tx(2)];
        let mut runner = TestRunner::deterministic();
        let mut replaced_final = false;

        for _ in 0..1000 {
            let result = SequenceMutator::repeat(&sequence, &mut runner);
            replaced_final |= result[1].sender != sequence[1].sender;
        }

        assert!(replaced_final, "repeat never replaced the final transaction");
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
                &FuzzFixtures::default(),
            )
            .unwrap()
        );
        let decoded = function.abi_decode_input(&input.call_details.calldata[4..]).unwrap();
        assert_eq!(decoded[0].as_uint().unwrap().0, U256::from(42));
        assert_eq!((input.warp, input.roll, input.sender, input.call_details.value), metadata);
    }

    #[test]
    fn cmp_mutation_rejects_dirty_value_words() {
        let mut runner = TestRunner::deterministic();
        for (signature, value, lhs, rhs) in [
            ("test(bool)", DynSolValue::Bool(true), U256::from(1), U256::from(4)),
            ("test(bool)", DynSolValue::Bool(false), U256::ZERO, U256::from(4)),
            (
                "test(bool[2])",
                DynSolValue::FixedArray(vec![DynSolValue::Bool(false); 2]),
                U256::ZERO,
                U256::from(4),
            ),
            ("test(uint8)", DynSolValue::Uint(U256::from(7), 8), U256::from(7), U256::from(0x107)),
            (
                "test(address)",
                DynSolValue::Address(Address::with_last_byte(1)),
                U256::from(1),
                (U256::from(1) << 160) | U256::from(1),
            ),
        ] {
            let function = Function::parse(signature).unwrap();
            let original = function.abi_encode_input(&[value]).unwrap();
            let mut input = tx(7);
            input.call_details.calldata = original.clone().into();
            for _ in 0..100 {
                assert!(
                    !SequenceMutator::cmp_mutate(
                        &mut input,
                        &function,
                        &[ComparisonHint { lhs, rhs }],
                        &mut runner,
                        &FuzzFixtures::default(),
                    )
                    .unwrap(),
                    "{signature}"
                );
                assert_eq!(input.call_details.calldata.as_ref(), original);
            }
        }
    }

    #[test]
    fn cmp_mutation_accepts_values_solidity_accepts() {
        let mut runner = TestRunner::deterministic();
        let encode = |signature, value| {
            Function::parse(signature).unwrap().abi_encode_input(&[value]).unwrap()
        };
        let replace_byte = |mut calldata: Vec<u8>, index: usize, byte| {
            calldata[index] = byte;
            calldata
        };
        let bytes = encode("test(bytes)", DynSolValue::Bytes(vec![0xaa; 40]));
        let string = encode("test(string)", DynSolValue::String("abcd".into()));
        for (signature, original, lhs, rhs, expected) in [
            (
                "test(bool)",
                encode("test(bool)", DynSolValue::Bool(false)),
                U256::ZERO,
                U256::from(1),
                encode("test(bool)", DynSolValue::Bool(true)),
            ),
            (
                "test(int8)",
                encode("test(int8)", DynSolValue::Int(I256::ZERO, 8)),
                U256::ZERO,
                U256::MAX,
                encode("test(int8)", DynSolValue::Int(I256::MINUS_ONE, 8)),
            ),
            // Shortened `bytes` length; the stale payload stays in the padding.
            (
                "test(bytes)",
                bytes.clone(),
                U256::from(40),
                U256::from(5),
                replace_byte(bytes, 67, 5),
            ),
            // Non-UTF-8 `string` payload.
            (
                "test(string)",
                string.clone(),
                U256::from(0x61),
                U256::from(0xff),
                replace_byte(string, 68, 0xff),
            ),
        ] {
            let function = Function::parse(signature).unwrap();
            let mut input = tx(7);
            input.call_details.calldata = original.into();
            assert!(
                SequenceMutator::cmp_mutate(
                    &mut input,
                    &function,
                    &[ComparisonHint { lhs, rhs }],
                    &mut runner,
                    &FuzzFixtures::default(),
                )
                .unwrap(),
                "{signature}"
            );
            assert_eq!(input.call_details.calldata.as_ref(), expected, "{signature}");
        }
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
            mutation_weight_reuse: 0,
            mutation_weight_swap: 0,
            mutation_weight_attacker: 0,
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

    #[test]
    fn applicable_hints_dedup_and_drop_operands_missing_from_calldata() {
        let function = Function::parse("testCmp(uint256)").unwrap();
        let calldata = function.abi_encode_input(&[DynSolValue::Uint(U256::from(7), 256)]).unwrap();
        let hint =
            |lhs: u64, rhs: u64| ComparisonHint { lhs: U256::from(lhs), rhs: U256::from(rhs) };

        let hints = [hint(7, 42), hint(1000, 2000), hint(7, 42), hint(5, 5), hint(42, 7)];
        let applicable = ComparisonHint::applicable(&calldata, hints);
        assert_eq!(applicable, [hint(7, 42), hint(42, 7)]);
        for hint in hints {
            let mut input = tx(7);
            input.call_details.calldata = calldata.clone().into();
            let mutated = SequenceMutator::cmp_mutate(
                &mut input,
                &function,
                &[hint],
                &mut TestRunner::default(),
                &FuzzFixtures::default(),
            )
            .unwrap();
            assert_eq!(mutated, applicable.contains(&hint));
        }
    }

    #[test]
    fn reuse_copies_same_typed_argument_from_another_call() {
        let (mint, burn) = ("mint(uint256)", "burn(uint256,address)");
        let targets = typed_targets(&[mint, burn]);
        let original =
            vec![typed_call(1, mint, &[uint(5)]), typed_call(2, burn, &[uint(9), address(3)])];
        let mut runner = TestRunner::deterministic();
        let (mut copied_forward, mut copied_backward) = (false, false);
        for _ in 0..100 {
            let mut sequence = original.clone();
            assert!(
                TypedArgs::decode(&sequence, &targets)
                    .reuse(&mut sequence, &mut runner, &FuzzFixtures::default(), |_| true)
                    .unwrap()
            );
            let minted = decoded(mint, &sequence[0]);
            let burned = decoded(burn, &sequence[1]);
            copied_forward |= burned == [uint(5), address(3)] && minted == [uint(5)];
            copied_backward |= minted == [uint(9)] && burned == [uint(9), address(3)];
            assert!(minted == [uint(9)] || burned[0] == uint(5));
            for (mutated, original) in sequence.iter().zip(&original) {
                assert_eq!(
                    (mutated.sender, mutated.warp, mutated.roll, mutated.call_details.value),
                    (original.sender, original.warp, original.roll, original.call_details.value)
                );
            }
        }
        assert!(copied_forward && copied_backward);
    }

    #[test]
    fn swap_exchanges_same_typed_arguments_within_one_call() {
        let transfer = "transfer(address,address,uint256)";
        let targets = typed_targets(&[transfer]);
        let mut sequence = vec![typed_call(1, transfer, &[address(1), address(2), uint(7)])];
        assert!(
            TypedArgs::decode(&sequence, &targets)
                .swap(&mut sequence, &mut TestRunner::deterministic(), &FuzzFixtures::default())
                .unwrap()
        );
        assert_eq!(decoded(transfer, &sequence[0]), [address(2), address(1), uint(7)]);
    }

    #[test]
    fn typed_mutations_skip_equal_and_unmatched_arguments() {
        let (set, call) = ("set(uint256)", "call(address)");
        let targets = typed_targets(&[set, call]);
        let mut runner = TestRunner::deterministic();
        for original in [
            vec![typed_call(1, set, &[uint(1)]), typed_call(2, call, &[address(1)])],
            vec![typed_call(1, set, &[uint(1)]), typed_call(2, set, &[uint(1)])],
            // Calls that do not decode against a target function are ignored.
            vec![typed_call(1, set, &[uint(1)]), tx(2)],
        ] {
            let mut sequence = original.clone();
            assert!(
                !TypedArgs::decode(&sequence, &targets)
                    .reuse(&mut sequence, &mut runner, &FuzzFixtures::default(), |_| true)
                    .unwrap()
            );
            assert!(
                !TypedArgs::decode(&sequence, &targets)
                    .swap(&mut sequence, &mut runner, &FuzzFixtures::default())
                    .unwrap()
            );
            assert_eq!(sequence, original);
        }
    }

    #[test]
    fn attacker_resends_call_from_another_sender() {
        let mut runner = TestRunner::deterministic();
        for _ in 0..100 {
            let mut sequence = vec![tx(1), tx(2), tx(3)];
            assert!(SequenceMutator::resend(&mut sequence, &mut runner));
            assert_eq!(sequence.len(), 4);
            let (position, copy) = sequence
                .iter()
                .enumerate()
                .find(|(_, tx)| tx.warp.is_none())
                .expect("resend inserts a copy without time advance");
            let source = sequence
                .iter()
                .position(|tx| tx.call_details == copy.call_details && tx.warp.is_some())
                .unwrap();
            assert!(source < position);
            assert_ne!(copy.sender, sequence[source].sender);
            assert!([tx(1).sender, tx(2).sender, tx(3).sender].contains(&copy.sender));
            assert_eq!(copy.roll, None);
        }
        let mut single_sender = vec![tx(1), tx(1)];
        assert!(!SequenceMutator::resend(&mut single_sender, &mut runner));
        assert_eq!(single_sender, [tx(1), tx(1)]);
    }

    #[test]
    fn attacker_reuses_bytes_argument_in_another_call() {
        let (permit, claim) = ("permit(address,uint256,bytes)", "claim(uint256,bytes)");
        let targets = typed_targets(&[permit, claim]);
        let signature = DynSolValue::Bytes(vec![0xaa; 65]);
        let empty = DynSolValue::Bytes(Vec::new());
        // One sender only, so the resend variant never applies.
        let original = vec![
            typed_call(1, permit, &[address(1), uint(1), signature.clone()]),
            typed_call(1, claim, &[uint(2), empty.clone()]),
        ];
        let mut runner = TestRunner::deterministic();
        let mut replayed = false;
        for _ in 0..100 {
            let mut sequence = original.clone();
            assert!(
                SequenceMutator::attacker(
                    &mut sequence,
                    &targets,
                    &mut runner,
                    &FuzzFixtures::default()
                )
                .unwrap()
            );
            let permitted = decoded(permit, &sequence[0]);
            let claimed = decoded(claim, &sequence[1]);
            // Only the `bytes` arguments change.
            assert_eq!((&permitted[..2], &claimed[0]), (&[address(1), uint(1)][..], &uint(2)));
            assert!(claimed[1] == signature || permitted[2] == empty);
            replayed |= claimed[1] == signature;
        }
        assert!(replayed);
    }

    #[test]
    fn invariant_typed_mutations_apply_without_abi_fallback() {
        let (mint, burn) = ("mint(uint256)", "burn(uint256,address)");
        let original =
            [typed_call(1, mint, &[uint(5)]), typed_call(2, burn, &[uint(9), address(3)])];
        let corpus = [CorpusEntryView::new(&original, &[]).unwrap()];
        for kind in 7..10 {
            let mut config = config();
            config.mutation_weights = forced_weights(kind);
            let generator = SequenceGenerator::invariant(
                generator_tx(9),
                state(),
                FuzzRunIdentifiedContracts::new(typed_targets(&[mint, burn]), false),
                &config,
            )
            .unwrap();
            let plan = generator
                .start(&mut TestRunner::deterministic(), corpus.len(), |i| Ok(corpus[i]), true)
                .unwrap();
            assert_eq!(plan.source(), Some(0));
            let minted = decoded(mint, &plan.initial()[0]);
            match kind {
                7 | 8 => {
                    let burned = decoded(burn, &plan.initial()[1]);
                    assert_eq!(plan.initial().len(), 2, "mutation family {kind}");
                    assert!(minted == [uint(9)] || burned[0] == uint(5), "mutation family {kind}");
                }
                // Both calls carry their own sender, so the attacker resends one of them.
                _ => assert_eq!(plan.initial().len(), 3),
            }
        }

        // Without a same-typed pair and without ABI fallback, the entry is replayed unchanged.
        let unmatched = [typed_call(1, mint, &[uint(5)])];
        let corpus = [CorpusEntryView::new(&unmatched, &[]).unwrap()];
        for kind in 7..10 {
            let mut config = config();
            config.mutation_weights = forced_weights(kind);
            let generator = SequenceGenerator::invariant(
                generator_tx(9),
                state(),
                FuzzRunIdentifiedContracts::new(typed_targets(&[mint]), false),
                &config,
            )
            .unwrap();
            let plan = generator
                .start(&mut TestRunner::deterministic(), corpus.len(), |i| Ok(corpus[i]), true)
                .unwrap();
            assert_eq!(plan.initial(), unmatched, "mutation family {kind}");
        }
    }

    #[test]
    fn zero_typed_weights_preserve_mutation_draws() {
        let generator = SequenceGenerator::invariant(
            generator_tx(9),
            state(),
            FuzzRunIdentifiedContracts::new(TargetedContracts::new(), false),
            &config(),
        )
        .unwrap();
        let reference = WeightedIndex::new([1u32; 7]).unwrap();
        let mut actual = TestRunner::deterministic();
        let mut expected = TestRunner::deterministic();
        for _ in 0..1000 {
            assert_eq!(generator.mutations.sample(actual.rng()), reference.sample(expected.rng()));
        }
    }
}
