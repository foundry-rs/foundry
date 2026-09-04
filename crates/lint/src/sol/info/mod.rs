use crate::sol::SolLint;

mod boolean_cst;
mod boolean_equal;
mod cyclomatic_complexity;
mod event_fields;
mod function_init_state;
mod imports;
mod incorrect_using_for;
mod inline_assembly;
mod interface_naming;
mod internal_function_used_once;
mod low_level_calls;
mod missing_inheritance;
mod mixed_case;
mod modifier_used_only_once;
mod multi_contract_file;
mod named_struct_fields;
mod pascal_case;
mod pragma_directive;
mod redundant_base_constructor_call;
mod screaming_snake_case;
mod todo;
mod too_many_digits;
mod unsafe_cheatcodes;
mod unused_error;

register_lints!(
    boolean_cst: (BooleanCst, early, (BOOLEAN_CST));
    boolean_equal: (BooleanEqual, early, (BOOLEAN_EQUAL));
    pascal_case: (PascalCaseStruct, early, (PASCAL_CASE_STRUCT), PascalCaseStructPass::new);
    screaming_snake_case: (
        ScreamingSnakeCase,
        early,
        (SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE)
    );
    mixed_case:
        (MixedCaseVariable, early, (MIXED_CASE_VARIABLE), MixedCaseVariablePass::new),
        (MixedCaseFunction, early, (MIXED_CASE_FUNCTION), MixedCaseFunctionPass::new);
    imports: (Imports, early, (UNALIASED_PLAIN_IMPORT, UNUSED_IMPORT));
    named_struct_fields: (NamedStructFields, late, (NAMED_STRUCT_FIELDS));
    unsafe_cheatcodes: (UnsafeCheatcodes, early, (UNSAFE_CHEATCODE_USAGE));
    multi_contract_file:
        (MultiContractFile, early, (MULTI_CONTRACT_FILE), MultiContractFilePass::new);
    interface_naming: (InterfaceFileNaming, early, (INTERFACE_FILE_NAMING, INTERFACE_NAMING));
    too_many_digits: (TooManyDigits, early, (TOO_MANY_DIGITS));
    pragma_directive: (PragmaDirective, project, (PRAGMA_INCONSISTENT));
    inline_assembly: (InlineAssembly, early, (INLINE_ASSEMBLY));
    low_level_calls: (LowLevelCalls, early, (LOW_LEVEL_CALLS));
    redundant_base_constructor_call:
        (RedundantBaseConstructorCall, late, (REDUNDANT_BASE_CONSTRUCTOR_CALL));
    missing_inheritance: (MissingInheritance, project, (MISSING_INHERITANCE));
    event_fields: (EventFields, early, (EVENT_FIELDS));
    todo: (TodoComment, early, (TODO_COMMENT));
    unused_error: (UnusedError, project, (UNUSED_ERROR));
    function_init_state: (FunctionInitState, late, (FUNCTION_INIT_STATE));
    internal_function_used_once: (InternalFunctionUsedOnce, project, (INTERNAL_FUNCTION_USED_ONCE));
    cyclomatic_complexity: (CyclomaticComplexity, late, (CYCLOMATIC_COMPLEXITY));
    incorrect_using_for: (IncorrectUsingFor, late, (INCORRECT_USING_FOR));
    modifier_used_only_once: (ModifierUsedOnlyOnce, project, (MODIFIER_USED_ONLY_ONCE));
);
