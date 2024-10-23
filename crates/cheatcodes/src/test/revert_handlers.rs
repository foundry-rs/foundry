use crate::{Error, Result};
use alloy_primitives::{address, hex, Address, Bytes};
use alloy_sol_types::{SolError, SolValue};
use foundry_common::ContractsByArtifact;
use foundry_evm_core::decode::RevertDecoder;
use revm::interpreter::{return_ok, InstructionResult};
use spec::Vm;

use super::{
    assume::{AcceptableRevertParameters, AssumeNoRevert},
    expect::ExpectedRevert,
};

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
///
/// 8192 bytes was arbitrarily chosen because it is long enough for return values up to 256 words in
/// size.
static DUMMY_CALL_OUTPUT: Bytes = Bytes::from_static(&[0u8; 8192]);

/// Same reasoning as [DUMMY_CALL_OUTPUT], but for creates.
const DUMMY_CREATE_ADDRESS: Address = address!("0000000000000000000000000000000000000001");

/// Common parameters for expected or assumed reverts. Allows for code reuse.
pub(crate) trait RevertParameters {
    fn reverter(&self) -> Option<Address>;
    fn reason(&self) -> Option<&[u8]>;
    fn partial_match(&self) -> bool;
}

impl RevertParameters for AcceptableRevertParameters {
    fn reverter(&self) -> Option<Address> {
        self.reverter
    }

    fn reason(&self) -> Option<&[u8]> {
        Some(&self.reason)
    }

    fn partial_match(&self) -> bool {
        self.partial_match
    }
}

/// Core logic for handling reverts that may or may not be expected (or assumed).
fn handle_revert(
    is_cheatcode: bool,
    revert_params: &impl RevertParameters,
    status: InstructionResult,
    retdata: &Bytes,
    known_contracts: &Option<ContractsByArtifact>,
    reverter: Option<&Address>,
) -> Result<(), Error> {
    // If expected reverter address is set then check it matches the actual reverter.
    if let (Some(expected_reverter), Some(&actual_reverter)) = (revert_params.reverter(), reverter)
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
        return Ok(());
    };

    if !expected_reason.is_empty() && retdata.is_empty() {
        bail!("call reverted as expected, but without data");
    }

    let mut actual_revert: Vec<u8> = retdata.to_vec();

    // Compare only the first 4 bytes if partial match.
    if revert_params.partial_match() && actual_revert.get(..4) == expected_reason.get(..4) {
        return Ok(());
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
        Ok(())
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

pub(crate) fn handle_assume_no_revert(
    assume_no_revert: &AssumeNoRevert,
    status: InstructionResult,
    retdata: &Bytes,
    known_contracts: &Option<ContractsByArtifact>,
) -> Result<()> {
    // if a generic AssumeNoRevert, return Ok(). Otherwise, iterate over acceptable reasons and try
    // to match against any, otherwise, return an Error with the revert data
    if assume_no_revert.reasons.is_empty() {
        Ok(())
    } else {
        assume_no_revert
            .reasons
            .iter()
            .find_map(|reason| {
                handle_revert(
                    false,
                    reason,
                    status,
                    retdata,
                    known_contracts,
                    assume_no_revert.reverted_by.as_ref(),
                )
                .ok()
            })
            .ok_or_else(|| retdata.clone().into())
    }
}

pub(crate) fn handle_expect_revert(
    is_cheatcode: bool,
    is_create: bool,
    expected_revert: &ExpectedRevert,
    status: InstructionResult,
    retdata: Bytes,
    known_contracts: &Option<ContractsByArtifact>,
) -> Result<(Option<Address>, Bytes)> {
    let success_return = || {
        if is_create {
            (Some(DUMMY_CREATE_ADDRESS), Bytes::new())
        } else {
            (None, DUMMY_CALL_OUTPUT.clone())
        }
    };

    ensure!(!matches!(status, return_ok!()), "next call did not revert as expected");

    handle_revert(
        is_cheatcode,
        expected_revert,
        status,
        &retdata,
        known_contracts,
        expected_revert.reverted_by.as_ref(),
    )?;
    Ok(success_return())
}
