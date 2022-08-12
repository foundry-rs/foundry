# Formatter (`fmt`)

Solidity formatter that respects (some parts of) the [Style Guide](https://docs.soliditylang.org/en/latest/style-guide.html) and
is tested on the [Prettier Solidity Plugin](https://github.com/prettier-solidity/prettier-plugin-solidity) cases.

## Features

### Directives & Definitions

- [x] Pragma directive
- [x] Import directive
- [x] Contract definition
- [x] Enum definition
- [x] Struct definition
- [x] Event definition
- [x] Error definition
- [x] Function / Modifier / Constructor definitions
- [x] Variable definition
- [x] Type definition
- [x] Using

### Statements

See [Statement](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L613-L649) enum in Solang

- [x] Block
- [ ] Assembly
- [x] Args
- [x] If
- [x] While
- [x] Expression
- [x] VariableDefinition
- [x] For
- [x] DoWhile
- [x] Continue
- [x] Break
- [x] Return
- [x] Revert
- [x] Emit
- [x] Try
- [x] DocComment

### Expressions

See [Expression](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L365-L431) enum in Solang

- [x] PostIncrement, PostDecrement, PreIncrement, PreDecrement, UnaryPlus, UnaryMinus, Not, Complement
- [x] Power, Multiply, Divide, Modulo, Add, Subtract
- [x] ShiftLeft, ShiftRight, BitwiseAnd, BitwiseXor, BitwiseOr
- [x] Assign, AssignOr, AssignAnd, AssignXor, AssignShiftLeft, AssignShiftRight, AssignAdd, AssignSubtract, AssignMultiply, AssignDivide, AssignModulo
- [x] Less, More, LessEqual, MoreEqual, Equal, NotEqual, And, Or
- [x] BoolLiteral, NumberLiteral, RationalNumberLiteral, HexNumberLiteral, StringLiteral, HexLiteral, AddressLiteral
- [x] ArraySubscript, ArraySlice
- [x] MemberAccess
- [x] FunctionCall
- [x] FunctionCallBlock
- [x] NamedFunctionCall
- [x] New
- [x] Delete
- [x] Ternary
- [x] Type
    - [x] Address
    - [x] Address Payable
    - [x] Payable
    - [x] Bool
    - [x] String
    - [x] Int
    - [x] Uint
    - [x] Bytes
    - [x] Rational
    - [x] Dynamic Bytes
    - [x] Mapping
    - [x] Function
- [x] Variable
- [x] List
- [x] ArrayLiteral
- [x] Unit
- [x] This

### Yul Statements

See [YulStatement](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L658-L670) enum in Solang

- [x] Assign
- [x] VariableDeclaration
- [x] If
- [x] For
- [x] Switch
- [x] Leave
- [x] Break
- [x] Continue
- [x] Block
- [x] FunctionDefinition
- [x] FunctionCall

### Yul Expressions

See [YulExpression](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L695-L704) enum in Solang

- [x] BoolLiteral
- [x] NumberLiteral
- [x] HexNumberLiteral
- [X] HexStringLiteral
- [x] StringLiteral
- [x] Variable
- [x] FunctionCall
- [x] SuffixAccess

### Other

- [x] Comments

## Configuration

### Options

- [x] Line Length
- [x] Tab Width
- [x] Bracket Spacing
- [x] Explicit Int Types
- [ ] Quote style
- [x] Function Modifiers with Parameter multiline
- [ ] Import Order

### Other

- [ ] Disable Formatter Range
- [ ] Disable Formatter Next Line

## Architecture

The formatter works in two steps:
1. Parse Solidity source code with [solang](https://github.com/hyperledger-labs/solang) into the PT (Parse Tree)
(not the same as Abstract Syntax Tree, [see difference](https://stackoverflow.com/a/9864571)).
2. Walk the PT and output new source code that's compliant with provided config and rule set.

The technique for walking the tree is based on [Visitor Pattern](https://en.wikipedia.org/wiki/Visitor_pattern)
and works as following:
1. Implement `Formatter` callback functions for each PT node type.
Every callback function should write formatted output for the current node
and call `Visitable::visit` function for child nodes delegating the output writing. 
1. Implement `Visitable` trait and its `visit` function for each PT node type. Every `visit` function should call corresponding `Formatter`'s callback function.

### Comments

The solang parser does not output comments as a type of parse tree node, but rather
in a list alongside the parse tree with location information. It is therefore necessary
to infer where to insert the comments and how to format them while traversing the parse tree.

To handle this, the formatter pre-parses the comments and puts them into two categories:
Prefix and Postfix comments. Prefix comments refer to the node directly after them, and
postfix comments refer to the node before them. As an illustration:

```solidity
// This is a prefix comment
/* This is also a prefix comment */
uint variable = 1 + 2; /* this is postfix */ // this is postfix too
    // and this is a postfix comment on the next line
```

To insert the comments into the appropriate areas, strings get converted to chunks
before being written to the buffer. A chunk is any string that cannot be split by
whitespace. A chunk also carries with it the surrounding comment information. Thereby
when writing the chunk the comments can be added before and after the chunk as well
as any any whitespace surrounding.

To construct a chunk, the string and the location of the string is given to the
Formatter and the pre-parsed comments before the start and end of the string are
associated with that string. The source code can then further be chunked before the
chunks are written to the buffer.

To write the chunk, first the comments associated with the start of the chunk get
written to the buffer. Then the Formatter checks if any whitespace is needed between
what's been written to the buffer and what's in the chunk and inserts it where appropriate.
If the chunk content fits on the same line, it will be written directly to the buffer,
otherwise it will be written on the next line. Finally, any associated postfix
comments also get written.

### Example

Source code
```solidity
pragma   solidity ^0.8.10 ;
contract  HelloWorld {
    string   public message;
    constructor(  string memory initMessage) { message = initMessage;}
}


event    Greet( string  indexed  name) ;
```

Parse Tree (simplified)
```text
SourceUnit
 | PragmaDirective("solidity", "^0.8.10")
 | ContractDefinition("HelloWorld")
    | VariableDefinition("string", "message", null, ["public"])
    | FunctionDefinition("constructor")
       | Parameter("string", "initMessage", ["memory"])
 | EventDefinition("string", "Greet", ["indexed"], ["name"])
```

Formatted source code that was reconstructed from the Parse Tree
```solidity
pragma solidity ^0.8.10;

contract HelloWorld {
    string public message;

    constructor(string memory initMessage) {
        message = initMessage;
    }
}

event Greet(string indexed name);
```
