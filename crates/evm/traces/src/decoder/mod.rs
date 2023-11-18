use crate::{
    identifier::{AddressIdentity, SingleSignaturesIdentifier, TraceIdentifier},
    CallTrace, CallTraceArena, TraceCallData, TraceLog, TraceRetData,
};
use alloy_dyn_abi::{DecodedEvent, DynSolValue, EventExt, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Event, Function, JsonAbi as Abi};
use alloy_primitives::{Address, Selector, B256};
use foundry_common::{abi::get_indexed_event, fmt::format_token, SELECTOR_LEN};
use foundry_evm_core::{
    abi::{CONSOLE_ABI, HARDHAT_CONSOLE_ABI, HEVM_ABI},
    constants::{
        CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS,
        TEST_CONTRACT_ADDRESS,
    },
    decode,
};
use foundry_utils::types::ToAlloy;
use itertools::Itertools;
use once_cell::sync::OnceCell;
use std::collections::{hash_map::Entry, BTreeMap, HashMap};

mod precompiles;

/// Build a new [CallTraceDecoder].
#[derive(Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct CallTraceDecoderBuilder {
    decoder: CallTraceDecoder,
}

impl CallTraceDecoderBuilder {
    /// Create a new builder.
    #[inline]
    pub fn new() -> Self {
        Self { decoder: CallTraceDecoder::new().clone() }
    }

    /// Add known labels to the decoder.
    #[inline]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = (Address, String)>) -> Self {
        self.decoder.labels.extend(labels);
        self
    }

    /// Add known events to the decoder.
    #[inline]
    pub fn with_events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        for event in events {
            self.decoder
                .events
                .entry((event.selector(), indexed_inputs(&event)))
                .or_default()
                .push(event);
        }
        self
    }

    /// Sets the verbosity level of the decoder.
    #[inline]
    pub fn with_verbosity(mut self, level: u8) -> Self {
        self.decoder.verbosity = level;
        self
    }

    /// Sets the signature identifier for events and functions.
    #[inline]
    pub fn with_signature_identifier(mut self, identifier: SingleSignaturesIdentifier) -> Self {
        self.decoder.signature_identifier = Some(identifier);
        self
    }

    /// Build the decoder.
    #[inline]
    pub fn build(self) -> CallTraceDecoder {
        self.decoder
    }
}

/// The call trace decoder.
///
/// The decoder collects address labels and ABIs from any number of [TraceIdentifier]s, which it
/// then uses to decode the call trace.
///
/// Note that a call trace decoder is required for each new set of traces, since addresses in
/// different sets might overlap.
#[derive(Clone, Default, Debug)]
pub struct CallTraceDecoder {
    /// Addresses identified to be a specific contract.
    ///
    /// The values are in the form `"<artifact>:<contract>"`.
    pub contracts: HashMap<Address, String>,
    /// Address labels.
    pub labels: HashMap<Address, String>,
    /// Contract addresses that have a receive function.
    pub receive_contracts: Vec<Address>,
    /// All known functions.
    pub functions: HashMap<Selector, Vec<Function>>,
    /// All known events.
    pub events: BTreeMap<(B256, usize), Vec<Event>>,
    /// All known errors.
    pub errors: Abi,
    /// A signature identifier for events and functions.
    pub signature_identifier: Option<SingleSignaturesIdentifier>,
    /// Verbosity level
    pub verbosity: u8,
}

impl CallTraceDecoder {
    /// Creates a new call trace decoder.
    ///
    /// The call trace decoder always knows how to decode calls to the cheatcode address, as well
    /// as DSTest-style logs.
    pub fn new() -> &'static Self {
        // If you want to take arguments in this function, assign them to the fields of the cloned
        // lazy instead of removing it
        static INIT: OnceCell<CallTraceDecoder> = OnceCell::new();
        INIT.get_or_init(Self::init)
    }

    fn init() -> Self {
        Self {
            contracts: Default::default(),

            labels: [
                (CHEATCODE_ADDRESS, "VM".to_string()),
                (HARDHAT_CONSOLE_ADDRESS, "console".to_string()),
                (DEFAULT_CREATE2_DEPLOYER, "Create2Deployer".to_string()),
                (CALLER, "DefaultSender".to_string()),
                (TEST_CONTRACT_ADDRESS, "DefaultTestContract".to_string()),
            ]
            .into(),

            functions: HARDHAT_CONSOLE_ABI
                .functions()
                .chain(HEVM_ABI.functions())
                .map(|func| {
                    let func = func.clone().to_alloy();
                    (func.selector(), vec![func])
                })
                .collect(),

            events: CONSOLE_ABI
                .events()
                .map(|event| {
                    let event = event.clone().to_alloy();
                    ((event.selector(), indexed_inputs(&event)), vec![event])
                })
                .collect(),

            errors: Default::default(),
            signature_identifier: None,
            receive_contracts: Default::default(),
            verbosity: 0,
        }
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    #[inline]
    pub fn identify(&mut self, trace: &CallTraceArena, identifier: &mut impl TraceIdentifier) {
        self.collect_identities(identifier.identify_addresses(self.addresses(trace)));
    }

    #[inline(always)]
    fn addresses<'a>(
        &'a self,
        trace: &'a CallTraceArena,
    ) -> impl Iterator<Item = (&'a Address, Option<&'a [u8]>)> + 'a {
        trace.addresses().into_iter().filter(|&(address, _)| {
            !self.labels.contains_key(address) || !self.contracts.contains_key(address)
        })
    }

    fn collect_identities(&mut self, identities: Vec<AddressIdentity<'_>>) {
        for identity in identities {
            let address = identity.address;

            if let Some(contract) = &identity.contract {
                self.contracts.entry(address).or_insert_with(|| contract.to_string());
            }

            if let Some(label) = &identity.label {
                self.labels.entry(address).or_insert_with(|| label.to_string());
            }

            if let Some(abi) = &identity.abi {
                // Store known functions for the address
                for function in abi.functions() {
                    match self.functions.entry(function.selector()) {
                        Entry::Occupied(entry) => {
                            // This shouldn't happen that often
                            debug!(target: "evm::traces", selector=%entry.key(), old=?entry.get(), new=?function, "Duplicate function");
                            entry.into_mut().push(function.clone());
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(vec![function.clone()]);
                        }
                    }
                }

                // Flatten events from all ABIs
                for event in abi.events() {
                    let sig = (event.selector(), indexed_inputs(event));
                    self.events.entry(sig).or_default().push(event.clone());
                }

                // Flatten errors from all ABIs
                for error in abi.errors() {
                    self.errors.errors.entry(error.name.clone()).or_default().push(error.clone());
                }

                if abi.receive.is_some() {
                    self.receive_contracts.push(address);
                }
            }
        }
    }

    /// Decodes all nodes in the specified call trace.
    pub async fn decode(&self, traces: &mut CallTraceArena) {
        for node in &mut traces.arena {
            self.decode_function(&mut node.trace).await;
            for log in node.logs.iter_mut() {
                self.decode_event(log).await;
            }
        }
    }

    async fn decode_function(&self, trace: &mut CallTrace) {
        // Decode precompile
        if precompiles::decode(trace, 1) {
            return
        }

        // Set label
        if trace.label.is_none() {
            if let Some(label) = self.labels.get(&trace.address) {
                trace.label = Some(label.clone());
            }
        }

        // Set contract name
        if trace.contract.is_none() {
            if let Some(contract) = self.contracts.get(&trace.address) {
                trace.contract = Some(contract.clone());
            }
        }

        let TraceCallData::Raw(cdata) = &trace.data else { return };

        if trace.address == DEFAULT_CREATE2_DEPLOYER {
            trace.data = TraceCallData::Decoded { signature: "create2".to_string(), args: vec![] };
            return
        }

        if cdata.len() >= SELECTOR_LEN {
            let selector = &cdata[..SELECTOR_LEN];
            let mut functions = Vec::new();
            let functions = match self.functions.get(selector) {
                Some(fs) => fs,
                None => {
                    if let Some(identifier) = &self.signature_identifier {
                        if let Some(function) =
                            identifier.write().await.identify_function(selector).await
                        {
                            functions.push(function);
                        }
                    }
                    &functions
                }
            };
            let [func, ..] = &functions[..] else { return };
            self.decode_function_input(trace, func);
            self.decode_function_output(trace, functions);
        } else {
            let has_receive = self.receive_contracts.contains(&trace.address);
            let signature =
                if cdata.is_empty() && has_receive { "receive()" } else { "fallback()" }.into();
            let args = if cdata.is_empty() { Vec::new() } else { vec![cdata.to_string()] };
            trace.data = TraceCallData::Decoded { signature, args };

            if let TraceRetData::Raw(rdata) = &trace.output {
                if !trace.success {
                    trace.output = TraceRetData::Decoded(decode::decode_revert(
                        rdata,
                        Some(&self.errors),
                        Some(trace.status),
                    ));
                }
            }
        }
    }

    /// Decodes a function's input into the given trace.
    fn decode_function_input(&self, trace: &mut CallTrace, func: &Function) {
        let TraceCallData::Raw(data) = &trace.data else { return };
        let mut args = None;
        if data.len() >= SELECTOR_LEN {
            if trace.address == CHEATCODE_ADDRESS {
                // Try to decode cheatcode inputs in a more custom way
                if let Some(v) = self.decode_cheatcode_inputs(func, data) {
                    args = Some(v);
                }
            }

            if args.is_none() {
                if let Ok(v) = func.abi_decode_input(&data[SELECTOR_LEN..], false) {
                    args = Some(v.iter().map(|value| self.apply_label(value)).collect());
                }
            }
        }
        trace.data =
            TraceCallData::Decoded { signature: func.signature(), args: args.unwrap_or_default() };
    }

    /// Custom decoding for cheatcode inputs.
    fn decode_cheatcode_inputs(&self, func: &Function, data: &[u8]) -> Option<Vec<String>> {
        match func.name.as_str() {
            "expectRevert" => Some(vec![decode::decode_revert(data, Some(&self.errors), None)]),
            "rememberKey" | "addr" | "startBroadcast" | "broadcast" => {
                // these functions accept a private key as uint256, which should not be
                // converted to plain text
                if !func.inputs.is_empty() && func.inputs[0].ty != "uint256" {
                    // redact private key input
                    Some(vec!["<pk>".to_string()])
                } else {
                    None
                }
            }
            "sign" => {
                // sign(uint256,bytes32)
                let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..], false).ok()?;
                if !decoded.is_empty() && func.inputs[0].ty != "uint256" {
                    decoded[0] = DynSolValue::String("<pk>".to_string());
                }
                Some(decoded.iter().map(format_token).collect())
            }
            "deriveKey" => Some(vec!["<pk>".to_string()]),
            "parseJson" |
            "parseJsonUint" |
            "parseJsonUintArray" |
            "parseJsonInt" |
            "parseJsonIntArray" |
            "parseJsonString" |
            "parseJsonStringArray" |
            "parseJsonAddress" |
            "parseJsonAddressArray" |
            "parseJsonBool" |
            "parseJsonBoolArray" |
            "parseJsonBytes" |
            "parseJsonBytesArray" |
            "parseJsonBytes32" |
            "parseJsonBytes32Array" |
            "writeJson" |
            "keyExists" |
            "serializeBool" |
            "serializeUint" |
            "serializeInt" |
            "serializeAddress" |
            "serializeBytes32" |
            "serializeString" |
            "serializeBytes" => {
                if self.verbosity >= 5 {
                    None
                } else {
                    let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..], false).ok()?;
                    let token =
                        if func.name.as_str() == "parseJson" || func.name.as_str() == "keyExists" {
                            "<JSON file>"
                        } else {
                            "<stringified JSON>"
                        };
                    decoded[0] = DynSolValue::String(token.to_string());
                    Some(decoded.iter().map(format_token).collect())
                }
            }
            _ => None,
        }
    }

    /// Decodes a function's output into the given trace.
    fn decode_function_output(&self, trace: &mut CallTrace, funcs: &[Function]) {
        let TraceRetData::Raw(data) = &trace.output else { return };
        if trace.success {
            if trace.address == CHEATCODE_ADDRESS {
                if let Some(decoded) =
                    funcs.iter().find_map(|func| self.decode_cheatcode_outputs(func))
                {
                    trace.output = TraceRetData::Decoded(decoded);
                    return
                }
            }

            if let Some(values) =
                funcs.iter().find_map(|func| func.abi_decode_output(data, false).ok())
            {
                // Functions coming from an external database do not have any outputs specified,
                // and will lead to returning an empty list of values.
                if values.is_empty() {
                    return
                }
                trace.output = TraceRetData::Decoded(
                    values.iter().map(|value| self.apply_label(value)).format(", ").to_string(),
                );
            }
        } else {
            trace.output = TraceRetData::Decoded(decode::decode_revert(
                data,
                Some(&self.errors),
                Some(trace.status),
            ));
        }
    }

    /// Custom decoding for cheatcode outputs.
    fn decode_cheatcode_outputs(&self, func: &Function) -> Option<String> {
        match func.name.as_str() {
            s if s.starts_with("env") => Some("<env var value>"),
            "deriveKey" => Some("<pk>"),
            "parseJson" if self.verbosity < 5 => Some("<encoded JSON value>"),
            "readFile" if self.verbosity < 5 => Some("<file>"),
            _ => None,
        }
        .map(Into::into)
    }

    /// Decodes an event.
    async fn decode_event(&self, log: &mut TraceLog) {
        let TraceLog::Raw(raw_log) = log else { return };
        let &[t0, ..] = raw_log.topics() else { return };

        let mut events = Vec::new();
        let events = match self.events.get(&(t0, raw_log.topics().len() - 1)) {
            Some(es) => es,
            None => {
                if let Some(identifier) = &self.signature_identifier {
                    if let Some(event) = identifier.write().await.identify_event(&t0[..]).await {
                        events.push(get_indexed_event(event, raw_log));
                    }
                }
                &events
            }
        };
        for event in events {
            if let Ok(decoded) = event.decode_log(raw_log, false) {
                let params = reconstruct_params(event, &decoded);
                *log = TraceLog::Decoded(
                    event.name.clone(),
                    params
                        .into_iter()
                        .zip(event.inputs.iter())
                        .map(|(param, input)| {
                            // undo patched names
                            let name = input.name.clone();
                            (name, self.apply_label(&param))
                        })
                        .collect(),
                );
                break
            }
        }
    }

    fn apply_label(&self, value: &DynSolValue) -> String {
        if let DynSolValue::Address(addr) = value {
            if let Some(label) = self.labels.get(addr) {
                return format!("{label}: [{addr}]");
            }
        }
        format_token(value)
    }
}

/// Restore the order of the params of a decoded event,
/// as Alloy returns the indexed and unindexed params separately.
fn reconstruct_params(event: &Event, decoded: &DecodedEvent) -> Vec<DynSolValue> {
    let mut indexed = 0;
    let mut unindexed = 0;
    let mut inputs = vec![];
    for input in event.inputs.iter() {
        if input.indexed {
            inputs.push(decoded.indexed[indexed].clone());
            indexed += 1;
        } else {
            inputs.push(decoded.body[unindexed].clone());
            unindexed += 1;
        }
    }

    inputs
}

fn indexed_inputs(event: &Event) -> usize {
    event.inputs.iter().filter(|param| param.indexed).count()
}
