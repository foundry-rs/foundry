//! CLI tests for address commands.

use super::*;

// tests that `cast create2` writes `address\tsalt` to stdout and prose to stderr
casttest!(create2_output_channels, |_prj, cmd| {
    cmd.args([
        "create2",
        "--starts-with",
        "cc",
        "--init-code-hash",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]	0x[..]

"#]]);
});

// tests that the machine-readable stdout record is omitted on an interactive terminal, where it
// would duplicate the stderr prose
casttest!(
    #[cfg(unix)]
    create2_tty_omits_stdout_record,
    |_prj, _cmd| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cast"));
        command.env("NO_COLOR", "1").env("TERM", "dumb").args([
            "create2",
            "--starts-with",
            "cc",
            "--init-code-hash",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
        ]);

        let mut session = spawn_with_options(
            command,
            Options {
                timeout_ms: Some(30_000),
                strip_ansi_escape_codes: true,
                encoding: Encoding::UTF8,
            },
        )
        .unwrap();

        session.exp_string("Successfully found contract address").unwrap();
        session.exp_string("Address: 0x").unwrap();
        session.exp_string("Salt: 0x").unwrap();
        // Only the salt value and its decimal representation may follow; the `address\tsalt`
        // record must not be printed to a tty.
        let rest = session.exp_eof().unwrap();
        assert!(
            !rest.contains("0x") && !rest.contains('\t'),
            "unexpected stdout record on tty: {rest:?}"
        );
    }
);

// tests that `cast create2 --salt` writes `address\tsalt` to stdout
casttest!(create2_fixed_salt_output_channels, |_prj, cmd| {
    cmd.args([
        "create2",
        "--salt",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--init-code-hash",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]	0x0000000000000000000000000000000000000000000000000000000000000001

"#]]);
});

casttest!(create2_init_code_hash, |prj, cmd| {
    prj.add_source(
        "InitCodeHash",
        r#"
contract InitCodeHash {
    int256 public immutable value;
    address public immutable owner;

    constructor(int256 value_, address owner_) {
        value = value_;
        owner = owner_;
    }
}
"#,
    );

    let owner = address!("0x0000000000000000000000000000000000000001");
    let bytecode = cmd
        .forge_fuse()
        .args(["inspect", "InitCodeHash", "bytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let mut expected_init_code = hex::decode(bytecode.trim()).unwrap();
    expected_init_code.extend((I256::unchecked_from(42), owner).abi_encode());
    let expected = keccak256(expected_init_code);

    cmd.cast_fuse()
        .current_dir(prj.root())
        .args([
            "create2",
            "init-code-hash",
            "src/InitCodeHash.sol:InitCodeHash",
            "42",
            &owner.to_string(),
        ])
        .assert_success()
        .stdout_eq(format!("{expected}\n"));

    let mut expected_init_code = hex::decode(bytecode.trim()).unwrap();
    expected_init_code.extend((I256::unchecked_from(-5), owner).abi_encode());
    let expected = keccak256(expected_init_code);
    let root = prj.root().to_str().unwrap();

    cmd.cast_fuse()
        .current_dir(prj.root())
        .args([
            "create2",
            "init-code-hash",
            "src/InitCodeHash.sol:InitCodeHash",
            "-5",
            &owner.to_string(),
            "--root",
            root,
        ])
        .assert_success()
        .stdout_eq(format!("{expected}\n"));

    cmd.cast_fuse()
        .current_dir(prj.root())
        .args([
            "--json",
            "create2",
            "init-code-hash",
            "src/InitCodeHash.sol:InitCodeHash",
            "-5",
            &owner.to_string(),
        ])
        .assert_json_stdout(format!(
            r#"{{"schema_version":1,"success":true,"data":"{expected}","errors":[],"warnings":[]}}"#
        ));
});

casttest!(create2_init_code_hash_rejects_abstract_contract, |prj, cmd| {
    prj.add_source(
        "AbstractInitCodeHash",
        r#"
abstract contract AbstractInitCodeHash {
    function value() public pure virtual returns (uint256);
}
"#,
    );

    cmd.cast_fuse()
        .current_dir(prj.root())
        .args(["create2", "init-code-hash", "src/AbstractInitCodeHash.sol:AbstractInitCodeHash"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: no bytecode found in bin object for AbstractInitCodeHash

"#]]);
});

casttest!(compute_address, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "compute-address",
        "0x0000000000000000000000000000000000000000",
        "--nonce",
        "0",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(format!("{}\n", Address::ZERO.create(0).to_checksum(None)));
});
