mod keccack;
use keccack::ASM_KECCAK256;

use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

register_lints!((AsmKeccak256, (ASM_KECCAK256)));
