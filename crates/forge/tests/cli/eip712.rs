use foundry_config::fs_permissions::PathPermission;

forgetest!(test_eip712, |prj, cmd| {
    let path = prj
        .add_source(
            "Structs",
            r#"
library Structs {
    struct Foo {
        Bar bar;
    }

    struct Bar {
        Art art;
    }

    struct Art {
        uint256 id;
    }

    struct Complex {
        Structs2.Foo foo2;
        Foo[] foos;
        Rec[][] recs;
    }

    struct Rec {
        Rec[] rec;
    }
}

library Structs2 {
    struct Foo {
        uint256 id;
    }

    struct Rec {
        Bar[] bar;
    }

    struct Bar {
        Rec rec;
    }

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

forgetest!(test_eip712_cheatcode_simple, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source(
        "Eip712",
        r#"
contract Eip712Structs {
    struct EIP712Domain {
        string name;
        string version;
        uint256 chainId;
        address verifyingContract;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source("Eip712Cheat.sol", r#"
import "./test.sol";
import "./Vm.sol";
import "./console.sol";

string constant CANONICAL = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

contract Eip712Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEip712HashType() public {
        bytes32 canonicalHash = keccak256(bytes(CANONICAL));
        console.logBytes32(canonicalHash);

        // Can figure out the canonical type from a messy string representation of the type,
        // with an invalid order and extra whitespaces
        bytes32 fromTypeDef = vm.eip712HashType(
            "EIP712Domain(string name, string version, uint256 chainId, address verifyingContract)"
        );
        assertEq(fromTypeDef, canonicalHash);

        // Can figure out the canonical type from the previously generated bindings
        bytes32 fromTypeName = vm.eip712HashType("EIP712Domain");
        assertEq(fromTypeName, canonicalHash);
    }
}
"#,
    )
    .unwrap();

    cmd.forge_fuse().args(["bind-json"]).assert_success();

    let bindings = prj.root().join("utils").join("JsonBindings.sol");
    assert!(bindings.exists(), "'JsonBindings.sol' was not generated at {bindings:?}");

    prj.update_config(|config| config.fs_permissions.add(PathPermission::read(bindings)));
    cmd.forge_fuse().args(["test", "--mc", "Eip712Test", "-vv"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Eip712Cheat.sol:Eip712Test
[PASS] testEip712HashType() ([GAS])
Logs:
  0x8b73c3c69bb8fe3d512ecc4cf759cc79239f7b179b0ffacaa9a75d522b39400f

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]
    ]);
});

forgetest!(test_eip712_cheatcode_nested, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source(
        "Eip712",
        r#"
contract Eip712Structs {
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

    prj.add_source("Eip712Cheat.sol", r#"
import "./test.sol";
import "./Vm.sol";

string constant CANONICAL = "Transaction(Person from,Person to,Asset tx)Asset(address token,uint256 amount)Person(address wallet,string name)";

contract Eip712Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEip712HashType_byDefinition() public {
        bytes32 canonicalHash = keccak256(bytes(CANONICAL));

        // Can figure out the canonical type from a messy string representation of the type,
        // with an invalid order and extra whitespaces
        bytes32 fromTypeDef = vm.eip712HashType(
            "Person(address wallet, string name) Asset(address token, uint256 amount) Transaction(Person from, Person to, Asset tx)"
        );
        assertEq(fromTypeDef, canonicalHash);
    }

    function testEip712HashType_byTypeName() public {
        bytes32 canonicalHash = keccak256(bytes(CANONICAL));

        // Can figure out the canonical type from the previously generated bindings
        bytes32 fromTypeName = vm.eip712HashType("Transaction");
        assertEq(fromTypeName, canonicalHash);
    }

    function testReverts_Eip712HashType_invalidName() public {
        // Reverts if the input type is not found in the bindings
        vm._expectCheatcodeRevert();
        bytes32 fromTypeName = vm.eip712HashType("InvalidTypeName");
    }

    function testEip712HashType_byCustomPathAndTypeName() public {
        bytes32 canonicalHash = keccak256(bytes(CANONICAL));

        // Can figure out the canonical type from the previously generated bindings
        bytes32 fromTypeName = vm.eip712HashType("utils/CustomJsonBindings.sol", "Transaction");
        assertEq(fromTypeName, canonicalHash);
    }
}
"#,
    )
    .unwrap();

    // cheatcode by type definition can run without bindings
    cmd.forge_fuse()
        .args(["test", "--mc", "Eip712Test", "--match-test", "testEip712HashType_byDefinition"])
        .assert_success();

    let bindings = prj.root().join("utils").join("JsonBindings.sol");
    prj.update_config(|config| config.fs_permissions.add(PathPermission::read(&bindings)));

    // cheatcode by type name fails if bindings haven't been generated
    cmd.forge_fuse()
        .args(["test", "--mc", "Eip712Test", "--match-test", "testEip712HashType_byTypeName"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for src/Eip712Cheat.sol:Eip712Test
[FAIL: vm.eip712HashType: failed to read from [..] testEip712HashType_byTypeName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in src/Eip712Cheat.sol:Eip712Test
[FAIL: vm.eip712HashType: failed to read from [..] testEip712HashType_byTypeName() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);

    cmd.forge_fuse().args(["bind-json"]).assert_success();
    assert!(bindings.exists(), "'JsonBindings.sol' was not generated at {bindings:?}");

    // with generated bindings, cheatcode by type name works
    cmd.forge_fuse()
        .args(["test", "--mc", "Eip712Test", "--match-test", "testEip712HashType_byTypeName"])
        .assert_success();

    // even with generated bindings, cheatcode by type name fails if name is not present
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "Eip712Test",
            "--match-test",
            "testReverts_Eip712HashType_invalidName",
        ])
        .assert_success();

    let bindings_2 = prj.root().join("utils").join("CustomJsonBindings.sol");
    prj.update_config(|config| {
        config.fs_permissions.add(PathPermission::read(&bindings_2));
    });

    // cheatcode by custom path and type name fails if bindings haven't been generated for that path
    cmd.forge_fuse()
        .args(["test", "--mc", "Eip712Test", "--match-test", "testEip712HashType_byCustomPathAndTypeName"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for src/Eip712Cheat.sol:Eip712Test
[FAIL: vm.eip712HashType: failed to read from [..] testEip712HashType_byCustomPathAndTypeName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in src/Eip712Cheat.sol:Eip712Test
[FAIL: vm.eip712HashType: failed to read from [..] testEip712HashType_byCustomPathAndTypeName() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);

    cmd.forge_fuse().args(["bind-json", "utils/CustomJsonBindings.sol"]).assert_success();
    assert!(bindings_2.exists(), "'CustomJsonBindings.sol' was not generated at {bindings_2:?}");

    // with generated bindings, cheatcode by custom path and type name works
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "Eip712Test",
            "--match-test",
            "testEip712HashType_byCustomPathAndTypeName",
        ])
        .assert_success();
});

forgetest!(test_eip712_hash_struct_simple, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source(
        "Eip712HashStructDomainTest.sol",
        r#"
import "./Vm.sol";
import "./test.sol";
import "./console.sol";

struct EIP712Domain {
    string name;
    string version;
    uint256 chainId;
    address verifyingContract;
}

string constant _EIP712_DOMAIN_TYPE_DEF = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
bytes32 constant _EIP712_DOMAIN_TYPE_HASH = keccak256(bytes(_EIP712_DOMAIN_TYPE_DEF));

contract Eip712HashStructDomainTest is DSTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testHashEIP712Domain() public {
        EIP712Domain memory domain = EIP712Domain({
            name: "Foo",
            version: "Bar",
            chainId: 1,
            verifyingContract: 0xdEADBEeF00000000000000000000000000000000
        });

        // simulate user-computed domain hash
        bytes memory encodedData = abi.encode(
            keccak256(bytes(domain.name)),
            keccak256(bytes(domain.version)),
            bytes32(domain.chainId),
            bytes32(uint256(uint160(domain.verifyingContract)))
        );
        bytes32 userStructHash = keccak256(abi.encodePacked(_EIP712_DOMAIN_TYPE_HASH, encodedData));

        // cheatcode-computed domain hash
        bytes32 cheatStructHash = vm.eip712HashStruct(_EIP712_DOMAIN_TYPE_DEF, abi.encode(domain));
        console.log("EIP712Domain struct hash from cheatcode:");
        console.logBytes32(cheatStructHash);

        assertEq(cheatStructHash, userStructHash, "EIP712Domain struct hash mismatch");
    }
}
"#,
        )
        .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "Eip712HashStructDomainTest", "-vvvv"]).assert_success();
});

forgetest!(test_eip712_hash_struct_complex, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source(
        "Eip712Permit.sol",
        r#"
struct PermitDetails {
    address token;
    uint160 amount;
    uint48 expiration;
    uint48 nonce;
}

bytes32 constant _PERMIT_DETAILS_TYPEHASH = keccak256(
    "PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
);

struct PermitSingle {
    PermitDetails details;
    address spender;
    uint256 sigDeadline;
}

bytes32 constant _PERMIT_SINGLE_TYPEHASH = keccak256(
    "PermitSingle(PermitDetails details,address spender,uint256 sigDeadline)PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
);

// borrowed from https://github.com/Uniswap/permit2/blob/main/src/libraries/PermitHash.sol
library PermitHash {
    function hash(PermitSingle memory permitSingle) internal pure returns (bytes32) {
        bytes32 permitHash = _hashDetails(permitSingle.details);
        return
            keccak256(abi.encode(_PERMIT_SINGLE_TYPEHASH, permitHash, permitSingle.spender, permitSingle.sigDeadline));
    }

    function _hashDetails(PermitDetails memory details) internal pure returns (bytes32) {
        return keccak256(abi.encode(_PERMIT_DETAILS_TYPEHASH, details));
    }
}
"#,
    )
    .unwrap();

    prj.add_source(
        "Eip712Transaction.sol",
        r#"
struct Asset {
    address token;
    uint256 amount;
}

bytes32 constant _ASSET_TYPEHASH = keccak256(
    "Asset(address token,uint256 amount)"
);

struct Person {
    address wallet;
    string name;
}

bytes32 constant _PERSON_TYPEHASH = keccak256(
    "Person(address wallet,string name)"
);

struct Transaction {
    Person from;
    Person to;
    Asset tx;
}

bytes32 constant _TRANSACTION_TYPEHASH = keccak256(
    "Transaction(Person from,Person to,Asset tx)Asset(address token,uint256 amount)Person(address wallet,string name)"
);


library TransactionHash {
    function hash(Transaction memory t) internal pure returns (bytes32) {
        bytes32 fromHash = _hashPerson(t.from);
        bytes32 toHash = _hashPerson(t.to);
        bytes32 assetHash = _hashAsset(t.tx);
        return
            keccak256(abi.encode(_TRANSACTION_TYPEHASH, fromHash, toHash, assetHash));
    }

    function _hashPerson(Person memory person) internal pure returns (bytes32) {
        return keccak256(
            abi.encode(_PERSON_TYPEHASH, person.wallet, keccak256(bytes(person.name)))
        );

    }

    function _hashAsset(Asset memory asset) internal pure returns (bytes32) {
        return keccak256(abi.encode(_ASSET_TYPEHASH, asset));
    }
}
    "#,
    )
    .unwrap();

    let bindings = prj.root().join("utils").join("JsonBindings.sol");
    prj.update_config(|config| config.fs_permissions.add(PathPermission::read(&bindings)));
    cmd.forge_fuse().args(["bind-json"]).assert_success();

    prj.add_source(
        "Eip712HashStructTest.sol",
        r#"
import "./Vm.sol";
import "./test.sol";
import "./console.sol";
import "./Eip712Permit.sol";
import "./Eip712Transaction.sol";

contract Eip712HashStructTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testHashPermitSingle_withTypeName() public {
        PermitDetails memory details = PermitDetails({
            token: 0x1111111111111111111111111111111111111111,
            amount: 1000 ether,
            expiration: 12345,
            nonce: 1
        });

        // user-computed permit (using uniswap hash library)
        bytes32 userStructHash = PermitHash._hashDetails(details);

        // cheatcode-computed permit
        bytes32 cheatStructHash = vm.eip712HashStruct("PermitDetails", abi.encode(details));

        assertEq(cheatStructHash, userStructHash, "details struct hash mismatch");

        PermitSingle memory permit = PermitSingle({
            details: details,
            spender: 0x2222222222222222222222222222222222222222,
            sigDeadline: 12345
        });

        // user-computed permit (using uniswap hash library)
        userStructHash = PermitHash.hash(permit);

        // cheatcode-computed permit
        cheatStructHash = vm.eip712HashStruct("PermitSingle", abi.encode(permit));
        console.log("PermitSingle struct hash from cheatcode:");
        console.logBytes32(cheatStructHash);

        assertEq(cheatStructHash, userStructHash, "permit struct hash mismatch");
    }

    function testHashPermitSingle_withTypeDefinion() public {
        PermitDetails memory details = PermitDetails({
            token: 0x1111111111111111111111111111111111111111,
            amount: 1000 ether,
            expiration: 12345,
            nonce: 1
        });

        // user-computed permit (using uniswap hash library)
        bytes32 userStructHash = PermitHash._hashDetails(details);

        // cheatcode-computed permit
        bytes32 cheatStructHash = vm.eip712HashStruct("PermitDetails(address token, uint160 amount, uint48 expiration, uint48 nonce)", abi.encode(details));

        assertEq(cheatStructHash, userStructHash, "details struct hash mismatch");

        PermitSingle memory permit = PermitSingle({
            details: details,
            spender: 0x2222222222222222222222222222222222222222,
            sigDeadline: 12345
        });

        // user-computed permit (using uniswap hash library)
        userStructHash = PermitHash.hash(permit);

        // cheatcode-computed permit (previously encoding)
        cheatStructHash = vm.eip712HashStruct("PermitDetails(address token, uint160 amount, uint48 expiration, uint48 nonce) PermitSingle(PermitDetails details,address spender,uint256 sigDeadline)", abi.encode(permit));
        console.log("PermitSingle struct hash from cheatcode:");
        console.logBytes32(cheatStructHash);

        assertEq(cheatStructHash, userStructHash, "permit struct hash mismatch");
    }

    function testHashTransaction_withTypeName() public {
        Asset memory asset = Asset ({ token: 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2, amount: 100 ether });

        bytes32 user = TransactionHash._hashAsset(asset);
        bytes32 cheat = vm.eip712HashStruct("Asset", abi.encode(asset));
        assertEq(user, cheat, "asset struct hash mismatch");

        Person memory from = Person ({ wallet: 0x0000000000000000000000000000000000000001, name: "alice" });
        Person memory to = Person ({ wallet: 0x0000000000000000000000000000000000000002, name: "bob" });

        user = TransactionHash._hashPerson(from);
        cheat = vm.eip712HashStruct("Person", abi.encode(from));
        assertEq(user, cheat, "person struct hash mismatch");

        Transaction memory t = Transaction ({ from: from, to: to, tx: asset });

        user = TransactionHash.hash(t);
        cheat = vm.eip712HashStruct("Transaction", abi.encode(t));
        assertEq(user, cheat, "transaction struct hash mismatch");
    }

    function testHashTransaction_withTypeDefinition() public {
        Asset memory asset = Asset ({ token: 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2, amount: 100 ether });

        bytes32 user = TransactionHash._hashAsset(asset);
        bytes32 cheat = vm.eip712HashStruct("Asset(address token, uint256 amount)", abi.encode(asset));
        assertEq(user, cheat, "asset struct hash mismatch");

        Person memory from = Person ({ wallet: 0x0000000000000000000000000000000000000001, name: "alice" });
        Person memory to = Person ({ wallet: 0x0000000000000000000000000000000000000002, name: "bob" });

        user = TransactionHash._hashPerson(from);
        cheat = vm.eip712HashStruct("Person(address wallet, string name)", abi.encode(from));
        assertEq(user, cheat, "person struct hash mismatch");

        Transaction memory t = Transaction ({ from: from, to: to, tx: asset });

        user = TransactionHash.hash(t);
        cheat = vm.eip712HashStruct("Person(address wallet, string name) Asset(address token, uint256 amount) Transaction(Person from, Person to, Asset tx)", abi.encode(t));
        assertEq(user, cheat, "transaction struct hash mismatch");
    }
}
"#,
    )
    .unwrap();

    cmd.forge_fuse()
        .args(["test", "--mc", "Eip712HashStructTest", "-vv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testHashPermitSingle_withTypeDefinion() ([GAS])
Logs:
  PermitSingle struct hash from cheatcode:
  0x3ed744fdcea02b6b9ad45a9db6e648bf6f18c221909f9ee425191f2a02f9e4a8

[PASS] testHashPermitSingle_withTypeName() ([GAS])
Logs:
  PermitSingle struct hash from cheatcode:
  0x3ed744fdcea02b6b9ad45a9db6e648bf6f18c221909f9ee425191f2a02f9e4a8
...
"#]]);
});

forgetest!(test_eip712_hash_typed_data, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();

    prj.add_source(
        "Eip712HashTypedData.sol",
        r#"
import "./Vm.sol";
import "./test.sol";
import "./console.sol";
contract Eip712HashTypedDataTest is DSTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testHashEIP712Message() public {
        string memory jsonData =
            '{"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"},{"name":"salt","type":"bytes32"}]},"primaryType":"EIP712Domain","domain":{"name":"example.metamask.io","version":"1","chainId":1,"verifyingContract":"0x0000000000000000000000000000000000000000"},"message":{}}';

        // since this cheatcode simply exposes an alloy fn, the test has been borrowed from:
        // <https://github.com/alloy-rs/core/blob/e0727c2224a5a83664d4ca1fb2275090d29def8b/crates/dyn-abi/src/eip712/typed_data.rs#L256>
        bytes32 expectedHash = hex"122d1c8ef94b76dad44dcb03fa772361e20855c63311a15d5afe02d1b38f6077";
        assertEq(vm.eip712HashTypedData(jsonData), expectedHash, "EIP712Domain struct hash mismatch");
    }
}
"#,
    )
    .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "Eip712HashTypedDataTest"]).assert_success();
});
