//! Bounded, opt-in online Choice decisions over an ABI-derived transaction grammar.

use crate::BasicTxDetails;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Selector};
use eyre::{Result, bail, eyre};
use serde_json::{Map, Value, json};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};

const MODEL: &str = "typesafe/jev-1.13";
const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
const BATCH: usize = 8;
const MAX_REQUESTS: usize = 32;
const MAX_FUNCTIONS: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

type Request = (Value, SyncSender<Result<Value>>);

/// Lives on a separate OS thread: no nested runtime or network work inside a proptest strategy.
struct Transport {
    requests: SyncSender<Request>,
}

impl Transport {
    fn new() -> Result<Self> {
        let key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| eyre!("OPENROUTER_API_KEY is not set"))?;
        Self::start(key, ENDPOINT.to_string())
    }

    fn start(key: String, endpoint: String) -> Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        thread::Builder::new().name("foundry-jev".into()).spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            let client = reqwest::Client::builder()
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build();
            while let Ok((request, response)) = receiver.recv() {
                let result = match (&runtime, &client) {
                    (Ok(runtime), Ok(client)) => runtime.block_on(async {
                        let mut res = client
                            .post(&endpoint)
                            .bearer_auth(key.trim())
                            .json(&request)
                            .send()
                            .await?
                            .error_for_status()?;
                        let mut body = Vec::new();
                        while let Some(chunk) = res.chunk().await? {
                            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                                bail!("Jev response exceeds size limit");
                            }
                            body.extend_from_slice(&chunk);
                        }
                        Ok(serde_json::from_slice(&body)?)
                    }),
                    _ => Err(eyre!("could not initialize Jev transport")),
                };
                let _ = response.send(result);
            }
        })?;
        Ok(Self { requests })
    }

    fn decide(&self, request: Value) -> Result<Value> {
        if serde_json::to_vec(&request)?.len() > MAX_REQUEST_BYTES {
            bail!("Jev request exceeds size limit");
        }
        let (sender, receiver): (_, Receiver<Result<Value>>) = mpsc::sync_channel(1);
        self.requests.try_send((request, sender)).map_err(|_| eyre!("Jev worker unavailable"))?;
        receiver
            .recv_timeout(TIMEOUT + Duration::from_millis(100))
            .map_err(|_| eyre!("Jev request timed out"))?
    }
}

/// Each eligible function has two typed productions: random leaves or dictionary-backed leaves.
/// The remote model never supplies code, addresses, ABI bytes, or concrete argument values.
pub(super) struct Jev {
    transport: Option<Transport>,
    targets: Vec<(Address, Selector)>,
    criteria: Map<String, Value>,
    pending: VecDeque<usize>,
    history: VecDeque<Value>,
    requests: usize,
    disabled: bool,
}

impl Jev {
    #[cfg(test)]
    pub(super) fn with_decider(
        mut decide: impl FnMut(Value) -> Result<Value> + Send + 'static,
    ) -> Self {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        thread::spawn(move || {
            while let Ok((request, response)) = receiver.recv() {
                let _ = response.send(decide(request));
            }
        });
        let mut jev = Self::new();
        jev.transport = Some(Transport { requests });
        jev
    }

    pub(super) fn new() -> Self {
        Self {
            transport: None,
            targets: Vec::new(),
            criteria: Map::new(),
            pending: VecDeque::new(),
            history: VecDeque::new(),
            requests: 0,
            disabled: false,
        }
    }

    /// Re-derive after target lifecycle changes; stale choices cannot refer to removed contracts.
    pub(super) fn refresh(&mut self, functions: &[(Address, Function)]) {
        self.pending.clear();
        self.history.clear();
        self.targets.clear();
        self.criteria.clear();
        if functions.len() > MAX_FUNCTIONS {
            self.disable("more than 64 eligible functions");
            return;
        }
        let mut contracts = Vec::new();
        for (index, (address, function)) in functions.iter().enumerate() {
            let target = contracts.iter().position(|a| a == address).unwrap_or_else(|| {
                contracts.push(*address);
                contracts.len() - 1
            });
            self.targets.push((*address, function.selector()));
            for (offset, leaves) in ["random", "dictionary-backed"].into_iter().enumerate() {
                self.criteria.insert(format!("p{}", index * 2 + offset), json!(format!(
                    "Call contract_{target}.{} with ABI-typed {leaves} arguments generated locally. Mutability: {:?}.",
                    function.signature(), function.state_mutability,
                )));
            }
        }
    }

    pub(super) fn begin_run(&mut self) {
        self.history.clear();
        self.pending.clear();
    }

    /// Record actual execution, including corpus-derived calls, rather than assuming generated
    /// calls succeeded. Only a local production ID and three booleans leave the process.
    pub(super) fn observe(
        &mut self,
        tx: &BasicTxDetails,
        reverted: bool,
        discarded: bool,
        new_coverage: bool,
    ) {
        let call = &tx.call_details;
        if let Some(index) = self.targets.iter().position(|(target, selector)| {
            *target == call.target && call.calldata.get(..4) == Some(selector.as_slice())
        }) {
            if self.history.len() == BATCH {
                self.history.pop_front();
            }
            self.history.push_back(json!({ "function": index, "reverted": reverted,
                "discarded": discarded, "new_coverage": new_coverage }));
        }
    }

    fn request(&self) -> Value {
        let questions: Map<_, _> = (0..BATCH).map(|slot| (format!("tx{slot}"), json!({
            "type": "choice",
            "instructions": format!("Choose a valid transaction production for slot {slot} of a short exploration batch. Prefer diverse useful API transitions. Sibling answers and future execution outcomes are unknown."),
            "criteria": self.criteria,
        }))).collect();
        json!({ "model": MODEL,
            "state": { "grammar": "transaction := eligible_function(random_typed_leaves | dictionary_typed_leaves)",
                "recent_execution": self.history,
                "constraints": "ABI shape does not imply semantic preconditions. Dictionary contents, calldata and storage are not provided. Calls may revert. This is local correctness testing." },
            "questions": questions })
    }

    /// Validate the entire batch before accepting any choice. Probabilities do not choose the
    /// production here: Jev's concrete `choice` replaces RNG at this grammar node.
    fn accept(&mut self, response: Value) -> Result<()> {
        let model = response["model"].as_str().ok_or_else(|| eyre!("missing model"))?;
        if model != MODEL && !model.starts_with(&format!("{MODEL}-")) {
            bail!("unexpected model");
        }
        let answers = response["answers"].as_object().ok_or_else(|| eyre!("missing answers"))?;
        if answers.len() != BATCH {
            bail!("incomplete decision batch");
        }
        let mut pending = VecDeque::new();
        for slot in 0..BATCH {
            let answer =
                answers.get(&format!("tx{slot}")).ok_or_else(|| eyre!("missing decision slot"))?;
            let choice = answer["choice"].as_str().ok_or_else(|| eyre!("missing choice"))?;
            if answer["type"] != "choice" || !self.criteria.contains_key(choice) {
                bail!("invalid grammar production");
            }
            let index = choice
                .strip_prefix('p')
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| eyre!("invalid production index"))?;
            pending.push_back(index);
        }
        self.pending = pending;
        Ok(())
    }

    pub(super) fn next(&mut self) -> Option<usize> {
        if self.disabled || self.criteria.is_empty() {
            return None;
        }
        if let Some(index) = self.pending.pop_front() {
            return Some(index);
        }
        if self.requests == MAX_REQUESTS {
            self.disable("32-request per-worker budget exhausted");
            return None;
        }
        let result = (|| {
            if self.transport.is_none() {
                self.transport = Some(Transport::new()?);
            }
            self.requests += 1;
            let response = self.transport.as_ref().expect("initialized").decide(self.request())?;
            self.accept(response)
        })();
        if result.is_err() {
            // Provider errors may contain sensitive response material. Never print them.
            self.disable("missing credential, transport failure, or invalid response");
        }
        self.pending.pop_front()
    }

    fn disable(&mut self, reason: &str) {
        if !self.disabled {
            let _ = foundry_common::sh_warn!(
                "Jev guidance disabled ({reason}); using RNG for fresh transactions"
            );
        }
        self.disabled = true;
        self.pending.clear();
        self.transport = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[test]
    fn rust_transport_sends_choices_and_reads_bounded_json() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/decisions", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        assert!(headers.starts_with("post /decisions http/1.1"));
                        assert!(headers.contains("authorization: bearer test-only"));
                        let body: Value = serde_json::from_slice(&request[end + 4..]).unwrap();
                        assert_eq!(body["questions"]["tx0"]["type"], "choice");
                        break;
                    }
                }
            }
            let body = response("p0").to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let transport = Transport::start("test-only".into(), endpoint).unwrap();
        let mut jev = grammar();
        jev.accept(transport.decide(jev.request()).unwrap()).unwrap();
        assert_eq!(jev.next(), Some(0));
        server.join().unwrap();
    }

    fn grammar() -> Jev {
        let mut jev = Jev::new();
        jev.refresh(&[
            (Address::with_last_byte(1), Function::parse("deposit(uint256)").unwrap()),
            (Address::with_last_byte(2), Function::parse("withdraw(address,uint256[])").unwrap()),
        ]);
        jev
    }

    fn response(choice: &str) -> Value {
        let answers: Map<_, _> = (0..BATCH)
            .map(|i| (format!("tx{i}"), json!({"type":"choice","choice":choice})))
            .collect();
        json!({"model":MODEL,"answers":answers})
    }

    #[test]
    fn derives_grammar_from_abi_without_addresses_or_values() {
        let jev = grammar();
        assert_eq!(jev.criteria.len(), 4);
        let request = jev.request().to_string();
        assert!(request.contains("withdraw(address,uint256[])"));
        assert!(request.contains("dictionary-backed"));
        assert!(!request.contains(&Address::with_last_byte(1).to_string()));
        assert!(!request.contains("OPENROUTER_API_KEY"));
    }

    #[test]
    fn model_choice_replaces_random_selection_and_refresh_invalidates_it() {
        let mut jev = grammar();
        jev.accept(response("p3")).unwrap();
        assert_eq!(jev.next(), Some(3));
        assert_eq!(jev.pending.len(), BATCH - 1);
        jev.refresh(&[(Address::with_last_byte(1), Function::parse("deposit(uint256)").unwrap())]);
        assert!(jev.pending.is_empty());
        assert!(jev.accept(response("p3")).is_err());
    }

    #[test]
    fn rejects_invented_and_partial_batches_atomically() {
        let mut jev = grammar();
        assert!(jev.accept(response("arbitrary_code")).is_err());
        let mut partial = response("p0");
        partial["answers"].as_object_mut().unwrap().remove("tx7");
        assert!(jev.accept(partial).is_err());
        let mut renamed = response("p0");
        let answer = renamed["answers"].as_object_mut().unwrap().remove("tx7").unwrap();
        renamed["answers"]["not_a_slot"] = answer;
        assert!(jev.accept(renamed).is_err());
        assert!(jev.pending.is_empty());
    }

    #[test]
    fn transport_failure_disables_further_requests() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_copy = count.clone();
        let mut jev = Jev::with_decider(move |_| {
            count_copy.fetch_add(1, Ordering::SeqCst);
            bail!("mock provider failure")
        });
        jev.refresh(&[(Address::ZERO, Function::parse("f()").unwrap())]);
        for _ in 0..10 {
            assert_eq!(jev.next(), None);
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_budget_never_resets_with_target_lifecycle() {
        let mut jev = grammar();
        jev.requests = MAX_REQUESTS;
        assert_eq!(jev.next(), None);
        jev.refresh(&[(Address::ZERO, Function::parse("f()").unwrap())]);
        assert_eq!(jev.next(), None);
        assert!(jev.transport.is_none());
    }
}
