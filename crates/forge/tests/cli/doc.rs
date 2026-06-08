use foundry_test_utils::util::{RemoteProject, setup_forge_remote};

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc"]).assert_success();
    // At least one MDX page was generated.
    assert!(
        std::fs::read_dir(prj.root().join("docs/src/pages/src")).is_ok(),
        "docs/src/pages/src directory should exist"
    );
}

// Test that overloaded functions in interfaces inherit the correct NatSpec comments
// fixes <https://github.com/foundry-rs/foundry/issues/11823>
forgetest_init!(can_generate_docs_for_overloaded_functions, |prj, cmd| {
    prj.add_source(
        "IExample.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IExample {
    /// @notice Deposit tokens into the vault
    /// @param amount The amount to deposit
    function deposit(uint256 amount) external;

    /// @notice Withdraw tokens from the vault
    /// @param amount The amount to withdraw
    function withdraw(uint256 amount) external;
}
"#,
    );

    prj.add_source(
        "Example.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IExample.sol";

contract Example is IExample {
    /// @inheritdoc IExample
    function deposit(uint256 amount) external {}

    /// @inheritdoc IExample
    function withdraw(uint256 amount) external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Example.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("Deposit tokens into the vault"),
        "deposit notice should be inherited"
    );
    assert!(
        content.contains("Withdraw tokens from the vault"),
        "withdraw notice should be inherited"
    );
});

// Test that {Ident} cross-references resolve to root-relative vocs links.
// fixes <https://github.com/foundry-rs/foundry/issues/12361>
forgetest_init!(hyperlinks_use_relative_paths, |prj, cmd| {
    prj.add_source(
        "IBase.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IBase {
    function baseFunction() external;
}
"#,
    );

    prj.add_source(
        "Derived.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IBase.sol";

/// @dev Inherits: {IBase}
contract Derived is IBase {
    function baseFunction() external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Derived.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("[IBase](/src/interface.IBase)"),
        "Hyperlink should be a root-relative vocs link, found: {:?}",
        content.lines().find(|line| line.contains("[IBase]")).unwrap_or("not found")
    );
});

// Test that constants and immutables are documented under "Constants" section when only constants
// are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(constants_and_immutables_are_documented_under_constants_section, |prj, cmd| {
    prj.add_source(
        "CounterConstants.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterConstants {
    uint256 public constant FOO = 1;
    uint256 public immutable BAR;

    constructor() {
        BAR = 2;
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.CounterConstants.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(content.contains("## Constants"), "Should have Constants section");
    assert!(!content.contains("## State Variables"), "Should not have State Variables section");

    let constants_pos = content.find("## Constants").unwrap();
    let functions_pos = content.find("## Functions").unwrap();

    assert!(content.contains("### FOO"), "Should have FOO constant");
    let foo_pos = content.find("### FOO").unwrap();
    assert!(foo_pos > constants_pos && foo_pos < functions_pos, "FOO should be inside Constants");

    assert!(content.contains("### BAR"), "Should have BAR immutable");
    let bar_pos = content.find("### BAR").unwrap();
    assert!(bar_pos > constants_pos && bar_pos < functions_pos, "BAR should be inside Constants");
});

// Test that state variables are documented under "State Variables" section when only state
// variables are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(state_variables_are_documented_under_state_variables_section, |prj, cmd| {
    prj.add_source(
        "CounterStateVariables.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterStateVariables {
    uint256 public baz;

    function increment() public {
        baz++;
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.CounterStateVariables.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(!content.contains("## Constants"), "Should not have Constants section");
    assert!(content.contains("## State Variables"), "Should have State Variables section");

    let state_vars_pos = content.find("## State Variables").unwrap();
    let functions_pos = content.find("## Functions").unwrap();

    assert!(content.contains("### baz"), "Should have baz state variable");
    let baz_pos = content.find("### baz").unwrap();
    assert!(
        baz_pos > state_vars_pos && baz_pos < functions_pos,
        "baz should be inside State Variables"
    );
});

// Test that constants/immutables and state-variables are documented under separate sections when
// both are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(
    constants_and_immutables_and_state_variables_are_documented_under_separate_sections,
    |prj, cmd| {
        prj.add_source(
            "CounterMixedVariables.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterMixedVariables {
    uint256 public constant FOO = 1;
    uint256 public immutable BAR;
    uint256 public baz;

    constructor() {
        BAR = 2;
    }

    function increment() public {
        baz++;
    }
}
"#,
        );

        cmd.args(["doc"]).assert_success();

        let doc_path = prj.root().join("docs/src/pages/src/contract.CounterMixedVariables.mdx");
        let content = std::fs::read_to_string(&doc_path).unwrap();

        assert!(content.contains("## Constants"), "Should have Constants section");
        assert!(content.contains("## State Variables"), "Should have State Variables section");

        let constants_pos = content.find("## Constants").unwrap();
        let state_vars_pos = content.find("## State Variables").unwrap();
        let functions_pos = content.find("## Functions").unwrap();

        assert!(
            constants_pos < state_vars_pos && state_vars_pos < functions_pos,
            "Constants < State Variables < Functions"
        );

        assert!(content.contains("### FOO"), "Should have FOO constant");
        let foo_pos = content.find("### FOO").unwrap();
        assert!(
            foo_pos > constants_pos && foo_pos < state_vars_pos,
            "FOO should be inside Constants"
        );

        assert!(content.contains("### BAR"), "Should have BAR immutable");
        let bar_pos = content.find("### BAR").unwrap();
        assert!(
            bar_pos > constants_pos && bar_pos < state_vars_pos,
            "BAR should be inside Constants"
        );

        assert!(content.contains("### baz"), "Should have baz state variable");
        let baz_pos = content.find("### baz").unwrap();
        assert!(
            baz_pos > state_vars_pos && baz_pos < functions_pos,
            "baz should be inside State Variables"
        );
    }
);

// Test that MDX-unsafe content coming through @inheritdoc is still escaped, and that
// unnamed return values are rendered as `&lt;none&gt;`.
forgetest_init!(inheritdoc_mdx_safety_and_unnamed_returns, |prj, cmd| {
    prj.add_source(
        "IUnsafe.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IUnsafe {
    /// @notice Transfer <amount> tokens using {magic} spell
    /// @param amount The value { in wei }
    /// @return The new balance
    function transfer(uint256 amount) external returns (uint256);
}
"#,
    );

    prj.add_source(
        "Safe.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IUnsafe.sol";

contract Safe is IUnsafe {
    /// @inheritdoc IUnsafe
    function transfer(uint256 amount) external returns (uint256) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Safe.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    // Inherited notice: bare `<` must be escaped (MDX only requires `<`, not `>`).
    assert!(
        content.contains("&lt;amount>"),
        "inherited `<amount>` should have `<` escaped to `&lt;`, found:\n{content}"
    );
    assert!(
        !content.contains("Transfer <amount>"),
        "raw `<` from inherited notice must not appear unescaped, found:\n{content}"
    );
    // Unresolved {magic} in inherited notice must become inline code.
    assert!(
        content.contains("`magic`"),
        "unresolved {{magic}} should become inline code, found:\n{content}"
    );
    // Unnamed return → &lt;none&gt;.
    assert!(
        content.contains("&lt;none&gt;"),
        "unnamed return should render as `&lt;none&gt;`, found:\n{content}"
    );
    assert!(
        content.contains("| &lt;none&gt; | `uint256` | new balance |"),
        "unnamed return description should be preserved positionally, found:\n{content}"
    );
});

// Test that inline-link labels containing MDX-sensitive characters are escaped.
forgetest_init!(inline_link_label_safety, |prj, cmd| {
    prj.add_source(
        "Token.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Token {
    function transfer(uint256 amount) external {}
}
"#,
    );

    prj.add_source(
        "Vault.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Token.sol";

/// @dev See {Token}[Token <contract>] for details
contract Vault {
    function deposit() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Vault.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    // The label `Token <contract>` must have `<` escaped; it must NOT appear raw.
    assert!(
        !content.contains("Token <contract>"),
        "raw `<` in link label must be escaped, found:\n{content}"
    );
    assert!(
        content.contains("Token &lt;contract>"),
        "link label `<` should be escaped to `&lt;`, found:\n{content}"
    );
});

// Test that the removed `--serve` flag prints a helpful migration message instead of a raw
// clap parse error.
forgetest_init!(serve_flag_prints_migration_message, |prj, cmd| {
    let output = cmd.args(["doc", "--serve"]).assert_failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("npm run dev") || stderr.contains("--serve has been removed"),
        "expected migration message in stderr, got:\n{stderr}"
    );
});

// Test that MDX-unsafe characters in NatSpec are properly escaped in the generated output.
forgetest_init!(mdx_safety_escaping, |prj, cmd| {
    prj.add_source(
        "Escaping.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @notice Contains a bare < angle bracket and a bare { brace.
/// @dev Reference to {UnresolvableRef} should become inline code.
contract Escaping {
    /// @notice Transfer tokens to recipient < address
    /// @param amount The amount { in wei }
    function transfer(uint256 amount) external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Escaping.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("&lt;"),
        "bare `<` should be escaped to `&lt;` in MDX output, found:\n{content}"
    );
    assert!(!content.contains(" < "), "bare `<` should not appear unescaped, found:\n{content}");
    assert!(
        content.contains("&#123;"),
        "bare `{{` should be escaped to `&#123;` in MDX output, found:\n{content}"
    );
    assert!(
        content.contains("`UnresolvableRef`"),
        "unresolved {{Ident}} should become inline code, found:\n{content}"
    );
    assert!(
        !content.contains("{UnresolvableRef}"),
        "unresolved {{Ident}} must not appear raw in MDX output, found:\n{content}"
    );
});

// Test that multiline @param and @return descriptions (continuation lines) are preserved.
forgetest_init!(param_return_multiline_continuation, |prj, cmd| {
    prj.add_source(
        "Multiline.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IMultiline {
    /// @notice Do something
    /// @param value The first line of the description.
    ///        Second line of the param description.
    /// @return result The first line of return.
    ///         Second line of return description.
    function action(uint256 value) external returns (uint256 result);
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/interface.IMultiline.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("The first line of the description."),
        "param first line should appear, found:\n{content}"
    );
    assert!(
        content.contains("Second line of the param description."),
        "param second line should appear, found:\n{content}"
    );
    assert!(
        content.contains("The first line of return."),
        "return first line should appear, found:\n{content}"
    );
    assert!(
        content.contains("Second line of return description."),
        "return second line should appear, found:\n{content}"
    );
});

// Test that overload matching normalizes uint/int aliases inside compound types so that
// `Base.foo(uint256[] calldata)` is correctly matched by `Child.foo(uint[] calldata)`.
forgetest_init!(inheritdoc_overload_normalizes_uint_aliases_in_arrays, |prj, cmd| {
    prj.add_source(
        "IAlias.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IAlias {
    /// @notice Process values
    /// @param values The input array
    function process(uint256[] calldata values) external;
}
"#,
    );

    prj.add_source(
        "Alias.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IAlias.sol";

contract Alias is IAlias {
    /// @inheritdoc IAlias
    function process(uint[] calldata values) external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Alias.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("Process values"),
        "@inheritdoc with uint[] calldata should match uint256[] calldata in base, found:\n{content}"
    );
});

// Test that @inheritdoc resolves docs from a deeply inherited chain
// (Base inherits from an interface without redeclaring NatSpec).
forgetest_init!(inheritdoc_resolves_deep_chain, |prj, cmd| {
    prj.add_source(
        "IBase.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IBase {
    /// @notice Perform the action
    /// @param value The input value
    function action(uint256 value) external;
}
"#,
    );

    prj.add_source(
        "Base.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IBase.sol";

abstract contract Base is IBase {
    // No NatSpec redeclaration, inherits from IBase
    function action(uint256 value) external virtual {}
}
"#,
    );

    prj.add_source(
        "Derived.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Base.sol";

contract Derived is Base {
    /// @inheritdoc Base
    function action(uint256 value) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Derived.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("Perform the action"),
        "@inheritdoc Base should resolve through Base's chain to IBase, found:\n{content}"
    );
});

// Test two rendering behaviors together:
// 1. /** */ block comments are stripped of their ` * ` line decoration.
// 2. `@dev` paragraphs are wrapped in `<i>...</i>` so multi-paragraph content and embedded lists
//    render as italic without breaking block-level markdown.
forgetest_init!(block_comments_strip_star_and_dev_renders_italic, |prj, cmd| {
    prj.add_source(
        "ECDSA.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @notice Library for verifying ECDSA signatures.
 * @dev Elliptic Curve Digital Signature Algorithm (ECDSA) operations.
 *
 * These functions can be used to verify that a message was signed by the holder
 * of the private keys of a given address.
 */
library ECDSA {
    /**
     * @notice Recover the signer address from a signed message hash.
     * @dev Returns the address that signed a hashed message (`hash`) with
     * `signature` or error string.
     *
     * The `ecrecover` EVM opcode allows for malleable (non-unique) signatures:
     * this function rejects them by requiring the `s` value to be in the lower
     * half order, and the `v` value to be either 27 or 28.
     *
     * @param hash The hash of the signed message.
     * @return signer The recovered signer address.
     */
    function tryRecover(bytes32 hash, bytes memory signature) internal pure returns (address signer) {}

    /**
     * @notice Recover the signer address from `v`, `r`, `s` components.
     * @dev Overload of {xref-ECDSA-tryRecover-bytes32-bytes-}[ECDSA.tryRecover] that receives the `v`,
     * `r` and `s` signature fields separately.
     *
     * Documentation for signature generation:
     *
     * - with https://web3js.readthedocs.io/en/v1.3.4/web3-eth-accounts.html#sign[Web3.js]
     * - with https://docs.ethers.io/v5/api/signer/#Signer-signMessage[ethers]
     */
    function tryRecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) internal pure returns (address signer) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/library.ECDSA.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    let expected = r#"---
title: "ECDSA"
description: "Library for verifying ECDSA signatures."
---

# ECDSA

Library for verifying ECDSA signatures.

<i>

Elliptic Curve Digital Signature Algorithm (ECDSA) operations.

These functions can be used to verify that a message was signed by the holder
of the private keys of a given address.

</i>

## Functions

<a id="tryrecover-bytes32-bytes"></a>

### tryRecover

Recover the signer address from a signed message hash.

<i>

Returns the address that signed a hashed message (`hash`) with
`signature` or error string.

The `ecrecover` EVM opcode allows for malleable (non-unique) signatures:
this function rejects them by requiring the `s` value to be in the lower
half order, and the `v` value to be either 27 or 28.

</i>

```solidity
function tryRecover(bytes32 hash, bytes memory signature) internal pure returns (address signer);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| hash | `bytes32` | The hash of the signed message. |
| signature | `bytes` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| signer | `address` | The recovered signer address. |

<a id="tryrecover-bytes32-uint8-bytes32-bytes32"></a>

### tryRecover

Recover the signer address from `v`, `r`, `s` components.

<i>

Overload of [ECDSA.tryRecover](/src/library.ECDSA#tryrecover-bytes32-bytes) that receives the `v`,
`r` and `s` signature fields separately.

Documentation for signature generation:

- with https://web3js.readthedocs.io/en/v1.3.4/web3-eth-accounts.html#sign[Web3.js]
- with https://docs.ethers.io/v5/api/signer/#Signer-signMessage[ethers]

</i>

```solidity
function tryRecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) internal pure returns (address signer);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| hash | `bytes32` |  |
| v | `uint8` |  |
| r | `bytes32` |  |
| s | `bytes32` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| signer | `address` |  |

"#;

    similar_asserts::assert_eq!(content, expected);
});

// Test that @inheritdoc on a public state variable resolves docs from the interface getter
// function (e.g. ERC20's `totalSupply()`).
// fixes <https://github.com/foundry-rs/foundry/pull/14568>
forgetest_init!(inheritdoc_variable_resolves_interface_getter, |prj, cmd| {
    prj.add_source(
        "IERC20.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    /// @notice Returns the total token supply.
    /// @return The total supply.
    function totalSupply() external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "ERC20.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IERC20.sol";

contract ERC20 is IERC20 {
    /// @inheritdoc IERC20
    uint256 public totalSupply;
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.ERC20.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("Returns the total token supply"),
        "@inheritdoc on state variable should inherit notice from interface getter, found:\n{content}"
    );
    assert!(
        content.contains("The total supply"),
        "@inheritdoc on state variable should inherit return docs from interface getter, found:\n{content}"
    );
});

// Test that `**Inherits:**` links resolve to the actually-inherited contract even
// when another contract with the same name lives in a directory closer to the
// consumer. Without exact-id resolution, the proximity heuristic in
// `resolve_page` would (wrongly) link to the same-directory namesake.
forgetest_init!(inheritance_links_use_exact_base_id, |prj, cmd| {
    // Two unrelated `Token` contracts in sibling directories.
    prj.add_source(
        "a/Token.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Token {}
"#,
    );
    prj.add_source(
        "b/Token.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Token {}
"#,
    );

    // Consumer lives next to `a/Token.sol` but explicitly inherits from `b/Token`.
    prj.add_source(
        "a/Consumer.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {Token} from "../b/Token.sol";

contract Consumer is Token {}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/a/contract.Consumer.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("**Inherits:** [Token](/src/b/contract.Token)"),
        "inheritance link must resolve via exact base id to `b/Token`, found:\n{content}"
    );
    assert!(
        !content.contains("[Token](/src/a/contract.Token)"),
        "inheritance link must not fall back to the same-directory namesake `a/Token`, found:\n{content}"
    );
});
