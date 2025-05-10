mod incorrect_shift;
use incorrect_shift::INCORRECT_SHIFT;

use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

register_lints!((IncorrectShift, (INCORRECT_SHIFT)));
