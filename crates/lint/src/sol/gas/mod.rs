use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod custom_errors;
mod immutable;
mod keccak;
mod unused_state_variables;
mod var_read_using_this;
use custom_errors::CUSTOM_ERRORS;
use immutable::COULD_BE_IMMUTABLE;
use keccak::ASM_KECCAK256;
use unused_state_variables::UNUSED_STATE_VARIABLES;
use var_read_using_this::VAR_READ_USING_THIS;

register_lints!(
    (AsmKeccak256, late, (ASM_KECCAK256)),
    (CustomErrors, early, (CUSTOM_ERRORS)),
    (CouldBeImmutable, late, (COULD_BE_IMMUTABLE)),
    (UnusedStateVariables, late, (UNUSED_STATE_VARIABLES)),
    (VarReadUsingThis, late, (VAR_READ_USING_THIS)),
);
