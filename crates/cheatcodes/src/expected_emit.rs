//! Engine-free matching for the `expectEmit` cheatcodes.

use crate::Error;
use alloy_dyn_abi::{DynSolValue, EventExt};
use alloy_json_abi::Event;
use alloy_primitives::{
    Address, Bytes, Log, LogData as RawLog, hex,
    map::{AddressHashMap, HashMap, hash_map::Entry},
};
use alloy_sol_types::SolValue;
use foundry_common::{abi::get_indexed_event, fmt::format_token};
use foundry_evm_traces::{DecodedCallLog, identifier::SignaturesIdentifier};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct ExpectedEmit {
    /// The depth at which we expect this emit to have occurred
    pub depth: usize,
    /// The log we expect
    pub log: Option<RawLog>,
    /// The checks to perform:
    /// ```text
    /// ┌───────┬───────┬───────┬───────┬────┐
    /// │topic 0│topic 1│topic 2│topic 3│data│
    /// └───────┴───────┴───────┴───────┴────┘
    /// ```
    pub checks: [bool; 5],
    /// If present, check originating address against this
    pub address: Option<Address>,
    /// If present, relax the requirement that topic 0 must be present. This allows anonymous
    /// events with no indexed topics to be matched.
    pub anonymous: bool,
    /// Whether the log was actually found in the subcalls
    pub found: bool,
    /// Number of times the log is expected to be emitted
    pub count: u64,
    /// Stores mismatch details if a log didn't match.
    pub mismatch_error: Option<EmitMismatch>,
}

#[derive(Clone, Debug)]
pub enum EmitMismatch {
    Log { actual: RawLog },
    Emitter { expected: Address, actual: Address },
}

impl EmitMismatch {
    /// Formats the mismatch, calling `identifier` only when event signatures are needed.
    pub fn to_error_msg<'a>(
        &self,
        identifier: impl FnOnce() -> Option<&'a SignaturesIdentifier>,
        checks: [bool; 5],
        expected: Option<&RawLog>,
        anonymous: bool,
    ) -> String {
        match self {
            Self::Log { actual } => {
                let Some(expected) = expected else {
                    return "log != expected log".to_string();
                };
                let (expected_decoded, actual_decoded) = if anonymous {
                    (None, None)
                } else {
                    identifier()
                        .map(|identifier| {
                            (decode_event(identifier, expected), decode_event(identifier, actual))
                        })
                        .unwrap_or_default()
                };
                get_emit_mismatch_message(
                    checks,
                    expected,
                    actual,
                    anonymous,
                    expected_decoded.as_ref(),
                    actual_decoded.as_ref(),
                )
            }
            Self::Emitter { expected, actual } => {
                format!("log emitter mismatch: expected={expected:#x}, got={actual:#x}")
            }
        }
    }
}

/// Fills or checks the expected emits against an emitted log.
///
/// Returns the failure reason if the log violates an expectation.
pub(crate) fn observe_log(tracker: &mut ExpectedEmitTracker, log: &Log) -> Option<&'static str> {
    // Fill or check the expected emits.
    // We expect for emit checks to be filled as they're declared (from oldest to newest),
    // so we fill them and push them to the back of the queue.
    // If the user has properly filled all the emits, they'll end up in their original order.
    // If not, the queue will not be in the order the events will be intended to be filled,
    // and we'll be able to later detect this and bail.

    // First, we can return early if all events have been matched.
    // This allows a contract to arbitrarily emit more events than expected (additive behavior),
    // as long as all the previous events were matched in the order they were expected to be.
    if tracker.iter().all(|(expected, _)| expected.found) {
        return None;
    }

    // Check count=0 expectations against this log - fail immediately if violated
    for (expected_emit, _) in tracker.iter() {
        if expected_emit.count == 0
            && !expected_emit.found
            && let Some(expected_log) = &expected_emit.log
            && checks_topics_and_data(expected_emit.checks, expected_log, log)
            // Check revert address 
            && (expected_emit.address.is_none_or(|address| address == log.address))
        {
            // This event was emitted but we expected it NOT to be (count=0).
            return Some("log emitted but expected 0 times");
        }
    }

    let should_fill_logs = tracker.iter().any(|(expected, _)| expected.log.is_none());
    let index_to_fill_or_check = if should_fill_logs {
        // If there's anything to fill, we start with the last event to match in the queue
        // (without taking into account events already matched).
        tracker.iter().position(|(emit, _)| emit.found).unwrap_or(tracker.len()).saturating_sub(1)
    } else {
        // if all expected logs are filled, check any unmatched event
        // in the declared order, so we start from the front (like a queue).
        // Skip count=0 expectations as they are handled separately above
        tracker.iter().position(|(emit, _)| !emit.found && emit.count > 0).unwrap_or(0)
    };

    // If there are only count=0 expectations left, we can return early
    if !should_fill_logs && tracker.iter().all(|(emit, _)| emit.found || emit.count == 0) {
        return None;
    }

    let (mut event_to_fill_or_check, mut count_map) =
        tracker.remove(index_to_fill_or_check).expect("we should have an emit to fill or check");

    let Some(expected) = &event_to_fill_or_check.log else {
        // Unless the caller is trying to match an anonymous event, the first topic must be
        // filled.
        if event_to_fill_or_check.anonymous || !log.topics().is_empty() {
            event_to_fill_or_check.log = Some(log.data.clone());
            // If we only filled the expected log then we put it back at the same position.
            tracker.insert(index_to_fill_or_check, (event_to_fill_or_check, count_map));
        } else {
            return Some("use vm.expectEmitAnonymous to match anonymous events");
        }

        return None;
    };

    // Increment/set `count` for `log.address` and `log.data`
    match count_map.entry(log.address) {
        Entry::Occupied(mut entry) => {
            let log_count_map = entry.get_mut();
            log_count_map.insert(&log.data);
        }
        Entry::Vacant(entry) => {
            let mut log_count_map = LogCountMap::new(&event_to_fill_or_check);
            if log_count_map.satisfies_checks(&log.data) {
                log_count_map.insert(&log.data);
                entry.insert(log_count_map);
            }
        }
    }

    event_to_fill_or_check.found = || -> bool {
        if !checks_topics_and_data(event_to_fill_or_check.checks, expected, log) {
            event_to_fill_or_check.mismatch_error =
                Some(EmitMismatch::Log { actual: log.data.clone() });
            return false;
        }

        // Maybe match source address.
        if let Some(expected) = event_to_fill_or_check.address
            && expected != log.address
        {
            event_to_fill_or_check.mismatch_error =
                Some(EmitMismatch::Emitter { expected, actual: log.address });
            return false;
        }

        let expected_count = event_to_fill_or_check.count;
        match event_to_fill_or_check.address {
            Some(emitter) => count_map
                .get(&emitter)
                .is_some_and(|log_map| log_map.count(&log.data) >= expected_count),
            None => count_map
                .values()
                .find(|log_map| log_map.satisfies_checks(&log.data))
                .is_some_and(|map| map.count(&log.data) >= expected_count),
        }
    }();

    // If we found the event, we can push it to the back of the queue
    // and begin expecting the next event.
    if event_to_fill_or_check.found {
        tracker.push_back((event_to_fill_or_check, count_map));
    } else {
        // We did not match this event, so we need to keep waiting for the right one to
        // appear.
        tracker.push_front((event_to_fill_or_check, count_map));
    }

    None
}

/// Handles expected emits specified by the `expectEmit` cheatcodes.
///
/// The second element of the tuple counts the number of times the log has been emitted by a
/// particular address
pub type ExpectedEmitTracker = VecDeque<(ExpectedEmit, AddressHashMap<LogCountMap>)>;

#[derive(Clone, Debug, Default)]
pub struct LogCountMap {
    checks: [bool; 5],
    expected_log: RawLog,
    map: HashMap<RawLog, u64>,
}

impl LogCountMap {
    /// Instantiates `LogCountMap`.
    fn new(expected_emit: &ExpectedEmit) -> Self {
        Self {
            checks: expected_emit.checks,
            expected_log: expected_emit.log.clone().expect("log should be filled here"),
            map: Default::default(),
        }
    }

    /// Inserts a log into the map and increments the count.
    ///
    /// The log must pass all checks against the expected log for the count to increment.
    ///
    /// Returns true if the log was inserted and count was incremented.
    fn insert(&mut self, log: &RawLog) -> bool {
        // If its already in the map, increment the count without checking.
        if self.map.contains_key(log) {
            self.map.entry(log.clone()).and_modify(|c| *c += 1);

            return true;
        }

        if !self.satisfies_checks(log) {
            return false;
        }

        self.map.entry(log.clone()).and_modify(|c| *c += 1).or_insert(1);

        true
    }

    /// Checks the incoming raw log against the expected logs topics and data.
    fn satisfies_checks(&self, log: &RawLog) -> bool {
        checks_topics_and_data(self.checks, &self.expected_log, log)
    }

    pub fn count(&self, log: &RawLog) -> u64 {
        if !self.satisfies_checks(log) {
            return 0;
        }

        self.count_unchecked()
    }

    pub fn count_unchecked(&self) -> u64 {
        self.map.values().sum()
    }
}

fn checks_topics_and_data(checks: [bool; 5], expected: &RawLog, log: &RawLog) -> bool {
    if log.topics().len() != expected.topics().len() {
        return false;
    }

    // Check topics.
    if !log
        .topics()
        .iter()
        .enumerate()
        .filter(|(i, _)| checks[*i])
        .all(|(i, topic)| topic == &expected.topics()[i])
    {
        return false;
    }

    // Check data
    if checks[4] && expected.data.as_ref() != log.data.as_ref() {
        return false;
    }

    true
}

fn decode_event(
    identifier: &foundry_evm_traces::identifier::SignaturesIdentifier,
    log: &RawLog,
) -> Option<DecodedCallLog> {
    let topics = log.topics();
    if topics.is_empty() {
        return None;
    }
    let t0 = topics[0]; // event sig
    // Try to identify the event
    let event = foundry_common::block_on(
        identifier.identify_event_with_indexed_count(t0, topics.len().saturating_sub(1)),
    )?;

    // Check if event already has indexed information from signatures
    let has_indexed_info = event.inputs.iter().any(|p| p.indexed);
    // Only use get_indexed_event if the event doesn't have indexing info
    let indexed_event = if has_indexed_info { event } else { get_indexed_event(event, log) };

    // Try to decode the event
    if let Ok(decoded) = indexed_event.decode_log(log) {
        let params = reconstruct_params(&indexed_event, &decoded);

        let decoded_params = params
            .into_iter()
            .zip(indexed_event.inputs.iter())
            .map(|(param, input)| (input.name.clone(), format_token(&param)))
            .collect();

        return Some(DecodedCallLog {
            name: Some(indexed_event.name),
            params: Some(decoded_params),
        });
    }

    None
}

/// Restore the order of the params of a decoded event
fn reconstruct_params(event: &Event, decoded: &alloy_dyn_abi::DecodedEvent) -> Vec<DynSolValue> {
    let mut indexed = 0;
    let mut unindexed = 0;
    let mut inputs = vec![];
    for input in &event.inputs {
        if input.indexed && indexed < decoded.indexed.len() {
            inputs.push(decoded.indexed[indexed].clone());
            indexed += 1;
        } else if unindexed < decoded.body.len() {
            inputs.push(decoded.body[unindexed].clone());
            unindexed += 1;
        }
    }
    inputs
}

/// Gets a detailed mismatch message for emit assertions
pub(crate) fn get_emit_mismatch_message(
    checks: [bool; 5],
    expected: &RawLog,
    actual: &RawLog,
    is_anonymous: bool,
    expected_decoded: Option<&DecodedCallLog>,
    actual_decoded: Option<&DecodedCallLog>,
) -> String {
    // Early return for completely different events or incompatible structures

    // 1. Different number of topics
    if actual.topics().len() != expected.topics().len() {
        let expected_name = expected_decoded.and_then(|d| d.name.as_deref()).unwrap_or("log");
        let actual_name = actual_decoded.and_then(|d| d.name.as_deref()).unwrap_or("log");
        let expected_topics = checked_topic_count(expected, is_anonymous);
        let actual_topics = checked_topic_count(actual, is_anonymous);

        if expected_name == actual_name {
            return format!(
                "{actual_name} indexed topic count mismatch: expected {expected_topics}, got {actual_topics}"
            );
        }

        return name_mismatched_logs(expected_decoded, actual_decoded);
    }

    // 2. Different event signatures (for non-anonymous events)
    if !is_anonymous
        && checks[0]
        && (!expected.topics().is_empty() && !actual.topics().is_empty())
        && expected.topics()[0] != actual.topics()[0]
    {
        return name_mismatched_logs(expected_decoded, actual_decoded);
    }

    let expected_data = expected.data.as_ref();
    let actual_data = actual.data.as_ref();

    // 3. Check data
    if checks[4] && expected_data != actual_data {
        // Different lengths or not ABI-encoded
        if expected_data.len() != actual_data.len()
            || !expected_data.len().is_multiple_of(32)
            || expected_data.is_empty()
        {
            return name_mismatched_logs(expected_decoded, actual_decoded);
        }
    }

    // expected and actual events are the same, so check individual parameters
    let mut mismatches = Vec::new();

    // Check topics (indexed parameters)
    for (i, (expected_topic, actual_topic)) in
        expected.topics().iter().zip(actual.topics().iter()).enumerate()
    {
        // Skip topic[0] for non-anonymous events (already checked above)
        if i == 0 && !is_anonymous {
            continue;
        }

        // Only check if the corresponding check flag is set
        if i < checks.len() && checks[i] && expected_topic != actual_topic {
            let param_idx = if is_anonymous {
                i // For anonymous events, topic[0] is param 0
            } else {
                i - 1 // For regular events, topic[0] is event signature, so topic[1] is param 0
            };
            mismatches
                .push(format!("param {param_idx}: expected={expected_topic}, got={actual_topic}"));
        }
    }

    // Check data (non-indexed parameters)
    if checks[4] && expected_data != actual_data {
        let num_indexed_params = if is_anonymous {
            expected.topics().len()
        } else {
            expected.topics().len().saturating_sub(1)
        };

        for (i, (expected_chunk, actual_chunk)) in
            expected_data.chunks(32).zip(actual_data.chunks(32)).enumerate()
        {
            if expected_chunk != actual_chunk {
                let param_idx = num_indexed_params + i;
                mismatches.push(format!(
                    "param {}: expected={}, got={}",
                    param_idx,
                    hex::encode_prefixed(expected_chunk),
                    hex::encode_prefixed(actual_chunk)
                ));
            }
        }
    }

    if mismatches.is_empty() {
        name_mismatched_logs(expected_decoded, actual_decoded)
    } else {
        // Build the error message with event names if available
        let event_prefix = match (expected_decoded, actual_decoded) {
            (Some(expected_dec), Some(actual_dec)) if expected_dec.name == actual_dec.name => {
                format!(
                    "{} param mismatch",
                    expected_dec.name.as_ref().unwrap_or(&"log".to_string())
                )
            }
            _ => {
                if is_anonymous {
                    "anonymous log mismatch".to_string()
                } else {
                    "log mismatch".to_string()
                }
            }
        };

        // Add parameter details if available from decoded events
        let detailed_mismatches = if let (Some(expected_dec), Some(actual_dec)) =
            (expected_decoded, actual_decoded)
            && let (Some(expected_params), Some(actual_params)) =
                (&expected_dec.params, &actual_dec.params)
        {
            mismatches
                .into_iter()
                .map(|basic_mismatch| {
                    // Try to find the parameter name and decoded value
                    if let Some(param_idx) = basic_mismatch
                        .split(' ')
                        .nth(1)
                        .and_then(|s| s.trim_end_matches(':').parse::<usize>().ok())
                        && param_idx < expected_params.len()
                        && param_idx < actual_params.len()
                    {
                        let (expected_name, expected_value) = &expected_params[param_idx];
                        let (_actual_name, actual_value) = &actual_params[param_idx];
                        let param_name = if expected_name.is_empty() {
                            &format!("param{param_idx}")
                        } else {
                            expected_name
                        };
                        return format!(
                            "{param_name}: expected={expected_value}, got={actual_value}",
                        );
                    }
                    basic_mismatch
                })
                .collect::<Vec<_>>()
        } else {
            mismatches
        };

        format!("{} at {}", event_prefix, detailed_mismatches.join(", "))
    }
}

/// Formats the generic mismatch message: "log != expected log" to include event names if available
fn name_mismatched_logs(
    expected_decoded: Option<&DecodedCallLog>,
    actual_decoded: Option<&DecodedCallLog>,
) -> String {
    let expected_name = expected_decoded.and_then(|d| d.name.as_deref()).unwrap_or("log");
    let actual_name = actual_decoded.and_then(|d| d.name.as_deref()).unwrap_or("log");
    format!("{actual_name} != expected {expected_name}")
}

fn checked_topic_count(log: &RawLog, is_anonymous: bool) -> usize {
    if is_anonymous { log.topics().len() } else { log.topics().len().saturating_sub(1) }
}

/// Why the tracked expected emits were not satisfied when a call ended.
#[derive(Debug)]
pub(crate) enum UnmetEmit {
    /// An expected log was never matched.
    Unmatched(ExpectedEmit),
    /// A log was emitted a different number of times than expected.
    Count { expected: u64, actual: u64 },
    /// The call did not succeed while logs were expected.
    Failed,
}

impl UnmetEmit {
    /// Encodes the revert data, calling `identifier` only when event signatures are needed.
    pub(crate) fn encode<'a>(
        self,
        identifier: impl FnOnce() -> Option<&'a SignaturesIdentifier>,
    ) -> Bytes {
        match self {
            Self::Unmatched(expected) => {
                let error_msg = expected
                    .mismatch_error
                    .as_ref()
                    .map(|mismatch| {
                        mismatch.to_error_msg(
                            identifier,
                            expected.checks,
                            expected.log.as_ref(),
                            expected.anonymous,
                        )
                    })
                    .unwrap_or_else(|| "log != expected log".to_string());
                error_msg.abi_encode().into()
            }
            Self::Count { expected, actual } => {
                Error::encode(format!("log emitted {actual} times, expected {expected}"))
            }
            Self::Failed => Error::encode(
                "expected an emit, but the call reverted instead. \
                 ensure you're testing the happy path when using `expectEmit`",
            ),
        }
    }
}

/// Checks the entire tracker when a non-static call ends at any tracked expectation's `depth`.
///
/// Clears the tracker only if all expectations are satisfied; otherwise leaves it untouched.
/// Callers must skip already-reverted calls to preserve their original failure.
pub(crate) fn check_call_emits(
    tracker: &mut ExpectedEmitTracker,
    depth: usize,
    is_static: bool,
    succeeded: bool,
) -> Option<UnmetEmit> {
    let should_check_emits =
        tracker.iter().any(|(expected, _)| expected.depth == depth) && !is_static;
    if !should_check_emits {
        return None;
    }

    let expected_counts = tracker
        .iter()
        .filter_map(|(expected, count_map)| {
            let count = match expected.address {
                Some(emitter) => match count_map.get(&emitter) {
                    Some(log_count) => expected
                        .log
                        .as_ref()
                        .map(|l| log_count.count(l))
                        .unwrap_or_else(|| log_count.count_unchecked()),
                    None => 0,
                },
                None => match &expected.log {
                    Some(log) => count_map.values().map(|logs| logs.count(log)).sum(),
                    None => count_map.values().map(|logs| logs.count_unchecked()).sum(),
                },
            };

            (count != expected.count).then_some((expected, count))
        })
        .collect::<Vec<_>>();

    if let Some((expected, _)) =
        tracker.iter().find(|(expected, _)| !expected.found && expected.count > 0)
    {
        return Some(UnmetEmit::Unmatched(expected.clone()));
    }

    if let Some((expected, count)) = expected_counts.first() {
        return Some(if succeeded {
            UnmetEmit::Count { expected: expected.count, actual: *count }
        } else {
            UnmetEmit::Failed
        });
    }

    // Later calls need their own expectations.
    tracker.clear();
    None
}

/// Checks for leftover expected emits when the root call ends without reverting.
///
/// Returns the failure message if any expectation is still unmatched.
/// Callers must skip already-reverted calls to preserve their original failure.
pub(crate) fn first_unmet_root_emit(
    tracker: &mut ExpectedEmitTracker,
    succeeded: bool,
) -> Option<&'static str> {
    // A count=0 expectation is met when its log was never emitted.
    for (expected, _) in tracker.iter_mut() {
        if expected.count == 0 && !expected.found {
            expected.found = true;
        }
    }
    tracker.retain(|(expected, _)| !expected.found);
    if tracker.is_empty() {
        return None;
    }
    Some(if succeeded {
        "expected an emit, but no logs were emitted afterwards. \
         you might have mismatched events or not enough events were emitted"
    } else {
        "expected an emit, but the call reverted instead. \
         ensure you're testing the happy path when using `expectEmit`"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{B256, address, b256};

    const EMITTER: Address = address!("0x00000000000000000000000000000000000000aa");
    const OTHER: Address = address!("0x00000000000000000000000000000000000000bb");
    const TOPIC_A: B256 =
        b256!("0x000000000000000000000000000000000000000000000000000000000000000a");
    const TOPIC_B: B256 =
        b256!("0x000000000000000000000000000000000000000000000000000000000000000b");
    const ALL: [bool; 5] = [true; 5];

    fn log(emitter: Address, topics: Vec<B256>, data: &'static [u8]) -> Log {
        Log::new_unchecked(emitter, topics, Bytes::from_static(data))
    }

    fn expect(tracker: &mut ExpectedEmitTracker, expected: Option<&Log>, checks: [bool; 5]) {
        expect_with(tracker, expected, checks, None, 1, false);
    }

    fn expect_with(
        tracker: &mut ExpectedEmitTracker,
        expected: Option<&Log>,
        checks: [bool; 5],
        address: Option<Address>,
        count: u64,
        anonymous: bool,
    ) {
        tracker.push_back((
            ExpectedEmit {
                depth: 0,
                log: expected.map(|log| log.data.clone()),
                checks,
                address,
                anonymous,
                found: false,
                count,
                mismatch_error: None,
            },
            Default::default(),
        ));
    }

    fn found(tracker: &ExpectedEmitTracker) -> Vec<bool> {
        tracker.iter().map(|(expected, _)| expected.found).collect()
    }

    #[test]
    fn matches_logs_in_order() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(EMITTER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        expect(&mut tracker, Some(&b), ALL);

        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(observe_log(&mut tracker, &b), None);
        assert_eq!(found(&tracker), [true, true]);
    }

    #[test]
    fn out_of_order_logs_leave_an_expectation_unmatched() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(EMITTER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        expect(&mut tracker, Some(&b), ALL);

        assert_eq!(observe_log(&mut tracker, &b), None);
        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [false, true]);
        assert_eq!(tracker[0].0.log.as_ref(), Some(&b.data));
    }

    #[test]
    fn fills_an_unfilled_expectation_with_the_next_log() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, None, ALL);

        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [false]);
        assert_eq!(tracker[0].0.log.as_ref(), Some(&a.data));

        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [true]);
    }

    #[test]
    fn fills_the_last_pending_expectation_before_found_ones() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(EMITTER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, None, ALL);
        expect(&mut tracker, None, ALL);
        expect(&mut tracker, Some(&a), ALL);
        tracker[2].0.found = true;

        assert_eq!(observe_log(&mut tracker, &b), None);
        assert_eq!(tracker[0].0.log, None);
        assert_eq!(tracker[1].0.log.as_ref(), Some(&b.data));
        assert_eq!(tracker[2].0.log.as_ref(), Some(&a.data));
        assert_eq!(found(&tracker), [false, false, true]);
    }

    #[test]
    fn counts_matching_logs() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, Some(&a), ALL, None, 2, false);

        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [false]);
        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [true]);
    }

    #[test]
    fn count_zero_fails_when_the_log_is_emitted() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(EMITTER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, Some(&a), ALL, None, 0, false);

        assert_eq!(observe_log(&mut tracker, &b), None);
        assert_eq!(observe_log(&mut tracker, &a), Some("log emitted but expected 0 times"));
        assert_eq!(found(&tracker), [false]);
    }

    #[test]
    fn anonymous_logs_require_expect_emit_anonymous() {
        let anonymous = log(EMITTER, vec![], b"a");

        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, None, ALL);
        assert_eq!(
            observe_log(&mut tracker, &anonymous),
            Some("use vm.expectEmitAnonymous to match anonymous events")
        );
        assert!(tracker.is_empty());

        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, None, ALL, None, 1, true);
        assert_eq!(observe_log(&mut tracker, &anonymous), None);
        assert_eq!(tracker[0].0.log.as_ref(), Some(&anonymous.data));
        assert_eq!(observe_log(&mut tracker, &anonymous), None);
        assert_eq!(found(&tracker), [true]);
    }

    #[test]
    fn filters_by_emitter_address() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let from_other = log(OTHER, vec![TOPIC_A], b"a");
        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, Some(&a), ALL, Some(EMITTER), 1, false);

        assert_eq!(observe_log(&mut tracker, &from_other), None);
        assert_eq!(found(&tracker), [false]);
        assert!(matches!(
            tracker[0].0.mismatch_error,
            Some(EmitMismatch::Emitter { expected: EMITTER, actual: OTHER })
        ));

        assert_eq!(observe_log(&mut tracker, &a), None);
        assert_eq!(found(&tracker), [true]);
    }

    #[test]
    fn applies_check_flags() {
        const TOPICS: [B256; 4] = [TOPIC_A, TOPIC_A, TOPIC_A, TOPIC_A];
        let expected = log(EMITTER, TOPICS.to_vec(), b"expected");

        // Each flag only compares its own topic or the data.
        for flag in 0..5 {
            let actual = if flag < 4 {
                let mut topics = TOPICS;
                topics[flag] = TOPIC_B;
                log(EMITTER, topics.to_vec(), b"expected")
            } else {
                log(EMITTER, TOPICS.to_vec(), b"other")
            };

            let mut unchecked = ALL;
            unchecked[flag] = false;
            let mut tracker = ExpectedEmitTracker::new();
            expect(&mut tracker, Some(&expected), unchecked);
            assert_eq!(observe_log(&mut tracker, &actual), None);
            assert_eq!(found(&tracker), [true], "flag {flag} unchecked");

            let mut tracker = ExpectedEmitTracker::new();
            expect(&mut tracker, Some(&expected), ALL);
            assert_eq!(observe_log(&mut tracker, &actual), None);
            assert_eq!(found(&tracker), [false], "flag {flag} checked");
            assert!(matches!(
                &tracker[0].0.mismatch_error,
                Some(EmitMismatch::Log { actual: mismatch }) if mismatch == &actual.data
            ));
        }
    }

    #[test]
    fn mismatch_messages() {
        let expected = log(EMITTER, vec![TOPIC_A], b"expected");
        let actual = log(EMITTER, vec![TOPIC_A], b"actual");

        let emitter = EmitMismatch::Emitter { expected: EMITTER, actual: OTHER };
        assert_eq!(
            emitter.to_error_msg(unused_identifier, ALL, Some(&expected.data), false),
            "log emitter mismatch: expected=0x00000000000000000000000000000000000000aa, \
             got=0x00000000000000000000000000000000000000bb"
        );

        let mismatch = EmitMismatch::Log { actual: actual.data };
        assert_eq!(
            mismatch.to_error_msg(unused_identifier, ALL, None, false),
            "log != expected log"
        );
        assert_eq!(
            mismatch.to_error_msg(unused_identifier, ALL, Some(&expected.data), true),
            "log != expected log"
        );
        let mut resolved = false;
        let resolve = || {
            resolved = true;
            None
        };
        assert_eq!(
            mismatch.to_error_msg(resolve, ALL, Some(&expected.data), false),
            "log != expected log"
        );
        assert!(resolved, "non-anonymous log mismatches resolve signatures");
    }

    /// Signatures are only resolved for non-anonymous log mismatches.
    fn unused_identifier<'a>() -> Option<&'a SignaturesIdentifier> {
        panic!("signatures identifier must not be resolved")
    }

    #[test]
    fn call_end_reports_logs_counted_across_emitters() {
        // Found after two logs from one emitter, with a third from another emitter.
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, Some(&a), ALL, None, 2, false);
        let (expected, count_map) = &mut tracker[0];
        expected.found = true;
        for (emitter, logs) in [(EMITTER, 2), (OTHER, 1)] {
            let mut counts = LogCountMap::new(expected);
            for _ in 0..logs {
                counts.insert(&a.data);
            }
            count_map.insert(emitter, counts);
        }

        let before = format!("{tracker:?}");
        for (succeeded, message) in [
            (true, "log emitted 3 times, expected 2"),
            (
                false,
                "expected an emit, but the call reverted instead. \
                 ensure you're testing the happy path when using `expectEmit`",
            ),
        ] {
            let unmet = check_call_emits(&mut tracker, 0, false, succeeded).unwrap();
            assert_eq!(unmet.encode(unused_identifier), Error::encode(message));
            assert_eq!(format!("{tracker:?}"), before);
        }
    }

    #[test]
    fn root_end_reports_leftover_expectations() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let mut tracker = ExpectedEmitTracker::new();
        expect_with(&mut tracker, Some(&a), ALL, None, 0, false);
        expect(&mut tracker, Some(&a), ALL);
        tracker[1].0.found = true;

        assert_eq!(first_unmet_root_emit(&mut tracker, true), None);
        assert!(tracker.is_empty());

        expect(&mut tracker, Some(&a), ALL);
        assert_eq!(
            first_unmet_root_emit(&mut tracker.clone(), true),
            Some(
                "expected an emit, but no logs were emitted afterwards. \
                 you might have mismatched events or not enough events were emitted"
            )
        );
        assert_eq!(
            first_unmet_root_emit(&mut tracker, false),
            Some(
                "expected an emit, but the call reverted instead. \
                 ensure you're testing the happy path when using `expectEmit`"
            )
        );
    }

    #[test]
    fn call_end_skips_static_and_unrelated_calls_then_clears_tracker() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(OTHER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        expect(&mut tracker, Some(&b), ALL);
        tracker[1].0.depth = 1;
        assert_eq!(observe_log(&mut tracker, &b), None);

        for satisfied in [false, true] {
            if satisfied {
                assert_eq!(observe_log(&mut tracker, &a), None);
                assert_eq!(observe_log(&mut tracker, &b), None);
            }
            let before = format!("{tracker:?}");
            for (depth, is_static) in [(0, true), (2, false)] {
                assert!(check_call_emits(&mut tracker, depth, is_static, true).is_none());
                assert_eq!(format!("{tracker:?}"), before);
            }
        }

        assert!(check_call_emits(&mut tracker, 0, false, true).is_none());
        assert!(tracker.is_empty());
    }

    #[test]
    fn call_end_checks_counts_at_other_depths() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(OTHER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        expect_with(&mut tracker, Some(&b), ALL, Some(OTHER), 0, false);
        tracker[1].0.depth = 1;
        assert_eq!(observe_log(&mut tracker, &a), None);
        let (expected, count_map) = &mut tracker[0];
        assert_eq!(expected.depth, 1);
        let mut counts = LogCountMap::new(expected);
        assert!(counts.insert(&b.data));
        count_map.insert(OTHER, counts);
        let before = format!("{tracker:?}");

        let unmet = check_call_emits(&mut tracker, 0, false, true).unwrap();
        assert!(matches!(unmet, UnmetEmit::Count { expected: 0, actual: 1 }));
        assert_eq!(format!("{tracker:?}"), before);
    }

    #[test]
    fn call_end_unmatched_expectation_precedes_wrong_count_and_failed_call() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let b = log(OTHER, vec![TOPIC_B], b"b");
        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        tracker[0].0.found = true;
        expect(&mut tracker, Some(&b), ALL);
        tracker[1].0.depth = 1;
        let before = format!("{tracker:?}");

        for succeeded in [false, true] {
            let unmet = check_call_emits(&mut tracker, 0, false, succeeded).unwrap();
            assert!(matches!(&unmet, UnmetEmit::Unmatched(expected)
                if expected.log.as_ref() == Some(&b.data)));
            assert_eq!(
                unmet.encode(unused_identifier),
                Bytes::from("log != expected log".abi_encode())
            );
            assert_eq!(format!("{tracker:?}"), before);
        }
    }

    #[test]
    fn unmatched_emit_uses_plain_abi_encoding_and_lazy_signature_resolution() {
        let a = log(EMITTER, vec![TOPIC_A], b"a");
        let actual = log(EMITTER, vec![TOPIC_A], b"actual");
        let mismatch = EmitMismatch::Log { actual: actual.data };
        for (expected_log, mismatch_error, anonymous, message) in [
            (None, Some(mismatch.clone()), false, "log != expected log"),
            (Some(&a), Some(mismatch.clone()), true, "log != expected log"),
            (
                Some(&a),
                Some(EmitMismatch::Emitter { expected: EMITTER, actual: OTHER }),
                false,
                "log emitter mismatch: expected=0x00000000000000000000000000000000000000aa, \
                 got=0x00000000000000000000000000000000000000bb",
            ),
        ] {
            let mut tracker = ExpectedEmitTracker::new();
            expect_with(&mut tracker, expected_log, ALL, None, 1, anonymous);
            tracker[0].0.mismatch_error = mismatch_error;
            let unmet = check_call_emits(&mut tracker, 0, false, true).unwrap();
            assert_eq!(unmet.encode(unused_identifier), Bytes::from(message.abi_encode()));
        }

        let mut tracker = ExpectedEmitTracker::new();
        expect(&mut tracker, Some(&a), ALL);
        tracker[0].0.mismatch_error = Some(mismatch);
        let unmet = check_call_emits(&mut tracker, 0, false, true).unwrap();
        let mut resolved = false;
        let encoded = unmet.encode(|| {
            resolved = true;
            None
        });
        assert!(resolved, "non-anonymous log mismatches resolve signatures");
        assert_eq!(encoded, Bytes::from("log != expected log".abi_encode()));
    }
}
