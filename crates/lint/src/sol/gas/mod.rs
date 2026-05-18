use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod costly_loop;
mod custom_errors;
mod external_function;
mod immutable;
mod keccak;
mod unused_state_variables;
mod var_read_using_this;
use costly_loop::COSTLY_LOOP;
use custom_errors::CUSTOM_ERRORS;
use external_function::EXTERNAL_FUNCTION;
use immutable::COULD_BE_IMMUTABLE;
use keccak::ASM_KECCAK256;
use unused_state_variables::UNUSED_STATE_VARIABLES;
use var_read_using_this::VAR_READ_USING_THIS;

register_lints!(
    (AsmKeccak256, late, (ASM_KECCAK256)),
    (CostlyLoop, late, (COSTLY_LOOP)),
    (CustomErrors, early, (CUSTOM_ERRORS)),
    (CouldBeImmutable, late, (COULD_BE_IMMUTABLE)),
    (ExternalFunction, late, (EXTERNAL_FUNCTION)),
    (UnusedStateVariables, late, (UNUSED_STATE_VARIABLES)),
    (VarReadUsingThis, late, (VAR_READ_USING_THIS)),
);
