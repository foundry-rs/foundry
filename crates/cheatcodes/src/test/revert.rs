use crate::{Error, Result};
use alloy_primitives::{hex, Address, Bytes};
use alloy_sol_types::{SolError, SolValue};
use foundry_common::ContractsByArtifact;
use foundry_evm_core::decode::RevertDecoder;
use revm::interpreter::InstructionResult;
use spec::Vm;

/// Common parameters for expected or assumed reverts. Allows for code reuse.
pub(crate) trait RevertParameters {
    fn reverter(&self) -> Option<Address>;
    fn reverted_by(&self) -> Option<Address>;
    fn reason(&self) -> Option<&[u8]>;
    fn partial_match(&self) -> bool;
}

/// Core logic for handling reverts that may or may not be expected (or assumed).
pub(crate) fn handle_revert(
    is_cheatcode: bool,
    revert_params: &impl RevertParameters,
    status: InstructionResult,
    retdata: Bytes,
    known_contracts: &Option<ContractsByArtifact>,
) -> Result<(), Error> {
    // If expected reverter address is set then check it matches the actual reverter.
    if let (Some(expected_reverter), Some(actual_reverter)) =
        (revert_params.reverter(), revert_params.reverted_by())
    {
        if expected_reverter != actual_reverter {
            return Err(fmt_err!(
                "Reverter != expected reverter: {} != {}",
                actual_reverter,
                expected_reverter
            ));
        }
    }

    let expected_reason = revert_params.reason();
    // If None, accept any revert.
    let Some(expected_reason) = expected_reason else {
        return Ok(Default::default());
    };

    if !expected_reason.is_empty() && retdata.is_empty() {
        bail!("call reverted as expected, but without data");
    }

    // todo: would love to not copy here but types don't seem to work easily
    let mut actual_revert: Vec<u8> = retdata.to_vec();

    // Compare only the first 4 bytes if partial match.
    if revert_params.partial_match() && actual_revert.get(..4) == expected_reason.get(..4) {
        return Ok(Default::default())
    }

    // Try decoding as known errors.
    if matches!(
        actual_revert.get(..4).map(|s| s.try_into().unwrap()),
        Some(Vm::CheatcodeError::SELECTOR | alloy_sol_types::Revert::SELECTOR)
    ) {
        if let Ok(decoded) = Vec::<u8>::abi_decode(&actual_revert[4..], false) {
            actual_revert = decoded;
        }
    }

    if actual_revert == expected_reason ||
        (is_cheatcode && memchr::memmem::find(&actual_revert, expected_reason).is_some())
    {
        Ok(Default::default())
    } else {
        let (actual, expected) = if let Some(contracts) = known_contracts {
            let decoder = RevertDecoder::new().with_abis(contracts.iter().map(|(_, c)| &c.abi));
            (
                &decoder.decode(actual_revert.as_slice(), Some(status)),
                &decoder.decode(expected_reason, Some(status)),
            )
        } else {
            let stringify = |data: &[u8]| {
                if let Ok(s) = String::abi_decode(data, true) {
                    return s;
                }
                if data.is_ascii() {
                    return std::str::from_utf8(data).unwrap().to_owned();
                }
                hex::encode_prefixed(data)
            };
            (&stringify(&actual_revert), &stringify(expected_reason))
        };
        Err(fmt_err!("Error != expected error: {} != {}", actual, expected,))
    }
}
