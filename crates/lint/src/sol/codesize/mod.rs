use crate::sol::SolLint;

mod unwrapped_modifier_logic;

register_lints!(
    unwrapped_modifier_logic: (UnwrappedModifierLogic, late, (UNWRAPPED_MODIFIER_LOGIC));
);
