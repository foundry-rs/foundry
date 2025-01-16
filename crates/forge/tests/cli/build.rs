use crate::utils::generate_large_contract;
use foundry_config::Config;
use foundry_test_utils::{forgetest, snapbox::IntoData, str};
use globset::Glob;

forgetest_init!(can_parse_build_filters, |prj, cmd| {
    prj.clear();

    cmd.args(["build", "--names", "--skip", "tests", "scripts"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
  compiler version: [..]
    - Counter

"#]
    ]);
});

forgetest!(throws_on_conflicting_args, |prj, cmd| {
    prj.clear();

    cmd.args(["compile", "--format-json", "--quiet"]).assert_failure().stderr_eq(str![[r#"
error: the argument '--json' cannot be used with '--quiet'

Usage: forge[..] build --json [PATHS]...

For more information, try '--help'.

"#]]);
});

// tests that json is printed when --format-json is passed
forgetest!(compile_json, |prj, cmd| {
    prj.add_source(
        "jsonError",
        r"
contract Dummy {
    uint256 public number;
    function something(uint256 newNumber) public {
        number = newnumber; // error here
    }
}
",
    )
    .unwrap();

    // set up command
    cmd.args(["compile", "--format-json"]).assert_success().stdout_eq(str![[r#"
{
  "errors": [
    {
      "sourceLocation": {
        "file": "src/jsonError.sol",
        "start": 184,
        "end": 193
      },
      "type": "DeclarationError",
      "component": "general",
      "severity": "error",
      "errorCode": "7576",
      "message": "Undeclared identifier. Did you mean \"newNumber\"?",
      "formattedMessage": "DeclarationError: Undeclared identifier. Did you mean \"newNumber\"?\n [FILE]:7:18:\n  |\n7 |         number = newnumber; // error here\n  |                  ^^^^^^^^^\n\n"
    }
  ],
  "sources": {},
  "contracts": {},
  "build_infos": "{...}"
}
"#]].is_json());
});

forgetest!(initcode_size_exceeds_limit, |prj, cmd| {
    prj.write_config(Config { optimizer: Some(true), ..Default::default() });
    prj.add_source("LargeContract", generate_large_contract(5450).as_str()).unwrap();
    cmd.args(["build", "--sizes"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

╭--------------+------------------+-------------------+--------------------+---------------------╮
| Contract     | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+================================================================================================+
| HugeContract | 194              | 49,344            | 24,382             | -192                |
╰--------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_failure().stdout_eq(
        str![[r#"
{
   "HugeContract":{
      "runtime_size":194,
      "init_size":49344,
      "runtime_margin":24382,
      "init_margin":-192
   }
}
"#]]
        .is_json(),
    );
});

forgetest!(initcode_size_limit_can_be_ignored, |prj, cmd| {
    prj.write_config(Config { optimizer: Some(true), ..Default::default() });
    prj.add_source("LargeContract", generate_large_contract(5450).as_str()).unwrap();
    cmd.args(["build", "--sizes", "--ignore-eip-3860"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

╭--------------+------------------+-------------------+--------------------+---------------------╮
| Contract     | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+================================================================================================+
| HugeContract | 194              | 49,344            | 24,382             | -192                |
╰--------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse()
        .args(["build", "--sizes", "--ignore-eip-3860", "--json"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "HugeContract": {
    "runtime_size": 194,
    "init_size": 49344,
    "runtime_margin": 24382,
    "init_margin": -192
  }
} 
"#]]
            .is_json(),
        );
});

// tests build output is as expected
forgetest_init!(exact_build_output, |prj, cmd| {
    cmd.args(["build", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// tests build output is as expected
forgetest_init!(build_sizes_no_forge_std, |prj, cmd| {
    prj.write_config(Config {
        optimizer: Some(true),
        solc: Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 27))),
        ..Default::default()
    });

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭----------+------------------+-------------------+--------------------+---------------------╮
| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+============================================================================================+
| Counter  | 236              | 263               | 24,340             | 48,889              |
╰----------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "Counter": {
    "runtime_size": 236,
    "init_size": 263,
    "runtime_margin": 24340,
    "init_margin": 48889
  }
}
"#]]
        .is_json(),
    );
});

// tests that skip key in config can be used to skip non-compilable contract
forgetest_init!(test_can_skip_contract, |prj, cmd| {
    prj.add_source(
        "InvalidContract",
        r"
contract InvalidContract {
    some_invalid_syntax
}
",
    )
    .unwrap();

    prj.add_source(
        "ValidContract",
        r"
contract ValidContract {}
",
    )
    .unwrap();

    let config = Config {
        skip: vec![Glob::new("src/InvalidContract.sol").unwrap().into()],
        ..Default::default()
    };
    prj.write_config(config);

    cmd.args(["build"]).assert_success();
});
