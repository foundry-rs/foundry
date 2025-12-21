//! Uncategorised utilities.

use alloy_primitives::{B256, Bytes, U256, hex, keccak256};
use foundry_compilers::{
    Project,
    artifacts::{BytecodeObject, SolcLanguage},
    error::SolcError,
    flatten::{Flattener, FlattenerError},
};
use regex::Regex;
use std::{path::Path, sync::LazyLock};

static BYTECODE_PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"__\$.{34}\$__").expect("invalid regex"));

/// Block on a future using the current tokio runtime on the current thread.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    block_on_handle(&tokio::runtime::Handle::current(), future)
}

/// Block on a future using the current tokio runtime on the current thread with the given handle.
pub fn block_on_handle<F: std::future::Future>(
    handle: &tokio::runtime::Handle,
    future: F,
) -> F::Output {
    tokio::task::block_in_place(|| handle.block_on(future))
}

/// Computes the storage slot as specified by `ERC-7201`, using the `erc7201` formula ID.
///
/// This is defined as:
///
/// ```text
/// erc7201(id: string) = keccak256(keccak256(id) - 1) & ~0xff
/// ```
///
/// # Examples
///
/// ```
/// use alloy_primitives::b256;
/// use foundry_common::erc7201;
///
/// assert_eq!(
///     erc7201("example.main"),
///     b256!("0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500"),
/// );
/// ```
pub fn erc7201(id: &str) -> B256 {
    let x = U256::from_be_bytes(keccak256(id).0) - U256::from(1);
    keccak256(x.to_be_bytes::<32>()) & B256::from(!U256::from(0xff))
}

/// Utility function to find the start of the metadata in the bytecode.
/// This assumes that the metadata is at the end of the bytecode.
pub fn find_metadata_start(bytecode: &[u8]) -> Option<usize> {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata.
    let (rest, metadata_len_bytes) = bytecode.split_last_chunk()?;
    let metadata_len = u16::from_be_bytes(*metadata_len_bytes) as usize;
    if metadata_len > rest.len() {
        return None;
    }
    ciborium::from_reader::<ciborium::Value, _>(&rest[rest.len() - metadata_len..])
        .is_ok()
        .then(|| rest.len() - metadata_len)
}

/// Utility function to ignore metadata hash of the given bytecode.
/// This assumes that the metadata is at the end of the bytecode.
pub fn ignore_metadata_hash(bytecode: &[u8]) -> &[u8] {
    if let Some(metadata) = find_metadata_start(bytecode) {
        &bytecode[..metadata]
    } else {
        bytecode
    }
}

/// Strips all __$xxx$__ placeholders from the bytecode if it's an unlinked bytecode.
/// by replacing them with 20 zero bytes.
/// This is useful for matching bytecodes to a contract source, and for the source map,
/// in which the actual address of the placeholder isn't important.
pub fn strip_bytecode_placeholders(bytecode: &BytecodeObject) -> Option<Bytes> {
    match &bytecode {
        BytecodeObject::Bytecode(bytes) => Some(bytes.clone()),
        BytecodeObject::Unlinked(s) => {
            // Replace all __$xxx$__ placeholders with 20 zero bytes (40 hex chars)
            let s = (*BYTECODE_PLACEHOLDER_RE).replace_all(s, "00".repeat(20));
            let bytes = hex::decode(s.as_bytes());
            Some(bytes.ok()?.into())
        }
    }
}

/// Flattens the given target of the project. Falls back to the old flattening implementation
/// if the target cannot be compiled successfully. This would be the case if the target has invalid
/// syntax. (e.g. Solang)
pub fn flatten(project: Project, target_path: &Path) -> eyre::Result<String> {
    // Save paths for fallback before Flattener::new takes ownership
    let paths = project.paths.clone();
    let flattened = match Flattener::new(project, target_path) {
        Ok(flattener) => Ok(flattener.flatten()),
        Err(FlattenerError::Compilation(_)) => {
            paths.with_language::<SolcLanguage>().flatten(target_path)
        }
        Err(FlattenerError::Other(err)) => Err(err),
    }
    .map_err(|err: SolcError| eyre::eyre!("Failed to flatten: {err}"))?;

    Ok(flattened)
}
