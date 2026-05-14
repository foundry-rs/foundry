use std::collections::BTreeSet;

use serde_json::{Value, json};

pub fn parse_json_lines(stdout: &str) -> Vec<Value> {
    let values = stdout
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let line = line.trim();
            (!line.is_empty()).then(|| {
                serde_json::from_str(line).unwrap_or_else(|err| {
                    panic!("stdout line {} is not JSON: {err}\n{line}", idx + 1)
                })
            })
        })
        .collect::<Vec<_>>();
    assert!(!values.is_empty(), "expected at least one JSON line");
    values
}

pub fn assert_json_event(value: &Value, event: &str) {
    let keys = value
        .as_object()
        .expect("event must be a JSON object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(keys, BTreeSet::from(["schema_version", "event", "data", "errors", "warnings"]));
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["event"], event);
    assert_eq!(value["errors"], json!([]));
    assert_eq!(value["warnings"], json!([]));
    assert!(value.get("data").is_some(), "event is missing data: {value}");
}

// tests enhanced `vm.parseJson` and `vm.serializeJson` cheatcodes, which are not constrained to
// alphabetical ordering of struct keys, but rather respect the Solidity struct definition.
forgetest_init!(test_parse_json, |prj, cmd| {
    prj.add_test(
        "JsonCheats",
        r#"
import {Test} from "forge-std/Test.sol";

// Definition order: color, sweetness, sourness
// Alphabetical order: color, sourness, sweetness
struct Apple {
    string color;
    uint8 sweetness;
    uint8 sourness;
}

// Definition order: name, apples
// Alphabetical order: apples, name
struct FruitStall {
    string name;
    Apple[] apples;
}

contract SimpleJsonCheatsTest is Test {
    function testJsonParseAndSerialize() public {
        // Initial JSON has keys in a custom order, different from definition and alphabetical.
        string memory originalJson =
            '{"name":"Fresh Fruit","apples":[{"sweetness":7,"sourness":3,"color":"Red"},{"sweetness":5,"sourness":5,"color":"Green"}]}';

        // Parse the original JSON. The parser should correctly handle the unordered keys.
        bytes memory decoded = vm.parseJson(originalJson);
        FruitStall memory originalType = abi.decode(decoded, (FruitStall));

        // Assert initial parsing is correct
        assertEq(originalType.name, "Fresh Fruit");
        assertEq(originalType.apples[0].color, "Red");
        assertEq(originalType.apples[0].sweetness, 7);
        assertEq(originalType.apples[1].sourness, 5);

        // Serialize the struct back to JSON. `vm.serializeJson` should respect the order for all keys.
        string memory serializedJson = vm.serializeJsonType(
            "FruitStall(Apple[] apples,string name)Apple(string color,uint8 sourness,uint8 sweetness)",
            abi.encode(originalType)
        );

        // The expected JSON should have keys ordered according to the struct definitions.
        string memory expectedJson =
            '{"name":"Fresh Fruit","apples":[{"color":"Red","sweetness":7,"sourness":3},{"color":"Green","sweetness":5,"sourness":5}]}';
        assertEq(serializedJson, expectedJson);

        // Parse the newly serialized JSON to complete the cycle.
        bytes memory redecoded = vm.parseJson(serializedJson);
        FruitStall memory finalType = abi.decode(redecoded, (FruitStall));

        // Assert that the struct from the full cycle is identical to the original parsed struct.
        assertEq(keccak256(abi.encode(finalType)), keccak256(abi.encode(originalType)));
    }
}
"#,
    );

    // Directly run the test. No `bind-json` or type schemas are needed.
    cmd.forge_fuse().args(["test"]).assert_success();
    // Should still work when the project is not compiled.
    cmd.forge_fuse().args(["test"]).assert_success();
});
