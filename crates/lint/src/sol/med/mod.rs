use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

mod incorrect_erc721_interface;
use incorrect_erc721_interface::INCORRECT_ERC721_INTERFACE;

mod unsafe_typecast;
use unsafe_typecast::UNSAFE_TYPECAST;

register_lints!(
    (DivideBeforeMultiply, early, (DIVIDE_BEFORE_MULTIPLY)),
    (IncorrectERC721Interface, late, (INCORRECT_ERC721_INTERFACE)),
    (UnsafeTypecast, late, (UNSAFE_TYPECAST))
);
