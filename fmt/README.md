# Formatter (`fmt`)

Solidity formatter that respects (some parts of) the [Style Guide](https://docs.soliditylang.org/en/latest/style-guide.html) and
is tested on the [Prettier Solidity Plugin](https://github.com/prettier-solidity/prettier-plugin-solidity) cases.

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

### Output

The formatted output is written into the output buffer in _chunks_. The `Chunk` struct holds the content to be written & metadata for it. This includes the comments surrounding the content as well as the `needs_space` flag specifying whether this _chunk_ needs a space. The flag overrides the default behavior of `Formatter::next_char_needs_space` method.

The content gets written into the `FormatBuffer` which contains the information about the current indentation level, indentation length, current state as well as the other data determining the rules for writing the content. `FormatBuffer` implements the `std::fmt::Write` trait where it evaluates the current information and decides how the content should be written to the destination.

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

### Configuration

The formatter supports multiple configuration options defined in `FormatterConfig`.

| Option                           | Default  | Description                                                                                    |
| -------------------------------- | -------- | ---------------------------------------------------------------------------------------------- |
| line_length                      | 120      | Maximum line length where formatter will try to wrap the line                                  |
| tab_width                        | 4        | Number of spaces per indentation level                                                         |
| bracket_spacing                  | false    | Print spaces between brackets                                                                  |
| int_types                        | long     | Style of uint/int256 types. Available options: `long`, `short`, `preserve`                     |
| func_attrs_with_params_multiline | true     | If function parameters are multiline then always put the function attributes on separate lines |
| quote_style                      | double   | Style of quotation marks. Available options: `double`, `single`, `preserve`                    |
| number_underscore                | preserve | Style of underscores in number literals. Available options: `remove`, `thousands`, `preserve`  |

TODO: update ^

### Disable Line

The formatter can be disabled on specific lines by adding a comment `// forgefmt: disable-line`, like this:

```solidity
// forgefmt: disable-line
uint x = 100;
```

The comment can also be placed at the end of the line:

```solidity
uint x = 100; // forgefmt: disable-line
```

### Testing

Tests reside under the `fmt/testdata` folder and specify the malformatted & expected Solidity code. The source code file is named `original.sol` and expected file(s) are named in a format `({prefix}.)?fmt.sol`. Multiple expected files are needed for tests covering available configuration options.

The default configuration values can be overridden from within the expected file by adding a comment in the format `// config: {config_entry} = {config_value}`. For example:

```solidity
// config: line_length = 160
```

The `test_directory` macro is used to specify a new folder with source files for the test suite. Each test suite has the following process:

1. Preparse comments with config values
2. Parse and compare the AST for source & expected files.
    - The `AstEq` trait defines the comparison rules for the AST nodes
3. Format the source file and assert the equality of the output with the expected file.
4. Format the expected files and assert the idempotency of the formatting operation.

## Contributing

Check out the [foundry contribution guide](https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md).

Guidelines for contributing to `forge fmt`:

### Opening an issue

1. Create a short concise title describing an issue.
    - Bad Title Examples
        ```text
        Forge fmt does not work
        Forge fmt breaks
        Forge fmt unexpected behavior
        ```
    - Good Title Examples
        ```text
        Forge fmt postfix comment misplaced
        Forge fmt does not inline short yul blocks
        ```
2. Fill in the issue template fields that include foundry version, platform & component info.
3. Provide the code snippets showing the current & expected behaviors.
4. If it's a feature request, specify why this feature is needed.
5. Besides the default label (`T-Bug` for bugs or `T-feature` for features), add `C-forge` and `Cmd-forge-fmt` labels.

### Fixing A Bug

1. Specify an issue that is being addressed in the PR description.
2. Add a note on the solution in the PR description.
3. Make sure the PR includes the acceptance test(s).

### Developing A Feature

1. Specify an issue that is being addressed in the PR description.
2. Add a note on the solution in the PR description.
3. Provide the test coverage for the new feature. These should include:
    - Adding malformatted & expected solidity code under `fmt/testdata/$dir/`
    - Testing the behavior of pre and postfix comments
    - If it's a new config value, tests covering **all** available options
