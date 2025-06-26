use crate::sol::{EarlyLintPass, SolLint};

mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

register_lints!((DivideBeforeMultiply, (DIVIDE_BEFORE_MULTIPLY)));
