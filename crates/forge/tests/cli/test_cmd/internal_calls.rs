//! Tests for internal call detection with `--decode-internal`.

use foundry_test_utils::{TestCommand, TestProject, file, snapbox::IntoData, util::OutputExt};

/// Compiles the fixture in the given mode and compares the `-vvvv --decode-internal` output.
#[track_caller]
fn assert_internal_calls(
    prj: &TestProject,
    cmd: &mut TestCommand,
    (name, source): (&str, &str),
    (via_ir, optimize): (bool, bool),
    expected: impl IntoData,
) {
    prj.update_config(|config| {
        config.via_ir = via_ir;
        config.optimizer = Some(optimize);
        config.optimizer_runs = Some(200);
    });
    prj.add_test(name, source);
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(expected);
}

const TARGET: (&str, &str) =
    ("Target.t.sol", include_str!("../../fixtures/internal_calls/Target.t.sol"));
const C: (&str, &str) = ("C.t.sol", include_str!("../../fixtures/internal_calls/C.t.sol"));

#[forgetest]
fn target_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (false, false),
        file!["../../fixtures/internal_calls/target_legacy.txt"],
    );
}

#[forgetest]
fn target_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (false, true),
        file!["../../fixtures/internal_calls/target_legacy_optimized.txt"],
    );
}

#[forgetest]
fn target_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (true, false),
        file!["../../fixtures/internal_calls/target_via_ir.txt"],
    );
}

#[forgetest]
fn target_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (true, true),
        file!["../../fixtures/internal_calls/target_via_ir_optimized.txt"],
    );
}

#[forgetest]
fn c_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (false, false),
        file!["../../fixtures/internal_calls/c_legacy.txt"],
    );
}

#[forgetest]
fn c_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (false, true),
        file!["../../fixtures/internal_calls/c_legacy_optimized.txt"],
    );
}

#[forgetest]
fn c_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (true, false),
        file!["../../fixtures/internal_calls/c_via_ir.txt"],
    );
}

#[forgetest]
fn c_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (true, true),
        file!["../../fixtures/internal_calls/c_via_ir_optimized.txt"],
    );
}
