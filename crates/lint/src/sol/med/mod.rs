use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

register_lints!((DivideBeforeMultiply, early, (DIVIDE_BEFORE_MULTIPLY)));
