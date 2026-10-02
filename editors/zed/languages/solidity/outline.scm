; Simplified Solidity outline for testing

; ==== TOP LEVEL DECLARATIONS ====

; Pragma declarations
(pragma_directive) @item


; Contract declarations
(contract_declaration
  "contract" @context
  name: (identifier) @name) @item

; Interface declarations
(interface_declaration
  "interface" @context
  name: (identifier) @name) @item

; Library declarations
(library_declaration
  "library" @context
  name: (identifier) @name) @item

; Struct declarations
(struct_declaration
  "struct" @context
  name: (identifier) @name) @item

; Enum declarations
(enum_declaration
  "enum" @context
  name: (identifier) @name) @item

; Event definitions
(event_definition
  "event" @context
  name: (identifier) @name) @item

; Error definitions
(error_declaration
  "error" @context
  name: (identifier) @name) @item

; Function definitions
(function_definition
  "function" @context
  name: (identifier) @name) @item

; Constructor definitions
(constructor_definition
  "constructor" @context) @item

; Modifier definitions
(modifier_definition
  "modifier" @context
  name: (identifier) @name) @item

; State variable declarations
(state_variable_declaration
  name: (identifier) @name) @item
