use crate::sol::{EarlyLintPass, SolLint};

mod keccak;
use keccak::ASM_KECCAK256;

mod unwrapped_modifier_logic;
use unwrapped_modifier_logic::UNWRAPPED_MODIFIER_LOGIC;

register_lints!((AsmKeccak256, (ASM_KECCAK256)), (ModifierLogic, (UNWRAPPED_MODIFIER_LOGIC)));
