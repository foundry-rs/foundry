use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod custom_errors;
mod immutable;
mod keccak;
use custom_errors::CUSTOM_ERRORS;
use immutable::COULD_BE_IMMUTABLE;
use keccak::ASM_KECCAK256;

register_lints!(
    (CustomErrors, early, (CUSTOM_ERRORS)),
    (CouldBeImmutable, late, (COULD_BE_IMMUTABLE)),
    (AsmKeccak256, late, (ASM_KECCAK256)),
);
