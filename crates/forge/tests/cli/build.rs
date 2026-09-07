use crate::utils::generate_large_init_contract;
use foundry_compilers::artifacts::{BytecodeHash, EvmVersion};
use foundry_config::{CompilationRestrictions, SettingsOverrides};
use foundry_test_utils::{forgetest, forgetest_init, snapbox::IntoData, str, util::OutputExt};
use globset::Glob;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn add_local_submodule(root: &Path, path: &str) -> String {
    let source = root.join("lib/forge-std");
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(source)
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    git(&root.join(path), &["rev-parse", "HEAD"])
}

#[cfg(unix)]
forgetest!(local_compiler_runs_without_warning, |prj, cmd| {
    let solc = prj.root().join("payload");
    let invoked = prj.root().join("payload.invoked");
    fs::write(
        &solc,
        r#"#!/bin/sh
touch "$0.invoked"
if [ "$1" = "--version" ]; then
    echo "solc, the solidity compiler commandline interface"
    echo "Version: 0.8.35+commit.69074fbd"
    exit 0
fi
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&solc).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&solc, permissions).unwrap();
    prj.add_source("Contract", "contract Contract {}");
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Local(solc.clone()));
    });

    let output = cmd.arg("build").assert_failure();
    let stderr = output.get_output().stderr_lossy();
    assert!(!stderr.contains("configured to use a local compiler executable"), "{stderr}");
    assert!(invoked.exists(), "local compiler did not run");
});

forgetest!(project_dotenv_loads_without_warning, |prj, cmd| {
    fs::write(prj.root().join(".env"), "FOUNDRY_SRC=dotenv-src").unwrap();

    let output = cmd.args(["config", "--json"]).assert_success();
    let stderr = output.get_output().stderr_lossy();
    assert!(!stderr.contains("Warning: loading project dotenv"), "{stderr}");
    let config: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(config["src"], "dotenv-src");
});

forgetest!(
    #[cfg(unix)]
    can_build_physical_and_symlinked_dependency_configs,
    |prj, cmd| {
        let external = tempfile::tempdir().unwrap();
        let physical = prj.root().join("lib/linked");
        let linked = external.path().join("cache/actual-package");
        let write_dependency = |dependency: &std::path::Path| {
            fs::create_dir_all(dependency.join("custom-source")).unwrap();
            fs::create_dir_all(dependency.join("vendor/inner/src")).unwrap();
            fs::create_dir_all(dependency.join("vendor/file/src")).unwrap();
            fs::write(
                dependency.join("foundry.toml"),
                r#"
[profile.default]
src = "custom-source"
remappings = ["special-alias/=vendor/inner/src/"]
"#,
            )
            .unwrap();
            fs::write(dependency.join("remappings.txt"), "file-alias/=vendor/file/src/\n").unwrap();
            fs::write(
                dependency.join("custom-source/Dep.sol"),
                r#"
pragma solidity >=0.8.0;

import {Thing} from "special-alias/Thing.sol";
import {FromFile} from "file-alias/FromFile.sol";

contract Dep is Thing, FromFile {}
"#,
            )
            .unwrap();
            fs::write(
                dependency.join("vendor/inner/src/Thing.sol"),
                r#"
pragma solidity >=0.8.0;

contract Thing {}
"#,
            )
            .unwrap();
            fs::write(
                dependency.join("vendor/file/src/FromFile.sol"),
                r#"
pragma solidity >=0.8.0;

contract FromFile {}
"#,
            )
            .unwrap();
        };

        write_dependency(&physical);
        write_dependency(&linked);
        prj.add_raw_source(
            "Root.sol",
            r#"
pragma solidity >=0.8.0;

import {Dep} from "linked/Dep.sol";

contract Root is Dep {}
"#,
        );

        let remappings = str![[r#"
file-alias/=lib/linked/vendor/file/src/
linked/=lib/linked/custom-source/
special-alias/=lib/linked/vendor/inner/src/

"#]];
        cmd.arg("remappings").assert_success().stdout_eq(remappings.clone());
        cmd.forge_fuse().arg("build").assert_success();

        fs::remove_dir_all(&physical).unwrap();
        symlink(&linked, &physical).unwrap();
        cmd.forge_fuse().arg("clean").assert_success();
        cmd.forge_fuse().arg("remappings").assert_success().stdout_eq(remappings);
        cmd.forge_fuse().arg("build").assert_success();
    }
);

forgetest!(
    #[cfg(unix)]
    can_build_symlinked_dependency_with_existing_standard_source,
    |prj, cmd| {
        let external = tempfile::tempdir().unwrap();
        let dependency = external.path().join("dependency");
        fs::create_dir_all(dependency.join("src")).unwrap();
        fs::write(dependency.join("foundry.toml"), "[profile.default]\nsrc = \"src\"\n").unwrap();
        fs::write(dependency.join("src/Dep.sol"), "pragma solidity >=0.8.0; contract Dep {}\n")
            .unwrap();
        prj.update_config(|config| config.libs = vec!["node_modules".into()]);
        fs::create_dir_all(prj.root().join("node_modules")).unwrap();
        symlink(&dependency, prj.root().join("node_modules/linked")).unwrap();
        prj.add_raw_source(
            "Root.sol",
            r#"
pragma solidity >=0.8.0;

import {Dep} from "linked/src/Dep.sol";

contract Root is Dep {}
"#,
        );

        cmd.arg("remappings").assert_success().stdout_eq(str![[r#"
linked/=node_modules/linked/

"#]]);
        cmd.forge_fuse().arg("build").assert_success();
    }
);

forgetest!(
    #[cfg(unix)]
    can_build_multiple_aliases_to_symlinked_dependency_config,
    |prj, cmd| {
        let external = tempfile::tempdir().unwrap();
        let dependency = external.path().join("dependency");
        fs::create_dir_all(dependency.join("custom-source")).unwrap();
        fs::write(dependency.join("foundry.toml"), "[profile.default]\nsrc = \"custom-source\"\n")
            .unwrap();
        fs::write(
            dependency.join("custom-source/Dep.sol"),
            "pragma solidity >=0.8.0; contract Dep {}\n",
        )
        .unwrap();
        symlink(&dependency, prj.root().join("lib/a-alias")).unwrap();
        symlink(&dependency, prj.root().join("lib/z-alias")).unwrap();
        prj.add_raw_source(
            "Root.sol",
            r#"
pragma solidity >=0.8.0;

import {Dep as ADep} from "a-alias/Dep.sol";
import {Dep as ZDep} from "z-alias/Dep.sol";

contract Root {
    ADep private a;
    ZDep private z;
}
"#,
        );

        cmd.arg("remappings").assert_success().stdout_eq(str![[r#"
a-alias/=lib/a-alias/custom-source/
z-alias/=lib/z-alias/custom-source/

"#]]);
        cmd.forge_fuse().arg("build").assert_success();
    }
);

forgetest_init!(can_parse_build_filters, |prj, cmd| {
    prj.initialize_default_contracts();
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
    );

    let expected = str![[r#"
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
"#]]
    .is_json();

    cmd.args(["compile", "--format-json"])
        .assert_failure()
        .stderr_eq("")
        .stdout_eq(expected.clone());
    cmd.forge_fuse()
        .args(["compile", "--format-json", "--sizes"])
        .assert_failure()
        .stderr_eq("")
        .stdout_eq(expected);
});

forgetest!(initcode_size_exceeds_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    cmd.args(["build", "--sizes"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 62               | 50,125            | 24,514             | -973                |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_failure().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 24514,
    "init_margin": -973
  }
}
"#]]
        .is_json(),
    );

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|---------------|------------------|-------------------|--------------------|---------------------|
| LargeContract | 62               | 50,125            | 24,514             | -973                |


"#]]);

    // Ignore EIP-3860

    cmd.forge_fuse().args(["build", "--sizes", "--ignore-eip-3860"]).assert_success().stdout_eq(
        str![[r#"
No files changed, compilation skipped

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 62               | 50,125            | 24,514             | -973                |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]],
    );

    cmd.forge_fuse()
        .args(["build", "--sizes", "--ignore-eip-3860", "--json"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 24514,
    "init_margin": -973
  }
}
"#]]
            .is_json(),
        );

    cmd.forge_fuse()
        .args(["build", "--sizes", "--ignore-eip-3860", "--md"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|---------------|------------------|-------------------|--------------------|---------------------|
| LargeContract | 62               | 50,125            | 24,514             | -973                |


"#]]);
});

forgetest!(build_sizes_respects_configured_code_size_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    prj.update_config(|config| {
        config.code_size_limit = Some(64_000);
    });

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 63938,
    "init_margin": 77875
  }
}
"#]]
        .is_json(),
    );
});

#[cfg(feature = "monad")]
forgetest!(build_sizes_respects_monad_network_code_size_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    prj.update_config(|config| {
        config.networks = foundry_evm_networks::NetworkConfigs::with_monad();
    });

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 131010,
    "init_margin": 212019
  }
}
"#]]
        .is_json(),
    );
});

forgetest!(build_sizes_respects_amsterdam_code_size_limits, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    prj.update_config(|config| {
        config.evm_version = EvmVersion::Amsterdam;
    });

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 65474,
    "init_margin": 80947
  }
}
"#]]
        .is_json(),
    );
});

// tests build output is as expected
forgetest_init!(exact_build_output, |prj, cmd| {
    prj.initialize_default_contracts();
    cmd.args(["build", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

forgetest_init!(verbose_build_displays_compiler_profiles_in_combined_output, |prj, cmd| {
    prj.add_source("Default.sol", "contract Default {}");
    prj.add_source("NoMetadata.sol", "contract NoMetadata {}");
    prj.update_config(|config| {
        config.optimizer = Some(true);
        config.optimizer_runs = Some(777);
        config.via_ir = true;
        config.evm_version = EvmVersion::Cancun;
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "no-metadata".to_string(),
            via_ir: None,
            evm_version: None,
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: Some(BytecodeHash::None),
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "src/NoMetadata.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: Some(BytecodeHash::None),
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
    });

    let combined_path = prj.root().join("combined-build-output.log");
    let stdout = fs::File::create(&combined_path).unwrap();
    let stderr = stdout.try_clone().unwrap();
    let status = cmd
        .cmd()
        .args(["build", "--force", "--no-lint", "-vv"])
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .unwrap();
    let output = fs::read_to_string(combined_path).unwrap();
    assert!(status.success(), "{output}");
    let mut settings = output
        .lines()
        .filter(|line| line.starts_with("Compiler settings for "))
        .map(|line| {
            let (_, settings) = line.split_once(" (profile: ").unwrap();
            format!("Compiler settings (profile: {settings}")
        })
        .collect::<Vec<_>>();
    settings.sort_unstable();
    assert_data_eq!(
        settings.join("\n").into_data(),
        str![[r#"
Compiler settings (profile: default): optimizer=true, optimizer_runs=777, via_ir=true, evm_version=cancun
Compiler settings (profile: no-metadata): optimizer=true, optimizer_runs=777, via_ir=true, evm_version=cancun
"#]],
    );
});

// tests build output is as expected
forgetest_init!(build_sizes_no_forge_std, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 27)));
    });

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭----------+------------------+-------------------+--------------------+---------------------╮
| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+============================================================================================+
| Counter  | 481              | 509               | 24,095             | 48,643              |
╰----------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "Counter": {
    "runtime_size": 481,
    "init_size": 509,
    "runtime_margin": 24095,
    "init_margin": 48643
  }
}
"#]]
        .is_json(),
    );

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_success().stdout_eq(str![[r#"
...

| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|----------|------------------|-------------------|--------------------|---------------------|
| Counter  | 481              | 509               | 24,095             | 48,643              |


"#]]);
});

// tests build output --sizes handles multiple contracts with the same name
forgetest_init!(build_sizes_multiple_contracts, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_source(
        "Foo",
        r"
contract Foo {
}
",
    );

    prj.add_source(
        "a/Counter",
        r"
contract Counter {
    uint256 public count;
    function increment() public {
        count++;
    }
}
",
    );

    prj.add_source(
        "b/Counter",
        r"
contract Counter {
    uint256 public count;
    function decrement() public {
        count--;
    }
}
",
    );

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭-----------------------------+------------------+-------------------+--------------------+---------------------╮
| Contract                    | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+===============================================================================================================+
| Counter (src/Counter.sol)   | 481              | 509               | 24,095             | 48,643              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Counter (src/a/Counter.sol) | 344              | 372               | 24,232             | 48,780              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Counter (src/b/Counter.sol) | 291              | 319               | 24,285             | 48,833              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Foo                         | 62               | 88                | 24,514             | 49,064              |
╰-----------------------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_success().stdout_eq(str![[r#"
...

| Contract                    | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|-----------------------------|------------------|-------------------|--------------------|---------------------|
| Counter (src/Counter.sol)   | 481              | 509               | 24,095             | 48,643              |
| Counter (src/a/Counter.sol) | 344              | 372               | 24,232             | 48,780              |
| Counter (src/b/Counter.sol) | 291              | 319               | 24,285             | 48,833              |
| Foo                         | 62               | 88                | 24,514             | 49,064              |


"#]]);
});

// tests build output --sizes --json handles multiple contracts with the same name
forgetest_init!(build_sizes_multiple_contracts_json, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_source(
        "Foo",
        r"
contract Foo {
}
",
    );

    prj.add_source(
        "a/Counter",
        r"
contract Counter {
    uint256 public count;
    function increment() public {
        count++;
    }
}
",
    );

    prj.add_source(
        "b/Counter",
        r"
contract Counter {
    uint256 public count;
    function decrement() public {
        count--;
    }
}
",
    );

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
   "Counter (src/Counter.sol)":{
      "runtime_size":481,
      "init_size":509,
      "runtime_margin":24095,
      "init_margin":48643
   },
   "Counter (src/a/Counter.sol)":{
      "runtime_size":344,
      "init_size":372,
      "runtime_margin":24232,
      "init_margin":48780
   },
   "Counter (src/b/Counter.sol)":{
      "runtime_size":291,
      "init_size":319,
      "runtime_margin":24285,
      "init_margin":48833
   },
   "Foo":{
      "runtime_size":62,
      "init_size":88,
      "runtime_margin":24514,
      "init_margin":49064
   }
}
"#]]
        .is_json(),
    );
});

// tests that `--sizes` filters out internal libraries (libraries without any external/public
// functions), which are never deployed on their own.
// <https://github.com/foundry-rs/foundry/issues/1356>
forgetest!(build_sizes_filters_internal_libraries, |prj, cmd| {
    prj.add_source(
        "Libraries",
        r"
// Internal library: all functions internal, never deployed on its own.
library InternalLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

// Internal library that declares an event and error but no external functions.
library EventLib {
    event Ping(uint256 x);
    error Boom();
    function ping(uint256 a) internal pure returns (uint256) {
        return a;
    }
}

// Public library: deployed and linked.
library PublicLib {
    function sub(uint256 a, uint256 b) public pure returns (uint256) {
        return a - b;
    }
}

contract Consumer {
    using InternalLib for uint256;
    function run(uint256 x) external pure returns (uint256) {
        return x.add(1);
    }
}
",
    );

    // `InternalLib` and `EventLib` must not appear; `PublicLib` and `Consumer` must.
    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭-----------+------------------+-------------------+--------------------+---------------------╮
| Contract  | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=============================================================================================+
| Consumer  | 430              | 458               | 24,146             | 48,694              |
|-----------+------------------+-------------------+--------------------+---------------------|
| PublicLib | 432              | 509               | 24,144             | 48,643              |
╰-----------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "Consumer": {
    "runtime_size": 430,
    "init_size": 458,
    "runtime_margin": 24146,
    "init_margin": 48694
  },
  "PublicLib": {
    "runtime_size": 432,
    "init_size": 509,
    "runtime_margin": 24144,
    "init_margin": 48643
  }
}
"#]]
        .is_json(),
    );
});

// tests that when a filtered internal library shares a name with a kept contract, the survivor is
// unique and prints without the `(path)` disambiguation suffix.
// <https://github.com/foundry-rs/foundry/issues/1356>
forgetest!(build_sizes_filtered_internal_library_frees_unique_name, |prj, cmd| {
    prj.add_source(
        "a/Foo",
        r"
library Foo {
    function add(uint256 a) internal pure returns (uint256) {
        return a + 1;
    }
}
",
    );
    prj.add_source(
        "b/Foo",
        r"
contract Foo {
    function f() external pure returns (uint256) {
        return 1;
    }
}
",
    );

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭----------+------------------+-------------------+--------------------+---------------------╮
| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+============================================================================================+
| Foo      | 175              | 201               | 24,401             | 48,951              |
╰----------+------------------+-------------------+--------------------+---------------------╯


"#]]);
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
    );

    prj.add_source(
        "ValidContract",
        r"
contract ValidContract {}
",
    );

    prj.update_config(|config| {
        config.skip = vec![Glob::new("src/InvalidContract.sol").unwrap().into()];
    });

    cmd.args(["build"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/11149>
forgetest_init!(test_consistent_build_output, |prj, cmd| {
    prj.add_source(
        "AContract.sol",
        r#"
import {B} from "/badpath/B.sol";

contract A is B {}
   "#,
    );

    prj.add_source(
        "CContract.sol",
        r#"
import {B} from "badpath/B.sol";

contract C is B {}
   "#,
    );

    cmd.args(["build", "src/AContract.sol"]).assert_failure().stdout_eq(str![[r#"
...
Unable to resolve imports:
      "/badpath/B.sol" in "[..]"
with remappings:
      forge-std/=[..]
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]

"#]]);
    cmd.forge_fuse().args(["build", "src/CContract.sol"]).assert_failure().stdout_eq(str![[r#"
Unable to resolve imports:
      "badpath/B.sol" in "[..]"
with remappings:
      forge-std/=[..]
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/12458>
// <https://github.com/foundry-rs/foundry/issues/12496>
forgetest!(build_with_invalid_natspec, |prj, cmd| {
    prj.add_source(
        "ContractWithInvalidNatspec.sol",
        r#"
contract ContractA {
    /// @deprecated quoteExactOutputSingle and exactOutput. Use QuoterV2 instead.
}

/// Some editors highlight `@note` or `@todo`
/// @note foo bar

/// @title ContractB
contract ContractB {
    /**
    some example code in a comment:
    import { Ownable } from "@openzeppelin/contracts/access/Ownable.sol";
    */
}
   "#,
    );

    cmd.args(["build", "src/ContractWithInvalidNatspec.sol"]).assert_success().stderr_eq(str![[
        r#"
warning[6546]: invalid natspec tag '@deprecated', custom tags must use format '@custom:name'
  [FILE]:5:5
  │
5 │     /// @deprecated quoteExactOutputSingle and exactOutput. Use QuoterV2 instead.
  │     ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  │
...

warning[6546]: invalid natspec tag '@note', custom tags must use format '@custom:name'
  [FILE]:9:1
  │
9 │ /// @note foo bar
  │ ━━━━━━━━━━━━━━━━━
  │
...

"#
    ]]);
});

// tests that build succeeds without warning when no soldeer.lock exists
forgetest_init!(build_no_warning_without_soldeer_lock, |prj, cmd| {
    let soldeer_lock = prj.root().join("soldeer.lock");
    // soldeer.lock should not exist in a fresh project
    assert!(!soldeer_lock.exists());

    cmd.args(["build"]).assert_success().stderr_eq(str![[r#"
"#]]);
});

forgetest_init!(build_locked_succeeds_when_dependencies_match, |_prj, cmd| {
    cmd.args(["build", "--locked"]).assert_success();
});

forgetest!(build_locked_succeeds_without_lockfile_or_dependencies, |prj, cmd| {
    assert!(!prj.root().join("foundry.lock").exists());

    cmd.args(["build"]).assert_success().stderr_eq("");
    cmd.forge_fuse().args(["build", "--locked"]).assert_success().stderr_eq("");
});

forgetest!(build_locked_rejects_lockfile_outside_git_repository, |prj, cmd| {
    fs::write(prj.root().join("foundry.lock"), "{}").unwrap();

    let output = cmd.args(["build", "--locked"]).assert_failure();
    assert!(
        output.get_output().stderr_lossy().contains("not a git repository"),
        "{}",
        output.get_output().stderr_lossy()
    );
});

forgetest!(build_locked_honors_git_environment, |prj, cmd| {
    let project = prj.root().join("nested");
    fs::create_dir(&project).unwrap();
    let repository = prj.root().join("repository");
    fs::create_dir(&repository).unwrap();
    git(&repository, &["init"]);
    git(&repository, &["config", "user.email", "foundry@example.com"]);
    git(&repository, &["config", "user.name", "Foundry"]);
    git(&repository, &["commit", "--allow-empty", "-m", "initial"]);
    let head = git(&repository, &["rev-parse", "HEAD"]);

    fs::write(
        prj.root().join(".gitmodules"),
        "[submodule \"nested/lib/dep\"]\n\tpath = nested/lib/dep\n\turl = ../dep\n",
    )
    .unwrap();
    let git_dir = Path::new("../repository/.git");
    let output = Command::new("git")
        .current_dir(&project)
        .env("GIT_DIR", git_dir)
        .env("GIT_WORK_TREE", "..")
        .args(["update-index", "--add", "--cacheinfo", "160000", &head, "nested/lib/dep"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    cmd.env("GIT_DIR", git_dir);
    cmd.env("GIT_WORK_TREE", "..");
    cmd.args(["build", "--locked", "--root", "nested"]).assert_failure().stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/dep: missing from foundry.lock
  lib/dep: dependency submodule is not initialized

"#]]);
});

forgetest!(locked_is_build_only, |_prj, cmd| {
    let output = cmd.args(["config", "--locked"]).assert_failure();
    assert!(
        output.get_output().stderr_lossy().contains("unexpected argument '--locked'"),
        "{}",
        output.get_output().stderr_lossy()
    );
});

forgetest_init!(build_locked_rejects_malformed_lockfile, |prj, cmd| {
    fs::write(prj.root().join("foundry.lock"), "not json").unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: Failed to read foundry.lock

Context:
- expected ident at line 1 column 2

"#]]);
});

forgetest_init!(build_checks_foundry_lock_only_when_locked, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    let lockfile = r#"{
  "lib/forge-std": {
    "rev": "0000000000000000000000000000000000000000"
  }
}"#;
    fs::write(&foundry_lock, lockfile).unwrap();

    cmd.args(["build"]).assert_success().stderr_eq("");

    fs::write(prj.root().join("src/Broken.sol"), "this is not Solidity").unwrap();

    cmd.forge_fuse().args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[
        r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: expected 0000000000000000000000000000000000000000, found [..]

"#
    ]]);
    assert_eq!(fs::read_to_string(foundry_lock).unwrap(), lockfile);
});

forgetest_init!(build_locked_reports_uninitialized_dependency_without_installing, |prj, cmd| {
    let root = prj.root();
    let foundry_lock = fs::read(root.join("foundry.lock")).unwrap();
    let status = std::process::Command::new("git")
        .current_dir(root)
        .args(["submodule", "deinit", "-f", "lib/forge-std"])
        .status()
        .unwrap();
    assert!(status.success());
    let index = fs::read(root.join(".git/index")).unwrap();
    let git_config = fs::read(root.join(".git/config")).unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: dependency submodule is not initialized (expected [..])

"#]]);
    assert_eq!(fs::read(root.join("foundry.lock")).unwrap(), foundry_lock);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(root.join(".git/config")).unwrap(), git_config);
    assert!(!root.join("lib/forge-std/.git").exists());
});

forgetest_init!(build_locked_preserves_uninitialized_state_without_lock_entry, |prj, cmd| {
    let root = prj.root();
    fs::remove_file(root.join("foundry.lock")).unwrap();
    let status = std::process::Command::new("git")
        .current_dir(root)
        .args(["submodule", "deinit", "-f", "lib/forge-std"])
        .status()
        .unwrap();
    assert!(status.success());

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: missing from foundry.lock
  lib/forge-std: dependency submodule is not initialized

"#]]);
    assert!(!root.join("foundry.lock").exists());
    assert!(!root.join("lib/forge-std/.git").exists());
});

forgetest_init!(build_locked_preserves_conflict_without_lock_entry, |prj, cmd| {
    let root = prj.root();
    let submodule = root.join("lib/forge-std");
    let rev = |args: &[&str]| {
        let output = Command::new("git").current_dir(&submodule).args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let current = rev(&["rev-parse", "HEAD"]);
    let previous = rev(&["rev-parse", "HEAD^"]);
    fs::remove_file(root.join("foundry.lock")).unwrap();
    assert!(
        Command::new("git")
            .current_dir(root)
            .args(["update-index", "--force-remove", "lib/forge-std"])
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.take().unwrap(),
        "160000 {previous} 1\tlib/forge-std\n160000 {current} 2\tlib/forge-std\n160000 {previous} 3\tlib/forge-std\n"
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    let index = fs::read(root.join(".git/index")).unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: missing from foundry.lock
  lib/forge-std: dependency submodule has merge conflicts

"#]]);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
});

forgetest_init!(build_locked_reports_stale_lockfile_entries, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "0000000000000000000000000000000000000000"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/stale: dependency submodule is missing (expected 0000000000000000000000000000000000000000)

"#]]);
});

forgetest!(build_locked_supports_projects_nested_in_parent_repository, |prj, cmd| {
    cmd.git_init();
    cmd.args(["init", "nested", "--use-parent-git"]).assert_success();

    let root = prj.root();
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(root.join("nested/lib/forge-std"))
        .arg("sibling")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    cmd.forge_fuse().args(["install", "--root", "nested"]).assert_success();
    let foundry_lock = root.join("nested/foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    assert!(lockfile.get("../sibling").is_some());
    for (path, dependency) in lockfile.as_object_mut().unwrap() {
        dependency["rev"] =
            serde_json::Value::String(git(&root.join("nested").join(path), &["rev-parse", "HEAD"]));
    }
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.forge_fuse().args(["build", "--locked", "--no-lint", "--root", "nested"]).assert_success();
});

forgetest_init!(build_locked_supports_dependency_paths_with_spaces, |prj, cmd| {
    let root = prj.root();
    git(root, &["mv", "lib/forge-std", "lib/forge std"]);

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: BTreeMap<PathBuf, serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    let forge_std = lockfile.remove(Path::new("lib/forge-std")).unwrap();
    lockfile.insert("lib/forge std".into(), forge_std);
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["build", "--locked"]).assert_success();
});

forgetest_init!(build_locked_accepts_modified_submodule_when_head_matches_lock, |prj, cmd| {
    let root = prj.root();
    let submodule = root.join("lib/forge-std");
    let previous = git(&submodule, &["rev-parse", "HEAD^"]);
    git(root, &["update-index", "--cacheinfo", "160000", &previous, "lib/forge-std"]);

    cmd.args(["build", "--locked"]).assert_success();
});

forgetest_init!(build_locked_reports_all_mismatches_in_path_order, |prj, cmd| {
    let root = prj.root();
    add_local_submodule(root, "lib/second");
    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/forge-std"]["rev"] =
        serde_json::Value::String("0000000000000000000000000000000000000000".to_string());
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "1111111111111111111111111111111111111111"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: expected 0000000000000000000000000000000000000000, found [..]
  lib/second: missing from foundry.lock (found [..])
  lib/stale: dependency submodule is missing (expected 1111111111111111111111111111111111111111)

"#]]);
});

forgetest_init!(build_locked_supports_custom_dependency_directory, |prj, cmd| {
    let root = prj.root();
    fs::create_dir(root.join("dependencies")).unwrap();
    git(root, &["mv", "lib/forge-std", "dependencies/forge-std"]);
    prj.update_config(|config| config.libs = vec!["dependencies".into()]);

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: BTreeMap<PathBuf, serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    let forge_std = lockfile.remove(Path::new("lib/forge-std")).unwrap();
    lockfile.insert("dependencies/forge-std".into(), forge_std);
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["build", "--locked"]).assert_success();
});

forgetest_init!(build_locked_supports_project_root_as_dependency_directory, |prj, cmd| {
    prj.update_config(|config| config.libs = vec![".".into()]);

    cmd.args(["build", "--locked"]).assert_success();
});

forgetest_init!(build_locked_matches_submodules_outside_dependency_directory, |prj, cmd| {
    let root = prj.root();
    add_local_submodule(root, "vendor/second");

    cmd.forge_fuse().args(["install"]).assert_success();

    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("foundry.lock")).unwrap()).unwrap();
    assert!(lockfile.get("vendor/second").is_some());
    lockfile["lib/forge-std"]["rev"] =
        serde_json::Value::String(git(&root.join("lib/forge-std"), &["rev-parse", "HEAD"]));
    fs::write(root.join("foundry.lock"), serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.forge_fuse().args(["build", "--locked"]).assert_success();
});

forgetest_init!(build_locked_aggregates_missing_submodule_mapping, |prj, cmd| {
    let root = prj.root();
    let head = git(&root.join("lib/forge-std"), &["rev-parse", "HEAD"]);
    git(root, &["update-index", "--add", "--cacheinfo", "160000", &head, "lib/unmapped"]);
    let mut gitmodules = fs::read_to_string(root.join(".gitmodules")).unwrap();
    gitmodules.push_str("\n[submodule \"lib/unmapped\"]\n\turl = ../unmapped\n");
    fs::write(root.join(".gitmodules"), gitmodules).unwrap();

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "1111111111111111111111111111111111111111"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["build", "--locked"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/stale: dependency submodule is missing (expected 1111111111111111111111111111111111111111)
  lib/unmapped: dependency submodule is missing from .gitmodules

"#]]);
});
