use foundry_test_utils::{
    TestProject, assert_data_eq,
    snapbox::{Data, IntoData},
    str,
    util::{RemoteProject, setup_forge_remote},
};
use std::fs;

mod site;

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc"]).assert_success();
}

#[forgetest_init]
fn doc_does_not_write_artifacts(prj: _, cmd: _) {
    prj.add_source(
        "DocTarget.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract DocTarget {
    /// @notice Returns a value.
    function value() external pure returns (uint256) {
        return 1;
    }
}
"#,
    );

    let artifact = prj.root().join("out/DocTarget.sol/DocTarget.json");
    cmd.args(["doc"]).assert_success();
    assert!(!artifact.exists());

    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, b"sentinel").unwrap();

    cmd.forge_fuse().args(["doc"]).assert_success();
    let after = fs::read(&artifact).unwrap();
    assert_eq!(after, b"sentinel");
}

#[forgetest_init]
fn doc_supports_empty_projects(cmd: _) {
    cmd.arg("doc").assert_success();
}

#[forgetest_init]
fn doc_supports_ignoring_all_sources(prj: _, cmd: _) {
    prj.add_source("Ignored.sol", "contract Ignored {}");
    prj.update_config(|config| config.doc.ignore = vec!["src/**".to_string()]);

    cmd.arg("doc").assert_success();
    assert!(prj.root().join("docs/src/pages/.forge-doc-manifest").exists());
}

#[forgetest_init]
fn doc_uses_configured_commit_for_source_links(prj: _, cmd: _) {
    prj.add_source(
        "Revision.sol",
        r#"
pragma solidity ^0.8.20;

contract Revision {}
"#,
    );
    prj.update_config(|config| {
        config.doc.repository = Some("https://github.com/foundry-rs/foundry".to_string());
        config.doc.commit = Some("v1.2.3".to_string());
    });

    cmd.arg("doc").assert_success();

    prj.assert_doc_page(
        "src/contract.Revision.mdx",
        str![[r#"
...
[Git Source](https://github.com/foundry-rs/foundry/blob/v1.2.3/src/Revision.sol)
...
"#]],
    );
}

#[forgetest]
fn doc_supports_mixed_solidity_versions(prj: _, cmd: _) {
    prj.add_source(
        "New.sol",
        r#"
pragma solidity ^0.8.20;

contract New {}
"#,
    );
    prj.add_source(
        "Old.sol",
        r#"
pragma solidity 0.7.6;

contract Old {}
"#,
    );

    cmd.arg("doc").assert_success();
    assert!(prj.root().join("docs/src/pages/src/contract.New.mdx").exists());
    assert!(prj.root().join("docs/src/pages/src/contract.Old.mdx").exists());
}

#[cfg(unix)]
#[forgetest_init]
fn doc_does_not_run_solc(prj: _, cmd: _) {
    use std::os::unix::fs::PermissionsExt;

    prj.add_source(
        "DocTarget.sol",
        r#"
pragma solidity ^0.8.35;

contract DocTarget {
    /// @notice Returns a value.
    function value() external pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    prj.add_source(
        "Skipped.sol",
        r#"
pragma solidity ^0.8.35;

contract Skipped {}
"#,
    );

    let solc = prj.root().join("fake-solc");
    let invoked = prj.root().join("fake-solc.invoked");
    fs::write(
        &solc,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
    echo "solc, the solidity compiler commandline interface"
    echo "Version: 0.8.35+commit.69074fbd"
    exit 0
fi
touch "$0.invoked"
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&solc).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&solc, permissions).unwrap();

    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Local(solc));
        config.skip = vec!["*Skipped*".parse().unwrap()];
    });

    cmd.arg("doc").assert_success();
    assert!(!invoked.exists(), "forge doc invoked the configured solc binary");
    assert!(!prj.root().join("docs/src/pages/src/contract.Skipped.mdx").exists());
}

// Test that overloaded functions in interfaces inherit the correct NatSpec comments
// fixes <https://github.com/foundry-rs/foundry/issues/11823>
#[forgetest_init]
fn can_generate_docs_for_overloaded_functions(prj: _, cmd: _) {
    prj.add_source(
        "IExample.sol",
        r#"
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
    assert_data_eq!(
        Data::read_from(&doc_path, None),
        str![[r#"
...
<a id="deposit-uint256"></a>

### deposit

Deposit tokens into the vault

```solidity
function deposit(uint256 amount) external;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| amount | `uint256` | The amount to deposit |

<a id="withdraw-uint256"></a>

### withdraw

Withdraw tokens from the vault
...
"#]],
    );
}

// Test that natspec is inherited implicitly from a base interface when the override carries
// no `@inheritdoc` tag.
// fixes <https://github.com/foundry-rs/foundry/issues/4070>
#[forgetest_init]
fn natspec_is_inherited_implicitly(prj: _, cmd: _) {
    prj.add_source(
        "IExample.sol",
        r#"
interface IExample {
    /// @notice Deposit tokens into the vault
    /// @param amount The amount to deposit
    /// @return shares The amount of shares minted
    function deposit(uint256 amount) external returns (uint256 shares);
}
"#,
    );

    prj.add_source(
        "Example.sol",
        r#"
import "./IExample.sol";

contract Example is IExample {
    function deposit(uint256 amount) external override returns (uint256 shares) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let doc_path = prj.root().join("docs/src/pages/src/contract.Example.mdx");
    assert_data_eq!(
        Data::read_from(&doc_path, None),
        str![[r#"
...
<a id="deposit-uint256"></a>

### deposit

Deposit tokens into the vault

```solidity
function deposit(uint256 amount) external override returns (uint256 shares);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| amount | `uint256` | The amount to deposit |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| shares | `uint256` | The amount of shares minted |
...
"#]],
    );
}

#[forgetest_init]
fn inheritdoc_uses_effective_positional_natspec(prj: _, cmd: _) {
    prj.add_source(
        "IRoot.sol",
        r#"
interface IRoot {
    /// @notice Root notice
    /// @dev Root dev
    /// @param first Root first
    /// @param second Root second
    /// @return firstResult Root first result
    /// @return secondResult Root second result
    function run(uint256 first, uint256 second)
        external
        returns (uint256 firstResult, uint256 secondResult);
}
"#,
    );
    prj.add_source(
        "Effective.sol",
        r#"
import {IRoot as RootAlias} from "./IRoot.sol";

interface IMid is RootAlias {
    /// @inheritdoc RootAlias
    /// @dev Mid dev
    function run(uint256 left, uint256 right)
        external
        override
        returns (uint256 leftResult, uint256 rightResult);
}

contract Effective is IMid {
    /// @inheritdoc IMid
    /// @param currentLeft Local left
    function run(uint256 currentLeft, uint256 currentRight)
        external
        override
        returns (uint256 currentLeftResult, uint256 currentRightResult)
    {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.Effective.mdx",
        str![[r#"
<a id="run-uint256-uint256"></a>

### run

Root notice

<i>

Mid dev

</i>

```solidity
function run(uint256 currentLeft, uint256 currentRight)
        external
        override
        returns (uint256 currentLeftResult, uint256 currentRightResult);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| currentLeft | `uint256` | Local left |
| currentRight | `uint256` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| currentLeftResult | `uint256` | Root first result |
| currentRightResult | `uint256` | Root second result |


"#]],
    );
}

#[forgetest_init]
fn inheritdoc_documents_unnamed_parameters(prj: _, cmd: _) {
    prj.add_source(
        "Unnamed.sol",
        r#"
interface IProcessor {
    /// @param amount The amount to process
    function single(uint256 amount) external;

    /// @param first The first value
    /// @param third The third value
    function sparse(uint256 first, address second, bytes32 third) external;

    /// @param first The named underscore
    /// @param second The unnamed value
    function underscoreCollision(uint256 first, uint256 second) external;

    /// @param first The named display value
    /// @param second The custom-named value
    function customNameCollision(uint256 first, uint256 second) external;
}

contract Processor is IProcessor {
    /// @inheritdoc IProcessor
    function single(uint256) external override {}

    /// @inheritdoc IProcessor
    function sparse(uint256, address, bytes32) external override {}

    /// @inheritdoc IProcessor
    function underscoreCollision(uint256 _, uint256) external override {}

    /// @inheritdoc IProcessor
    /// @custom:name display
    function customNameCollision(uint256 display, uint256) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.Processor.mdx",
        str![[r#"
...
### single
...
| Name | Type | Description |
| ---- | ---- | ----------- |
| _ | `uint256` | The amount to process |
...
### sparse
...
| Name | Type | Description |
| ---- | ---- | ----------- |
| _ | `uint256` | The first value |
| _ | `address` |  |
| _ | `bytes32` | The third value |
...
### underscoreCollision
...
| Name | Type | Description |
| ---- | ---- | ----------- |
| _ | `uint256` | The named underscore |
| _ | `uint256` | The unnamed value |
...
### customNameCollision
...
| Name | Type | Description |
| ---- | ---- | ----------- |
| display | `uint256` | The named display value |
| display | `uint256` | The custom-named value |
...
"#]],
    );
}

#[forgetest_init]
fn inheritdoc_mapping_getter_uses_generated_signature(prj: _, cmd: _) {
    prj.add_source(
        "ExplicitGetter.sol",
        r#"
interface IValues {
    /// @notice Reads a value
    /// @param key The lookup key
    /// @return value The stored value
    function values(uint256 key) external view returns (uint256 value);
}

contract ExplicitGetter is IValues {
    /// @inheritdoc IValues
    mapping(uint256 => uint256) public override values;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.ExplicitGetter.mdx",
        str![[r#"
...
### values

Reads a value

```solidity
mapping(uint256 => uint256) public override values;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | The lookup key |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | The stored value |
...
"#]],
    );
}

#[forgetest_init]
fn inheritdoc_does_not_skip_exact_custom_documentation(prj: _, cmd: _) {
    prj.add_source(
        "Exact.sol",
        r#"
abstract contract Root {
    /// @notice Must not leak through Mid
    function run(uint256 value) public virtual {}
}

abstract contract Mid is Root {
    /// @custom:audit reviewed
    function run(uint256 value) public virtual override {}
}

contract Exact is Mid {
    /// @inheritdoc Mid
    function run(uint256 value) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.Exact.mdx",
        str![[r#"
<a id="run-uint256"></a>

### run

```solidity
function run(uint256 value) public override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` |  |


"#]],
    );
}

#[forgetest_init]
fn implicit_inheritance_requires_compatible_override(prj: _, cmd: _) {
    prj.add_source(
        "Compatibility.sol",
        r#"
abstract contract CompatibilityBase {
    /// @notice Must not inherit from a non-virtual function
    function nonVirtual() public {}

    /// @notice Must not inherit across visibility changes
    function visibilityChange() public virtual {}

    /// @notice Must not inherit across mutability changes
    function mutabilityChange() public view virtual {}

    /// @notice Must not inherit across return type changes
    function returnChange() public virtual returns (uint256) {}
}

contract Compatibility is CompatibilityBase {
    function nonVirtual() public override {}
    function visibilityChange() external override {}
    function mutabilityChange() public override {}
    function returnChange() public override returns (address) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.Compatibility.mdx",
        str![[r#"
<a id="nonvirtual"></a>

### nonVirtual

```solidity
function nonVirtual() public override;
```

<a id="visibilitychange"></a>

### visibilityChange

```solidity
function visibilityChange() external override;
```

<a id="mutabilitychange"></a>

### mutabilityChange

```solidity
function mutabilityChange() public override;
```

<a id="returnchange"></a>

### returnChange

```solidity
function returnChange() public override returns (address);
```

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `address` |  |


"#]],
    );
}

#[forgetest_init]
fn inheritdoc_getter_handles_malformed_return_arity(prj: _, cmd: _) {
    prj.add_source(
        "MalformedGetter.sol",
        r#"
interface IFlag {
    /// @notice Reads the flag
    function flag() external view;
}

contract MalformedGetter is IFlag {
    /// @inheritdoc IFlag
    uint256 public override flag;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.MalformedGetter.mdx"))
            .unwrap();
    assert!(rendered.contains("Reads the flag"), "{rendered}");
}

#[forgetest_init]
fn inheritdoc_uses_first_duplicate_target(prj: _, cmd: _) {
    prj.add_source(
        "DuplicateInheritdoc.sol",
        r#"
abstract contract RootA {
    /// @notice First target
    function run() public virtual {}
}

abstract contract A is RootA {
    function run() public virtual override {}
}

abstract contract RootB {
    /// @notice Second target
    function run() public virtual {}
}

abstract contract B is RootB {
    function run() public virtual override {}
}

contract DuplicateInheritdoc is A, B {
    /// @inheritdoc A
    /// @inheritdoc B
    function run() public override(A, B) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.DuplicateInheritdoc.mdx"))
            .unwrap();
    assert!(rendered.contains("First target"), "{rendered}");
    assert!(!rendered.contains("Second target"), "{rendered}");
}

#[forgetest_init]
fn implicit_inheritance_matches_constant_getter_mutability(prj: _, cmd: _) {
    prj.add_source(
        "ConstantGetter.sol",
        r#"
interface IConstant {
    /// @notice The constant value
    function VALUE() external pure returns (uint256);
}

contract ConstantGetter is IConstant {
    uint256 public constant override VALUE = 1;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.ConstantGetter.mdx"))
            .unwrap();
    assert!(rendered.contains("The constant value"), "{rendered}");
}

#[forgetest_init]
fn implicit_inheritance_rejects_external_return_location_mismatch(prj: _, cmd: _) {
    prj.add_source(
        "ReturnLocation.sol",
        r#"
abstract contract ReturnBase {
    /// @notice Must not cross a return-location mismatch
    function data() external view virtual returns (bytes memory);
}

contract ReturnLocation is ReturnBase {
    bytes private stored;

    function data() public view override returns (bytes storage value) {
        value = stored;
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.ReturnLocation.mdx",
        str![[r#"
<a id="data"></a>

### data

```solidity
function data() public view override returns (bytes storage value);
```

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `bytes` |  |


"#]],
    );
}

// NatSpec text must never reach the MDX page as executable ESM: MDX runs a line whose first
// token is `import`/`export` as code. The text can even be inherited from another contract
// through `@inheritdoc`, so a dependency's doc comment could inject into the derived page.
#[forgetest_init]
fn natspec_neutralizes_esm_statement_lines(prj: _, cmd: _) {
    prj.add_source(
        "EsmBase.sol",
        r#"
interface IEsm {
    /// @notice export const injected = 1
    function act(uint256 v) external;
}
"#,
    );
    prj.add_source(
        "EsmChild.sol",
        r#"
import "./EsmBase.sol";

/// @notice import somesecret from the outside
contract EsmChild is IEsm {
    /// @inheritdoc IEsm
    function act(uint256 v) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.EsmChild.mdx",
        str![[r#"
---
title: "EsmChild"
description: "import somesecret from the outside"
---

# EsmChild

**Inherits:** [IEsm](/src/interface.IEsm)

&#105;&#109;port somesecret from the outside

## Functions

<a id="act-uint256"></a>

### act

&#101;xport const injected = 1

```solidity
function act(uint256 v) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| v | `uint256` |  |


"#]],
    );
}

#[forgetest_init]
fn homepage_neutralizes_esm_statement_lines(prj: _, cmd: _) {
    prj.add_source("Probe.sol", "contract Probe {}");
    fs::write(
        prj.root().join("README.md"),
        concat!(
            "\u{feff}",
            r#"import fs from "node:fs"

# Probe

export const generated = fs.writeFileSync("marker", "")

Replace <TOKEN>, see `wrapped
import span` here.

```js
import inert from "fenced"
```
"#
        ),
    )
    .unwrap();

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "index.mdx",
        str![[r#"
&#105;&#109;port fs from "node:fs"

# Probe

&#101;xport const generated = fs.writeFileSync("marker", "")

Replace &lt;TOKEN>, see `wrapped
import span` here.

```js
import inert from "fenced"
```

"#]],
    );
}

#[forgetest_init]
fn natspec_fences_are_limited_to_standalone_descriptions(prj: _, cmd: _) {
    prj.add_source(
        "FenceScope.sol",
        r#"
interface IFenced {
    /// @dev Example:
    /// ```solidity
    /**
     * if (value < 1) {
     * ```
     * Outside < and {
     */
    /// @param value Parameter example:
    /// ~~~solidity
    /// if (value < 1) {
    /// ~~~
    function inspect(uint256 value) external;
}

contract Child is IFenced {
    /// @inheritdoc IFenced
    function inspect(uint256 value) external override {}
}

/**
 * @title Metadata
 * ~~~
 * @author Author
 * ```
 * @custom:note Note
 * ~~~~
 * @notice ~~~
 * {1+1}
 * ~~~
 */
contract Metadata {
    /// @custom:name {1+1}
    /// @custom:name <name>
    function inspect(uint256, uint256) external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    for page in ["interface.IFenced.mdx", "contract.Child.mdx"] {
        assert_data_eq!(
            Data::read_from(&prj.root().join("docs/src/pages/src").join(page), None),
            str![[r#"
...
### inspect

<i>

Example:
```solidity
if (value < 1) {
```
Outside &lt; and &#123;

</i>

...
**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` | Parameter example:<br/>~~~solidity<br/>if (value &lt; 1) &#123;<br/>~~~ |
...
"#]],
        );
    }
    prj.assert_doc_page(
        "src/contract.Metadata.mdx",
        str![[r#"
...
# Metadata

**Title:** Metadata
&#126;~~

**Author:** Author
&#96;``

~~~
{1+1}
~~~

**Note:**

- **note:** Note
&#126;~~~

...
### inspect

...
| `1+1` | `uint256` |  |
| &lt;name> | `uint256` |  |
...
"#]],
    );
}

#[forgetest_init]
fn multiline_notice_populates_frontmatter_description(prj: _, cmd: _) {
    prj.add_source(
        "Vault.sol",
        r#"
/// @notice Stores deposited assets for users
///         and enforces withdrawal limits.
contract Vault {}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.Vault.mdx",
        str![[r#"
---
title: "Vault"
description: "Stores deposited assets for users and enforces withdrawal limits."
---

# Vault

Stores deposited assets for users
and enforces withdrawal limits.


"#]],
    );
}

// An override inherits the base overload with the matching signature, continuing past a nearer
// base that declares a different same-name overload.
#[forgetest_init]
fn implicit_inheritance_matches_the_overload_signature(prj: _, cmd: _) {
    prj.add_source(
        "Bases.sol",
        r#"
interface INear {
    function g(address a) external returns (bool);
}

interface IFar {
    /// @notice Far documents g(uint256)
    function g(uint256 n) external returns (bool);
}
"#,
    );

    prj.add_source(
        "Impl.sol",
        r#"
import "./Bases.sol";

contract Impl is INear, IFar {
    function g(uint256 n) external override(IFar) returns (bool) {}
    function g(address a) external override(INear) returns (bool) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_functions(
        "src/contract.Impl.mdx",
        str![[r#"
<a id="g-uint256"></a>

### g

Far documents g(uint256)

```solidity
function g(uint256 n) external override(IFar) returns (bool);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| n | `uint256` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `bool` |  |

<a id="g-address"></a>

### g

```solidity
function g(address a) external override(INear) returns (bool);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| a | `address` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `bool` |  |


"#]],
    );
}

// A public mapping variable inherits the NatSpec of the interface getter it implements, matched
// through the getter's generated signature (`balanceOf(address)`).
#[forgetest_init]
fn implicit_inheritance_matches_mapping_getter_signature(prj: _, cmd: _) {
    prj.add_source(
        "IERC.sol",
        r#"
interface IERC {
    /// @notice The balance of an account
    function balanceOf(address account) external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "Token.sol",
        r#"
import "./IERC.sol";

contract Token is IERC {
    mapping(address owner => uint256 amount) public override balanceOf;
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Token.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    assert!(rendered.contains("The balance of an account"), "{rendered}");
}

// A public mapping with a `string` key inherits through its synthetic getter: the getter's
// generated signature matches the interface function with the location normalized.
#[forgetest_init]
fn implicit_inheritance_matches_string_key_getter(prj: _, cmd: _) {
    prj.add_source(
        "IRegistry.sol",
        r#"
interface IRegistry {
    /// @notice The balance registered for a name
    function balances(string memory name) external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "Registry.sol",
        r#"
import "./IRegistry.sol";

contract Registry is IRegistry {
    mapping(string => uint256) public override balances;
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Registry.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    assert!(rendered.contains("The balance registered for a name"), "{rendered}");
}

// `calldata` in a base member and `memory` in the override are the same signature: locations
// are normalized before comparison and the NatSpec is inherited.
#[forgetest_init]
fn implicit_inheritance_normalizes_calldata_location(prj: _, cmd: _) {
    prj.add_source(
        "Base.sol",
        r#"
interface Base {
    /// @notice Configures the value
    function configure(bytes calldata data) external;
}
"#,
    );

    prj.add_source(
        "Child.sol",
        r#"
import "./Base.sol";

contract Child is Base {
    function configure(bytes memory data) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Child.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    assert!(rendered.contains("Configures the value"), "{rendered}");
}

// Implicit inheritance normalizes divergent mapping spellings, but must not document a
// different mapping overload even when the base has a single same-name declaration.
#[forgetest_init]
fn implicit_inheritance_rejects_non_abi_overload_mismatch(prj: _, cmd: _) {
    prj.add_source(
        "Store.sol",
        r#"
abstract contract Store {
    /// @notice Configures the store
    function configure(mapping(uint => uint) storage store_) internal virtual;
}
"#,
    );

    prj.add_source(
        "MyStore.sol",
        r#"
import "./Store.sol";

contract MyStore is Store {
    function configure(mapping(uint=>uint) storage store_) internal override {}
    function configure(mapping(address => address) storage other) internal {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_functions(
        "src/contract.MyStore.mdx",
        str![[r#"
<a id="configure-mapping-uint256-uint256"></a>

### configure

Configures the store

```solidity
function configure(mapping(uint=>uint) storage store_) internal override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| store_ | `mapping(uint=>uint)` |  |

<a id="configure-mapping-address-address"></a>

### configure

```solidity
function configure(mapping(address => address) storage other) internal;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| other | `mapping(address => address)` |  |


"#]],
    );
}
// Point 2 (mablr review): names are compared at every level. A leaf cannot jump across an
// intermediate rename just because it restores the original name.
#[forgetest_init]
fn implicit_inheritance_requires_matching_param_names(prj: _, cmd: _) {
    prj.add_source(
        "Rename.sol",
        r#"
contract Base {
    /// @notice Deposits into the vault
    function deposit(uint256 amount) public virtual returns (uint256) {}
}

contract Mid is Base {
    function deposit(uint256 shares) public virtual override returns (uint256) {}
}

contract Leaf is Mid {
    function deposit(uint256 amount) public override returns (uint256) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    for contract in ["Mid", "Leaf"] {
        let rendered = fs::read_to_string(
            prj.root().join(format!("docs/src/pages/src/contract.{contract}.mdx")),
        )
        .unwrap();
        assert!(!rendered.contains("Deposits into the vault"), "{rendered}");
    }
}

// Point 3 (mablr review): the target needs a public getter, and the source needs to be an
// external function implemented by that getter. A same-name base variable is not a source.
#[forgetest_init]
fn implicit_inheritance_requires_public_getter_and_function_source(prj: _, cmd: _) {
    prj.add_source(
        "Variables.sol",
        r#"
contract Base {
    /// @notice Must not reach a private target
    uint256 private privateTarget;

    /// @notice A variable is not a getter function
    uint256 private variableSource;
}

contract Child is Base {
    uint256 private privateTarget;
    uint256 public variableSource;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.Child.mdx")).unwrap();
    assert!(!rendered.contains("Must not reach a private target"), "{rendered}");
    assert!(!rendered.contains("A variable is not a getter function"), "{rendered}");
}

// Point 1 (mablr review): automatic inheritance needs one semantic base function. Distinct
// declarations on separate branches are ambiguous; a declaration shared by both branches is not.
#[forgetest_init]
fn implicit_inheritance_resolves_base_ambiguity_per_branch(prj: _, cmd: _) {
    prj.add_source(
        "Ambiguity.sol",
        r#"
interface IAlpha {
    /// @notice From IAlpha
    function direct(uint256 x) external;
}

interface IBeta {
    /// @notice From IBeta
    function direct(uint256 x) external;
}

contract Direct is IAlpha, IBeta {
    function direct(uint256 x) external override(IAlpha, IBeta) {}
}

contract Root {
    /// @notice Root branch doc
    function act(uint256 x) public virtual {}
}

contract A is Root {
    /// @notice A branch doc
    function act(uint256 x) public virtual override {}
}

contract B is Root {}

contract Asymmetric is A, B {
    function act(uint256 x) public virtual override(A, Root) {}
}

contract Leaf is Asymmetric {
    function act(uint256 x) public override {}
}

contract SharedRoot {
    /// @notice Shared root doc
    function shared(uint256 x) public virtual {}
}

contract Left is SharedRoot {}
contract Right is SharedRoot {}

contract Shared is Left, Right {
    function shared(uint256 x) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.Direct.mdx",
        str![[r#"
...
### direct

```solidity
function direct(uint256 x) external override(IAlpha, IBeta);
```
...
"#]],
    );
    prj.assert_doc_page(
        "src/contract.Asymmetric.mdx",
        str![[r#"
...
### act

```solidity
function act(uint256 x) public virtual override(A, Root);
```
...
"#]],
    );
    prj.assert_doc_page(
        "src/contract.Leaf.mdx",
        str![[r#"
...
### act

```solidity
function act(uint256 x) public override;
```
...
"#]],
    );
    prj.assert_doc_page(
        "src/contract.Shared.mdx",
        str![[r#"
...
### shared

Shared root doc

```solidity
function shared(uint256 x) public override;
```
...
"#]],
    );
}

// Point 5 (mablr review): any local NatSpec item suppresses automatic inheritance. A leaf
// cannot reach around an intermediate override carrying only a custom tag.
#[forgetest_init]
fn implicit_inheritance_skips_custom_tagged_members(prj: _, cmd: _) {
    prj.add_source(
        "Tagged.sol",
        r#"
contract Base {
    /// @notice Base notice
    function run(uint256 amount) public virtual returns (uint256) {}
}

contract Mid is Base {
    /// @custom:audit reviewed
    function run(uint256 amount) public virtual override returns (uint256) {}
}

contract Leaf is Mid {
    function run(uint256 amount) public override returns (uint256) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    for contract in ["Mid", "Leaf"] {
        let rendered = fs::read_to_string(
            prj.root().join(format!("docs/src/pages/src/contract.{contract}.mdx")),
        )
        .unwrap();
        assert!(!rendered.contains("Base notice"), "{rendered}");
    }
}

// Implicit inheritance only runs when the override has no NatSpec of its own: a local `@notice`
// keeps the base `@param`/`@return` from being pulled in.
#[forgetest_init]
fn implicit_inheritance_skips_documented_members(prj: _, cmd: _) {
    prj.add_source(
        "IExample.sol",
        r#"
interface IExample {
    /// @notice Base notice
    /// @param amount base amount doc
    /// @return shares base shares doc
    function deposit(uint256 amount) external returns (uint256 shares);
}
"#,
    );

    prj.add_source(
        "Example.sol",
        r#"
import "./IExample.sol";

contract Example is IExample {
    /// @notice Local notice only
    function deposit(uint256 amount) external override returns (uint256 shares) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Example.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    // The local notice is kept.
    assert!(rendered.contains("Local notice only"), "{rendered}");
    // The base param and return docs are not pulled in, since the override is documented.
    assert!(!rendered.contains("base amount doc"), "{rendered}");
    assert!(!rendered.contains("base shares doc"), "{rendered}");
}

// Point 4 (mablr review): render every parameter and return of the implemented getter. A
// missing parameter tag leaves its own row empty instead of borrowing another description.
#[forgetest_init]
fn inherited_getter_renders_param_and_return(prj: _, cmd: _) {
    prj.add_source(
        "Entries.sol",
        r#"
struct Entry {
    uint256 amount;
    bool active;
}

interface IEntries {
    /// @notice The entry for an account
    /// @param account the account to query
    /// @return amount the stored amount
    /// @return active whether the entry is active
    function entries(address account, uint256 tokenId)
        external
        view
        returns (uint256 amount, bool active);
}

contract Entries is IEntries {
    mapping(address owner => mapping(uint256 id => Entry entry)) public override entries;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.Entries.mdx")).unwrap();
    assert!(rendered.contains("| owner | `address` | the account to query |"), "{rendered}");
    assert!(rendered.contains("| id | `uint256` |  |"), "{rendered}");
    assert!(rendered.contains("| amount | `uint256` | the stored amount |"), "{rendered}");
    assert!(rendered.contains("| active | `bool` | whether the entry is active |"), "{rendered}");
}

// steven review: an intermediate override's `@inheritdoc` is resolved and merged, not treated
// as terminal, so documentation propagates through it. A (documented) -> B (@inheritdoc A) ->
// C (undocumented): C receives A's documentation through B.
#[forgetest_init]
fn implicit_inheritance_resolves_intermediate_inheritdoc(prj: _, cmd: _) {
    prj.add_source(
        "Chain.sol",
        r#"
interface IChainBase {
    /// @notice Documented on the interface
    function act(uint256 amount) external;
}

abstract contract ChainMid is IChainBase {
    /// @inheritdoc IChainBase
    function act(uint256 amount) public virtual override {}
}

contract ChainLeaf is ChainMid {
    function act(uint256 amount) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.ChainLeaf.mdx")).unwrap();
    assert!(rendered.contains("Documented on the interface"), "{rendered}");
}

// steven review: an inherited `@return` maps positionally onto a renamed override's return
// slot, instead of gluing the base return name into the description.
#[forgetest_init]
fn implicit_inheritance_remaps_renamed_returns(prj: _, cmd: _) {
    prj.add_source(
        "Renamed.sol",
        r#"
interface IRenamed {
    /// @notice Reads a value
    /// @return first the first result
    function take(uint256 v) external returns (uint256 first);
}

contract Renamed is IRenamed {
    function take(uint256 v) external override returns (uint256 renamedFirst) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.Renamed.mdx")).unwrap();
    assert!(rendered.contains("| renamedFirst | `uint256` | the first result |"), "{rendered}");
    assert!(!rendered.contains("first the first result"), "{rendered}");
}

// Regression: return-name resolution for the implicit path must not leak into the explicit
// `@inheritdoc` path. With a partial local `@return` over a named-return override, the local
// description must win and the base's other returns must not be injected (matches master).
#[forgetest_init]
fn explicit_inheritdoc_partial_return_keeps_local_and_skips_base(prj: _, cmd: _) {
    prj.add_source(
        "PartialReturn.sol",
        r#"
interface IPartial {
    /// @notice Base notice
    /// @return a base A-text
    /// @return b base B-text
    function f() external view returns (uint256 a, uint256 b);
}

contract Partial is IPartial {
    /// @inheritdoc IPartial
    /// @return a local A-text
    function f() external view override returns (uint256 a, uint256 b) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.Partial.mdx",
        str![[r#"
<a id="f"></a>

### f

Base notice

```solidity
function f() external view override returns (uint256 a, uint256 b);
```

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| a | `uint256` | local A-text |
| b | `uint256` |  |


"#]],
    );
}

// A public state variable's generated getter inherits implicitly through an interface chain:
// a base function redeclared without NatSpec still propagates its ancestor's documentation, like
// solc (Impl.data() resolves to IRoot's `@notice` through the undocumented IMid redeclaration).
#[forgetest_init]
fn implicit_getter_inherits_through_interface_chain(prj: _, cmd: _) {
    prj.add_source(
        "GetterChain.sol",
        r#"
interface IRoot {
    /// @notice Root getter doc
    /// @return value the stored value
    function data() external view returns (uint256 value);
}

interface IMid is IRoot {
    function data() external view override returns (uint256 value);
}

contract Impl is IMid {
    uint256 public override data;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.Impl.mdx")).unwrap();
    assert!(rendered.contains("Root getter doc"), "{rendered}");
    assert!(rendered.contains("the stored value"), "{rendered}");
}

// A private base function is not overridden by a same-signature child function and cannot donate
// its documentation to it.
// `forge doc` can render parseable sources that Solidity would reject later. A private
// same-signature declaration is still not a valid override source for inherited docs.
#[forgetest_init]
fn implicit_inheritance_rejects_private_base(prj: _, cmd: _) {
    prj.add_source(
        "PrivateBase.sol",
        r#"
contract PrivateBase {
    /// @notice Must not escape a private declaration
    function privateCandidate(uint256 value) private {}
}

contract PrivateLeaf is PrivateBase {
    function privateCandidate(uint256 value) public {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.PrivateLeaf.mdx",
        str![[r#"
<a id="privatecandidate-uint256"></a>

### privateCandidate

```solidity
function privateCandidate(uint256 value) public;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` |  |


"#]],
    );
}

// A lowered Yul helper is not part of Solidity's override frontier. It must not shadow the real
// Solidity declaration in the next ancestor. Solar lowers Yul helpers as private, so this pins the
// effective boundary instead of proving `is_yul` independently from private visibility.
#[forgetest_init]
fn implicit_inheritance_ignores_yul_shadow(prj: _, cmd: _) {
    prj.add_source(
        "YulShadow.sol",
        r#"
contract YulRoot {
    /// @notice Must pass through the Yul-only intermediate declaration
    function yulCandidate(uint256 value) public virtual {}
}

contract YulMid is YulRoot {
    function helper(uint256 input) public pure returns (uint256 output) {
        assembly {
            function yulCandidate(shadow) -> result { result := shadow }
            output := yulCandidate(input)
        }
    }
}

contract YulMid2 is YulMid {
    function helper2(uint256 input) public pure returns (uint256 output) {
        assembly {
            function yulCandidate(shadow) -> result { result := shadow }
            output := yulCandidate(input)
        }
    }
}

contract YulLeaf is YulMid2 {
    function yulCandidate(uint256 value) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.YulLeaf.mdx")).unwrap();
    assert!(rendered.contains("Must pass through"), "{rendered}");
}

// A generated getter is not an ordinary function declaration on the override frontier.
#[forgetest_init]
fn implicit_inheritance_rejects_generated_getter_base(prj: _, cmd: _) {
    prj.add_source(
        "GetterBase.sol",
        r#"
contract GetterBase {
    /// @notice Must not escape a generated getter
    uint256 public getterCandidate;
}

contract GetterLeaf is GetterBase {
    function getterCandidate() public {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.GetterLeaf.mdx",
        str![[r#"
<a id="gettercandidate"></a>

### getterCandidate

```solidity
function getterCandidate() public;
```


"#]],
    );
}

// `forge doc` lowers parseable sources without running Solidity's full override validation.
// Even for an invalid cross-domain collision, it must not copy modifier docs onto a function.
#[forgetest_init]
fn implicit_inheritance_keeps_function_modifier_domains_separate(prj: _, cmd: _) {
    prj.add_source(
        "FunctionModifier.sol",
        r#"
contract ModifierBase {
    /// @notice A modifier is not a function override
    modifier sameSpelling() { _; }
}

contract FunctionLeaf is ModifierBase {
    function sameSpelling() public {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_functions(
        "src/contract.FunctionLeaf.mdx",
        str![[r#"
<a id="samespelling"></a>

### sameSpelling

```solidity
function sameSpelling() public;
```


"#]],
    );
}

// Fallback and receive have no AST header name, but they still take part in explicit and
// implicit NatSpec inheritance through their HIR function kinds.
#[forgetest_init]
fn inheritance_supports_fallback_and_receive(prj: _, cmd: _) {
    prj.add_source(
        "SpecialFunctions.sol",
        r#"
abstract contract SpecialBase {
    /// @notice Base fallback documentation
    fallback() external virtual {}

    /// @notice Base receive documentation
    receive() external payable virtual {}
}

contract SpecialImplicit is SpecialBase {
    fallback() external override {}
    receive() external payable override {}
}

contract SpecialExplicit is SpecialBase {
    /// @inheritdoc SpecialBase
    fallback(bytes calldata input) external override returns (bytes memory output) {
        input;
        return output;
    }

    /// @inheritdoc SpecialBase
    receive() external payable override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    for contract in ["SpecialImplicit", "SpecialExplicit"] {
        let rendered = fs::read_to_string(
            prj.root().join(format!("docs/src/pages/src/contract.{contract}.mdx")),
        )
        .unwrap();
        let fallback_start = rendered.find("### fallback").unwrap();
        let receive_start = rendered.find("### receive").unwrap();
        let fallback = &rendered[fallback_start..receive_start];
        let receive = &rendered[receive_start..];
        assert!(fallback.contains("Base fallback documentation"), "{rendered}");
        assert!(!fallback.contains("Base receive documentation"), "{rendered}");
        assert!(receive.contains("Base receive documentation"), "{rendered}");
        assert!(!receive.contains("Base fallback documentation"), "{rendered}");
    }
}

// Return descriptions are remapped at each override hop before a generated getter consumes
// them. The final rows use the getter field names, not either interface's return names.
#[forgetest_init]
fn implicit_getter_remaps_returns_at_every_hop(prj: _, cmd: _) {
    prj.add_source(
        "ReturnChain.sol",
        r#"
struct Pair {
    uint256 getterFirst;
    uint256 getterSecond;
}

interface IRootPair {
    /// @return originalFirst first value documentation
    /// @return originalSecond second value documentation
    function pair(uint256 key)
        external
        view
        returns (uint256 originalFirst, uint256 originalSecond);
}

interface IMiddlePair is IRootPair {
    function pair(uint256 key)
        external
        view
        override
        returns (uint256 middleFirst, uint256 middleSecond);
}

contract PairStore is IMiddlePair {
    mapping(uint256 key => Pair value) public override pair;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.PairStore.mdx")).unwrap();
    assert!(
        rendered.contains("| getterFirst | `uint256` | first value documentation |"),
        "{rendered}"
    );
    assert!(
        rendered.contains("| getterSecond | `uint256` | second value documentation |"),
        "{rendered}"
    );
}

// An explicit `@inheritdoc` relay remaps inherited return names before the getter consumes them.
#[forgetest_init]
fn implicit_getter_remaps_returns_after_inheritdoc_relay(prj: _, cmd: _) {
    prj.add_source(
        "ExplicitReturnChain.sol",
        r#"
struct ExplicitPair {
    uint256 getterFirst;
    uint256 getterSecond;
}

interface IExplicitRoot {
    /// @return originalFirst first relayed value
    /// @return originalSecond second relayed value
    function relayedPair(uint256 key)
        external
        view
        returns (uint256 originalFirst, uint256 originalSecond);
}

interface IExplicitMiddle is IExplicitRoot {
    /// @inheritdoc IExplicitRoot
    /// @notice Relayed through the middle interface
    function relayedPair(uint256 key)
        external
        view
        override
        returns (uint256 middleFirst, uint256 middleSecond);
}

contract ExplicitPairStore is IExplicitMiddle {
    mapping(uint256 key => ExplicitPair value) public override relayedPair;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.ExplicitPairStore.mdx",
        str![[r#"
...
### relayedPair

Relayed through the middle interface
...
**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| getterFirst | `uint256` | first relayed value |
| getterSecond | `uint256` | second relayed value |
...
"#]],
    );
}

// Getter tables use the same NatSpec sanitizer as ordinary functions, including the escaped
// placeholder for an unnamed generated return.
#[forgetest_init]
fn inherited_getter_sanitizes_mdx_and_unnamed_returns(prj: _, cmd: _) {
    prj.add_source(
        "UnsafeGetter.sol",
        r#"
interface IUnsafeGetter {
    /// @param key Locate <amount> with {Reference}
    /// @return Result <amount> from {Reference}
    function values(uint256 key) external view returns (uint256);
}

contract UnsafeGetter is IUnsafeGetter {
    mapping(uint256 => uint256) public override values;
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.UnsafeGetter.mdx",
        str![[r#"
...
**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | Locate &lt;amount> with `Reference` |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | Result &lt;amount> from `Reference` |
...
"#]],
    );
}

// Test that {Ident} cross-references resolve to root-relative vocs links.
// fixes <https://github.com/foundry-rs/foundry/issues/12361>
#[forgetest_init]
fn hyperlinks_use_relative_paths(prj: _, cmd: _) {
    prj.add_source(
        "IBase.sol",
        r#"
interface IBase {
    function baseFunction() external;
}
"#,
    );

    prj.add_source(
        "Derived.sol",
        r#"
import "./IBase.sol";

/// @dev Inherits: {IBase}
contract Derived is IBase {
    function baseFunction() external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Derived.mdx",
        str![[r#"
...
Inherits: [IBase](/src/interface.IBase)
...
"#]],
    );
}

// Keep constants-only, state-only, and mixed layouts covered independently.
#[forgetest_init]
fn variable_sections(prj: _, cmd: _) {
    let constants = "uint256 public constant FOO = 1;\nuint256 public immutable BAR;";
    let state = "uint256 public baz;";
    let constructor = "constructor() { BAR = 2; }";
    let increment = "function increment() public { baz++; }";
    let constants_page = r#"## Constants

### FOO

```solidity
uint256 public constant FOO = 1;
```

### BAR

```solidity
uint256 public immutable BAR;
```

"#;
    let state_page = r#"## State Variables

### baz

```solidity
uint256 public baz;
```

"#;
    let constructor_page = r#"<a id="constructor"></a>

### constructor

```solidity
constructor();
```

"#;
    let increment_page = r#"<a id="increment"></a>

### increment

```solidity
function increment() public;
```

"#;
    let cases = [
        (
            "CounterConstants",
            constants.to_string(),
            constructor.to_string(),
            constants_page.to_string(),
            constructor_page.to_string(),
        ),
        (
            "CounterStateVariables",
            state.to_string(),
            increment.to_string(),
            state_page.to_string(),
            increment_page.to_string(),
        ),
        (
            "CounterMixedVariables",
            format!("{constants}\n{state}"),
            format!("{constructor}\n{increment}"),
            format!("{constants_page}{state_page}"),
            format!("{constructor_page}{increment_page}"),
        ),
    ];
    for (name, declarations, functions, _, _) in &cases {
        prj.add_source(
            &format!("{name}.sol"),
            &format!(
                "pragma solidity >=0.8.19;\ncontract {name} {{\n{declarations}\n{functions}\n}}"
            ),
        );
    }
    cmd.arg("doc").assert_success();
    for (name, _, _, sections, function_sections) in cases {
        prj.assert_doc_page(
            &format!("src/contract.{name}.mdx"),
            format!(
                "---\ntitle: \"{name}\"\n---\n\n# {name}\n\n{sections}## Functions\n\n{function_sections}"
            ),
        );
    }
}

// Test that MDX-unsafe content coming through @inheritdoc is still escaped, and that
// unnamed return values are rendered as `&lt;none&gt;`.
#[forgetest_init]
fn inheritdoc_mdx_safety_and_unnamed_returns(prj: _, cmd: _) {
    prj.add_source(
        "IUnsafe.sol",
        r#"
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
import "./IUnsafe.sol";

contract Safe is IUnsafe {
    /// @inheritdoc IUnsafe
    function transfer(uint256 amount) external returns (uint256) {}

    /// @return First local result
    /// @return Second local result
    function localResults() external pure returns (uint256, address) {
        return (1, address(0));
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Safe.mdx",
        str![[r#"
...
### transfer

Transfer &lt;amount> tokens using `magic` spell

```solidity
function transfer(uint256 amount) external returns (uint256);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| amount | `uint256` | The value ` in wei ` |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | The new balance |
...
### localResults
...
| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | First local result |
| &lt;none&gt; | `address` | Second local result |
...
"#]],
    );
}

// Test that inline-link labels containing MDX-sensitive characters are escaped.
#[forgetest_init]
fn inline_link_label_safety(prj: _, cmd: _) {
    prj.add_source(
        "Token.sol",
        r#"
contract Token {
    function transfer(uint256 amount) external {}
}
"#,
    );

    prj.add_source(
        "Vault.sol",
        r#"
import "./Token.sol";

/// @dev See {Token}[Token <contract>] for details
contract Vault {
    function deposit() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Vault.mdx",
        str![[r#"
...
See [Token &lt;contract>](/src/contract.Token) for details
...
"#]],
    );
}

// Test that the removed `--serve` flag prints a helpful migration message instead of a raw
// clap parse error.
#[forgetest_init]
fn serve_flag_prints_migration_message(cmd: _) {
    cmd.args(["doc", "--serve"]).assert_failure().stderr_eq(str![[r#"
Error: `--serve` has been removed. Generate the docs with `forge doc`, then run `npm run dev` from the generated docs directory.

"#]]);
}

// Test that MDX-unsafe characters in NatSpec are properly escaped in the generated output.
#[forgetest_init]
fn mdx_safety_escaping(prj: _, cmd: _) {
    prj.add_source(
        "Escaping.sol",
        r#"
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

    prj.assert_doc_page(
        "src/contract.Escaping.mdx",
        str![[r#"
...
Contains a bare &lt; angle bracket and a bare &#123; brace.

<i>

Reference to `UnresolvableRef` should become inline code.
...
### transfer

Transfer tokens to recipient &lt; address

```solidity
function transfer(uint256 amount) external;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| amount | `uint256` | The amount ` in wei ` |


"#]],
    );
}

// Test that multiline @param and @return descriptions (continuation lines) are preserved.
#[forgetest_init]
fn param_return_multiline_continuation(prj: _, cmd: _) {
    prj.add_source(
        "Multiline.sol",
        r#"
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

    prj.assert_doc_page(
        "src/interface.IMultiline.mdx",
        str![[r#"
...
**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` | The first line of the description.<br/>Second line of the param description. |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| result | `uint256` | The first line of return.<br/>Second line of return description. |
...
"#]],
    );
}

#[forgetest_init]
fn inheritdoc_multiline_param_preserves_inherited_notice(prj: _, cmd: _) {
    prj.add_source(
        "MultilineInheritdoc.sol",
        r#"
interface Root {
    /// @notice Runs the operation
    /// @param value The input value
    function run(uint256 value) external;
}

interface Mid is Root {
    function run(uint256 value) external override;
}

contract Child is Mid {
    /// @inheritdoc Mid
    /// @param value Local first line.
    ///        Local second line.
    function run(uint256 value) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    prj.assert_doc_page(
        "src/contract.Child.mdx",
        str![[r#"
...
### run

Runs the operation
...
| value | `uint256` | Local first line.<br/>Local second line. |
...
"#]],
    );
}

// Inherited multiline NatSpec retains continuation lines and strips block-comment decorations.
#[forgetest_init]
fn inherited_param_return_multiline_continuation(prj: _, cmd: _) {
    prj.add_source(
        "InheritedMultiline.sol",
        r#"
interface IInheritedMultiline {
    /// @param value The first explicit parameter line.
    ///        The second explicit parameter line.
    /// @return result The first explicit return line.
    ///         The second explicit return line.
    function explicitAction(uint256 value) external returns (uint256 result);

    /// @param value The inherited parameter line.
    ///        The inherited parameter continuation.
    function explicitActionWithLocalNotice(uint256 value) external;

    /**
     * @param value The first implicit parameter line.
     * The second implicit parameter line.
     * @return result The first implicit return line.
     * The second implicit return line.
     */
    function implicitAction(uint256 value) external returns (uint256 result);

    /// An untagged inherited notice.
    function untaggedNotice() external;

    /// @param value The parameter description.
    ///
    /// A separate inherited notice.
    function separatedNotice(uint256 value) external;

    /// @param value The base parameter line.
    ///        The base parameter continuation.
    function replacedParameter(uint256 value) external;

    /**
     * @param value Run this code:
     *     value += 1;
     */
    function indentedBlock(uint256 value) external;

    /// @param value *important*
    function markdown(uint256 value) external;
}

contract InheritedMultiline is IInheritedMultiline {
    /// @inheritdoc IInheritedMultiline
    function explicitAction(uint256 value) external override returns (uint256 result) {}

    /// @inheritdoc IInheritedMultiline
    /// @notice A local notice.
    function explicitActionWithLocalNotice(uint256 value) external override {}

    function implicitAction(uint256 value) external override returns (uint256 result) {}

    function untaggedNotice() external override {}

    function separatedNotice(uint256 value) external override {}

    /// @inheritdoc IInheritedMultiline
    /// @param value The local parameter description.
    function replacedParameter(uint256 value) external override {}

    function indentedBlock(uint256 value) external override {}

    function markdown(uint256 value) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.InheritedMultiline.mdx",
        str![[r#"
...
### explicitAction
...
| value | `uint256` | The first explicit parameter line.<br/>The second explicit parameter line. |
...
| result | `uint256` | The first explicit return line.<br/>The second explicit return line. |
...
### explicitActionWithLocalNotice

A local notice.
...
| value | `uint256` | The inherited parameter line.<br/>The inherited parameter continuation. |
...
### implicitAction
...
| value | `uint256` | The first implicit parameter line.<br/>The second implicit parameter line. |
...
| result | `uint256` | The first implicit return line.<br/>The second implicit return line. |
...
### untaggedNotice

An untagged inherited notice.
...
### separatedNotice

A separate inherited notice.
...
| value | `uint256` | The parameter description. |
...
### replacedParameter

```solidity
function replacedParameter(uint256 value) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` | The local parameter description. |

<a id="indentedblock-uint256"></a>

### indentedBlock
...
| value | `uint256` | Run this code:<br/>    value += 1; |
...
### markdown
...
| value | `uint256` | *important* |
...
"#]],
    );
}

// Explicit inheritance must select the correct overload across canonical type spellings,
// qualified enums, and non-ABI-printable mapping parameters.
#[forgetest_init]
fn inheritdoc_overload_matching(prj: _, cmd: _) {
    for (base, child, expected) in [
        // uint_alias.
        (
            r#"
interface I {
    /// @notice Configure by amount.
    /// @param amount The configured amount
    function configure(uint amount) external;

    /// @notice Configure by account.
    /// @param account The configured account
    function configure(address account) external;
}
"#,
            r#"
import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function configure(uint256 amount) external override {}

    /// @inheritdoc I
    function configure(address account) external override {}
}
"#,
            str![[r#"
...
<a id="configure-uint256"></a>

### configure

Configure by amount.

```solidity
function configure(uint256 amount) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| amount | `uint256` | The configured amount |

<a id="configure-address"></a>

### configure

Configure by account.
...
"#]],
        ),
        // uint_array_alias.
        (
            r#"
interface I {
    /// @notice Batch values.
    /// @param values The input array
    function batch(uint[] calldata values) external;

    /// @notice Batch accounts.
    /// @param accounts The account array
    function batch(address[] calldata accounts) external;
}
"#,
            r#"
import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function batch(uint256[] calldata values) external override {}

    /// @inheritdoc I
    function batch(address[] calldata accounts) external override {}
}
"#,
            str![[r#"
...
<a id="batch-uint256"></a>

### batch

Batch values.

```solidity
function batch(uint256[] calldata values) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| values | `uint256[]` | The input array |

<a id="batch-address"></a>

### batch

Batch accounts.
...
"#]],
        ),
        // qualified_enum_alias.
        (
            r#"
interface I {
    enum Status { Inactive, Active }

    /// @notice Sets the account status.
    /// @param s the new status
    function configure(I.Status s) external;

    /// @notice Configures by raw id.
    /// @param id the raw id
    function configure(uint256 id) external;
}
"#,
            r#"
import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function configure(Status s) external override {}

    /// @inheritdoc I
    function configure(uint256 id) external override {}
}
"#,
            str![[r#"
...
<a id="configure-status"></a>

### configure

Sets the account status.

```solidity
function configure(Status s) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| s | `Status` | the new status |

<a id="configure-uint256"></a>

### configure

Configures by raw id.
...
"#]],
        ),
        // mapping_fallback.
        (
            r#"
abstract contract Base {
    /// @notice Configure by store.
    /// @param store The storage mapping
    function configure(mapping(uint256 => uint256) storage store) internal virtual;

    /// @notice Configure by account.
    /// @param account The configured account
    function configure(address account) internal virtual;
}
"#,
            r#"
import "./I.sol";

contract C is Base {
    /// @inheritdoc Base
    function configure(mapping(uint256 => uint256) storage store) internal override {}

    /// @inheritdoc Base
    function configure(address account) internal override {}
}
"#,
            str![[r#"
...
<a id="configure-mapping-uint256-uint256"></a>

### configure

Configure by store.

```solidity
function configure(mapping(uint256 => uint256) storage store) internal override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| store | `mapping(uint256 => uint256)` | The storage mapping |

<a id="configure-address"></a>

### configure

Configure by account.
...
"#]],
        ),
    ] {
        prj.add_source("I.sol", base);
        prj.add_source("C.sol", child);
        cmd.forge_fuse().args(["doc"]).assert_success();
        prj.assert_doc_page("src/contract.C.mdx", expected);
    }
}

// Test that @inheritdoc parameter descriptions are matched when an implementation
// prefixes or suffixes interface parameter names with underscores.
#[forgetest_init]
fn inheritdoc_matches_underscore_wrapped_param_names(prj: _, cmd: _) {
    prj.add_source(
        "I.sol",
        r#"
interface I {
    /// @notice Mints tokens.
    /// @param recipient The account receiving minted tokens.
    /// @param amount The number of tokens to mint.
    function mint(address recipient, uint256 amount) external;
}
"#,
    );

    prj.add_source(
        "C.sol",
        r#"
import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function mint(address recipient_, uint256 _amount) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.C.mdx",
        str![[r#"
...
### mint

Mints tokens.

```solidity
function mint(address recipient_, uint256 _amount) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| recipient_ | `address` | The account receiving minted tokens. |
| _amount | `uint256` | The number of tokens to mint. |
...
"#]],
    );
}

// Explicit inheritance maps parameters positionally, even when an override renames one to a name
// that would have been ambiguous under the old fuzzy name matching.
#[forgetest_init]
fn inheritdoc_maps_ambiguous_renames_positionally(prj: _, cmd: _) {
    prj.add_source(
        "I.sol",
        r#"
interface I {
    /// @notice Updates values.
    /// @param amount Docs for first param.
    /// @param _amount Docs for second param.
    function update(uint256 amount, uint256 _amount) external;
}
"#,
    );

    prj.add_source(
        "C.sol",
        r#"
import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function update(uint256 other, uint256 _amount) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.C.mdx",
        str![[r#"
...
### update

Updates values.

```solidity
function update(uint256 other, uint256 _amount) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| other | `uint256` | Docs for first param. |
| _amount | `uint256` | Docs for second param. |
...
"#]],
    );
}

// Test that @inheritdoc resolves docs from a deeply inherited chain
// (Base inherits from an interface without redeclaring NatSpec).
#[forgetest_init]
fn inheritdoc_resolves_deep_chain(prj: _, cmd: _) {
    prj.add_source(
        "IBase.sol",
        r#"
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
import "./Base.sol";

contract Derived is Base {
    /// @inheritdoc Base
    function action(uint256 value) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Derived.mdx",
        str![[r#"
...
### action

Perform the action

```solidity
function action(uint256 value) external override;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| value | `uint256` | The input value |
...
"#]],
    );
}

// Test two rendering behaviors together:
// 1. /** */ block comments are stripped of their ` * ` line decoration.
// 2. `@dev` paragraphs are wrapped in `<i>...</i>` so multi-paragraph content and embedded lists
//    render as italic without breaking block-level markdown.
#[forgetest_init]
fn block_comments_strip_star_and_dev_renders_italic(prj: _, cmd: _) {
    prj.add_source(
"ECDSA.sol",
        r#"
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

    prj.assert_doc_page(
        "src/library.ECDSA.mdx",
        str![[r#"
---
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

Overload of [ECDSA.tryRecover](#tryrecover-bytes32-bytes) that receives the `v`,
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


"#]],
    );
}

// Test that @inheritdoc on a public state variable resolves docs from the interface getter
// function (e.g. ERC20's `totalSupply()`).
// fixes <https://github.com/foundry-rs/foundry/pull/14568>
#[forgetest_init]
fn inheritdoc_variable_resolves_interface_getter(prj: _, cmd: _) {
    prj.add_source(
        "IERC20.sol",
        r#"
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
import "./IERC20.sol";

contract ERC20 is IERC20 {
    /// @inheritdoc IERC20
    uint256 public totalSupply;
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.ERC20.mdx",
        str![[r#"
...
### totalSupply

Returns the total token supply.

```solidity
uint256 public totalSupply;
```

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | The total supply. |
...
"#]],
    );
}

// Test that `**Inherits:**` links resolve to the actually-inherited contract even
// when another contract with the same name lives in a directory closer to the
// consumer. Without exact-id resolution, the proximity heuristic in
// `resolve_page` would (wrongly) link to the same-directory namesake.
// Test that references naming a member of the current contract resolve to anchor-only
// links on the same page ({member} and {Contract-member} self-references), and that
// same-file inheritance links to the same-file base instead of a same-named decoy.
// fixes <https://github.com/foundry-rs/foundry/issues/11677>
#[forgetest_init]
fn same_contract_references_resolve_to_anchors(prj: _, cmd: _) {
    // Decoys: same-named library and interface in a sibling directory that sorts
    // first; references in `external/OlympusERC20.sol` must not resolve to them.
    prj.add_source(
        "decoys/Decoys.sol",
        r#"
library ECDSA {
    function tryRecover(bytes32 hash) internal pure returns (address) {}
}

interface IERC20 {
    function balanceOf(address owner) external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "external/OlympusERC20.sol",
        r#"
library ECDSA {
    /// @dev A safe way to ensure this is by receiving a hash of the original
    /// message and then calling {toEthSignedMessageHash} on it.
    function recover(bytes32 hash) internal pure returns (address) {}

    /// @dev Overload of {ECDSA-tryRecover-bytes32-bytes32}; not {ECDSA-tryRecover-address}.
    function tryRecover(bytes32 hash, bytes32 r) internal pure returns (address) {}

    function toEthSignedMessageHash(bytes32 hash) internal pure returns (bytes32) {}
}

interface IERC20 {
    function totalSupply() external view returns (uint256);
}

interface IOHM is IERC20 {
    function mint(address account_) external;
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    // Same-contract member references become anchor-only links.
    prj.assert_doc_page(
        "src/external/library.ECDSA.mdx",
        str![[r#"
---
title: "ECDSA"
---

# ECDSA

## Functions

<a id="recover-bytes32"></a>

### recover

<i>

A safe way to ensure this is by receiving a hash of the original
message and then calling [toEthSignedMessageHash](#toethsignedmessagehash) on it.

</i>

```solidity
function recover(bytes32 hash) internal pure returns (address);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| hash | `bytes32` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `address` |  |

<a id="tryrecover-bytes32-bytes32"></a>

### tryRecover

<i>

Overload of [ECDSA.tryRecover-bytes32-bytes32](#tryrecover-bytes32-bytes32); not `ECDSA`.

</i>

```solidity
function tryRecover(bytes32 hash, bytes32 r) internal pure returns (address);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| hash | `bytes32` |  |
| r | `bytes32` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `address` |  |

<a id="toethsignedmessagehash-bytes32"></a>

### toEthSignedMessageHash

```solidity
function toEthSignedMessageHash(bytes32 hash) internal pure returns (bytes32);
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| hash | `bytes32` |  |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `bytes32` |  |


"#]],
    );

    // Same-file inheritance links to the same-file interface, not the decoy.
    prj.assert_doc_page(
        "src/external/interface.IOHM.mdx",
        str![[r#"
---
title: "IOHM"
---

# IOHM

**Inherits:** [IERC20](/src/external/interface.IERC20)

## Functions

<a id="mint-address"></a>

### mint

```solidity
function mint(address account_) external;
```

**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| account_ | `address` |  |


"#]],
    );
}

#[forgetest_init]
fn inherited_member_references_resolve_to_base_page(prj: _, cmd: _) {
    prj.add_source(
        "base/A.sol",
        r#"
contract A {
    struct Payload {
        uint256 value;
    }

    uint256 public balance$raw;
    uint256 private secret;

    error Failure();
    event Fired();
    enum State { Ready }

    function foo() external {}
    function overloaded(uint256 value) external {}
    function hidden() private {}

    function withAssembly() external pure {
        assembly {
            function helper() {}
        }
    }
}
"#,
    );
    prj.add_source(
        "consumer/A.sol",
        r#"
contract A {
    function foo() external {}
}

contract Utility {
    function work() external {}
}
"#,
    );
    prj.add_source(
        "consumer/B.sol",
        r#"
import {A as BaseA} from "../base/A.sol";

contract B is BaseA {
    /// @notice See {foo} or {A-foo}.
    /// Also see {Payload}, {Failure}, {Fired}, {State}, and {balance$raw}.
    /// The Yul function {helper} has no documentation heading.
    /// Private members {hidden} and {secret} are not inherited.
    /// The qualified Yul function {A-helper} has no documentation heading.
    /// Exact overload {A-overloaded-uint256}; missing overload {A-overloaded-address}.
    /// Non-inherited qualified reference {Utility-work} still resolves globally.
    function bar() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/consumer/contract.B.mdx",
        str![[r#"
...
See [foo](/src/base/contract.A#foo) or [A.foo](/src/base/contract.A#foo).
Also see [Payload](/src/base/contract.A#payload), [Failure](/src/base/contract.A#failure), [Fired](/src/base/contract.A#fired), [State](/src/base/contract.A#state), and [balance$raw](/src/base/contract.A#balanceraw).
The Yul function `helper` has no documentation heading.
Private members `hidden` and `secret` are not inherited.
The qualified Yul function `A` has no documentation heading.
Exact overload [A.overloaded-uint256](/src/base/contract.A#overloaded-uint256); missing overload `A`.
Non-inherited qualified reference [Utility.work](/src/consumer/contract.Utility#work) still resolves globally.
...
"#]],
    );
}

#[forgetest_init]
fn unrendered_override_does_not_link_to_ancestor(prj: _, cmd: _) {
    prj.add_source(
        "ancestor/A.sol",
        r#"
contract A {
    function foo() public virtual {}
}
"#,
    );
    prj.add_source(
        "hidden/Middle.sol",
        r#"
import {A} from "../ancestor/A.sol";

contract Middle is A {
    function foo() public virtual override {}
}
"#,
    );
    prj.add_source(
        "Middle.sol",
        r#"
contract Middle {
    function foo() public {}
}
"#,
    );
    prj.add_source(
        "Child.sol",
        r#"
import {Middle} from "./hidden/Middle.sol";

contract Child is Middle {
    /// @notice See {foo} and {Middle-foo}.
    function bar() external {}
}
"#,
    );
    prj.update_config(|config| config.doc.ignore = vec!["src/hidden/Middle.sol".to_string()]);

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Child.mdx",
        str![[r#"
...
See `foo` and `Middle`.
...
"#]],
    );
}

#[forgetest_init]
fn ambiguous_inherited_contract_name_does_not_link(prj: _, cmd: _) {
    prj.add_source(
        "left/A.sol",
        r#"
contract A {
    function left() external {}
}
"#,
    );
    prj.add_source(
        "right/A.sol",
        r#"
contract A {
    function right() external {}
}
"#,
    );
    prj.add_source(
        "Child.sol",
        r#"
import {A as LeftA} from "./left/A.sol";
import {A as RightA} from "./right/A.sol";

contract Child is LeftA, RightA {
    /// @notice See {A-right}.
    function child() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Child.mdx",
        str![[r#"
...
See `A`.
...
"#]],
    );
}

#[forgetest_init]
fn inherited_special_function_links_use_declaring_page(prj: _, cmd: _) {
    prj.add_source(
        "Special.sol",
        r#"
contract A {
    constructor() {}
    fallback() external payable {}
    receive() external payable {}
}

contract Middle is A {}

contract Child is Middle {
    /// @notice Bare {constructor}, {fallback}, and {receive}.
    /// Middle {Middle-constructor}, {Middle-fallback}, and {Middle-receive}.
    /// A {A-constructor}, {A-fallback}, and {A-receive}.
    function child() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/contract.Child.mdx",
        str![[r#"
...
Bare `constructor`, [fallback](/src/contract.A#fallback), and [receive](/src/contract.A#receive).
Middle `Middle`, `Middle`, and `Middle`.
A [A.constructor](/src/contract.A#constructor), [A.fallback](/src/contract.A#fallback), and [A.receive](/src/contract.A#receive).
...
"#]],
    );
}

#[forgetest_init]
fn inheritance_links_use_exact_base_id(prj: _, cmd: _) {
    // Two unrelated `Token` contracts in sibling directories.
    prj.add_source(
        "a/Token.sol",
        r#"
contract Token {}
"#,
    );
    prj.add_source(
        "b/Token.sol",
        r#"
contract Token {}
"#,
    );

    // Consumer lives next to `a/Token.sol` but explicitly inherits from `b/Token`.
    prj.add_source(
        "a/Consumer.sol",
        r#"
import {Token} from "../b/Token.sol";

contract Consumer is Token {}
"#,
    );

    cmd.args(["doc"]).assert_success();

    prj.assert_doc_page(
        "src/a/contract.Consumer.mdx",
        str![[r#"
---
title: "Consumer"
---

# Consumer

**Inherits:** [Token](/src/b/contract.Token)


"#]],
    );
}

trait DocProject {
    fn assert_doc_page(&self, page: &str, expected: impl IntoData);
    fn assert_doc_functions(&self, page: &str, expected: impl IntoData);
}

impl DocProject for TestProject {
    #[track_caller]
    fn assert_doc_page(&self, page: &str, expected: impl IntoData) {
        assert_data_eq!(
            fs::read_to_string(self.root().join("docs/src/pages").join(page)).unwrap(),
            expected
        );
    }

    #[track_caller]
    fn assert_doc_functions(&self, page: &str, expected: impl IntoData) {
        let page = fs::read_to_string(self.root().join("docs/src/pages").join(page)).unwrap();
        let (_, functions) = page.split_once("## Functions\n\n").expect("missing function section");
        assert_data_eq!(functions, expected);
    }
}
