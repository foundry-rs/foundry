use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod keccak;
use keccak::ASM_KECCAK256;

register_lints!((AsmKeccak256, early, (ASM_KECCAK256)));
