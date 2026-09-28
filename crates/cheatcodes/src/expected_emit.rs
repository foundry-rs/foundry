//! Log expectations shared by EVM inspectors.

use alloy_primitives::{
    Address, Log, LogData as RawLog, hex,
    map::{AddressHashMap, HashMap, hash_map::Entry},
};
use foundry_evm_traces::DecodedCallLog;
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

pub fn register(
    tracker: &mut ExpectedEmitTracker,
    depth: usize,
    checks: [bool; 5],
    address: Option<Address>,
    anonymous: bool,
    count: u64,
) {
    let expected_emit = ExpectedEmit {
        depth,
        checks,
        address,
        found: false,
        log: None,
        anonymous,
        count,
        mismatch_error: None,
    };
    if let Some(found_emit_pos) = tracker.iter().position(|(emit, _)| emit.found) {
        // The order of emits already found (back of queue) should not be modified, hence push any
        // new emit before first found emit.
        tracker.insert(found_emit_pos, (expected_emit, Default::default()));
    } else {
        // If no expected emits then push new one at the back of queue.
        tracker.push_back((expected_emit, Default::default()));
    }
}

pub fn observe(tracker: &mut ExpectedEmitTracker, log: &Log) -> Option<&'static str> {
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
            // Check revert address.
            && (expected_emit.address.is_none_or(|address| address == log.address))
        {
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

/// An unsatisfied event expectation at the declaring call boundary.
pub enum EmitValidation {
    Missing(ExpectedEmit),
    WrongCount { expected: u64, actual: u64 },
}

/// Checks expectations declared at `depth` and clears them after success.
pub fn validate(
    tracker: &mut ExpectedEmitTracker,
    depth: usize,
    is_static: bool,
) -> Option<EmitValidation> {
    if is_static || !tracker.iter().any(|(expected, _)| expected.depth == depth) {
        return None;
    }

    if let Some((expected, _)) =
        tracker.iter().find(|(expected, _)| !expected.found && expected.count > 0)
    {
        return Some(EmitValidation::Missing(expected.clone()));
    }

    for (expected, count_map) in tracker.iter() {
        let count = match expected.address {
            Some(emitter) => match count_map.get(&emitter) {
                Some(logs) => expected
                    .log
                    .as_ref()
                    .map(|log| logs.count(log))
                    .unwrap_or_else(|| logs.count_unchecked()),
                None => 0,
            },
            None => match &expected.log {
                Some(log) => count_map.values().map(|logs| logs.count(log)).sum(),
                None => count_map.values().map(|logs| logs.count_unchecked()).sum(),
            },
        };
        if count != expected.count {
            return Some(EmitValidation::WrongCount { expected: expected.count, actual: count });
        }
    }

    tracker.clear();
    None
}

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
