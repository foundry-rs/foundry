use serde_json::json;

const LIMITATION: &str = "Only entries in the compiler-reported `storage` array are compared. \
    Namespaced (including EIP-7201) and manually computed slots are outside this check unless \
    represented in that array; enum member changes and state-variable behavior are not checked.";

forgetest_init!(accepts_semantic_append_with_json_report, |prj, cmd| {
    prj.add_source(
        "Current.sol",
        r#"
contract Current {
    uint128 value;
    uint128 count;
    uint256 appended;
}
"#,
    );
    prj.create_file(
        "previous.json",
        r#"{
  "storage": [
    {
      "astId": 1,
      "contract": "old/location/Current.sol:Current",
      "label": "value",
      "offset": 0,
      "slot": "0",
      "type": "old_uint128"
    },
    {
      "astId": 2,
      "contract": "old/location/Current.sol:Current",
      "label": "count",
      "offset": 16,
      "slot": "0",
      "type": "old_uint128"
    }
  ],
  "types": {
    "old_uint128": {
      "encoding": "inplace",
      "label": "uint128",
      "numberOfBytes": "16"
    }
  }
}"#,
    );

    cmd.args(["storage-layout", "Current", "--reference", "previous.json", "--json"])
        .assert_json_stdout(
            serde_json::to_string_pretty(&json!({
                "compatible": true,
                "contract": "Current",
                "reference": "previous.json",
                "scope": "compilerStorageLayout",
                "changes": [{
                    "severity": "info",
                    "kind": "appended",
                    "previous": null,
                    "current": {
                        "label": "appended",
                        "contract": "src/Current.sol:Current",
                        "slot": "1",
                        "offset": 0,
                        "type": "uint256"
                    },
                    "message": "State variable `appended` was appended at slot 1 offset 0."
                }],
                "limitations": [LIMITATION]
            }))
            .unwrap(),
        );
});

forgetest_init!(rejects_inserted_variable_with_json_report, |prj, cmd| {
    prj.add_source(
        "Current.sol",
        r#"
contract Current {
    uint256 inserted;
    uint256 value;
}
"#,
    );
    prj.create_file(
        "previous.json",
        r#"{
  "storage": [{
    "astId": 1,
    "contract": "src/Current.sol:Current",
    "label": "value",
    "offset": 0,
    "slot": "0",
    "type": "old_uint256"
  }],
  "types": {
    "old_uint256": {
      "encoding": "inplace",
      "label": "uint256",
      "numberOfBytes": "32"
    }
  }
}"#,
    );

    cmd.args(["storage-layout", "Current", "--reference", "previous.json", "--json"])
        .assert_json_stdout_with_status(
        false,
        serde_json::to_string_pretty(&json!({
            "compatible": false,
            "contract": "Current",
            "reference": "previous.json",
            "scope": "compilerStorageLayout",
            "changes": [
                {
                    "severity": "error",
                    "kind": "labelChanged",
                    "previous": {
                        "label": "value",
                        "contract": "src/Current.sol:Current",
                        "slot": "0",
                        "offset": 0,
                        "type": "uint256"
                    },
                    "current": {
                        "label": "inserted",
                        "contract": "src/Current.sol:Current",
                        "slot": "0",
                        "offset": 0,
                        "type": "uint256"
                    },
                    "message": "State variable at slot 0 offset 0 changed label from `value` to \
                        `inserted`; use `--allow-renames` only for an intentional rename."
                },
                {
                    "severity": "error",
                    "kind": "addedOrReordered",
                    "previous": null,
                    "current": {
                        "label": "value",
                        "contract": "src/Current.sol:Current",
                        "slot": "1",
                        "offset": 0,
                        "type": "uint256"
                    },
                    "message": "State variable `value` was added at slot 1 offset 0, but the \
                        previous layout is not an unchanged prefix."
                }
            ],
            "limitations": [LIMITATION]
        }))
        .unwrap(),
    );
});

forgetest_init!(uses_clone_metadata_by_default, |prj, cmd| {
    prj.add_source(
        "Current.sol",
        r#"
contract Current {
    uint256 value;
}
"#,
    );
    prj.create_file(
        ".clone.meta",
        r#"{
  "path": "src/Current.sol",
  "targetContract": "Current",
  "storageLayout": {
    "storage": [{
      "astId": 1,
      "contract": "old/location/Current.sol:Current",
      "label": "value",
      "offset": 0,
      "slot": "0",
      "type": "old_uint256"
    }],
    "types": {
      "old_uint256": {
        "encoding": "inplace",
        "label": "uint256",
        "numberOfBytes": "32"
      }
    }
  }
}"#,
    );

    cmd.args(["storage-layout", "--json"]).assert_json_stdout(
        serde_json::to_string_pretty(&json!({
            "compatible": true,
            "contract": "Current",
            "reference": prj.root().join(".clone.meta").display().to_string(),
            "scope": "compilerStorageLayout",
            "changes": [],
            "limitations": [LIMITATION]
        }))
        .unwrap(),
    );
});
