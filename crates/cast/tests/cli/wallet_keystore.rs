//! CLI tests for wallet keystore commands.

use super::*;

casttest!(browser_wallet_commands_expose_browser_option, |_prj, cmd| {
    for (name, args) in [
        ("call", &["call", "--help"][..]),
        ("estimate", &["estimate", "--help"]),
        ("access-list", &["access-list", "--help"]),
        ("wallet address", &["wallet", "address", "--help"]),
        ("wallet sign", &["wallet", "sign", "--help"]),
    ] {
        let output = cmd.cast_fuse().args(args).assert_success().get_output().stdout_lossy();
        assert!(
            output.contains("--browser"),
            "expected {name} help to expose --browser:\n{output}"
        );
    }
});

// tests that we can create a new wallet
casttest!(new_wallet, |_prj, cmd| {
    cmd.args(["wallet", "new"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]	0x[..]

"#]])
        .stderr_eq(str![[r#"
Successfully created new keypair.
[ADDRESS]
[PRIVATE_KEY]

"#]]);
});

// tests that we can create a new wallet (verbose variant)
casttest!(new_wallet_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", "-v"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]	0x[..]

"#]])
        .stderr_eq(str![[r#"
Successfully created new keypair.
[ADDRESS]
[PUBLIC_KEY]
[PRIVATE_KEY]

"#]]);
});

// tests that the machine-readable stdout record is omitted on an interactive terminal, where it
// would duplicate the stderr prose
casttest!(
    #[cfg(unix)]
    new_wallet_tty_omits_stdout_record,
    |_prj, _cmd| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cast"));
        command.env("NO_COLOR", "1").env("TERM", "dumb").args(["wallet", "new"]);

        let mut session = spawn_with_options(
            command,
            Options {
                timeout_ms: Some(30_000),
                strip_ansi_escape_codes: true,
                encoding: Encoding::UTF8,
            },
        )
        .unwrap();

        session.exp_string("Successfully created new keypair.").unwrap();
        session.exp_string("Address:").unwrap();
        session.exp_string("Private key: 0x").unwrap();
        // Only the private key value may follow; the `address\tprivate_key` record must not be
        // printed to a tty.
        let rest = session.exp_eof().unwrap();
        assert!(
            !rest.contains("0x") && !rest.contains('\t'),
            "unexpected stdout record on tty: {rest:?}"
        );
    }
);

// tests that we can create a new wallet with json output
casttest!(new_wallet_json, |_prj, cmd| {
    cmd.args(["wallet", "new", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "{...}",
      "public_key": "{...}",
      "private_key": "{...}"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
        .is_json(),
    );
});

// tests that `--json -v` does not alter stdout (verbosity is stderr-only)
casttest!(new_wallet_json_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", "--json", "-v"]).assert_success().stdout_eq(
        str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "{...}",
      "public_key": "{...}",
      "private_key": "{...}"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
        .is_json(),
    );
});

// tests that `cast wallet address --json` wraps output in envelope
casttest!(wallet_address_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "address",
        "--json",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":"0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf","errors":[],"warnings":[]}

"#]]);
});

// tests that keystore `--json` output includes address, public_key, path
casttest!(new_wallet_keystore_json, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "--json"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "{...}",
      "public_key": "{...}",
      "path": "{...}"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
            .is_json(),
        );
});

// tests that keystore `--json -v` does not alter stdout (verbosity is stderr-only)
casttest!(new_wallet_keystore_json_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "--json", "-v"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "{...}",
      "public_key": "{...}",
      "path": "{...}"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
            .is_json(),
        );
});

// tests that we can create a new wallet with keystore
casttest!(new_wallet_keystore_with_password, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);
});

// tests that we can create a new wallet with keystore (verbose variant)
casttest!(new_wallet_keystore_with_password_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "-v"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]
[PUBLIC_KEY]

"#]]);
});

// tests that `cast wallet new` prompts before overwriting an existing keystore file
casttest!(new_wallet_keystore_overwrite_protection, |prj, cmd| {
    // Create the initial keystore
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test"]).assert_success();

    // Attempt to overwrite with stdin "n" — should be cancelled
    cmd.cast_fuse()
        .current_dir(prj.root())
        .args(["wallet", "new", ".", "test-account", "--unsafe-password", "test"])
        .stdin("n\n")
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
The following keystore file(s) already exist:
   - test-account

Do you want to overwrite all 1 file(s)? [y/N]: Error: Operation cancelled. No keystores were modified.

"#]]);
});

// tests that `cast wallet new --force` overwrites existing keystore files without prompting
casttest!(new_wallet_keystore_overwrite_force, |prj, cmd| {
    // Create the initial keystore
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test"]).assert_success();

    // Overwrite with --force — should succeed without prompting
    cmd.cast_fuse()
        .current_dir(prj.root())
        .args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "--force"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);
});

// tests that `cast wallet new -n 2` prompts before overwriting existing keystore files
casttest!(new_wallet_keystore_overwrite_protection_multiple, |prj, cmd| {
    // Create 2 keystores: test-account_1 and test-account_2
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "-n", "2"])
        .assert_success();

    // Attempt to overwrite with stdin "n" — should list both and cancel
    cmd.cast_fuse()
        .current_dir(prj.root())
        .args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "-n", "2"])
        .stdin("n\n")
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
The following keystore file(s) already exist:
   - test-account_1
   - test-account_2

Do you want to overwrite all 2 file(s)? [y/N]: Error: Operation cancelled. No keystores were modified.

"#]]);
});

// tests that we can create a new wallet with default keystore location
casttest!(new_wallet_default_keystore, |_prj, cmd| {
    cmd.args(["wallet", "new", "--unsafe-password", "test"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);

    // Verify the default keystore directory was created
    let keystore_path = dirs::home_dir().unwrap().join(".foundry").join("keystores");
    assert!(keystore_path.exists());
    assert!(keystore_path.is_dir());
});

casttest!(new_wallet_multiple_keys, |_prj, cmd| {
    cmd.args(["wallet", "new", "-n", "2"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]	0x[..]
0x[..]	0x[..]

"#]])
        .stderr_eq(str![[r#"
Successfully created new keypair.
[ADDRESS]
[PRIVATE_KEY]
Successfully created new keypair.
[ADDRESS]
[PRIVATE_KEY]

"#]]);
});

// tests that we can get the address of a keystore file
casttest!(wallet_address_keystore_with_password_file, |_prj, cmd| {
    let keystore_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystore");

    cmd.args([
        "wallet",
        "address",
        "--keystore",
        keystore_dir
            .join("UTC--2022-12-20T10-30-43.591916000Z--ec554aeafe75601aaab43bd4621a22284db566c2")
            .to_str()
            .unwrap(),
        "--password-file",
        keystore_dir.join("password-ec554").to_str().unwrap(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xeC554aeAFE75601AaAb43Bd4621A22284dB566C2

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/16523
casttest!(
    #[cfg(unix)]
    wallet_address_keystore_from_stdin,
    |_prj, _cmd| {
        let keystore =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystore/UTC--2022-12-20T10-30-43.591916000Z--ec554aeafe75601aaab43bd4621a22284db566c2");
        let mut command = Command::new("sh");
        command
            .env("CAST_BIN", env!("CARGO_BIN_EXE_cast"))
            .env("KEYSTORE", keystore)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .args(["-c", r#"cat "$KEYSTORE" | "$CAST_BIN" wallet address --keystore /dev/stdin"#]);

        let mut session = spawn_with_options(
            command,
            Options {
                timeout_ms: Some(30_000),
                strip_ansi_escape_codes: true,
                encoding: Encoding::UTF8,
            },
        )
        .unwrap();

        session.exp_string("Enter keystore password:").unwrap();
        session.send_line("keystorepassword").unwrap();
        let output = session.exp_eof().unwrap();
        assert!(
            matches!(session.process.wait().unwrap(), WaitStatus::Exited(_, 0)),
            "cast command failed: {output}"
        );
        assert!(
            output.contains("0xeC554aeAFE75601AaAb43Bd4621A22284dB566C2"),
            "missing keystore address: {output}"
        );
    }
);

// Tests that `cast wallet remove` can successfully remove a keystore file and validates password.
casttest!(wallet_remove_keystore_with_unsafe_password, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);

    assert!(keystore_file.exists());
    // Remove the wallet
    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            account_name,
            "--dir",
            keystore_path.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was removed successfully.

"#]]);

    assert!(!keystore_file.exists());
});

// `cast wallet import` treats ACCOUNT_NAME as a file name under the keystore dir.
// A path segment would write the encrypted keystore outside that directory.
casttest!(wallet_import_rejects_path_account_name, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    fs::create_dir_all(&keystore_dir).unwrap();
    let escaped = prj.root().join("pwned_foundry_alias");

    cmd.set_current_dir(prj.root());
    cmd.args([
        "wallet",
        "import",
        "../pwned_foundry_alias",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: account name must be a single path segment

"#]]);

    assert!(!escaped.exists());
    assert!(!keystore_dir.join("../pwned_foundry_alias").exists());
});

// Tests that `cast wallet list` outputs the local accounts.
casttest!(wallet_list_local_accounts, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(&keystore_path).unwrap();
    cmd.set_current_dir(prj.root());

    // empty results
    cmd.cast_fuse()
        .args(["wallet", "list", "--dir", "keystore"])
        .assert_success()
        .stdout_eq(str![""]);

    // create 10 wallets
    cmd.cast_fuse()
        .args(["wallet", "new", "keystore", "-n", "10", "--unsafe-password", "test"])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]
0x[..]

"#]])
        .stderr_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);

    fs::write(
        keystore_path.join("ignored.touchid"),
        valid_touch_id_sidecar_fixture(1, "user-presence"),
    )
    .unwrap();

    // Test listing new wallets while omitting the Touch ID sidecar.
    cmd.cast_fuse().args(["wallet", "list", "--dir", "keystore"]).assert_success().stdout_eq(str![
        [r#"
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)

"#]
    ]);
});

// Tests that `cast wallet list --json --dir` wraps local accounts in the shared envelope.
casttest!(wallet_list_local_accounts_json, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(&keystore_path).unwrap();
    cmd.set_current_dir(prj.root());

    cmd.args(["wallet", "new", "keystore", "--unsafe-password", "test"]).assert_success();
    fs::write(
        keystore_path.join("ignored.touchid"),
        valid_touch_id_sidecar_fixture(1, "user-presence"),
    )
    .unwrap();

    cmd.cast_fuse()
        .args(["wallet", "list", "--json", "--dir", "keystore"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "{...}",
      "source": "Local"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
            .is_json(),
        );
});

// tests that `cast wallet list` preserves custom keystore names
casttest!(wallet_list_named_local_account, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(&keystore_path).unwrap();
    fs::write(keystore_path.join("my_account"), "{}").unwrap();
    cmd.set_current_dir(prj.root());

    cmd.cast_fuse().args(["wallet", "list", "--dir", "keystore"]).assert_success().stdout_eq(str![
        [r#"
my_account (Local)

"#]
    ]);

    cmd.cast_fuse()
        .args(["wallet", "list", "--json", "--dir", "keystore"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "my_account",
      "source": "Local"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
            .is_json(),
        );
});

// tests that `cast wallet vanity --json --nonce` wraps wallet and contract address output
casttest!(wallet_vanity_json_nonce_contract_address, |_prj, cmd| {
    cmd.args(["wallet", "vanity", "--starts-with", ".", "--nonce", "1", "--json"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": {
    "address": "{...}",
    "private_key": "{...}",
    "contract_address": "{...}"
  },
  "errors": [],
  "warnings": []
}

"#]]
            .is_json(),
        );
});

// tests that `cast wallet import` creates a keystore for a private key and that `cast wallet
// decrypt-keystore` can access it
casttest!(wallet_import_and_decrypt, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);

    assert!(keystore_file.exists());

    // decrypt the keystore file
    let decrypt_output = cmd.cast_fuse().args([
        "wallet",
        "decrypt-keystore",
        account_name,
        "-k",
        "keystore",
        "--unsafe-password",
        "test",
    ]);

    // get the PK out of the output (last word in the output)
    let decrypt_output = decrypt_output.assert_success().get_output().stdout_lossy();
    let private_key_string = decrypt_output.split_whitespace().last().unwrap();
    // check that the decrypted private key matches the imported private key
    let decrypted_private_key = B256::from_str(private_key_string).unwrap();
    // the form
    assert_eq!(decrypted_private_key, test_private_key);
});

// tests that `cast wallet change-password` can successfully change the password of a keystore file
casttest!(wallet_change_password, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key with initial password
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "old_password",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);
    assert!(keystore_file.exists());

    // change the password
    cmd.cast_fuse()
        .args([
            "wallet",
            "change-password",
            account_name,
            "--keystore-dir",
            "keystore",
            "--unsafe-password",
            "old_password",
            "--unsafe-new-password",
            "new_password",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Password for keystore `testAccount` was changed successfully. [ADDRESS]

"#]])
        .stderr_eq(str![""]);

    // verify the old password no longer works
    cmd.cast_fuse()
        .args([
            "wallet",
            "decrypt-keystore",
            account_name,
            "-k",
            "keystore",
            "--unsafe-password",
            "old_password",
        ])
        .assert_failure();

    // verify the new password works
    let decrypt_output = cmd.cast_fuse().args([
        "wallet",
        "decrypt-keystore",
        account_name,
        "-k",
        "keystore",
        "--unsafe-password",
        "new_password",
    ]);

    // get the PK out of the output (last word in the output)
    let decrypt_output = decrypt_output.assert_success().get_output().stdout_lossy();
    let private_key_string = decrypt_output.split_whitespace().last().unwrap();

    // check that the decrypted private key matches the imported private key
    let decrypted_private_key = B256::from_str(private_key_string).unwrap();
    assert_eq!(decrypted_private_key, test_private_key);
});
