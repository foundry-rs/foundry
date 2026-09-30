; Comprehensive indentation rules for Solidity

; Block structures that require indentation
[
  (contract_body)
  (struct_body)
  (enum_body)
  (block_statement)
  (assembly_statement)
  (yul_block)
] @indent

; Function and modifier definitions
[
  (function_definition)
  (modifier_definition)
  (constructor_definition)
  (fallback_receive_definition)
] @indent

; Control flow structures
[
  (if_statement)
  (for_statement)
  (while_statement)
  (do_while_statement)
  (try_statement)
  (catch_clause)
] @indent

; Array and mapping literals
[
  (inline_array_expression)
  (struct_expression)
] @indent

; Opening braces
[
  "{"
] @indent

; Closing braces
[
  "}"
] @outdent

; Special indentation for multi-line function parameters
; Note: Solidity grammar uses individual (parameter) nodes, not parameter_list

; Special indentation for multi-line function returns
(return_type_definition) @indent @outdent

; Multi-line expressions
(binary_expression) @indent @outdent
(ternary_expression) @indent @outdent

; Yul-specific indentation
(yul_function_definition) @indent
(yul_for_statement) @indent
(yul_if_statement) @indent
(yul_switch_statement) @indent
