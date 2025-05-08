mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

register_lints!((DivideBeforeMultiply, (DIVIDE_BEFORE_MULTIPLY)));
