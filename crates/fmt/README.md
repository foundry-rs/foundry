# Formatter (`fmt`)

Solidity formatter that respects (some parts of)
the [Style Guide](https://docs.soliditylang.org/en/latest/style-guide.html) and
is tested on the [Prettier Solidity Plugin](https://github.com/prettier-solidity/prettier-plugin-solidity) cases.

## Architecture

The formatter is built on top of [Solar](https://github.com/paradigmxyz/solar), and the architecture is based on a Wadler-style pretty-printing engine. The formatting process consists of two main steps:

1.  **Parsing**: The Solidity source code is parsed using **`solar`** into an **Abstract Syntax Tree (AST)**. The AST is a tree representation of the code's syntactic structure.
2.  **Printing**: The AST is traversed by a visitor, which generates a stream of abstract tokens that are then processed by a pretty-printing engine to produce the final formatted code.

### The Pretty Printer (`pp`)

The core of the formatter is a pretty-printing engine inspired by Philip Wadler's algorithm, and adapted from the implementations in `rustc_ast_pretty` and `prettyplease`. Its goal is to produce an optimal and readable layout by making intelligent decisions about line breaks.

The process works like this:

1.  **AST to Abstract Tokens**: The formatter's `State` object walks the `solar` AST. Instead of directly writing strings, it translates the AST nodes into a stream of abstract formatting "commands" called `Token`s. This decouples the code's structure from the final text output. The primary tokens are:
    *   **`String`**: An atomic, unbreakable piece of text, like a keyword (`function`), an identifier (`myVar`), or a literal (`42`).
    *   **`Break`**: A potential line break. This is the core of the engine's flexibility. The `Printer` later decides whether to render a `Break` as a single space or as a newline with appropriate indentation.
    *   **`Begin`/`End`**: These tokens define a logical group of tokens that should be formatted as a single unit. This allows the printer to decide how to format the entire group at once.

2.  **Grouping and Breaking Strategy**: The `Begin` and `End` tokens create formatting "boxes" that guide the breaking strategy. There are two main types of boxes:
    *   **Consistent Box (`cbox`)**: If *any* `Break` inside this box becomes a newline, then *all* `Break`s inside it must also become newlines. This is ideal for lists like function parameters or struct fields, ensuring they are either all on one line or neatly arranged with one item per line.
    *   **Inconsistent Box (`ibox`)**: `Break`s within this box are independent. The printer can wrap a long line at any `Break` point without forcing other breaks in the same box to become newlines. This is useful for formatting long expressions or comments.

3.  **The `Printer` Engine**: The `Printer` consumes this stream of tokens and makes the final decisions:
    *   It maintains a buffer of tokens and tracks the remaining space on the current line based on the configured `line_length`.
    *   When it encounters a `Begin` token for a group, it calculates whether the entire group could fit on the current line if all its `Break`s were spaces.
    *   **If it fits**, all `Break`s in that group are rendered as spaces.
    *   **If it doesn't fit**, `Break`s are rendered as newlines, and the indentation level is adjusted accordingly based on the box's rules (consistent or inconsistent).

Crucially, this entire process is deterministic. Because the formatter completely rebuilds the code from the AST, it discards all original whitespace, line breaks, and other stylistic variations. This means that for a given AST and configuration, the output will always be identical. No matter how inconsistently the input code is formatted, the result is a single, canonical representation, ensuring predictability and consistency across any codebase.

> **Debug Mode**: To visualize the debug output, and understand how the pretty-printer makes its decisions about boxes and breaks, see the [Debug](#debug) section in Testing.

### Comments

Comment handling is a critical aspect of the formatter, designed to preserve developer intent while restructuring the code.

1.  **Categorization**: Comments are parsed and categorized by their position and style: `Isolated` (on its own line), `Mixed` (on a line with code), and `Trailing` (at the end of a line).

2.  **Blank Line Handling**: Blank lines in the source code are treated as a special `BlankLine` comment type, allowing the formatter to preserve vertical spacing that separates logical blocks of code. However, to maintain a clean and consistent vertical rhythm, any sequence of multiple blank lines is collapsed into a single blank line. This prevents excessive empty space in the formatted output.

3.  **Integration with Printing**: During the AST traversal, the formatter queries for comments that appear before the current code element. These comments, including blank lines, are then strategically inserted into the `Printer`'s token stream. The formatter inserts `Break` tokens around comments to ensure they are correctly spaced from the surrounding code, and emits one or two `hardbreak`s for blank lines to maintain the original vertical rhythm.

This approach allows the formatter to respect both the syntactic structure of the code and the developer's textual annotations and spacing, producing a clean, readable, and intentional layout.

### Example

**Source Code**
```solidity
pragma   solidity ^0.8.10 ;
contract  HelloWorld {
    string   public message;
    constructor(  string memory initMessage) { message = initMessage;}
}



event    Greet( string  indexed  name) ;
```

**Abstract Syntax Tree (AST) (simplified)**
```text
SourceUnit
 ├─ PragmaDirective("solidity", "^0.8.10")
 ├─ ItemContract("HelloWorld")
 │   ├─ VariableDefinition { name: "message", ty: "string", visibility: "public" }
 │   └─ ItemFunction {
 │         kind: Constructor,
 │         header: FunctionHeader {
 │            parameters: [
 │               VariableDefinition { name: "initMessage", ty: "string", data_location: "memory" }
 │            ]
 │         },
 │         body: Block {
 │            stmts: [
 │               Stmt { kind: Expr(Assign {lhs: Ident("message"), rhs: Ident("initMessage")}) }
 │            ]
 │         }
 │      }
 └─ ItemEvent { name: "Greet", parameters: [
       VariableDefinition { name: "name", ty: "string", indexed: true }
    ] }
```


**Formatted Source Code**
The code is reconstructed from the AST using the pretty-printer.
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

The formatter supports multiple configuration options defined in `foundry.toml`.

| Option | Default | Description |
| :--- | :--- | :--- |
| `line_length` | `120` | Maximum line length where the formatter will try to wrap the line. |
| `tab_width` | `4` | Number of spaces per indentation level. Ignored if `style` is `tab`. |
| `style` | `space` | The style of indentation. Options: `space`, `tab`. |
| `bracket_spacing` | `false` | Print spaces between brackets. |
| `int_types` | `long` | Style for `uint256`/`int256` types. Options: `long`, `short`, `preserve`. |
| `multiline_func_header` | `attributes_first` | The style of multiline function headers. Options: `attributes_first`, `params_always`, `params_first_multi`, `all`, `all_params`. |
| `prefer_compact` | `all` | Style that determines if a broken list, should keep its elements together on their own line, before breaking individually. Options: `none`, `calls`, `events`, `errors`, `events_errors`, `all`. |
| `quote_style` | `double` | The style of quotation marks. Options: `double`, `single`, `preserve`. |
| `number_underscore` | `preserve` | The style of underscores in number literals. Options: `preserve`, `remove`, `thousands`. |
| `hex_underscore` | `remove` | The style of underscores in hex literals. Options: `preserve`, `remove`, `bytes`. |
| `single_line_statement_blocks` | `preserve` | The style of single-line blocks in statements. Options: `preserve`, `single`, `multi`. |
| `override_spacing` | `false` | Print a space in the `override` attribute. |
| `wrap_comments` | `false` | Wrap comments when `line_length` is reached. |
| `docs_style` | `preserve` | Enforces the style of doc (natspec) comments. Options: `preserve`, `line`, `block`. |
| `ignore` | `[]` | Globs to ignore. |
| `contract_new_lines` | `false` | Add a new line at the start and end of contract declarations. |
| `sort_imports` | `false` | Sort import statements alphabetically in groups. A group is a set of imports separated by a newline. |
| `pow_no_space` | `false` | Suppress spaces around the power operator (`**`). |
| `single_line_imports` | `false` | Keep single imports on a single line, even if they exceed the line length limit. |

> Check [`FormatterConfig`](../config/src/fmt.rs) for a more detailed explanation.

### Inline Configuration

The formatter can be instructed to skip specific sections of code using inline comments. While the tool supports fine-grained control, it is generally more robust and efficient to disable formatting for entire AST items or statements.

This approach is preferred because it allows the formatter to treat the entire disabled item as a single, opaque unit. It can simply copy the original source text for that item's span instead of partially formatting a line, switching to copy mode, and then resuming formatting. This leads to more predictable output and avoids potential edge cases with complex, partially-disabled statements.

#### Disable Line

These directives are best used when they apply to a complete, self-contained AST statement, as shown below. In this case, `uint x = 100;` is a full statement, making it a good candidate for a line-based disable.

To disable the next line:
```solidity
// forgefmt: disable-next-line
uint x = 100;
```

To disable the current line:
```solidity
uint x = 100; // forgefmt: disable-line
```
#### Disable Block

This is the recommended approach for complex, multi-line constructs where you want to preserve specific formatting. In the example below, the entire `function` definition is disabled. This is preferable to trying to disable individual lines within the signature, because lines like `uint256 b /* a comment that goes inside the comma */,` do not correspond to a complete AST item or statement on their own. Disabling the whole item is cleaner and more aligned with the code's structure.

```solidity
// forgefmt: disable-start
function fnWithManyArguments(
    uint a,
    uint256 b /* a comment that goes inside the comma */,
    uint256      c
) external returns (bool) {
// forgefmt: disable-end
```

## Contributing

Check out the [foundry contribution guide](https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md).

Guidelines for contributing to `forge fmt`:

### Opening an issue

1.  Create a short, concise title describing the issue.
    *   **Bad**: `Forge fmt does not work`
    *   **Good**: `bug(forge-fmt): misplaces postfix comment on if-statement`
2.  Fill in the issue template fields, including Foundry version, platform, and component info.
3.  Provide code snippets showing the current and expected behaviors.
4.  If it's a feature request, explain why the feature is needed.
5.  Add the `C-forge` and `Cmd-forge-fmt` labels.

### Fixing a Bug or Developing a Feature

1.  Specify the issue being addressed in the PR description.
2.  Add a note on your solution in the PR description.
3.  Ensure the PR includes comprehensive acceptance tests under `fmt/testdata/`, covering:
    *   The specific case being fixed/added.
    *   Behavior with different kinds of comments (isolated, mixed, trailing).
    *   If it's a new config value, tests covering all available options.

### Testing

Tests are located in the `fmt/testdata` folder. Each test consists of an `original.sol` file and one or more expected output files, named `*.fmt.sol`.

The default configuration can be overridden from within an expected output file by adding a comment in the format `// config: {config_key} = {config_value}`. For example:
```solidity
// config: line_length = 160
```

The testing process for each test suite is as follows:
1.  Read `original.sol` and the corresponding `*.fmt.sol` expected output.
2.  Parse any `// config:` comments from the expected file to create a test-specific configuration.
3.  Format `original.sol` and assert that the output matches the content of `*.fmt.sol`.
4.  To ensure **idempotency**, format the content of `*.fmt.sol` again and assert that the output does not change.

### Debug

The formatter includes a debug mode that provides visual insight into the pretty-printer's decision-making process. This is invaluable for troubleshooting complex formatting issues and understanding how the boxes and breaks described in [The Pretty Printer](#the-pretty-printer-pp) section work.

To enable it, run the formatter with the `FMT_DEBUG` environment variable set:
```sh
FMT_DEBUG=1 cargo test -p forge-fmt --test formatter Repros
```

When enabled, the output will be annotated with special characters representing the printer's internal state:

*   **Boxes**:
    *   `«` and `»`: Mark the start and end of a **consistent** box (`cbox`).
    *   `‹` and `›`: Mark the start and end of an **inconsistent** box (`ibox`).

*   **Breaks**:
    *   `·`: Represents a `Break` token, which could be a space or a newline.

For example, running debug mode on the `HelloWorld` contract from earlier would produce an output like this:

```text
pragma solidity ^0.8.10;·
·
«‹«contract HelloWorld »{›·
‹‹    string· public· message››;·
·
«    constructor«(«‹‹string memory initMessage››»)» {»·
«‹        message = ·initMessage›·;·
»    }·
»}·
·
event Greet(««‹‹string indexed name››»»);·
```

This annotated output allows you to see exactly how the printer is grouping tokens and where it considers inserting a space or a newline. This makes it much easier to diagnose why a certain layout is being produced.
