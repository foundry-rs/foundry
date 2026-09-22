//! CLI tests for wallet touch id commands.

use super::*;

// tests that we can outputting multiple keys without a keystore path

#[cfg(all(target_os = "macos", feature = "touch-id"))]
casttest!(wallet_new_help_describes_touch_id_fallback, |_prj, cmd| {
    let assert = cmd.args(["wallet", "new", "--help"]).assert_success();
    let output = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(output.contains("Touch ID-assisted authentication"));
    assert!(output.contains("explicit keystore passwords remain available"));
});

#[cfg(all(target_os = "macos", feature = "touch-id"))]
casttest!(wallet_import_help_describes_touch_id_fallback, |_prj, cmd| {
    let assert = cmd.args(["wallet", "import", "--help"]).assert_success();
    let output = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(output.contains("Touch ID-assisted authentication"));
    assert!(output.contains("explicit keystore passwords remain available"));
});

casttest!(wallet_touch_id_help_lists_lifecycle_commands, |_prj, cmd| {
    let assert = cmd.args(["wallet", "touch-id", "--help"]).assert_success();
    let output = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(output.contains("enroll"));
    assert!(output.contains("status"));
    assert!(output.contains("remove"));
});

casttest!(wallet_touch_id_status_missing_and_json, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "status",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Touch ID is not enrolled for keystore `testAccount`.

"#]]);

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "status",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--json",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":{"account":"testAccount","status":"not-enrolled"},"errors":[],"warnings":[]}

"#]]);
});

casttest!(wallet_touch_id_status_recognized_is_non_mutating, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let sidecar = keystore_dir.join("testAccount.touchid");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::write(&sidecar, valid_touch_id_sidecar_fixture(1, "current-biometry")).unwrap();
    let original_sidecar = fs::read(&sidecar).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "status",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Touch ID is enrolled for keystore `testAccount` with `current-biometry` policy.

"#]]);

    assert_eq!(fs::read(sidecar).unwrap(), original_sidecar);
});

casttest!(wallet_touch_id_remove_is_idempotent, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore = keystore_dir.join("testAccount");
    let sidecar = keystore_dir.join("testAccount.touchid");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::write(&sidecar, valid_touch_id_sidecar_fixture(1, "user-presence")).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "remove",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Touch ID enrollment removed for keystore `testAccount`.

"#]]);
    assert!(keystore.exists());
    assert!(!sidecar.exists());

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "remove",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Touch ID is not enrolled for keystore `testAccount`.

"#]]);
});

casttest!(wallet_touch_id_remove_refuses_unknown_sidecar, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let sidecar = keystore_dir.join("testAccount.touchid");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::write(&sidecar, valid_touch_id_sidecar_fixture(2, "user-presence")).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "status",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Touch ID status for keystore `testAccount` is unknown: [..]/testAccount.touchid is not a recognized Touch ID sidecar.

"#]]);

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "remove",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: refusing to remove [..]/testAccount.touchid because it is not a recognized Touch ID sidecar

"#]]);
    assert!(sidecar.exists());
});

casttest!(wallet_touch_id_remove_refuses_keystore_collision, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore = keystore_dir.join("testAccount");
    let sidecar = keystore_dir.join("testAccount.touchid");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::copy(&keystore, &sidecar).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "remove",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: refusing to remove existing keystore at [..]/testAccount.touchid

"#]]);
    assert!(keystore.exists());
    assert!(sidecar.exists());
});

casttest!(wallet_remove_touch_id_sidecar, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");

    let account_name = "testAccount";
    let keystore_file = keystore_dir.join(account_name);

    cmd.args([
        "wallet",
        "import",
        account_name,
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();

    let sidecar = keystore_dir.join(format!("{account_name}.touchid"));
    fs::write(&sidecar, valid_touch_id_sidecar_fixture(1, "user-presence")).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            account_name,
            "--dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "wrong",
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: Invalid password - wallet removal cancelled

"#]]);
    assert!(keystore_file.exists());
    assert!(sidecar.exists());

    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            account_name,
            "--dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was removed successfully.

"#]]);
    assert!(!keystore_file.exists());
    assert!(!sidecar.exists());
});

casttest!(wallet_remove_preserves_invalid_touch_id_payload, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore_path = keystore_dir.join("testAccount");
    let sidecar_path = keystore_dir.join("testAccount.touchid");

    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();

    let invalid_sidecar_content = serde_json::json!({
        "version": 1,
        "policy": "user-presence",
        "se_key": "aa",
        "sealed_password": format!("04{}", "00".repeat(90)),
    })
    .to_string();

    fs::write(&sidecar_path, &invalid_sidecar_content).unwrap();

    let original_keystore_bytes = fs::read(&keystore_path).unwrap();
    let original_sidecar_bytes = fs::read(&sidecar_path).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            "testAccount",
            "--dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_failure();

    assert!(keystore_path.exists());
    assert!(sidecar_path.exists());
    assert_eq!(fs::read(&keystore_path).unwrap(), original_keystore_bytes);
    assert_eq!(fs::read(&sidecar_path).unwrap(), original_sidecar_bytes);
});

casttest!(wallet_remove_preserves_touch_id_named_keystore, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore_file = keystore_dir.join("testAccount");
    let legacy_keystore = keystore_dir.join("testAccount.touchid");

    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::copy(&keystore_file, &legacy_keystore).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            "testAccount",
            "--dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: refusing to remove existing keystore at [..]/testAccount.touchid

"#]]);
    assert!(keystore_file.exists());
    assert!(legacy_keystore.exists());
});

casttest!(wallet_remove_preserves_ambiguous_touch_id_file, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore_file = keystore_dir.join("testAccount");
    let sidecar = keystore_dir.join("testAccount.touchid");

    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();
    fs::write(&sidecar, "malformed").unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            "testAccount",
            "--dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: failed to parse json file: "[..]/testAccount.touchid": expected value at line 1 column 1

"#]]);
    assert!(keystore_file.exists());
    assert!(sidecar.exists());
});

casttest!(wallet_import_rejects_touch_id_suffix, |prj, cmd| {
    cmd.args([
        "wallet",
        "import",
        "testAccount.touchid",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        prj.root().to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: account names ending in `.touchid` are reserved

"#]]);
});

casttest!(wallet_list_preserves_ambiguous_touch_id_file, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(&keystore_path).unwrap();
    fs::write(keystore_path.join("ambiguous.touchid"), "malformed").unwrap();
    cmd.set_current_dir(prj.root());

    cmd.cast_fuse().args(["wallet", "list", "--dir", "keystore"]).assert_success().stdout_eq(str![
        [r#"ambiguous.touchid (Local)

"#]
    ]);
});

casttest!(wallet_list_retains_unknown_touch_id_sidecars_and_hides_recognized, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(&keystore_path).unwrap();

    fs::write(
        keystore_path.join("recognized.touchid"),
        valid_touch_id_sidecar_fixture(1, "user-presence"),
    )
    .unwrap();

    fs::write(
        keystore_path.join("future.touchid"),
        valid_touch_id_sidecar_fixture(2, "user-presence"),
    )
    .unwrap();

    fs::write(
        keystore_path.join("invalid_payload.touchid"),
        r#"{"version":1,"policy":"user-presence","se_key":"aa","sealed_password":"bb"}"#,
    )
    .unwrap();

    cmd.set_current_dir(prj.root());

    cmd.cast_fuse().args(["wallet", "list", "--dir", "keystore"]).assert_success().stdout_eq(str![
        [r#"future.touchid (Local)
invalid_payload.touchid (Local)

"#]
    ]);
});

// Tests that `cast wallet import --touch-id` fails without Touch ID support.
#[cfg(not(all(target_os = "macos", feature = "touch-id")))]
casttest!(wallet_import_touch_id_unsupported, |prj, cmd| {
    let keystore_path = prj.root().join("touch-id-keystore");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--keystore-dir",
        keystore_path.to_str().unwrap(),
        "--unsafe-password",
        "test",
        "--touch-id",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: `--touch-id` requires macOS and a cast build with the `touch-id` feature

"#]]);

    assert!(!keystore_path.exists());
    assert!(!keystore_path.join("testAccount").exists());
    assert!(!keystore_path.join("testAccount.touchid").exists());
});

// Tests that `cast wallet new --touch-id` fails without Touch ID support.
#[cfg(not(all(target_os = "macos", feature = "touch-id")))]
casttest!(wallet_new_touch_id_unsupported, |prj, cmd| {
    let keystore_path = prj.root().join("touch-id-keystore");
    cmd.args([
        "wallet",
        "new",
        keystore_path.to_str().unwrap(),
        "testAccount",
        "--unsafe-password",
        "test",
        "--touch-id",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: `--touch-id` requires macOS and a cast build with the `touch-id` feature

"#]]);

    assert!(!keystore_path.exists());
    assert!(!keystore_path.join("testAccount").exists());
    assert!(!keystore_path.join("testAccount.touchid").exists());
});

#[cfg(not(all(target_os = "macos", feature = "touch-id")))]
casttest!(wallet_touch_id_enroll_unsupported, |prj, cmd| {
    let keystore_dir = prj.root().join("touch-id-keystore");
    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "test",
    ])
    .assert_success();

    cmd.cast_fuse()
        .args([
            "wallet",
            "touch-id",
            "enroll",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: `--touch-id` requires macOS and a cast build with the `touch-id` feature

"#]]);
    assert!(!keystore_dir.join("testAccount.touchid").exists());
});

#[cfg(all(target_os = "macos", feature = "touch-id"))]
casttest!(
    wallet_change_password_rejects_unsupported_touch_id_sidecar_before_rewrite,
    |prj, cmd| {
        let keystore_dir = prj.root().join("keystore");
        let sidecar = keystore_dir.join("testAccount.touchid");
        let sidecar_content = valid_touch_id_sidecar_fixture(2, "user-presence");

        cmd.args([
            "wallet",
            "import",
            "testAccount",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "old_password",
        ])
        .assert_success();
        fs::write(&sidecar, &sidecar_content).unwrap();

        cmd.cast_fuse()
            .args([
                "wallet",
                "change-password",
                "testAccount",
                "--keystore-dir",
                keystore_dir.to_str().unwrap(),
                "--unsafe-password",
                "old_password",
                "--unsafe-new-password",
                "new_password",
            ])
            .assert_failure()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Error: unsupported Touch ID sidecar version 2; re-enroll this keystore to regenerate it, or delete its `.touchid` sidecar to use the password prompt

"#]]);
        assert!(sidecar.exists());
        assert_eq!(fs::read_to_string(&sidecar).unwrap(), sidecar_content);

        cmd.cast_fuse()
            .args([
                "wallet",
                "decrypt-keystore",
                "testAccount",
                "--keystore-dir",
                keystore_dir.to_str().unwrap(),
                "--unsafe-password",
                "old_password",
            ])
            .assert_success();

        cmd.cast_fuse()
            .args([
                "wallet",
                "decrypt-keystore",
                "testAccount",
                "--keystore-dir",
                keystore_dir.to_str().unwrap(),
                "--unsafe-password",
                "new_password",
            ])
            .assert_failure();
    }
);

casttest!(wallet_change_password_refuses_unknown_touch_id_sidecar, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let keystore_path = keystore_dir.join("testAccount");
    let sidecar_path = keystore_dir.join("testAccount.touchid");

    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "old_password",
    ])
    .assert_success();

    let unknown_sidecar_content = r#"{"application":"unrelated"}"#;
    fs::write(&sidecar_path, unknown_sidecar_content).unwrap();

    let original_keystore_bytes = fs::read(&keystore_path).unwrap();
    let original_sidecar_bytes = fs::read(&sidecar_path).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "change-password",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "old_password",
            "--unsafe-new-password",
            "new_password",
        ])
        .assert_failure();

    assert_eq!(fs::read(&keystore_path).unwrap(), original_keystore_bytes);
    assert_eq!(fs::read(&sidecar_path).unwrap(), original_sidecar_bytes);

    cmd.cast_fuse()
        .args([
            "wallet",
            "decrypt-keystore",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "old_password",
        ])
        .assert_success();

    cmd.cast_fuse()
        .args([
            "wallet",
            "decrypt-keystore",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "new_password",
        ])
        .assert_failure();
});

#[cfg(not(all(target_os = "macos", feature = "touch-id")))]
casttest!(wallet_change_password_removes_touch_id_sidecar, |prj, cmd| {
    let keystore_dir = prj.root().join("keystore");
    let sidecar = keystore_dir.join("testAccount.touchid");

    cmd.args([
        "wallet",
        "import",
        "testAccount",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--keystore-dir",
        keystore_dir.to_str().unwrap(),
        "--unsafe-password",
        "old_password",
    ])
    .assert_success();
    fs::write(&sidecar, valid_touch_id_sidecar_fixture(1, "user-presence")).unwrap();

    cmd.cast_fuse()
        .args([
            "wallet",
            "change-password",
            "testAccount",
            "--keystore-dir",
            keystore_dir.to_str().unwrap(),
            "--unsafe-password",
            "old_password",
            "--unsafe-new-password",
            "new_password",
        ])
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Removed the stale Touch ID enrollment after changing the password

"#]]);
    assert!(!sidecar.exists());
});
