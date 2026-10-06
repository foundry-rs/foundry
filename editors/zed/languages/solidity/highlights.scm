; Comprehensive Solidity syntax highlighting
; Based on tree-sitter-solidity grammar

; identifiers
; -----------
(identifier) @variable
(yul_identifier) @variable

; Pragma
(pragma_directive) @tag
(solidity_version_comparison_operator _ @tag)

; Literals
; --------

[
 (string)
 (hex_string_literal)
 (unicode_string_literal)
 (yul_string_literal)
] @string
[
 (number_literal)
 (yul_decimal_number)
 (yul_hex_number)
] @number
(number_unit) @type

(hex_string_literal "hex" @string.special)
(unicode_string_literal "unicode" @string.special)
(yul_hex_string_literal "hex" @string.special)
[
 (true)
 (false)
] @boolean

(comment) @comment

; Built-ins
; ---------

((identifier) @variable.builtin
  (#match? @variable.builtin "^(abi|block|msg|now|super|this|tx)$"))

((identifier) @function.builtin
  (#match? @function.builtin "^(addmod|assert|blockhash|blobhash|ecrecover|gasleft|keccak256|mulmod|require|ripemd160|selfdestruct|sha256|sha3|suicide)$"))

; Definitions and references
; -----------

(type_name) @type
(primitive_type) @type
(user_defined_type (identifier) @type)

(payable_conversion_expression "payable" @type)
; Ensures that delimiters in mapping( ... => .. ) are not colored like types
(type_name "(" @punctuation.bracket "=>" @punctuation.delimiter ")" @punctuation.bracket)

; Definitions
(struct_declaration
  name: (identifier) @type)
(enum_declaration
  name: (identifier) @type)
(contract_declaration
  name: (identifier) @type)
(library_declaration
  name: (identifier) @type)
(interface_declaration
  name: (identifier) @type)
(event_definition
  name: (identifier) @type)

(function_definition
  name:  (identifier) @function)

(modifier_definition
  name:  (identifier) @function)
(yul_evm_builtin) @function.builtin

; Use constructor coloring for special functions
(constructor_definition "constructor" @constructor)
(fallback_receive_definition "receive" @constructor)
(fallback_receive_definition "fallback" @constructor)

(struct_member name: (identifier) @property)
(enum_value) @constant

; Invocations
(emit_statement . (_) @type)
(modifier_invocation (identifier) @function)

(call_expression . (_(member_expression property: (_) @function.method)))
(call_expression . (expression(identifier)) @function)

; Function parameters
(call_struct_argument name: (_) @function.kwargs)
(event_parameter name: (identifier) @variable.parameter)
(parameter name: (identifier) @variable.parameter)

; Yul functions
(yul_function_call function: (yul_identifier) @function)
(yul_function_definition . (yul_identifier) @function (yul_identifier) @variable.parameter)


; Structs and members
(member_expression property: (identifier) @property)
(struct_expression type: (expression (identifier) @type))
(struct_field_assignment name: (identifier) @property)


; Tokens
; -------

; Keywords
(meta_type_expression "type" @keyword)
; Keywords
[
 "pragma"
 "abstract"
 "contract"
 "error"
 "interface"
 "library"
 "layout"
 "at"
 "type"
 "is"
 "struct"
 "enum"
 "event"
 "anonymous"
 "using"
 "global"
 "assembly"
 "emit"
 "public"
 "internal"
 "private"
 "external"
 "pure"
 "view"
 "payable"
 "modifier"
 "memory"
 "storage"
 "calldata"
 "var"
 "constant"
 "let"
 (virtual)
 (override_specifier)
 (immutable)
 (state_location)
 (unchecked)
 (yul_leave)
] @keyword

[
 "for"
 "while"
 "do"
 "break"
 "continue"
 "if"
 "else"
 "switch"
 "case"
 "default"
 "try"
 "catch"
 "revert"
] @keyword.control

"return" @keyword.control
"returns" @keyword

"function" @keyword.declaration

"import" @keyword.import
(import_directive "as" @keyword.import)
(import_directive "from" @keyword.import)
(using_alias "as" @keyword)

(event_parameter "indexed" @keyword)

; Punctuation

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket


[
  "."
  ","
  ";"
  ":"
  "=>"
] @punctuation.delimiter


; Operators

[
  "&&"
  "||"
  ">>"
  "<<"
  "&"
  "^"
  "|"
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
  "<"
  "<="
  "=="
  "!="
  ">="
  ">"
  "!"
  "~"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "^="
  "&="
  "|="
  ">>="
  "<<="
  "->"
  ":="
  "-"
  "+"
  "++"
  "--"
] @operator

(ternary_expression
  [
    "?"
    ":"
  ] @operator)

[
  "delete"
  "new"
] @keyword.operator
