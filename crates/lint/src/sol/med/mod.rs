use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

mod unsafe_typecast;
use unsafe_typecast::UNSAFE_TYPECAST;

register_lints!(
    (DivideBeforeMultiply, early, (DIVIDE_BEFORE_MULTIPLY)),
    (UnsafeTypecast, late, (UNSAFE_TYPECAST))
);
