//! Commonly used contract types and functions.

use crate::{compile::PathOrContractInfo, find_metadata_start, strip_bytecode_placeholders};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Event, Function, JsonAbi};
use alloy_primitives::{Address, B256, Bytes, Selector, address, hex};
use eyre::{OptionExt, Result};
use foundry_compilers::{
    ArtifactId, Project, ProjectCompileOutput,
    artifacts::{
        BytecodeObject, CompactBytecode, CompactContractBytecode, CompactContractBytecodeCow,
        CompactDeployedBytecode, ConfigurableContractArtifact, ContractBytecodeSome, Offsets,
        StorageLayout,
    },
    utils::canonicalized,
};
use std::{
    collections::BTreeMap,
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Libraries' runtime code always starts with the following instruction:
/// `PUSH20 0x0000000000000000000000000000000000000000`
///
/// See: <https://docs.soliditylang.org/en/latest/contracts.html#call-protection-for-libraries>
const CALL_PROTECTION_BYTECODE_PREFIX: [u8; 21] =
    hex!("730000000000000000000000000000000000000000");

/// Isolated account used to deploy libraries needed only while executing locally.
///
/// `address(uint160(uint256(keccak256("foundry library deployer"))))`
pub const LIBRARY_DEPLOYER: Address = address!("0x1F95D37F27EA0dEA9C252FC09D5A6eaA97647353");

/// Returns whether `creation_code` is exactly this contract's linked creation bytecode followed by
/// a complete, canonically encoded constructor argument tuple.
pub fn matches_contract_creation(contract: &ContractData, creation_code: &[u8]) -> bool {
    let Some(bytecode) = contract.bytecode() else { return false };
    let Some(arguments) = creation_code.strip_prefix(bytecode.as_ref()) else { return false };
    match contract.abi.constructor() {
        Some(constructor) => constructor
            .abi_decode_input(arguments)
            .ok()
            .and_then(|values| constructor.abi_encode_input(&values).ok())
            .is_some_and(|encoded| encoded == arguments),
        None => arguments.is_empty(),
    }
}

/// Subset of [CompactBytecode] excluding sourcemaps.
#[expect(missing_docs)]
#[derive(Debug, Clone)]
pub struct BytecodeData {
    pub object: Option<BytecodeObject>,
    pub link_references: BTreeMap<String, BTreeMap<String, Vec<Offsets>>>,
    pub immutable_references: BTreeMap<String, Vec<Offsets>>,
}

impl BytecodeData {
    fn bytes(&self) -> Option<&Bytes> {
        self.object.as_ref().and_then(|b| b.as_bytes())
    }
}

impl From<CompactBytecode> for BytecodeData {
    fn from(bytecode: CompactBytecode) -> Self {
        Self {
            object: Some(bytecode.object),
            link_references: bytecode.link_references,
            immutable_references: BTreeMap::new(),
        }
    }
}

impl From<CompactDeployedBytecode> for BytecodeData {
    fn from(bytecode: CompactDeployedBytecode) -> Self {
        let (object, link_references) = if let Some(compact) = bytecode.bytecode {
            (Some(compact.object), compact.link_references)
        } else {
            (None, BTreeMap::new())
        };
        Self { object, link_references, immutable_references: bytecode.immutable_references }
    }
}

/// Container for commonly used contract data.
#[derive(Debug, Clone)]
pub struct ContractData {
    /// Contract name.
    pub name: String,
    /// Contract ABI.
    pub abi: JsonAbi,
    /// Contract creation code.
    pub bytecode: Option<BytecodeData>,
    /// Contract runtime code.
    pub deployed_bytecode: Option<BytecodeData>,
    /// Contract storage layout, if available.
    pub storage_layout: Option<Arc<StorageLayout>>,
}

impl ContractData {
    /// Returns reference to bytes of contract creation code, if present.
    pub fn bytecode(&self) -> Option<&Bytes> {
        self.bytecode.as_ref()?.bytes().filter(|b| !b.is_empty())
    }

    /// Returns reference to bytes of contract deployed code, if present.
    pub fn deployed_bytecode(&self) -> Option<&Bytes> {
        self.deployed_bytecode.as_ref()?.bytes().filter(|b| !b.is_empty())
    }

    /// Returns the bytecode without placeholders, if present.
    pub fn bytecode_without_placeholders(&self) -> Option<Bytes> {
        strip_bytecode_placeholders(self.bytecode.as_ref()?.object.as_ref()?)
    }

    /// Returns the deployed bytecode without placeholders, if present.
    pub fn deployed_bytecode_without_placeholders(&self) -> Option<Bytes> {
        strip_bytecode_placeholders(self.deployed_bytecode.as_ref()?.object.as_ref()?)
    }
}

/// Builder for creating a `ContractsByArtifact` instance, optionally including storage layouts
/// from project compile output.
pub struct ContractsByArtifactBuilder<'a> {
    /// All compiled artifact bytecodes (borrowed).
    artifacts: BTreeMap<ArtifactId, CompactContractBytecodeCow<'a>>,
    /// Optionally collected storage layouts for matching artifact IDs.
    storage_layouts: BTreeMap<ArtifactId, StorageLayout>,
}

impl<'a> ContractsByArtifactBuilder<'a> {
    /// Creates a new builder from artifacts with present bytecode iterator.
    pub fn new(
        artifacts: impl IntoIterator<Item = (ArtifactId, CompactContractBytecodeCow<'a>)>,
    ) -> Self {
        Self { artifacts: artifacts.into_iter().collect(), storage_layouts: BTreeMap::new() }
    }

    /// Add storage layouts from the given `ProjectCompileOutput` to known artifacts.
    pub fn with_output(self, output: &ProjectCompileOutput, base: &Path) -> Self {
        self.with_storage_layouts(output.artifact_ids().filter_map(|(id, artifact)| {
            artifact
                .storage_layout
                .as_ref()
                .map(|layout| (id.with_stripped_file_prefixes(base), layout.clone()))
        }))
    }

    /// Add storage layouts.
    pub fn with_storage_layouts(
        mut self,
        layouts: impl IntoIterator<Item = (ArtifactId, StorageLayout)>,
    ) -> Self {
        self.storage_layouts.extend(layouts);
        self
    }

    /// Builds `ContractsByArtifact`.
    pub fn build(self) -> ContractsByArtifact {
        let map = self
            .artifacts
            .into_iter()
            .filter_map(|(id, artifact)| {
                let name = id.name.clone();
                let CompactContractBytecodeCow { abi, bytecode, deployed_bytecode } = artifact;

                Some((
                    id.clone(),
                    ContractData {
                        name,
                        abi: abi?.into_owned(),
                        bytecode: bytecode.map(|b| b.into_owned().into()),
                        deployed_bytecode: deployed_bytecode.map(|b| b.into_owned().into()),
                        storage_layout: self.storage_layouts.get(&id).map(|l| Arc::new(l.clone())),
                    },
                ))
            })
            .collect();

        ContractsByArtifact::from_map(map)
    }
}

type ArtifactWithContractRef<'a> = (&'a ArtifactId, &'a ContractData);

struct UnlinkedBytecodeMatch {
    bytes: Bytes,
    valid: Vec<bool>,
    has_trailing_nibble: bool,
}

impl UnlinkedBytecodeMatch {
    fn new(unlinked: &str) -> Self {
        let raw = unlinked.as_bytes();
        let mut bytes = Vec::with_capacity(raw.len() / 2);
        let mut valid = Vec::with_capacity(raw.len() / 2);
        // Decode valid byte pairs once while retaining invalid placeholder positions.
        let (pairs, trailing) = raw.as_chunks::<2>();
        for &[high, low] in pairs {
            if let (Some(high), Some(low)) = (hex_nibble(high), hex_nibble(low)) {
                bytes.push(high << 4 | low);
                valid.push(true);
            } else {
                bytes.push(0);
                valid.push(false);
            }
        }
        Self { bytes: bytes.into(), valid, has_trailing_nibble: !trailing.is_empty() }
    }

    fn matches(&self, code: &[u8], left: usize, right: usize, through_end: bool) -> bool {
        let valid = &self.valid[left..right];
        valid.iter().all(|&valid| valid)
            && (!through_end || !self.has_trailing_nibble)
            && self.bytes[left..right] == code[left..right]
    }
}

enum MatchBytecode {
    Bytecode(Bytes),
    Unlinked(UnlinkedBytecodeMatch),
}

enum DeployedCodeMatchKind {
    NoMatch,
    Partial,
    Exact,
}

#[inline]
fn match_simple_deployed_code(
    contract: &ContractData,
    code: &[u8],
    metadata_start: Option<usize>,
) -> Option<DeployedCodeMatchKind> {
    let data = contract.deployed_bytecode.as_ref()?;
    if !data.immutable_references.is_empty() || !data.link_references.is_empty() {
        return None;
    }
    let BytecodeObject::Bytecode(bytecode) = data.object.as_ref()? else { return None };
    if bytecode.starts_with(&CALL_PROTECTION_BYTECODE_PREFIX) {
        return None;
    }
    if bytecode.len() != code.len() {
        return Some(DeployedCodeMatchKind::NoMatch);
    }

    let Some(metadata) = metadata_start else {
        return Some(if bytecode.as_ref() == code {
            DeployedCodeMatchKind::Exact
        } else {
            DeployedCodeMatchKind::NoMatch
        });
    };
    if bytecode[..metadata] != code[..metadata] {
        Some(DeployedCodeMatchKind::NoMatch)
    } else if bytecode[metadata..] == code[metadata..] {
        Some(DeployedCodeMatchKind::Exact)
    } else {
        Some(DeployedCodeMatchKind::Partial)
    }
}

impl MatchBytecode {
    fn new(bytecode: &BytecodeObject) -> Self {
        match bytecode {
            BytecodeObject::Bytecode(bytes) => Self::Bytecode(bytes.clone()),
            BytecodeObject::Unlinked(unlinked) => {
                Self::Unlinked(UnlinkedBytecodeMatch::new(unlinked))
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Bytecode(bytes) => bytes.len(),
            Self::Unlinked(unlinked) => unlinked.bytes.len(),
        }
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        match self {
            Self::Bytecode(bytes) => bytes.starts_with(prefix),
            Self::Unlinked(unlinked) => {
                prefix.len() <= unlinked.bytes.len()
                    && unlinked.valid[..prefix.len()].iter().all(|&valid| valid)
                    && unlinked.bytes.starts_with(prefix)
            }
        }
    }

    fn matches(&self, code: &[u8], left: usize, right: usize, through_end: bool) -> bool {
        match self {
            Self::Bytecode(bytes) => bytes[left..right] == code[left..right],
            Self::Unlinked(unlinked) => unlinked.matches(code, left, right, through_end),
        }
    }
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct DeployedCodeMatch {
    id: ArtifactId,
    // Retain the selected data here to avoid a second artifact tree lookup after matching.
    contract: ContractData,
    bytecode: MatchBytecode,
    ignored: Vec<Offsets>,
}

impl DeployedCodeMatch {
    fn new(id: &ArtifactId, contract: &ContractData) -> Option<Self> {
        let data = contract.deployed_bytecode.as_ref()?;
        let bytecode = MatchBytecode::new(data.object.as_ref()?);
        let mut ignored = data
            .immutable_references
            .values()
            .chain(data.link_references.values().flat_map(|references| references.values()))
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        if bytecode.starts_with(&CALL_PROTECTION_BYTECODE_PREFIX) {
            ignored.push(Offsets { start: 1, length: 20 });
        }
        ignored.sort_by_key(|offset| offset.start);
        Some(Self { id: id.clone(), contract: contract.clone(), bytecode, ignored })
    }

    fn matches(&self, code: &[u8], metadata_start: Option<usize>) -> bool {
        let metadata = metadata_start
            .map(|start| Offsets { start: start as u32, length: (code.len() - start) as u32 });
        let mut metadata = metadata.as_ref();
        let mut left = 0;

        for offset in &self.ignored {
            if metadata.is_some_and(|metadata| metadata.start < offset.start) {
                let metadata = metadata.take().unwrap();
                if !self.matches_before(code, &mut left, metadata) {
                    return false;
                }
            }
            if !self.matches_before(code, &mut left, offset) {
                return false;
            }
        }
        if let Some(metadata) = metadata
            && !self.matches_before(code, &mut left, metadata)
        {
            return false;
        }

        left >= code.len() || self.bytecode.matches(code, left, code.len(), true)
    }

    fn matches_before(&self, code: &[u8], left: &mut usize, offset: &Offsets) -> bool {
        let right = offset.start as usize;
        if !self.bytecode.matches(code, *left, right, false) {
            return false;
        }
        *left = right + offset.length as usize;
        true
    }

    fn matches_metadata(&self, code: &[u8], metadata: usize) -> bool {
        self.bytecode.matches(code, metadata, code.len(), true)
    }
}

type DeployedCodeIndex = BTreeMap<usize, Vec<DeployedCodeMatch>>;

#[derive(Default)]
struct ContractsByArtifactInner {
    contracts: BTreeMap<ArtifactId, ContractData>,
    first_deployed_code: Option<DeployedCodeMatch>,
    deployed_code_index: OnceLock<DeployedCodeIndex>,
}

impl ContractsByArtifactInner {
    const fn first_deployed_code(&self) -> Option<&DeployedCodeMatch> {
        self.first_deployed_code.as_ref()
    }

    fn deployed_code_index(&self) -> &DeployedCodeIndex {
        self.deployed_code_index.get_or_init(|| {
            let mut index = DeployedCodeIndex::new();
            let first = self.first_deployed_code().map(|candidate| &candidate.id);
            // Inserting from the artifact tree retains its selection order within each bucket.
            for (id, contract) in &self.contracts {
                if first == Some(id) {
                    continue;
                }
                let Some(candidate) = DeployedCodeMatch::new(id, contract) else { continue };
                index.entry(candidate.bytecode.len()).or_default().push(candidate);
            }
            index
        })
    }
}

/// Wrapper type that maps an artifact to a contract ABI and bytecode.
#[derive(Clone, Default)]
pub struct ContractsByArtifact(Arc<ContractsByArtifactInner>);

impl fmt::Debug for ContractsByArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ContractsByArtifact").field(&self.0.contracts).finish()
    }
}

impl ContractsByArtifact {
    /// Creates a new instance by collecting all artifacts with present bytecode from an iterator.
    pub fn new(artifacts: impl IntoIterator<Item = (ArtifactId, CompactContractBytecode)>) -> Self {
        let map = artifacts
            .into_iter()
            .filter_map(|(id, artifact)| {
                let name = id.name.clone();
                let CompactContractBytecode { abi, bytecode, deployed_bytecode } = artifact;
                Some((
                    id,
                    ContractData {
                        name,
                        abi: abi?,
                        bytecode: bytecode.map(Into::into),
                        deployed_bytecode: deployed_bytecode.map(Into::into),
                        storage_layout: None,
                    },
                ))
            })
            .collect();
        Self::from_map(map)
    }

    fn from_map(contracts: BTreeMap<ArtifactId, ContractData>) -> Self {
        let first_deployed_code = contracts
            .first_key_value()
            .and_then(|(id, contract)| DeployedCodeMatch::new(id, contract));
        Self(Arc::new(ContractsByArtifactInner {
            contracts,
            first_deployed_code,
            deployed_code_index: OnceLock::new(),
        }))
    }

    /// Clears all contracts.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Finds a contract which has a similar bytecode as `code`.
    pub fn find_by_creation_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef<'_>> {
        self.find_by_code(code, 0.1, true, ContractData::bytecode)
    }

    /// Finds a contract which has a similar deployed bytecode as `code`.
    pub fn find_by_deployed_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef<'_>> {
        self.find_by_code(code, 0.15, false, ContractData::deployed_bytecode)
    }

    /// Finds a contract based on provided bytecode and accepted match score.
    /// If strip constructor args flag is true then removes args from bytecode to compare.
    fn find_by_code(
        &self,
        code: &[u8],
        accepted_score: f64,
        strip_ctor_args: bool,
        get: impl Fn(&ContractData) -> Option<&Bytes>,
    ) -> Option<ArtifactWithContractRef<'_>> {
        self.iter()
            .filter_map(|(id, contract)| {
                if let Some(deployed_bytecode) = get(contract) {
                    let mut code = code;
                    if strip_ctor_args && code.len() > deployed_bytecode.len() {
                        // Try to decode ctor args with contract abi.
                        if let Some(constructor) = contract.abi.constructor() {
                            let constructor_args = &code[deployed_bytecode.len()..];
                            if constructor.abi_decode_input(constructor_args).is_ok() {
                                // If we can decode args with current abi then remove args from
                                // code to compare.
                                code = &code[..deployed_bytecode.len()]
                            }
                        }
                    };

                    let score = bytecode_diff_score(deployed_bytecode.as_ref(), code);
                    (score <= accepted_score).then_some((score, (id, contract)))
                } else {
                    None
                }
            })
            .min_by(|(score1, _), (score2, _)| score1.total_cmp(score2))
            .map(|(_, data)| data)
    }

    /// Finds a contract which deployed bytecode exactly matches the given code. Accounts for link
    /// references and immutables.
    pub fn find_by_deployed_code_exact(&self, code: &[u8]) -> Option<ArtifactWithContractRef<'_>> {
        // Immediately return None if the code is empty.
        if code.is_empty() {
            return None;
        }

        let metadata_start = find_metadata_start(code);
        let mut partial_match = None;
        let first = self.0.contracts.iter().next();
        let mut checked_first = false;
        // Preserve the original constant-time path for a common first, fully linked artifact.
        if let Some(contract) = first
            && let Some(matched) = match_simple_deployed_code(contract.1, code, metadata_start)
        {
            checked_first = true;
            match matched {
                DeployedCodeMatchKind::Exact => return Some(contract),
                DeployedCodeMatchKind::Partial => partial_match = Some(contract),
                DeployedCodeMatchKind::NoMatch => {}
            }
        }
        if !checked_first
            && let Some(candidate) = self.0.first_deployed_code()
            && candidate.bytecode.len() == code.len()
            && candidate.matches(code, metadata_start)
        {
            let contract = first.unwrap();
            if metadata_start.is_none_or(|metadata| candidate.matches_metadata(code, metadata)) {
                return Some(contract);
            }
            partial_match = Some(contract);
        }

        let Some(candidates) = self.0.deployed_code_index().get(&code.len()) else {
            return partial_match;
        };
        for candidate in candidates {
            if !candidate.matches(code, metadata_start) {
                continue;
            }

            let contract = (&candidate.id, &candidate.contract);
            let Some(metadata) = metadata_start else { return Some(contract) };
            if candidate.matches_metadata(code, metadata) {
                return Some(contract);
            }
            partial_match = Some(contract);
        }
        partial_match
    }

    /// Finds a contract which has the same contract name or identifier as `id`. If more than one is
    /// found, return error.
    pub fn find_by_name_or_identifier(
        &self,
        id: &str,
    ) -> Result<Option<ArtifactWithContractRef<'_>>> {
        let mut iter =
            self.iter().filter(|(artifact, _)| artifact.name == id || artifact.identifier() == id);
        let first = iter.next();
        if first.is_some() && iter.next().is_some() {
            eyre::bail!("{id} has more than one implementation.");
        }

        Ok(first)
    }

    /// Finds abi by name or source path
    ///
    /// Returns the abi and the contract name.
    pub fn find_abi_by_name_or_src_path(&self, name_or_path: &str) -> Option<(JsonAbi, String)> {
        self.iter()
            .find(|(artifact, _)| {
                artifact.name == name_or_path || artifact.source == Path::new(name_or_path)
            })
            .map(|(_, contract)| (contract.abi.clone(), contract.name.clone()))
    }

    /// Flattens the contracts into functions, events and errors.
    pub fn flatten(&self) -> (BTreeMap<Selector, Function>, BTreeMap<B256, Event>, JsonAbi) {
        let mut funcs = BTreeMap::new();
        let mut events = BTreeMap::new();
        let mut errors_abi = JsonAbi::new();
        for contract in self.values() {
            for func in contract.abi.functions() {
                funcs.insert(func.selector(), func.clone());
            }
            for event in contract.abi.events() {
                events.insert(event.selector(), event.clone());
            }
            for error in contract.abi.errors() {
                errors_abi.errors.entry(error.name.clone()).or_default().push(error.clone());
            }
        }
        (funcs, events, errors_abi)
    }
}

impl From<ProjectCompileOutput> for ContractsByArtifact {
    fn from(value: ProjectCompileOutput) -> Self {
        Self::new(value.into_artifacts().map(|(id, ar)| {
            (
                id,
                CompactContractBytecode {
                    abi: ar.abi,
                    bytecode: ar.bytecode,
                    deployed_bytecode: ar.deployed_bytecode,
                },
            )
        }))
    }
}

impl Deref for ContractsByArtifact {
    type Target = BTreeMap<ArtifactId, ContractData>;

    fn deref(&self) -> &Self::Target {
        &self.0.contracts
    }
}

/// Wrapper type that maps an address to a contract identifier and contract ABI.
pub type ContractsByAddress = BTreeMap<Address, (String, JsonAbi)>;

/// Very simple fuzzy matching of contract bytecode.
///
/// Returns a value between `0.0` (identical) and `1.0` (completely different).
pub fn bytecode_diff_score<'a>(mut a: &'a [u8], mut b: &'a [u8]) -> f64 {
    // Make sure `a` is the longer one.
    if a.len() < b.len() {
        std::mem::swap(&mut a, &mut b);
    }

    // Account for different lengths.
    let mut n_different_bytes = a.len() - b.len();

    // If the difference is more than 32 bytes and more than 10% of the total length,
    // we assume the bytecodes are completely different.
    // This is a simple heuristic to avoid checking every byte when the lengths are very different.
    // 32 is chosen to be a reasonable minimum as it's the size of metadata hashes and one EVM word.
    if n_different_bytes > 32 && n_different_bytes * 10 > a.len() {
        return 1.0;
    }

    // Count different bytes.
    // SAFETY: `a` is longer than `b`.
    n_different_bytes += unsafe { count_different_bytes(a, b) };

    n_different_bytes as f64 / a.len() as f64
}

/// Returns the amount of different bytes between two slices.
///
/// # Safety
///
/// `a` must be at least as long as `b`.
const unsafe fn count_different_bytes(a: &[u8], b: &[u8]) -> usize {
    // This could've been written as `std::iter::zip(a, b).filter(|(x, y)| x != y).count()`,
    // however this function is very hot, and has been written to be as primitive as
    // possible for lower optimization levels.

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let len = b.len();

    let mut sum = 0;
    let mut i = 0;
    while i < len {
        // SAFETY: `a` is at least as long as `b`, and `i` is in bound of `b`.
        sum += unsafe { *a_ptr.add(i) != *b_ptr.add(i) } as usize;
        i += 1;
    }
    sum
}

/// Returns contract name for a given contract identifier.
///
/// Artifact/Contract identifier can take the following form:
/// `<artifact file name>:<contract name>`, the `artifact file name` is the name of the json file of
/// the contract's artifact and the contract name is the name of the solidity contract, like
/// `SafeTransferLibTest.json:SafeTransferLibTest`
///
/// This returns the `contract name` part
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest",
///     get_contract_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_contract_name(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// This returns the `file name` part, See [`get_contract_name`]
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest.json",
///     get_file_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_file_name(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// Helper function to convert CompactContractBytecode ~> ContractBytecodeSome
pub fn compact_to_contract(contract: CompactContractBytecode) -> Result<ContractBytecodeSome> {
    Ok(ContractBytecodeSome {
        abi: contract.abi.ok_or_else(|| eyre::eyre!("No contract abi"))?,
        bytecode: contract.bytecode.ok_or_else(|| eyre::eyre!("No contract bytecode"))?.into(),
        deployed_bytecode: contract
            .deployed_bytecode
            .ok_or_else(|| eyre::eyre!("No contract deployed bytecode"))?
            .into(),
    })
}

/// Returns the canonicalized target path for the given identifier.
pub fn find_target_path(project: &Project, identifier: &PathOrContractInfo) -> Result<PathBuf> {
    match identifier {
        PathOrContractInfo::Path(path) => Ok(canonicalized(project.root().join(path))),
        PathOrContractInfo::ContractInfo(info) => {
            if let Some(path) = info.path.as_ref() {
                let path = canonicalized(project.root().join(path));
                if !path.is_file() {
                    eyre::bail!(
                        "Could not find source file for contract `{}` at {}",
                        info.name,
                        path.strip_prefix(project.root()).unwrap_or(&path).display()
                    );
                }
                return Ok(path);
            }
            // If ContractInfo.path hasn't been provided we try to find the contract using the name.
            // This will fail if projects have multiple contracts with the same name. In that case,
            // path must be specified.
            let path = project.find_contract_path(&info.name)?;
            Ok(path)
        }
    }
}

/// Returns the target artifact given the path and name.
pub fn find_matching_contract_artifact(
    output: &mut ProjectCompileOutput,
    target_path: &Path,
    target_name: Option<&str>,
) -> eyre::Result<ConfigurableContractArtifact> {
    if let Some(name) = target_name {
        if let Some(artifact) = output.remove(target_path, name) {
            return Ok(artifact);
        }

        let target_path = canonicalized(target_path);
        let matching_source = output.artifact_ids().find_map(|(id, _artifact)| {
            (id.name == name && canonicalized(&id.source) == target_path).then(|| id.source.clone())
        });

        matching_source
            .and_then(|source| output.remove(&source, name))
            .ok_or_eyre(format!("Could not find artifact `{name}` in the compiled artifacts"))
    } else {
        let possible_targets = output
            .artifact_ids()
            .filter(|(id, _artifact)| id.source == target_path)
            .collect::<Vec<_>>();

        if possible_targets.is_empty() {
            eyre::bail!(
                "Could not find artifact linked to source `{target_path:?}` in the compiled artifacts"
            );
        }

        let (target_id, target_artifact) = possible_targets[0].clone();
        if possible_targets.len() == 1 {
            return Ok(target_artifact.clone());
        }

        // If all artifact_ids in `possible_targets` have the same name (without ".", indicates
        // additional compiler profiles), it means that there are multiple contracts in the
        // same file.
        if !target_id.name.contains('.')
            && possible_targets.iter().any(|(id, _)| id.name != target_id.name)
        {
            eyre::bail!(
                "Multiple contracts found in the same file, please specify the target <path>:<contract> or <contract>"
            );
        }

        // Otherwise, we're dealing with additional compiler profiles wherein `id.source` is the
        // same but `id.path` is different.
        let artifact = possible_targets
            .iter()
            .find_map(|(id, artifact)| (id.profile == "default").then_some(*artifact))
            .unwrap_or(target_artifact);

        Ok(artifact.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::DynSolValue;
    use alloy_primitives::U256;
    use proptest::prelude::*;
    use std::str::FromStr;

    fn artifact_id(index: usize) -> ArtifactId {
        let name = format!("Contract{index:05}");
        ArtifactId {
            path: format!("out/{name}.sol/{name}.json").into(),
            name: name.clone(),
            source: format!("src/{name}.sol").into(),
            version: "0.8.30".parse().unwrap(),
            build_id: "test".to_owned(),
            profile: "default".to_owned(),
        }
    }

    fn artifact(
        index: usize,
        object: BytecodeObject,
        link_references: Vec<Offsets>,
        immutable_references: Vec<Offsets>,
    ) -> (ArtifactId, CompactContractBytecode) {
        let link_references = if link_references.is_empty() {
            BTreeMap::new()
        } else {
            BTreeMap::from([(
                "src/Library.sol".to_owned(),
                BTreeMap::from([("Library".to_owned(), link_references)]),
            )])
        };
        let immutable_references = if immutable_references.is_empty() {
            BTreeMap::new()
        } else {
            BTreeMap::from([("0".to_owned(), immutable_references)])
        };
        let deployed_bytecode = CompactDeployedBytecode {
            bytecode: Some(CompactBytecode { object, source_map: None, link_references }),
            immutable_references,
        };
        (
            artifact_id(index),
            CompactContractBytecode {
                abi: Some(JsonAbi::new()),
                bytecode: None,
                deployed_bytecode: Some(deployed_bytecode),
            },
        )
    }

    fn find_by_deployed_code_exact_linear<'a>(
        contracts: &'a ContractsByArtifact,
        code: &[u8],
    ) -> Option<ArtifactWithContractRef<'a>> {
        if code.is_empty() {
            return None;
        }

        let mut partial_match = None;
        contracts
            .iter()
            .find(|(id, contract)| {
                let Some(deployed_bytecode) = &contract.deployed_bytecode else {
                    return false;
                };
                let Some(deployed_code) = &deployed_bytecode.object else {
                    return false;
                };
                let len = match deployed_code {
                    BytecodeObject::Bytecode(bytes) => bytes.len(),
                    BytecodeObject::Unlinked(bytes) => bytes.len() / 2,
                };
                if len != code.len() {
                    return false;
                }

                let mut ignored = deployed_bytecode
                    .immutable_references
                    .values()
                    .chain(deployed_bytecode.link_references.values().flat_map(|v| v.values()))
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>();
                let has_call_protection = match deployed_code {
                    BytecodeObject::Bytecode(bytes) => {
                        bytes.starts_with(&CALL_PROTECTION_BYTECODE_PREFIX)
                    }
                    BytecodeObject::Unlinked(bytes) => {
                        Bytes::from_str(&bytes[..CALL_PROTECTION_BYTECODE_PREFIX.len() * 2])
                            .is_ok_and(|bytes| bytes.starts_with(&CALL_PROTECTION_BYTECODE_PREFIX))
                    }
                };
                if has_call_protection {
                    ignored.push(Offsets { start: 1, length: 20 });
                }

                let metadata_start = find_metadata_start(code);
                if let Some(metadata) = metadata_start {
                    ignored.push(Offsets {
                        start: metadata as u32,
                        length: (code.len() - metadata) as u32,
                    });
                }
                ignored.sort_by_key(|offset| offset.start);

                let mut left = 0;
                for offset in ignored {
                    let right = offset.start as usize;
                    let matched = match deployed_code {
                        BytecodeObject::Bytecode(bytes) => bytes[left..right] == code[left..right],
                        BytecodeObject::Unlinked(bytes) => {
                            Bytes::from_str(&bytes[left * 2..right * 2])
                                .is_ok_and(|bytes| bytes == code[left..right])
                        }
                    };
                    if !matched {
                        return false;
                    }
                    left = right + offset.length as usize;
                }

                let is_partial = left >= code.len()
                    || match deployed_code {
                        BytecodeObject::Bytecode(bytes) => bytes[left..] == code[left..],
                        BytecodeObject::Unlinked(bytes) => Bytes::from_str(&bytes[left * 2..])
                            .is_ok_and(|bytes| bytes == code[left..]),
                    };
                if !is_partial {
                    return false;
                }

                let Some(metadata) = metadata_start else { return true };
                let exact_match = match deployed_code {
                    BytecodeObject::Bytecode(bytes) => bytes[metadata..] == code[metadata..],
                    BytecodeObject::Unlinked(bytes) => Bytes::from_str(&bytes[metadata * 2..])
                        .is_ok_and(|bytes| bytes == code[metadata..]),
                };
                if exact_match {
                    true
                } else {
                    partial_match = Some((*id, *contract));
                    false
                }
            })
            .or(partial_match)
    }

    fn selected_id(result: Option<ArtifactWithContractRef<'_>>) -> Option<ArtifactId> {
        result.map(|(id, _)| id.clone())
    }

    #[test]
    fn exact_creation_match_requires_canonical_constructor_suffix() {
        let abi = JsonAbi::parse(["constructor(uint256 value)"]).unwrap();
        let bytecode = Bytes::from_static(&[0x60, 0x00]);
        let contract = ContractData {
            name: "C".to_owned(),
            abi,
            bytecode: Some(BytecodeData {
                object: Some(BytecodeObject::Bytecode(bytecode.clone())),
                link_references: BTreeMap::new(),
                immutable_references: BTreeMap::new(),
            }),
            deployed_bytecode: None,
            storage_layout: None,
        };
        let arguments = contract
            .abi
            .constructor()
            .unwrap()
            .abi_encode_input(&[DynSolValue::Uint(U256::from(1), 256)])
            .unwrap();
        let creation = [bytecode.as_ref(), &arguments].concat();

        assert!(matches_contract_creation(&contract, &creation));
        assert!(!matches_contract_creation(&contract, &creation[..creation.len() - 1]));
        assert!(!matches_contract_creation(&contract, &[creation, vec![0]].concat()));
    }

    #[test]
    fn bytecode_diffing() {
        assert_eq!(bytecode_diff_score(b"a", b"a"), 0.0);
        assert_eq!(bytecode_diff_score(b"a", b"b"), 1.0);

        let a_100 = &b"a".repeat(100)[..];
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(100)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(99)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(101)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(120)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(1000)), 1.0);

        let a_99 = &b"a".repeat(99)[..];
        assert!(bytecode_diff_score(a_100, a_99) <= 0.01);
    }

    #[test]
    fn find_by_deployed_code_exact_with_empty_deployed() {
        let contracts = ContractsByArtifact::new(vec![]);

        assert!(contracts.find_by_deployed_code_exact(&[]).is_none());
    }

    #[test]
    fn exact_match_index_handles_all_dynamic_regions() {
        let mut query = vec![0x60; 64];
        query[0] = 0x73;
        query[61..].copy_from_slice(&[0xa0, 0x00, 0x01]);

        let exact =
            artifact(0, BytecodeObject::Bytecode(query.clone().into()), Vec::new(), Vec::new());

        let mut immutable = query.clone();
        immutable[30..34].fill(0xaa);
        let immutable = artifact(
            1,
            BytecodeObject::Bytecode(immutable.into()),
            Vec::new(),
            vec![Offsets { start: 30, length: 4 }],
        );

        let mut linked = query.clone();
        linked[5..25].fill(0xbb);
        let linked = artifact(
            2,
            BytecodeObject::Bytecode(linked.into()),
            vec![Offsets { start: 5, length: 20 }],
            Vec::new(),
        );

        let mut unlinked = hex::encode(&query);
        unlinked.replace_range(10..50, "__$0123456789abcdef0123456789abcdef01$__");
        let unlinked = artifact(
            3,
            BytecodeObject::Unlinked(unlinked),
            vec![Offsets { start: 5, length: 20 }],
            Vec::new(),
        );

        let mut library = query.clone();
        library[1..21].fill(0);
        let library = artifact(4, BytecodeObject::Bytecode(library.into()), Vec::new(), Vec::new());

        let mut combined = query.clone();
        combined[30..34].fill(0xcc);
        let mut combined = hex::encode(combined);
        combined.replace_range(10..50, "__$0123456789abcdef0123456789abcdef01$__");
        let combined = artifact(
            5,
            BytecodeObject::Unlinked(combined),
            vec![Offsets { start: 5, length: 20 }],
            vec![Offsets { start: 30, length: 4 }],
        );

        for artifact in [exact, immutable, linked, unlinked, library, combined] {
            let contracts = ContractsByArtifact::new([artifact]);
            assert_eq!(
                selected_id(contracts.find_by_deployed_code_exact(&query)),
                selected_id(find_by_deployed_code_exact_linear(&contracts, &query))
            );
            assert!(contracts.find_by_deployed_code_exact(&query).is_some());
        }
    }

    #[test]
    fn exact_match_index_preserves_exact_and_partial_order() {
        let query = [vec![0x60; 32], vec![0xa0, 0x00, 0x01]].concat();
        let mut first_partial = query.clone();
        first_partial[32] = 0xa1;
        let mut last_partial = query.clone();
        last_partial[32] = 0xa2;

        let contracts = ContractsByArtifact::new([
            artifact(
                0,
                BytecodeObject::Bytecode(first_partial.clone().into()),
                Vec::new(),
                Vec::new(),
            ),
            artifact(
                1,
                BytecodeObject::Bytecode(last_partial.clone().into()),
                Vec::new(),
                Vec::new(),
            ),
            artifact(2, BytecodeObject::Bytecode(query.clone().into()), Vec::new(), Vec::new()),
            artifact(3, BytecodeObject::Bytecode(query.clone().into()), Vec::new(), Vec::new()),
        ]);
        let exact = contracts.find_by_deployed_code_exact(&query).unwrap().0;
        assert_eq!(exact, &artifact_id(2));
        assert_eq!(
            selected_id(Some((exact, contracts.get(exact).unwrap()))),
            selected_id(find_by_deployed_code_exact_linear(&contracts, &query))
        );

        let contracts = ContractsByArtifact::new([
            artifact(0, BytecodeObject::Bytecode(first_partial.into()), Vec::new(), Vec::new()),
            artifact(1, BytecodeObject::Bytecode(last_partial.into()), Vec::new(), Vec::new()),
        ]);
        let partial = contracts.find_by_deployed_code_exact(&query).unwrap().0;
        assert_eq!(partial, &artifact_id(1));
        assert_eq!(
            selected_id(Some((partial, contracts.get(partial).unwrap()))),
            selected_id(find_by_deployed_code_exact_linear(&contracts, &query))
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn exact_match_index_is_equivalent_to_linear_search(
            inputs in prop::collection::vec(
                (prop::collection::vec(any::<u8>(), 24..96), 0u8..3, any::<u8>()),
                1..24,
            ),
            selected in any::<usize>(),
            mutations in prop::collection::vec((any::<usize>(), any::<u8>()), 0..8),
            query_mode in 0u8..4,
        ) {
            let mut query = inputs[selected % inputs.len()].0.clone();
            for (position, value) in mutations {
                let position = position % query.len();
                query[position] = value;
            }
            match query_mode {
                1 => {
                    let len = query.len();
                    query[len - 3..].copy_from_slice(&[0xa0, 0x00, 0x01]);
                }
                2 => {
                    query.pop();
                }
                3 => query.push(0xff),
                _ => {}
            }

            let artifacts = inputs.into_iter().enumerate().map(|(index, (bytes, kind, seed))| {
                let body_end = bytes.len() - 3;
                match kind {
                    1 => {
                        let start = usize::from(seed) % body_end;
                        let length = (body_end - start).min(8);
                        artifact(
                            index,
                            BytecodeObject::Bytecode(bytes.into()),
                            Vec::new(),
                            vec![Offsets { start: start as u32, length: length as u32 }],
                        )
                    }
                    2 => {
                        let start = usize::from(seed) % (body_end - 20 + 1);
                        let mut unlinked = hex::encode(bytes);
                        unlinked.replace_range(
                            start * 2..(start + 20) * 2,
                            "__$0123456789abcdef0123456789abcdef01$__",
                        );
                        artifact(
                            index,
                            BytecodeObject::Unlinked(unlinked),
                            vec![Offsets { start: start as u32, length: 20 }],
                            Vec::new(),
                        )
                    }
                    _ => artifact(
                        index,
                        BytecodeObject::Bytecode(bytes.into()),
                        Vec::new(),
                        Vec::new(),
                    ),
                }
            });
            let contracts = ContractsByArtifact::new(artifacts);

            prop_assert_eq!(
                selected_id(contracts.find_by_deployed_code_exact(&query)),
                selected_id(find_by_deployed_code_exact_linear(&contracts, &query)),
            );
        }
    }
}
