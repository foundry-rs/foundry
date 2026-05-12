use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod dead_code;
mod unwrapped_modifier_logic;
use dead_code::DEAD_CODE;
use unwrapped_modifier_logic::UNWRAPPED_MODIFIER_LOGIC;

register_lints!(
    (DeadCode, late, (DEAD_CODE)),
    (UnwrappedModifierLogic, late, (UNWRAPPED_MODIFIER_LOGIC)),
);
