use std::{fs, str::FromStr};

use foundry_test_utils::snapbox::IntoData;
use semver::Version;

use crate::utils::generate_large_init_contract;
pub const OTHER_RESOLC_VERSION: &str = "0.1.0-dev.13";
pub const NEWEST_RESOLC_VERSION: &str = "0.1.0-dev.16";

forgetest_init!(can_build_with_resolc, |prj, cmd| {
    cmd.args(["build", "--resolc-compile"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
usually needed in the following cases:
  1. To detect whether an address belongs to a smart contract.
  2. To detect whether the deploy code execution has finished.
Polkadot comes with native account abstraction support (so smart contracts are just accounts
coverned by code), and you should avoid differentiating between contracts and non-contract
addresses.
[FILE]
Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
usually needed in the following cases:
  1. To detect whether an address belongs to a smart contract.
  2. To detect whether the deploy code execution has finished.
Polkadot comes with native account abstraction support (so smart contracts are just accounts
coverned by code), and you should avoid differentiating between contracts and non-contract
addresses.
[FILE]
Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
usually needed in the following cases:
  1. To detect whether an address belongs to a smart contract.
  2. To detect whether the deploy code execution has finished.
Polkadot comes with native account abstraction support (so smart contracts are just accounts
coverned by code), and you should avoid differentiating between contracts and non-contract
addresses.
[FILE]

"#]]);
});

forgetest_init!(force_buid_with_resolc, |prj, cmd| {
    cmd.args(["build", "--resolc-compile", "--force"]).assert_success();
});

forgetest!(code_size_exceeds_limit_with_resolc, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version(Version::from_str("0.8.28").unwrap()))
    });
    let path = prj
        .add_source("LargeContract.sol", generate_large_init_contract(100_000).as_str())
        .unwrap();

    let contents = fs::read_to_string(path).unwrap();
    let new = contents.replace("=0.8.30", "=0.8.28");
    prj.wipe_contracts();
    prj.add_raw_source("LargeContract.sol", new.as_str()).unwrap();
    cmd.args([
        "build",
        "--resolc-compile",
        "--sizes",
        "--use-resolc",
        &format!("resolc:{OTHER_RESOLC_VERSION}"),
    ])
    .assert_failure()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 527,706          | 527,706           | -277,706           | -277,706            |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse()
        .args([
            "build",
            "--resolc-compile",
            "--use-resolc",
            &format!("resolc:{OTHER_RESOLC_VERSION}"),
            "--sizes",
            "--json",
        ])
        .assert_failure()
        .stdout_eq(
            str![[r#"
{
  "LargeContract": {
    "runtime_size": 527706,
    "init_size": 527706,
    "runtime_margin": -277706,
    "init_margin": -277706
  }
}
"#]]
            .is_json(),
        );
});

forgetest_init!(build_contracts_with_optimization, |prj, cmd| {
    cmd.args([
        "build",
        "--resolc-compile",
        "--use-resolc",
        &format!("resolc:{NEWEST_RESOLC_VERSION}"),
        "--resolc-optimization",
        "0",
        "--sizes",
        "--json",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{"Counter":{"runtime_size":11175,"init_size":11175,"runtime_margin":238825,"init_margin":238825}}


"#]]);

    cmd.forge_fuse()
        .args([
            "build",
            "--resolc-compile",
            "--use-resolc",
            &format!("resolc:{NEWEST_RESOLC_VERSION}"),
            "--resolc-optimization",
            "z",
            "--sizes",
            "--json",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
{"Counter":{"runtime_size":4994,"init_size":4994,"runtime_margin":245006,"init_margin":245006}}


"#]]);
});
