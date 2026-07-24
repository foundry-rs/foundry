use foundry_test_utils::{
    assert_data_eq,
    snapbox::Data,
    str,
    util::{RemoteProject, setup_forge_remote},
};
use std::fs;

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc"]).assert_success();
}

forgetest_init!(doc_does_not_write_artifacts, |prj, cmd| {
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
});

forgetest_init!(doc_supports_empty_projects, |_prj, cmd| {
    cmd.arg("doc").assert_success();
});

forgetest_init!(doc_uses_configured_commit_for_source_links, |prj, cmd| {
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Revision.mdx"), None),
        str![[r#"
...
[Git Source](https://github.com/foundry-rs/foundry/blob/v1.2.3/src/Revision.sol)
...
"#]],
    );
});

forgetest!(doc_supports_mixed_solidity_versions, |prj, cmd| {
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
});

#[cfg(unix)]
forgetest_init!(doc_does_not_run_solc, |prj, cmd| {
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
});

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
});

// Test that natspec is inherited implicitly from a base interface when the override carries
// no `@inheritdoc` tag.
// fixes <https://github.com/foundry-rs/foundry/issues/4070>
forgetest_init!(natspec_is_inherited_implicitly, |prj, cmd| {
    prj.add_source(
        "IExample.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// An override inherits the base overload with the matching signature, continuing past a nearer
// base that declares a different same-name overload.
forgetest_init!(implicit_inheritance_matches_the_overload_signature, |prj, cmd| {
    prj.add_source(
        "Bases.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Bases.sol";

contract Impl is INear, IFar {
    function g(uint256 n) external override(IFar) returns (bool) {}
    function g(address a) external override(INear) returns (bool) {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Impl.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    assert!(rendered.contains("Far documents g(uint256)"), "{rendered}");
});

// Implicit inheritance matches through resolved types as well: the same divergent spellings
// must still inherit when the override carries no NatSpec at all.
forgetest_init!(implicit_inheritance_matches_semantic_types, |prj, cmd| {
    prj.add_source(
        "Store.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract Store {
    /// @notice Configures the store
    function configure(mapping(uint => uint) storage store_) internal virtual;
}
"#,
    );

    prj.add_source(
        "MyStore.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Store.sol";

contract MyStore is Store {
    function configure(mapping(uint=>uint) storage store_) internal override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.MyStore.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    assert!(rendered.contains("Configures the store"), "{rendered}");
});

// A public mapping variable inherits the NatSpec of the interface getter it implements, matched
// through the getter's generated signature (`balanceOf(address)`).
forgetest_init!(implicit_inheritance_matches_mapping_getter_signature, |prj, cmd| {
    prj.add_source(
        "IERC.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC {
    /// @notice The balance of an account
    function balanceOf(address account) external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "Token.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// A public mapping with a `string` key inherits through its synthetic getter: the getter's
// generated signature matches the interface function with the location normalized.
forgetest_init!(implicit_inheritance_matches_string_key_getter, |prj, cmd| {
    prj.add_source(
        "IRegistry.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IRegistry {
    /// @notice The balance registered for a name
    function balances(string memory name) external view returns (uint256);
}
"#,
    );

    prj.add_source(
        "Registry.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// `calldata` in a base member and `memory` in the override are the same signature: locations
// are normalized before comparison and the NatSpec is inherited.
forgetest_init!(implicit_inheritance_normalizes_calldata_location, |prj, cmd| {
    prj.add_source(
        "Base.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Base {
    /// @notice Configures the value
    function configure(bytes calldata data) external;
}
"#,
    );

    prj.add_source(
        "Child.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// A documented base overload with a different non-ABI signature must NOT be inherited: the
// signature gate stays strict even when the base has a single name match.
forgetest_init!(implicit_inheritance_rejects_non_abi_overload_mismatch, |prj, cmd| {
    prj.add_source(
        "Store.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract Store {
    /// @notice Configures the store
    function configure(mapping(uint => uint) storage store_) internal virtual;
}
"#,
    );

    prj.add_source(
        "MyStore.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Store.sol";

contract MyStore is Store {
    function configure(mapping(uint => uint) storage store_) internal override {}
    function configure(mapping(address => address) storage other) internal {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.MyStore.mdx");
    let rendered = fs::read_to_string(&doc_path).unwrap();
    let occurrences = rendered.matches("Configures the store").count();
    assert_eq!(occurrences, 1, "only the matching overload may inherit:\n{rendered}");
});

// Point 2 (mablr review): names are compared at every level. A leaf cannot jump across an
// intermediate rename just because it restores the original name.
forgetest_init!(implicit_inheritance_requires_matching_param_names, |prj, cmd| {
    prj.add_source(
        "Rename.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Point 3 (mablr review): the target needs a public getter, and the source needs to be an
// external function implemented by that getter. A same-name base variable is not a source.
forgetest_init!(implicit_inheritance_requires_public_getter_and_function_source, |prj, cmd| {
    prj.add_source(
        "Variables.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Point 1 (mablr review): automatic inheritance needs one semantic base function. Distinct
// declarations on separate branches are ambiguous; a declaration shared by both branches is not.
forgetest_init!(implicit_inheritance_resolves_base_ambiguity_per_branch, |prj, cmd| {
    prj.add_source(
        "Ambiguity.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let page = |contract: &str| {
        fs::read_to_string(prj.root().join(format!("docs/src/pages/src/contract.{contract}.mdx")))
            .unwrap()
    };

    let direct = page("Direct");
    assert!(!direct.contains("From IAlpha"), "{direct}");
    assert!(!direct.contains("From IBeta"), "{direct}");

    for contract in ["Asymmetric", "Leaf"] {
        let rendered = page(contract);
        assert!(!rendered.contains("A branch doc"), "{rendered}");
        assert!(!rendered.contains("Root branch doc"), "{rendered}");
    }

    let shared = page("Shared");
    assert!(shared.contains("Shared root doc"), "{shared}");
});

// Point 5 (mablr review): any local NatSpec item suppresses automatic inheritance. A leaf
// cannot reach around an intermediate override carrying only a custom tag.
forgetest_init!(implicit_inheritance_skips_custom_tagged_members, |prj, cmd| {
    prj.add_source(
        "Tagged.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Implicit inheritance only runs when the override has no NatSpec of its own: a local `@notice`
// keeps the base `@param`/`@return` from being pulled in.
forgetest_init!(implicit_inheritance_skips_documented_members, |prj, cmd| {
    prj.add_source(
        "IExample.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Point 4 (mablr review): render every parameter and return of the implemented getter. A
// missing parameter tag leaves its own row empty instead of borrowing another description.
forgetest_init!(inherited_getter_renders_param_and_return, |prj, cmd| {
    prj.add_source(
        "Entries.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// steven review: an intermediate override's `@inheritdoc` is resolved and merged, not treated
// as terminal, so documentation propagates through it. A (documented) -> B (@inheritdoc A) ->
// C (undocumented): C receives A's documentation through B.
forgetest_init!(implicit_inheritance_resolves_intermediate_inheritdoc, |prj, cmd| {
    prj.add_source(
        "Chain.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// steven review: an inherited `@return` maps positionally onto a renamed override's return
// slot, instead of gluing the base return name into the description.
forgetest_init!(implicit_inheritance_remaps_renamed_returns, |prj, cmd| {
    prj.add_source(
        "Renamed.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Regression: return-name resolution for the implicit path must not leak into the explicit
// `@inheritdoc` path. With a partial local `@return` over a named-return override, the local
// description must win and the base's other returns must not be injected (matches master).
forgetest_init!(explicit_inheritdoc_partial_return_keeps_local_and_skips_base, |prj, cmd| {
    prj.add_source(
        "PartialReturn.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.Partial.mdx")).unwrap();
    assert!(rendered.contains("local A-text"), "{rendered}");
    assert!(!rendered.contains("base A-text"), "{rendered}");
    assert!(!rendered.contains("base B-text"), "{rendered}");
});

// A public state variable's generated getter inherits implicitly through an interface chain:
// a base function redeclared without NatSpec still propagates its ancestor's documentation, like
// solc (Impl.data() resolves to IRoot's `@notice` through the undocumented IMid redeclaration).
forgetest_init!(implicit_getter_inherits_through_interface_chain, |prj, cmd| {
    prj.add_source(
        "GetterChain.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// A private base function is not overridden by a same-signature child function and cannot donate
// its documentation to it.
// `forge doc` can render parseable sources that Solidity would reject later. A private
// same-signature declaration is still not a valid override source for inherited docs.
forgetest_init!(implicit_inheritance_rejects_private_base, |prj, cmd| {
    prj.add_source(
        "PrivateBase.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.PrivateLeaf.mdx")).unwrap();
    assert!(!rendered.contains("Must not escape"), "{rendered}");
});

// A lowered Yul helper is not part of Solidity's override frontier. It must not shadow the real
// Solidity declaration in the next ancestor. Solar lowers Yul helpers as private, so this pins the
// effective boundary instead of proving `is_yul` independently from private visibility.
forgetest_init!(implicit_inheritance_ignores_yul_shadow, |prj, cmd| {
    prj.add_source(
        "YulShadow.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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

contract YulLeaf is YulMid {
    function yulCandidate(uint256 value) public override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.YulLeaf.mdx")).unwrap();
    assert!(rendered.contains("Must pass through"), "{rendered}");
});

// A generated getter is not an ordinary function declaration on the override frontier.
forgetest_init!(implicit_inheritance_rejects_generated_getter_base, |prj, cmd| {
    prj.add_source(
        "GetterBase.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.GetterLeaf.mdx")).unwrap();
    assert!(!rendered.contains("Must not escape"), "{rendered}");
});

// `forge doc` lowers parseable sources without running Solidity's full override validation.
// Even for an invalid cross-domain collision, it must not copy modifier docs onto a function.
forgetest_init!(implicit_inheritance_keeps_function_modifier_domains_separate, |prj, cmd| {
    prj.add_source(
        "FunctionModifier.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.FunctionLeaf.mdx"))
            .unwrap();
    assert!(!rendered.contains("A modifier is not"), "{rendered}");
});

// Fallback and receive have no AST header name, but they still take part in explicit and
// implicit NatSpec inheritance through their HIR function kinds.
forgetest_init!(inheritance_supports_fallback_and_receive, |prj, cmd| {
    prj.add_source(
        "SpecialFunctions.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// Return descriptions are remapped at each override hop before a generated getter consumes
// them. The final rows use the getter field names, not either interface's return names.
forgetest_init!(implicit_getter_remaps_returns_at_every_hop, |prj, cmd| {
    prj.add_source(
        "ReturnChain.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
});

// An explicit `@inheritdoc` relay remaps inherited return names before the getter consumes them.
forgetest_init!(implicit_getter_remaps_returns_after_inheritdoc_relay, |prj, cmd| {
    prj.add_source(
        "ExplicitReturnChain.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    let rendered =
        fs::read_to_string(prj.root().join("docs/src/pages/src/contract.ExplicitPairStore.mdx"))
            .unwrap();
    assert!(rendered.contains("Relayed through the middle interface"), "{rendered}");
    assert!(rendered.contains("| getterFirst | `uint256` | first relayed value |"), "{rendered}");
    assert!(rendered.contains("| getterSecond | `uint256` | second relayed value |"), "{rendered}");
});

// Getter tables use the same NatSpec sanitizer as ordinary functions, including the escaped
// placeholder for an unnamed generated return.
forgetest_init!(inherited_getter_sanitizes_mdx_and_unnamed_returns, |prj, cmd| {
    prj.add_source(
        "UnsafeGetter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.UnsafeGetter.mdx"), None,),
        str![[r#"
...
**Parameters**

| Name | Type | Description |
| ---- | ---- | ----------- |
| key | `uint256` | Locate &lt;amount> with `Reference` |

**Returns**

| Name | Type | Description |
| ---- | ---- | ----------- |
| &lt;none&gt; | `uint256` | Result &lt;amount> from `Reference` |
...
"#]],
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Derived.mdx"), None),
        str![[r#"
...
Inherits: [IBase](/src/interface.IBase)
...
"#]],
    );
});

forgetest_init!(doc_without_manifest_preserves_user_pages, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

contract Counter {
    uint256 public value;
}
"#,
    );

    let user_page = prj.root().join("docs/src/pages/src/overview.mdx");
    std::fs::create_dir_all(user_page.parent().unwrap()).unwrap();
    std::fs::write(&user_page, "# Overview\n\nHand-written page.\n").unwrap();

    cmd.args(["doc"]).assert_success();

    assert!(user_page.exists(), "user-authored page should survive first run without manifest");
    assert!(prj.root().join("docs/src/pages/.forge-doc-manifest").exists());
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.CounterConstants.mdx"), None),
        str![[r#"
---
title: "CounterConstants"
---

# CounterConstants

## Constants

### FOO

```solidity
uint256 public constant FOO = 1;
```

### BAR

```solidity
uint256 public immutable BAR;
```

## Functions

<a id="constructor"></a>

### constructor

```solidity
constructor();
```


"#]],
    );
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

    assert_data_eq!(
        Data::read_from(
            &prj.root().join("docs/src/pages/src/contract.CounterStateVariables.mdx"),
            None,
        ),
        str![[r#"
---
title: "CounterStateVariables"
---

# CounterStateVariables

## State Variables

### baz

```solidity
uint256 public baz;
```

## Functions

<a id="increment"></a>

### increment

```solidity
function increment() public;
```


"#]],
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

        assert_data_eq!(
            Data::read_from(
                &prj.root().join("docs/src/pages/src/contract.CounterMixedVariables.mdx"),
                None,
            ),
            str![[r#"
---
title: "CounterMixedVariables"
---

# CounterMixedVariables

## Constants

### FOO

```solidity
uint256 public constant FOO = 1;
```

### BAR

```solidity
uint256 public immutable BAR;
```

## State Variables

### baz

```solidity
uint256 public baz;
```

## Functions

<a id="constructor"></a>

### constructor

```solidity
constructor();
```

<a id="increment"></a>

### increment

```solidity
function increment() public;
```


"#]],
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Safe.mdx"), None),
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
| &lt;none&gt; | `uint256` | new balance |
...
"#]],
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Vault.mdx"), None),
        str![[r#"
...
See [Token &lt;contract>](/src/contract.Token) for details
...
"#]],
    );
});

// Test that the removed `--serve` flag prints a helpful migration message instead of a raw
// clap parse error.
forgetest_init!(serve_flag_prints_migration_message, |prj, cmd| {
    cmd.args(["doc", "--serve"]).assert_failure().stderr_eq(str![[r#"
Error: `--serve` has been removed. Generate the docs with `forge doc`, then run `npm run dev` from the generated docs directory.

"#]]);
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Escaping.mdx"), None),
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/interface.IMultiline.mdx"), None),
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
});

// Test that overload matching uses canonical HIR/ABI parameter types so that
// `Base.configure(uint)` is correctly matched by `Child.configure(uint256)`.
forgetest_init!(inheritdoc_overload_matches_uint_alias, |prj, cmd| {
    prj.add_source(
        "I.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface I {
    /// @notice Configure by amount.
    /// @param amount The configured amount
    function configure(uint amount) external;

    /// @notice Configure by account.
    /// @param account The configured account
    function configure(address account) external;
}
"#,
    );

    prj.add_source(
        "C.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function configure(uint256 amount) external override {}

    /// @inheritdoc I
    function configure(address account) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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
    );
});

// Test that @inheritdoc parameter descriptions are matched when an implementation
// prefixes or suffixes interface parameter names with underscores.
forgetest_init!(inheritdoc_matches_underscore_wrapped_param_names, |prj, cmd| {
    prj.add_source(
        "I.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function mint(address recipient_, uint256 _amount) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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
});

// If two inherited params normalize to the same underscore-trimmed name, fuzzy matching must not
// let the first inherited param steal the exact current param's docs.
forgetest_init!(inheritdoc_does_not_fuzzy_match_ambiguous_inherited_params, |prj, cmd| {
    prj.add_source(
        "I.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function update(uint256 other, uint256 _amount) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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
| other | `uint256` |  |
| _amount | `uint256` | Docs for second param. |
...
"#]],
    );
});

// Test that overload matching uses canonical HIR/ABI parameter types so that
// `Base.batch(uint[])` is correctly matched by `Child.batch(uint256[])`.
forgetest_init!(inheritdoc_overload_matches_uint_array_alias, |prj, cmd| {
    prj.add_source(
        "I.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface I {
    /// @notice Batch values.
    /// @param values The input array
    function batch(uint[] calldata values) external;

    /// @notice Batch accounts.
    /// @param accounts The account array
    function batch(address[] calldata accounts) external;
}
"#,
    );

    prj.add_source(
        "C.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function batch(uint256[] calldata values) external override {}

    /// @inheritdoc I
    function batch(address[] calldata accounts) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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
    );
});

// Test that overload matching uses canonical HIR/ABI parameter types so that
// semantically identical type spellings (`I.Status` vs `Status`) still match.
forgetest_init!(inheritdoc_overload_matches_qualified_enum_alias, |prj, cmd| {
    prj.add_source(
        "I.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    );

    prj.add_source(
        "C.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./I.sol";

contract C is I {
    /// @inheritdoc I
    function configure(Status s) external override {}

    /// @inheritdoc I
    function configure(uint256 id) external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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
    );
});

// Test that internal overloads with non-ABI-printable parameters use source text
// as a fallback instead of panicking while resolving @inheritdoc.
forgetest_init!(inheritdoc_overload_matches_mapping_fallback, |prj, cmd| {
    prj.add_source(
        "Base.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract Base {
    /// @notice Configure by store.
    /// @param store The storage mapping
    function configure(mapping(uint256 => uint256) storage store) internal virtual;

    /// @notice Configure by account.
    /// @param account The configured account
    function configure(address account) internal virtual;
}
"#,
    );

    prj.add_source(
        "C.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Base.sol";

contract C is Base {
    /// @inheritdoc Base
    function configure(mapping(uint256 => uint256) storage store) internal override {}

    /// @inheritdoc Base
    function configure(address account) internal override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.C.mdx"), None),
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Derived.mdx"), None),
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/library.ECDSA.mdx"), None),
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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.ERC20.mdx"), None),
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
| &lt;none&gt; | `uint256` |  The total supply. |
...
"#]],
    );
});

// Test that `**Inherits:**` links resolve to the actually-inherited contract even
// when another contract with the same name lives in a directory closer to the
// consumer. Without exact-id resolution, the proximity heuristic in
// `resolve_page` would (wrongly) link to the same-directory namesake.
// Test that references naming a member of the current contract resolve to anchor-only
// links on the same page ({member} and {Contract-member} self-references), and that
// same-file inheritance links to the same-file base instead of a same-named decoy.
// fixes <https://github.com/foundry-rs/foundry/issues/11677>
forgetest_init!(same_contract_references_resolve_to_anchors, |prj, cmd| {
    // Decoys: same-named library and interface in a sibling directory that sorts
    // first; references in `external/OlympusERC20.sol` must not resolve to them.
    prj.add_source(
        "decoys/Decoys.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/external/library.ECDSA.mdx"), None),
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
    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/external/interface.IOHM.mdx"), None),
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
});

forgetest_init!(inherited_member_references_resolve_to_base_page, |prj, cmd| {
    prj.add_source(
        "base/A.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/consumer/contract.B.mdx"), None),
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
});

forgetest_init!(unrendered_override_does_not_link_to_ancestor, |prj, cmd| {
    prj.add_source(
        "ancestor/A.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract A {
    function foo() public virtual {}
}
"#,
    );
    prj.add_source(
        "hidden/Middle.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {A} from "../ancestor/A.sol";

contract Middle is A {
    function foo() public virtual override {}
}
"#,
    );
    prj.add_source(
        "Middle.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Middle {
    function foo() public {}
}
"#,
    );
    prj.add_source(
        "Child.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {Middle} from "./hidden/Middle.sol";

contract Child is Middle {
    /// @notice See {foo} and {Middle-foo}.
    function bar() external {}
}
"#,
    );
    prj.update_config(|config| config.doc.ignore = vec!["src/hidden/Middle.sol".to_string()]);

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Child.mdx"), None),
        str![[r#"
...
See `foo` and `Middle`.
...
"#]],
    );
});

forgetest_init!(ambiguous_inherited_contract_name_does_not_link, |prj, cmd| {
    prj.add_source(
        "left/A.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract A {
    function left() external {}
}
"#,
    );
    prj.add_source(
        "right/A.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract A {
    function right() external {}
}
"#,
    );
    prj.add_source(
        "Child.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {A as LeftA} from "./left/A.sol";
import {A as RightA} from "./right/A.sol";

contract Child is LeftA, RightA {
    /// @notice See {A-right}.
    function child() external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Child.mdx"), None),
        str![[r#"
...
See `A`.
...
"#]],
    );
});

forgetest_init!(inherited_special_function_links_use_declaring_page, |prj, cmd| {
    prj.add_source(
        "Special.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/contract.Child.mdx"), None),
        str![[r#"
...
Bare `constructor`, [fallback](/src/contract.A#fallback), and [receive](/src/contract.A#receive).
Middle `Middle`, `Middle`, and `Middle`.
A [A.constructor](/src/contract.A#constructor), [A.fallback](/src/contract.A#fallback), and [A.receive](/src/contract.A#receive).
...
"#]],
    );
});

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

    assert_data_eq!(
        Data::read_from(&prj.root().join("docs/src/pages/src/a/contract.Consumer.mdx"), None),
        str![[r#"
---
title: "Consumer"
---

# Consumer

**Inherits:** [Token](/src/b/contract.Token)


"#]],
    );
});
