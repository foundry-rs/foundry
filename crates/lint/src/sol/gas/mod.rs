use crate::sol::SolLint;

mod cache_array_length;
mod costly_loop;
mod custom_errors;
mod external_function;
mod immutable;
mod keccak;
mod unused_state_variables;
mod var_read_using_this;
mod write_after_write;

register_lints!(
    keccak: (AsmKeccak256, late, (ASM_KECCAK256));
    cache_array_length: (CacheArrayLength, late, (CACHE_ARRAY_LENGTH));
    costly_loop: (CostlyLoop, late, (COSTLY_LOOP));
    custom_errors: (CustomErrors, early, (CUSTOM_ERRORS));
    immutable: (UnchangedStateVariables, late, (COULD_BE_IMMUTABLE, COULD_BE_CONSTANT));
    external_function: (ExternalFunction, late, (EXTERNAL_FUNCTION));
    unused_state_variables: (UnusedStateVariables, late, (UNUSED_STATE_VARIABLES));
    var_read_using_this: (VarReadUsingThis, late, (VAR_READ_USING_THIS));
    write_after_write: (WriteAfterWrite, late, (WRITE_AFTER_WRITE));
);
