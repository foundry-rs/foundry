//! Tests for the `forge compiler` command.

use foundry_test_utils::snapbox::IntoData;

const CONTRACT_A: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.4;

contract ContractA {}
"#;

const CONTRACT_B: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.11;

contract ContractB {}
"#;

const CONTRACT_C: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.33;

contract ContractC {}
"#;

const CONTRACT_D: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.33;

contract ContractD {}
"#;

const VYPER_INTERFACE: &str = r#"
# pragma version >=0.4.0

@external
@view
def number() -> uint256:
    return empty(uint256)

@external
def set_number(new_number: uint256):
    pass

@external
def increment() -> uint256:
    return empty(uint256)
"#;

const VYPER_CONTRACT: &str = r#"
import ICounter
implements: ICounter

number: public(uint256)

@external
def set_number(new_number: uint256):
    self.number = new_number

@external
def increment() -> uint256:
    self.number += 1
    return self.number
"#;

forgetest!(can_resolve_path, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve", "--root", prj.root().to_str().unwrap()])
        .assert_success()
        .stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
});

forgetest!(can_list_resolved_compiler_versions, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
});

forgetest!(can_list_resolved_compiler_versions_json, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
   "Solidity":[
      {
         "version":"0.8.4"
      }
   ]
}
"#]]
        .is_json(),
    );
});

forgetest!(can_list_resolved_compiler_versions_verbose, |prj, cmd| {
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);

    cmd.args(["compiler", "resolve", "-v"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.33:
├── src/ContractC.sol
└── src/ContractD.sol


"#]]);
});

forgetest!(can_list_resolved_compiler_versions_verbose_json, |prj, cmd| {
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);

    cmd.args(["compiler", "resolve", "--json", "-v"]).assert_success().stdout_eq(
        str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.33",
      "paths": [
        "src/ContractC.sol",
        "src/ContractD.sol"
      ]
    }
  ]
}
"#]]
        .is_json(),
    );
});

forgetest!(can_list_resolved_multiple_compiler_versions, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4
- 0.8.11
- 0.8.33

Vyper:
- 0.4.3


"#]]);
});

forgetest!(can_list_resolved_multiple_compiler_versions_skipped, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--skip", ".sol", "-v"]).assert_success().stdout_eq(str![[
        r#"
Vyper:

0.4.3:
├── src/Counter.vy
└── src/ICounter.vyi


"#
    ]]);
});

forgetest!(can_list_resolved_multiple_compiler_versions_skipped_json, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--skip", "Contract(A|B|C)", "--json", "-v"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.33",
      "paths": [
        "src/ContractD.sol"
      ]
    }
  ],
  "Vyper": [
    {
      "version": "0.4.3",
      "paths": [
        "src/Counter.vy",
        "src/ICounter.vyi"
      ]
    }
  ]
}
"#]]
            .is_json(),
        );
});

forgetest!(can_list_resolved_multiple_compiler_versions_verbose, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "-vv"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.4 (<= istanbul):
└── src/ContractA.sol

0.8.11 (<= london):
└── src/ContractB.sol

0.8.33 (<= prague):
├── src/ContractC.sol
└── src/ContractD.sol

Vyper:

0.4.3 (<= prague):
├── src/Counter.vy
└── src/ICounter.vyi


"#]]);
});

forgetest!(can_list_resolved_multiple_compiler_versions_verbose_json, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--json", "-vv"]).assert_success().stdout_eq(
        str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.4",
      "evm_version": "Istanbul",
      "paths": [
        "src/ContractA.sol"
      ]
    },
    {
      "version": "0.8.11",
      "evm_version": "London",
      "paths": [
        "src/ContractB.sol"
      ]
    },
    {
      "version": "0.8.33",
      "evm_version": "[..]",
      "paths": [
        "src/ContractC.sol",
        "src/ContractD.sol"
      ]
    }
  ],
  "Vyper": [
    {
      "version": "0.4.3",
      "evm_version": "[..]",
      "paths": [
        "src/Counter.vy",
        "src/ICounter.vyi"
      ]
    }
  ]
}
"#]]
        .is_json(),
    );
});

// Regression coverage for https://github.com/foundry-rs/foundry-core/pull/59:
// the upstream `foundry-compilers` previously dropped *every* Vyper `pragma version` constraint
// silently (the regex did not accept a space after `#`, and even when it matched, the parser
// passed the outer regex match instead of the named `version` group to `VersionReq::parse`).
// The tests below exercise pragma syntaxes that round-trip through the fixed parser end-to-end.
//
// Verified by reverting the `[patch.crates-io]` entry in this branch:
// `vyper_pragma_version_constraint_is_enforced` fails against released `foundry-compilers`
// (the broken parser silently picks Vyper 0.4.3, then `vyper` itself rejects the pragma later
// with a different message), while it passes once the fix is pulled in. The other two tests
// guard against accidental regressions of the new and legacy pragma spellings.

const VYPER_PEP440_COMPAT_RELEASE: &str = r#"
# pragma version ~=0.4.0

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber
"#;

const VYPER_LEGACY_AT_VERSION: &str = r#"
#@version ^0.4.0

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber
"#;

const VYPER_UNSATISFIABLE_PRAGMA: &str = r#"
# pragma version ==0.3.10

number: public(uint256)
"#;

// `~=0.4.0` is PEP 440's "compatible release" operator (snekmate-style). The fixed parser
// translates it to `>=0.4.0, <0.5.0`, which the locally-installed Vyper 0.4.3 satisfies.
forgetest!(vyper_pep440_compatible_release_pragma_resolves, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_PEP440_COMPAT_RELEASE);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Vyper:
- 0.4.3


"#]]);
});

// The legacy `#@version <req>` spelling (the only form Vyper supported prior to 0.3.10) must
// keep resolving so existing contracts in the wild are not broken by the parser change.
forgetest!(vyper_legacy_at_version_pragma_resolves, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_LEGACY_AT_VERSION);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Vyper:
- 0.4.3


"#]]);
});

// Distinguishing test: with the old code, `version_req` was always `None`, so the resolver
// silently picked the only installed Vyper (0.4.3) and the build only failed later inside the
// Vyper compiler with a different error. With the fix the constraint `==0.3.10` is honoured
// and resolution fails up front.
forgetest!(vyper_pragma_version_constraint_is_enforced, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_UNSATISFIABLE_PRAGMA);

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.3.10

"#]]);
});

// PEP 440 pre-release coverage for the upcoming Vyper 0.5 line. The resolver only has Vyper
// 0.4.3 available in CI, so the build is expected to fail; the value of these tests is asserting
// the *translated* version requirement that surfaces in the error, which proves the PEP 440 →
// semver translation in `foundry-compilers` runs end-to-end through foundry. Without the fix,
// these requirements would be silently dropped and the resolver would just pick 0.4.3.

// Compatible-release operator with a bare alpha tag (snekmate-style):
// `~=0.5.0a1`  →  `>=0.5.0-a1, <0.6.0`
forgetest!(vyper_pep440_compatible_release_alpha_pragma_is_translated, |prj, cmd| {
    prj.add_raw_source(
        "Counter.vy",
        r#"
# pragma version ~=0.5.0a1

number: public(uint256)
"#,
    );

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: >=0.5.0-a1, <0.6.0

"#]]);
});

// Exact-equality operator with a bare beta tag:
// `==0.5.0b1`  →  `=0.5.0-b1`
forgetest!(vyper_pep440_exact_beta_pragma_is_translated, |prj, cmd| {
    prj.add_raw_source(
        "Counter.vy",
        r#"
# pragma version ==0.5.0b1

number: public(uint256)
"#,
    );

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.5.0-b1

"#]]);
});

// Exact-equality operator with a bare release-candidate tag:
// `==0.5.0rc1`  →  `=0.5.0-rc1`
forgetest!(vyper_pep440_exact_rc_pragma_is_translated, |prj, cmd| {
    prj.add_raw_source(
        "Counter.vy",
        r#"
# pragma version ==0.5.0rc1

number: public(uint256)
"#,
    );

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.5.0-rc1

"#]]);
});
