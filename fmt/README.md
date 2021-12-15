# Formatter (`fmt`)

Solidity formatter that respects (some parts of) the [Style Guide](https://docs.soliditylang.org/en/latest/style-guide.html) and
is tested on the [Prettier Solidity Plugin](https://github.com/prettier-solidity/prettier-plugin-solidity) cases.

## Features (WIP)

- [x] Pragma directive
- [x] Import directive
- [ ] Contract definition
- [x] Enum definition
- [ ] Struct definition
- [ ] Event definition
- [ ] Function definition
- [ ] Function body
- [ ] Variable definition
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
