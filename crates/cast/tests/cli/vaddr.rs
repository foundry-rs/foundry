//! CLI tests for vaddr commands.

use super::*;

// Tests for `cast vaddr` JSON output
casttest!(vaddr_create_json_output, |_prj, cmd| {
    // Use a pre-computed salt that satisfies the 4-byte PoW requirement for this owner.
    // Salt: 0x0000000000000000000000000000000000000000000000003ee0a78d00000000
    // Owner: 0x1234567890123456789012345678901234567890
    let out = cmd
        .args([
            "--json",
            "vaddr",
            "create",
            "--owner",
            "0x1234567890123456789012345678901234567890",
            "--salt",
            "0x0000000000000000000000000000000000000000000000003ee0a78d00000000",
            "--no-register",
            "--count",
            "2",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let envelope: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
    let v = &envelope["data"];
    assert_eq!(v["salt"], "0x0000000000000000000000000000000000000000000000003ee0a78d00000000");
    assert_eq!(
        v["registration_hash"],
        "0x000000002f51c0c4f66f3910f799c6b98e2123ef43a401a062eb8ee07498c396"
    );
    assert_eq!(v["master_id"], "0x2f51c0c4");
    let addrs = v["virtual_addresses"].as_array().expect("array");
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0]["tag"], "0x000000000000");
    assert_eq!(
        addrs[0]["address"].as_str().unwrap().to_lowercase(),
        "0x2f51c0c4fdfdfdfdfdfdfdfdfdfd000000000000"
    );
    assert_eq!(addrs[1]["tag"], "0x000000000001");
    assert_eq!(
        addrs[1]["address"].as_str().unwrap().to_lowercase(),
        "0x2f51c0c4fdfdfdfdfdfdfdfdfdfd000000000001"
    );
});

casttest!(vaddr_create_plain_output, |_prj, cmd| {
    cmd.args([
        "vaddr",
        "create",
        "--owner",
        "0x1234567890123456789012345678901234567890",
        "--salt",
        "0x0000000000000000000000000000000000000000000000003ee0a78d00000000",
        "--no-register",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Salt:              0x0000000000000000000000000000000000000000000000003ee0a78d00000000
Registration hash: 0x000000002f51c0c4f66f3910f799c6b98e2123ef43a401a062eb8ee07498c396
Master ID:         0x2f51c0c4

Virtual addresses:
  tag=0x000000000000  [..]

"#]]);
});
