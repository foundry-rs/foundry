use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

mod mixed_case;
use mixed_case::{MIXED_CASE_FUNCTION, MIXED_CASE_VARIABLE};

mod pascal_case;
use pascal_case::PASCAL_CASE_STRUCT;

mod screaming_snake_case;
use screaming_snake_case::{SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE};

register_lints!(
    (PascalCaseStruct, (PASCAL_CASE_STRUCT)),
    (MixedCaseVariable, (MIXED_CASE_VARIABLE)),
    (MixedCaseFunction, (MIXED_CASE_FUNCTION)),
    (ScreamingSnakeCase, (SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE))
);
