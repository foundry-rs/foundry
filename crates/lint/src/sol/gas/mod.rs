use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod cache_array_length;
mod costly_loop;
mod custom_errors;
mod external_function;
mod immutable;
mod keccak;
mod unused_state_variables;
mod var_read_using_this;
mod write_after_write;
use cache_array_length::CACHE_ARRAY_LENGTH;
use costly_loop::COSTLY_LOOP;
use custom_errors::CUSTOM_ERRORS;
use external_function::EXTERNAL_FUNCTION;
use immutable::{COULD_BE_CONSTANT, COULD_BE_IMMUTABLE};
use keccak::ASM_KECCAK256;
use unused_state_variables::UNUSED_STATE_VARIABLES;
use var_read_using_this::VAR_READ_USING_THIS;
use write_after_write::WRITE_AFTER_WRITE;

register_lints!(
    (AsmKeccak256, late, (ASM_KECCAK256)),
    (CacheArrayLength, late, (CACHE_ARRAY_LENGTH)),
    (CostlyLoop, late, (COSTLY_LOOP)),
    (CustomErrors, early, (CUSTOM_ERRORS)),
    (UnchangedStateVariables, late, (COULD_BE_IMMUTABLE, COULD_BE_CONSTANT)),
    (ExternalFunction, late, (EXTERNAL_FUNCTION)),
    (UnusedStateVariables, late, (UNUSED_STATE_VARIABLES)),
    (VarReadUsingThis, late, (VAR_READ_USING_THIS)),
    (WriteAfterWrite, late, (WRITE_AFTER_WRITE)),
);
