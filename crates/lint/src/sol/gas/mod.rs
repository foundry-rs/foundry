use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod custom_errors;
mod keccak;
use custom_errors::CUSTOM_ERRORS;
use keccak::ASM_KECCAK256;

register_lints!((CustomErrors, early, (CUSTOM_ERRORS)), (AsmKeccak256, late, (ASM_KECCAK256)));
