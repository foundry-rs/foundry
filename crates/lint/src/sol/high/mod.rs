use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

mod incorrect_shift;
use incorrect_shift::INCORRECT_SHIFT;

register_lints!((IncorrectShift, (INCORRECT_SHIFT)));
