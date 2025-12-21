use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod unwrapped_modifier_logic;
use unwrapped_modifier_logic::UNWRAPPED_MODIFIER_LOGIC;

register_lints!((UnwrappedModifierLogic, late, (UNWRAPPED_MODIFIER_LOGIC)));
