use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod mixed_case;
use mixed_case::{MIXED_CASE_FUNCTION, MIXED_CASE_VARIABLE};

mod pascal_case;
use pascal_case::PASCAL_CASE_STRUCT;

mod screaming_snake_case;
use screaming_snake_case::{SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE};

mod imports;
use imports::{UNALIASED_PLAIN_IMPORT, UNUSED_IMPORT};

mod named_struct_fields;
use named_struct_fields::NAMED_STRUCT_FIELDS;

mod unsafe_cheatcodes;
use unsafe_cheatcodes::UNSAFE_CHEATCODE_USAGE;

register_lints!(
    (PascalCaseStruct, early, (PASCAL_CASE_STRUCT)),
    (MixedCaseVariable, early, (MIXED_CASE_VARIABLE)),
    (MixedCaseFunction, early, (MIXED_CASE_FUNCTION)),
    (ScreamingSnakeCase, early, (SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE)),
    (Imports, early, (UNALIASED_PLAIN_IMPORT, UNUSED_IMPORT)),
    (NamedStructFields, late, (NAMED_STRUCT_FIELDS)),
    (UnsafeCheatcodes, early, (UNSAFE_CHEATCODE_USAGE))
);
