mod mixed_case;
use mixed_case::{MIXED_CASE_FUNCTION, MIXED_CASE_VARIABLE};

mod pascal_case;
use pascal_case::PASCAL_CASE_STRUCT;

mod screaming_snake_case;
use screaming_snake_case::SCREAMING_SNAKE_CASE;

use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

register_lints!(
    (MixedCaseVariable, MIXED_CASE_VARIABLE),
    (ScreamingSnakeCase, SCREAMING_SNAKE_CASE),
    (PascalCaseStruct, PASCAL_CASE_STRUCT),
    (MixedCaseFunction, MIXED_CASE_FUNCTION)
);
