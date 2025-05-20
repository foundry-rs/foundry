use foundry_config::fs_permissions::PathPermission;

forgetest!(test_eip712, |prj, cmd| {
    let path = prj
        .add_source(
            "Structs",
            r#"
library Structs {
    struct Foo { Bar bar; }
    struct Bar { Art art; }
    struct Art { uint256 id; }
    struct Complex {
        Structs2.Foo foo2;
        Foo[] foos;
        Rec[][] recs;
    }
    struct Rec { Rec[] rec; }
}

library Structs2 {
    struct Foo { uint256 id; }
    struct Rec { Bar[] bar; }
    struct Bar { Rec rec; }
    struct FooBar {
        Foo[] foos;
        Bar[] bars;
        Structs.Foo foo;
        Structs.Bar bar;
        Rec[] recs;
        Structs.Rec rec;
    }
}
    "#,
        )
        .unwrap();

    cmd.forge_fuse().args(["eip712", path.to_string_lossy().as_ref()]).assert_success().stdout_eq(
        str![[r#"
Foo(Bar bar)Art(uint256 id)Bar(Art art)

Bar(Art art)Art(uint256 id)

Art(uint256 id)

Complex(Foo foo2,Foo_1[] foos,Rec[][] recs)Art(uint256 id)Bar(Art art)Foo(uint256 id)Foo_1(Bar bar)Rec(Rec[] rec)

Rec(Rec[] rec)

Foo(uint256 id)

Rec(Bar[] bar)Bar(Rec rec)

Bar(Rec rec)Rec(Bar[] bar)

FooBar(Foo[] foos,Bar[] bars,Foo_1 foo,Bar_1 bar,Rec[] recs,Rec_1 rec)Art(uint256 id)Bar(Rec rec)Bar_1(Art art)Foo(uint256 id)Foo_1(Bar_1 bar)Rec(Bar[] bar)Rec_1(Rec_1[] rec)

"#]],
    );
});

forgetest!(test_eip712_cheatcode, |prj, cmd| {
    prj.add_source(
        "Eip712",
        r#"
contract Eip712 {
    struct Transaction {
        Person from;
        Person to;
        Asset tx;
    }
    struct Person {
        address wallet;
        string name;
    }
    struct Asset {
        address token;
        uint256 amount;
    }
}
    "#,
    )
    .unwrap();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source("Eip712Cheat.sol", r#"
// Note Used in forge-cli tests to assert failures.
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";
import "./console.sol";

string constant CANONICAL = "Transaction(Person from,Person to,Asset tx)Asset(address token,uint256 amount)Person(address wallet,string name)";
string constant MESSY = "Person(address wallet, string name) Asset(address token, uint256 amount) Transaction(Person from, Person to, Asset tx)";

contract Eip712Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEip712HashType() public {
        bytes32 hashCanonical = keccak256(bytes(CANONICAL));

        bytes32 hashTypeName = vm.eip712HashType("Transaction");
        assertEq(hashTypeName, hashCanonical);

        bytes32 hashTypeDef = vm.eip712HashType(MESSY);
        assertEq(hashTypeDef, hashCanonical);
    }
}
"#,
    )
    .unwrap();

    cmd.forge_fuse().args(["bind-json"]).assert_success();

    let bindings = prj.root().join("utils").join("JsonBindings.sol");
    assert!(bindings.exists(), "JsonBindings.sol was not generated at {:?}", bindings);

    prj.update_config(|config| config.fs_permissions.add(PathPermission::read(bindings)));
    cmd.forge_fuse().args(["test", "--mc", "Eip712Test", "-vvvv"]).assert_success();
});
