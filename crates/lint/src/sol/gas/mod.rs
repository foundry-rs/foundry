use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod keccak;
use keccak::ASM_KECCAK256;

mod unwrapped_modifier_logic;
use unwrapped_modifier_logic::UNWRAPPED_MODIFIER_LOGIC;

register_lints!(
    (AsmKeccak256, late, (ASM_KECCAK256)),
    (ModifierLogic, early, (UNWRAPPED_MODIFIER_LOGIC))
);
