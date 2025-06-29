use crate::sol::{EarlyLintPass, SolLint};

mod keccak;
use keccak::ASM_KECCAK256;

register_lints!((AsmKeccak256, (ASM_KECCAK256)));
