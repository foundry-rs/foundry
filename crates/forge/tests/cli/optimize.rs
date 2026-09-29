//! Tests for `forge optimize`.

/// A contract whose `sumBelow` the model is offered.
const TRIANGLE: &str = r#"
contract Triangle {
    function triangle(uint256 n) external pure returns (uint256) {
        return sumBelow(n);
    }

    function triangles(uint256 a, uint256 b) external pure returns (uint256) {
        unchecked {
            return sumBelow(a) + sumBelow(b);
        }
    }

    function sumBelow(uint256 n) internal pure returns (uint256 s) {
        unchecked {
            for (uint256 i; i < n; ++i) {
                s += i;
            }
        }
    }
}
"#;

forgetest!(optimize_requires_a_model_and_a_gateway, |_prj, cmd| {
    cmd.arg("optimize").assert_failure().stderr_eq(str![[r#"
error: the following required arguments were not provided:
  --model <MODEL>
  --endpoint <URL>

Usage: forge[..] optimize --model <MODEL> --endpoint <URL> [PATH]...

For more information, try '--help'.

"#]]);
});

forgetest!(optimize_replays_without_a_model, |prj, cmd| {
    prj.add_source("Triangle", TRIANGLE);
    cmd.args(["optimize", "--replay"])
        .assert_success()
        .stdout_eq(str![[r#"
[..]/out/optimize/combined.json

"#]])
        .stderr_eq(str![[r#"
warning[2264]: code generation is experimental

Kept rewrites are in [..]/cache/llm-optimize

"#]]);
    assert!(prj.root().join("out/optimize/combined.json").is_file());
});
