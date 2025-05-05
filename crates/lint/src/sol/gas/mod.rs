mod keccack;
use keccack::ASM_KECCACK256;

use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

register_lints!((AsmKeccak256, ASM_KECCACK256));
