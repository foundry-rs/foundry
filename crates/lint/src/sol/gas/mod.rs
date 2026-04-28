use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod custom_errors;
mod keccak;
mod unused_state_variables;
use custom_errors::CUSTOM_ERRORS;
use keccak::ASM_KECCAK256;
use unused_state_variables::UNUSED_STATE_VARIABLES;

register_lints!(
    (CustomErrors, early, (CUSTOM_ERRORS)),
    (AsmKeccak256, late, (ASM_KECCAK256)),
    (UnusedStateVariables, late, (UNUSED_STATE_VARIABLES)),
);
