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
- [ ] Using (waiting for https://github.com/hyperledger-labs/solang/issues/801, https://github.com/hyperledger-labs/solang/issues/802, https://github.com/hyperledger-labs/solang/issues/803, https://github.com/hyperledger-labs/solang/issues/804)

### Statements

See [Statement](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L613-L649) enum in Solang

- [x] Block
- [ ] Assembly
- [ ] Args
- [ ] If
- [ ] While
- [x] Expression
- [ ] VariableDefinition
- [ ] For
- [ ] DoWhile
- [x] Continue
- [x] Break
- [ ] Return
- [ ] Revert
- [ ] Emit
- [ ] Try
- [ ] DocComment

### Expressions

See [Expression](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L365-L431) enum in Solang

- [ ] PostIncrement, PostDecrement, PreIncrement, PreDecrement, UnaryPlus, UnaryMinus, Not, Complement
- [ ] Power, Multiply, Divide, Modulo, Add, Subtract
- [ ] ShiftLeft, ShiftRight, BitwiseAnd, BitwiseXor, BitwiseOr
- [ ] AssignOr, AssignAnd, AssignXor, AssignShiftLeft, AssignShiftRight, AssignAdd, AssignSubtract, AssignMultiply, AssignDivide, AssignModulo
- [ ] Less, More, LessEqual, MoreEqual, Equal, NotEqual, And, Or
- [ ] BoolLiteral, NumberLiteral, RationalNumberLiteral, HexNumberLiteral, StringLiteral, HexLiteral , AddressLiteral
- [ ] ArraySubscript, ArraySlice
- [ ] MemberAccess
- [ ] FunctionCall
- [ ] FunctionCallBlock
- [ ] NamedFunctionCall
- [ ] New
- [ ] Delete
- [ ] Ternary
- [ ] Assign
- [ ] Type
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
    - [ ] Function
- [ ] Variable
- [ ] List
- [ ] ArrayLiteral
- [ ] Unit
- [ ] This

### Yul Statements

See [YulStatement](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L658-L670) enum in Solang

- [ ] Assign
- [ ] VariableDeclaration
- [ ] If
- [ ] For
- [ ] Switch
- [ ] Leave
- [ ] Break
- [ ] Continue
- [ ] Block
- [ ] FunctionDefinition
- [ ] FunctionCall

### Yul Expressions

See [YulExpression](https://github.com/hyperledger-labs/solang/blob/413841b5c759eb86d684bed0114ff5f74fffbbb1/solang-parser/src/pt.rs#L695-L704) enum in Solang

- [ ] BoolLiteral
- [ ] NumberLiteral
- [ ] HexNumberLiteral
- [ ] HexStringLiteral
- [ ] StringLiteral
- [ ] Variable
- [ ] FunctionCall
- [ ] Member

### Other

- [ ] Comments

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
2. Implement `Visitable` trait and its `visit` function for each PT node type. Every `visit` function should call corresponding `Formatter`'s callback function.

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
