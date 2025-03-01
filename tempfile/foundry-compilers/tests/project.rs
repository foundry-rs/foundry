//! project tests

use alloy_primitives::{Address, Bytes};
use foundry_compilers::{
    buildinfo::BuildInfo,
    cache::{CompilerCache, SOLIDITY_FILES_CACHE_FILENAME},
    compilers::{
        multi::{
            MultiCompiler, MultiCompilerLanguage, MultiCompilerParsedSource, MultiCompilerSettings,
        },
        solc::{Solc, SolcCompiler, SolcLanguage},
        vyper::{Vyper, VyperLanguage, VyperSettings},
        CompilerOutput,
    },
    flatten::Flattener,
    info::ContractInfo,
    multi::MultiCompilerRestrictions,
    project_util::*,
    solc::{Restriction, SolcRestrictions, SolcSettings},
    take_solc_installer_lock, Artifact, ConfigurableArtifacts, ExtraOutputValues, Graph, Project,
    ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig, RestrictionsWithVersion,
    TestFileFilter,
};
use foundry_compilers_artifacts::{
    output_selection::OutputSelection, remappings::Remapping, BytecodeHash, Contract, DevDoc,
    Error, ErrorDoc, EventDoc, EvmVersion, Libraries, MethodDoc, ModelCheckerEngine::CHC,
    ModelCheckerSettings, Settings, Severity, SolcInput, UserDoc, UserDocNotice,
};
use foundry_compilers_core::{
    error::SolcError,
    utils::{self, canonicalize, RuntimeOrHandle},
};
use semver::Version;
use similar_asserts::assert_eq;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self},
    io,
    path::{Path, PathBuf, MAIN_SEPARATOR},
    str::FromStr,
    sync::LazyLock,
};
use svm::{platform, Platform};

pub static VYPER: LazyLock<Vyper> = LazyLock::new(|| {
    RuntimeOrHandle::new().block_on(async {
        #[cfg(target_family = "unix")]
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};

        if let Ok(vyper) = Vyper::new("vyper") {
            return vyper;
        }

        take_solc_installer_lock!(_lock);
        let path = std::env::temp_dir().join("vyper");

        if path.exists() {
            return Vyper::new(&path).unwrap();
        }

        let base = "https://github.com/vyperlang/vyper/releases/download/v0.4.0/vyper.0.4.0+commit.e9db8d9f";
        let url = format!(
            "{base}.{}",
            match platform() {
                Platform::MacOsAarch64 => "darwin",
                Platform::LinuxAmd64 => "linux",
                Platform::WindowsAmd64 => "windows.exe",
                platform => panic!("unsupported platform: {platform:?}"),
            }
        );

        let mut retry = 3;
        let mut res = None;
        while retry > 0 {
            match reqwest::get(&url).await.unwrap().error_for_status() {
                Ok(res2) => {
                    res = Some(res2);
                    break;
                }
                Err(e) => {
                    eprintln!("{e}");
                    retry -= 1;
                }
            }
        }
        let res = res.expect("failed to get vyper binary");

        let bytes = res.bytes().await.unwrap();

        std::fs::write(&path, bytes).unwrap();

        #[cfg(target_family = "unix")]
        std::fs::set_permissions(&path, Permissions::from_mode(0o755)).unwrap();

        Vyper::new(&path).unwrap()
    })
});

#[test]
fn can_get_versioned_linkrefs() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/test-versioned-linkrefs");
    let paths = ProjectPathsConfig::builder()
        .sources(root.join("src"))
        .lib(root.join("lib"))
        .build()
        .unwrap();

    let project = Project::builder()
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .build(Default::default())
        .unwrap();
    project.compile().unwrap().assert_success();
}

#[test]
fn can_compile_hardhat_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/hardhat-sample");
    let paths = ProjectPathsConfig::builder()
        .sources(root.join("contracts"))
        .lib(root.join("node_modules"));
    let project = TempProject::<SolcCompiler, ConfigurableArtifacts>::new(paths).unwrap();

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Greeter").is_some());
    assert!(compiled.find_first("console").is_some());
    compiled.assert_success();

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Greeter").is_some());
    assert!(compiled.find_first("console").is_some());
    assert!(compiled.is_unchanged());

    // delete artifacts
    std::fs::remove_dir_all(&project.paths().artifacts).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Greeter").is_some());
    assert!(compiled.find_first("console").is_some());
    assert!(!compiled.is_unchanged());
}

#[test]
fn can_compile_dapp_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
    let paths = ProjectPathsConfig::builder().sources(root.join("src")).lib(root.join("lib"));
    let project = TempProject::<SolcCompiler, ConfigurableArtifacts>::new(paths).unwrap();

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    compiled.assert_success();

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.is_unchanged());

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    // delete artifacts
    std::fs::remove_dir_all(&project.paths().artifacts).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(!compiled.is_unchanged());

    let updated_cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();
    assert_eq!(cache, updated_cache);
}

#[test]
fn can_compile_yul_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/yul-sample");
    let paths = ProjectPathsConfig::builder().sources(root);
    let project = TempProject::<SolcCompiler, ConfigurableArtifacts>::new(paths).unwrap();

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.find_first("SimpleStore").is_some());
    compiled.assert_success();

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.find_first("SimpleStore").is_some());
    assert!(compiled.is_unchanged());

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    // delete artifacts
    std::fs::remove_dir_all(&project.paths().artifacts).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.find_first("SimpleStore").is_some());
    assert!(!compiled.is_unchanged());

    let updated_cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();
    assert_eq!(cache, updated_cache);
}

#[test]
fn can_compile_configured() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
    let paths = ProjectPathsConfig::builder().sources(root.join("src")).lib(root.join("lib"));

    let handler = ConfigurableArtifacts {
        additional_values: ExtraOutputValues {
            metadata: true,
            ir: true,
            ir_optimized: true,
            opcodes: true,
            legacy_assembly: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let settings = handler.solc_settings();
    let project = TempProject::with_artifacts(paths, handler).unwrap().with_solc_settings(settings);
    let compiled = project.compile().unwrap();
    let artifact = compiled.find_first("Dapp").unwrap();
    assert!(artifact.metadata.is_some());
    assert!(artifact.raw_metadata.is_some());
    assert!(artifact.ir.is_some());
    assert!(artifact.ir_optimized.is_some());
    assert!(artifact.opcodes.is_some());
    assert!(artifact.opcodes.is_some());
    assert!(artifact.legacy_assembly.is_some());
}

#[test]
fn can_compile_dapp_detect_changes_in_libs() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let remapping = project.paths().libraries[0].join("remapping");
    project
        .paths_mut()
        .remappings
        .push(Remapping::from_str(&format!("remapping/={}/", remapping.display())).unwrap());

    let src = project
        .add_source(
            "Foo",
            r#"
    pragma solidity ^0.8.10;
    import "remapping/Bar.sol";

    contract Foo {}
   "#,
        )
        .unwrap();

    let lib = project
        .add_lib(
            "remapping/Bar",
            r"
    pragma solidity ^0.8.10;

    contract Bar {}
    ",
        )
        .unwrap();

    let graph = Graph::<MultiCompilerParsedSource>::resolve(project.paths()).unwrap();
    assert_eq!(graph.files().len(), 2);
    assert_eq!(graph.files().clone(), HashMap::from([(src, 0), (lib, 1),]));

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Foo").is_some());
    assert!(compiled.find_first("Bar").is_some());
    compiled.assert_success();

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Foo").is_some());
    assert!(compiled.is_unchanged());

    let cache = CompilerCache::<SolcSettings>::read(&project.paths().cache).unwrap();
    assert_eq!(cache.files.len(), 2);

    // overwrite lib
    project
        .add_lib(
            "remapping/Bar",
            r"
    pragma solidity ^0.8.10;

    // changed lib
    contract Bar {}
    ",
        )
        .unwrap();

    let graph = Graph::<MultiCompilerParsedSource>::resolve(project.paths()).unwrap();
    assert_eq!(graph.files().len(), 2);

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Foo").is_some());
    assert!(compiled.find_first("Bar").is_some());
    // ensure change is detected
    assert!(!compiled.is_unchanged());
}

#[test]
fn can_compile_dapp_detect_changes_in_sources() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let src = project
        .add_source(
            "DssSpell.t",
            r#"
    pragma solidity ^0.8.10;
    import "./DssSpell.t.base.sol";

   contract DssSpellTest is DssSpellTestBase { }
   "#,
        )
        .unwrap();

    let base = project
        .add_source(
            "DssSpell.t.base",
            r"
    pragma solidity ^0.8.10;

  contract DssSpellTestBase {
       address deployed_spell;
       function setUp() public {
           deployed_spell = address(0xA867399B43aF7790aC800f2fF3Fa7387dc52Ec5E);
       }
  }
   ",
        )
        .unwrap();

    let graph = Graph::<MultiCompilerParsedSource>::resolve(project.paths()).unwrap();
    assert_eq!(graph.files().len(), 2);
    assert_eq!(graph.files().clone(), HashMap::from([(base, 0), (src, 1),]));
    assert_eq!(graph.imported_nodes(1).to_vec(), vec![0]);

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("DssSpellTest").is_some());
    assert!(compiled.find_first("DssSpellTestBase").is_some());

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.is_unchanged());
    assert!(compiled.find_first("DssSpellTest").is_some());
    assert!(compiled.find_first("DssSpellTestBase").is_some());

    let cache = CompilerCache::<SolcSettings>::read(&project.paths().cache).unwrap();
    assert_eq!(cache.files.len(), 2);

    let artifacts = compiled.into_artifacts().collect::<HashMap<_, _>>();

    // overwrite import
    let _ = project
        .add_source(
            "DssSpell.t.base",
            r"
    pragma solidity ^0.8.10;

  contract DssSpellTestBase {
       address deployed_spell;
       function setUp() public {
           deployed_spell = address(0);
       }
  }
   ",
        )
        .unwrap();
    let graph = Graph::<MultiCompilerParsedSource>::resolve(project.paths()).unwrap();
    assert_eq!(graph.files().len(), 2);

    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("DssSpellTest").is_some());
    assert!(compiled.find_first("DssSpellTestBase").is_some());
    // ensure change is detected
    assert!(!compiled.is_unchanged());

    // and all recompiled artifacts are different
    for (p, artifact) in compiled.into_artifacts() {
        let other = artifacts
            .iter()
            .find(|(id, _)| id.name == p.name && id.version == p.version && id.source == p.source)
            .unwrap()
            .1;
        assert_ne!(artifact, *other);
    }
}

#[test]
fn can_emit_build_info() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    project.project_mut().build_info = true;
    project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r"
pragma solidity ^0.8.10;
contract B { }
",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let info_dir = project.project().build_info_path();
    assert!(info_dir.exists());

    let mut build_info_count = 0;
    for entry in fs::read_dir(info_dir).unwrap() {
        let _info =
            BuildInfo::<SolcInput, CompilerOutput<Error, Contract>>::read(&entry.unwrap().path())
                .unwrap();
        build_info_count += 1;
    }
    assert_eq!(build_info_count, 1);
}

#[test]
fn can_clean_build_info() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project.project_mut().build_info = true;
    project.project_mut().paths.build_infos = project.project_mut().paths.root.join("build-info");
    project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r"
pragma solidity ^0.8.10;
contract B { }
",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let info_dir = project.project().build_info_path();
    assert!(info_dir.exists());

    let mut build_info_count = 0;
    for entry in fs::read_dir(info_dir).unwrap() {
        let _info =
            BuildInfo::<SolcInput, CompilerOutput<Error, Contract>>::read(&entry.unwrap().path())
                .unwrap();
        build_info_count += 1;
    }
    assert_eq!(build_info_count, 1);

    project.project().cleanup().unwrap();

    assert!(!project.project().build_info_path().exists());
}

#[test]
fn can_compile_dapp_sample_with_cache() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let root = tmp_dir.path();
    let cache = root.join("cache").join(SOLIDITY_FILES_CACHE_FILENAME);
    let artifacts = root.join("out");

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let orig_root = manifest_dir.join("../../test-data/dapp-sample");
    let cache_testdata_dir = manifest_dir.join("../../test-data/cache-sample/");
    copy_dir_all(&orig_root, tmp_dir.path()).unwrap();
    let paths = ProjectPathsConfig::builder()
        .cache(cache)
        .sources(root.join("src"))
        .artifacts(artifacts)
        .lib(root.join("lib"))
        .root(root)
        .build()
        .unwrap();

    // first compile
    let project = Project::builder().paths(paths).build(Default::default()).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    compiled.assert_success();

    // cache is used when nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.is_unchanged());

    // deleted artifacts cause recompile even with cache
    std::fs::remove_dir_all(project.artifacts_path()).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(!compiled.is_unchanged());

    // new file is compiled even with partial cache
    std::fs::copy(cache_testdata_dir.join("NewContract.sol"), root.join("src/NewContract.sol"))
        .unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_some());
    assert!(compiled.find_first("NewContract").is_some());
    assert!(!compiled.is_unchanged());
    assert_eq!(
        compiled.into_artifacts().map(|(artifact_id, _)| artifact_id.name).collect::<HashSet<_>>(),
        HashSet::from([
            "Dapp".to_string(),
            "DappTest".to_string(),
            "DSTest".to_string(),
            "NewContract".to_string(),
        ])
    );

    // old cached artifact is not taken from the cache
    std::fs::copy(cache_testdata_dir.join("Dapp.sol"), root.join("src/Dapp.sol")).unwrap();
    let compiled = project.compile().unwrap();
    assert_eq!(
        compiled.into_artifacts().map(|(artifact_id, _)| artifact_id.name).collect::<HashSet<_>>(),
        HashSet::from([
            "DappTest".to_string(),
            "NewContract".to_string(),
            "DSTest".to_string(),
            "Dapp".to_string(),
        ])
    );

    // deleted artifact is not taken from the cache
    std::fs::remove_file(project.paths.sources.join("Dapp.sol")).unwrap();
    let compiled: ProjectCompileOutput<_> = project.compile().unwrap();
    assert!(compiled.find_first("Dapp").is_none());
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

// Runs both `flatten` implementations, asserts that their outputs match and runs additional checks
// against the output.
fn test_flatteners(project: &TempProject, target: &Path, additional_checks: fn(&str)) {
    let target = canonicalize(target).unwrap();
    let result =
        project.project().paths.clone().with_language::<SolcLanguage>().flatten(&target).unwrap();
    let solc_result = Flattener::new(project.project().clone(), &target).unwrap().flatten();

    assert_eq!(result, solc_result);

    additional_checks(&result);
}

#[test]
fn can_flatten_file_with_external_lib() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/hardhat-sample");
    let paths = ProjectPathsConfig::builder()
        .sources(root.join("contracts"))
        .lib(root.join("node_modules"));
    let project = TempProject::<MultiCompiler>::new(paths).unwrap();

    let target = root.join("contracts").join("Greeter.sol");

    test_flatteners(&project, &target, |result| {
        assert!(!result.contains("import"));
        assert!(result.contains("library console"));
        assert!(result.contains("contract Greeter"));
    });
}

#[test]
fn can_flatten_file_in_dapp_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
    let paths = ProjectPathsConfig::builder().sources(root.join("src")).lib(root.join("lib"));
    let project = TempProject::<MultiCompiler>::new(paths).unwrap();

    let target = root.join("src/Dapp.t.sol");

    test_flatteners(&project, &target, |result| {
        assert!(!result.contains("import"));
        assert!(result.contains("contract DSTest"));
        assert!(result.contains("contract Dapp"));
        assert!(result.contains("contract DappTest"));
    });
}

#[test]
fn can_flatten_unique() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
import "./C.sol";
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"
pragma solidity ^0.8.10;
import "./C.sol";
contract B { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "C",
            r#"
pragma solidity ^0.8.10;
import "./A.sol";
contract C { }
"#,
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r#"pragma solidity ^0.8.10;

// src/B.sol

contract B { }

// src/C.sol

contract C { }

// src/A.sol

contract A { }
"#
        );
    });
}

#[test]
fn can_flatten_experimental_pragma() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
pragma experimental ABIEncoderV2;
import "./C.sol";
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"
pragma solidity ^0.8.10;
pragma experimental ABIEncoderV2;
import "./C.sol";
contract B { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "C",
            r#"
pragma solidity ^0.8.10;
pragma experimental ABIEncoderV2;
import "./A.sol";
contract C { }
"#,
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity ^0.8.10;
pragma experimental ABIEncoderV2;

// src/B.sol

contract B { }

// src/C.sol

contract C { }

// src/A.sol

contract A { }
"
        );
    });
}

#[test]
fn cannot_flatten_on_failure() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "Lib",
            r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.10;

library Lib {}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "Contract",
            r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.10;

import { Lib } from "./Lib.sol";

// Intentionally erroneous code
contract Contract {
    failure();
}
"#,
        )
        .unwrap();

    let result = project.paths().clone().with_language::<SolcLanguage>().flatten(target.as_path());
    assert!(result.is_err());
    println!("{}", result.unwrap_err());
}

#[test]
fn can_flatten_multiline() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
import "./C.sol";
import {
    IllegalArgument,
    IllegalState
} from "./Errors.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "Errors",
            r"
pragma solidity ^0.8.10;
error IllegalArgument();
error IllegalState();
",
        )
        .unwrap();

    project
        .add_source(
            "C",
            r"
pragma solidity ^0.8.10;
contract C { }
",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity ^0.8.10;

// src/C.sol

contract C { }

// src/Errors.sol

error IllegalArgument();
error IllegalState();

// src/A.sol

contract A { }
"
        );
    });
}

#[test]
fn can_flatten_remove_extra_spacing() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"pragma solidity ^0.8.10;
import "./C.sol";
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"// This is a B Contract
pragma solidity ^0.8.10;

import "./C.sol";

contract B { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "C",
            r"pragma solidity ^0.8.10;
contract C { }
",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity ^0.8.10;

// src/C.sol

contract C { }

// src/B.sol
// This is a B Contract

contract B { }

// src/A.sol

contract A { }
"
        );
    });
}

#[test]
fn can_flatten_with_alias() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "Contract",
            r#"pragma solidity ^0.8.10;

import { ParentContract as Parent } from "./Parent.sol";
import { AnotherParentContract as AnotherParent } from "./AnotherParent.sol";
import { PeerContract as Peer } from "./Peer.sol";
import { MathLibrary as Math } from "./Math.sol";
import * as Lib from "./SomeLib.sol";

contract Contract is Parent,
    AnotherParent {
    using Math for uint256;

    string public usingString = "using Math for uint256;";
    string public inheritanceString = "\"Contract is Parent {\"";
    string public castString = 'Peer(smth) ';
    string public methodString = '\' Math.max()';

    Peer public peer;

    constructor(address _peer) {
        peer = Peer(_peer);
        peer = new Peer();
        uint256 x = Math.minusOne(Math.max());
    }
}
"#,
        )
        .unwrap();

    project
        .add_source(
            "Parent",
            r"pragma solidity ^0.8.10;

contract ParentContract { }
",
        )
        .unwrap();

    project
        .add_source(
            "AnotherParent",
            r"pragma solidity ^0.8.10;

contract AnotherParentContract { }
",
        )
        .unwrap();

    project
        .add_source(
            "Peer",
            r"pragma solidity ^0.8.10;

contract PeerContract { }
",
        )
        .unwrap();

    project
        .add_source(
            "Math",
            r"pragma solidity ^0.8.10;

library MathLibrary {
    function minusOne(uint256 val) internal returns (uint256) {
        return val - 1;
    }

    function max() internal returns (uint256) {
        return type(uint256).max;
    }

    function diffMax(uint256 value) internal returns (uint256) {
        return type(uint256).max - value;
    }
}
",
        )
        .unwrap();

    project
        .add_source(
            "SomeLib",
            r"pragma solidity ^0.8.10;

library SomeLib { }
",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r#"pragma solidity ^0.8.10;

// src/AnotherParent.sol

contract AnotherParentContract { }

// src/Math.sol

library MathLibrary {
    function minusOne(uint256 val) internal returns (uint256) {
        return val - 1;
    }

    function max() internal returns (uint256) {
        return type(uint256).max;
    }

    function diffMax(uint256 value) internal returns (uint256) {
        return type(uint256).max - value;
    }
}

// src/Parent.sol

contract ParentContract { }

// src/Peer.sol

contract PeerContract { }

// src/SomeLib.sol

library SomeLib { }

// src/Contract.sol

contract Contract is ParentContract,
    AnotherParentContract {
    using MathLibrary for uint256;

    string public usingString = "using Math for uint256;";
    string public inheritanceString = "\"Contract is Parent {\"";
    string public castString = 'Peer(smth) ';
    string public methodString = '\' Math.max()';

    PeerContract public peer;

    constructor(address _peer) {
        peer = PeerContract(_peer);
        peer = new PeerContract();
        uint256 x = MathLibrary.minusOne(MathLibrary.max());
    }
}
"#
        );
    });
}

#[test]
fn can_flatten_with_version_pragma_after_imports() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;

import * as B from "./B.sol";

contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"
import {D} from "./D.sol";
pragma solidity ^0.8.10;
import * as C from "./C.sol";
contract B { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "C",
            r"
pragma solidity ^0.8.10;
contract C { }
",
        )
        .unwrap();

    project
        .add_source(
            "D",
            r"
pragma solidity ^0.8.10;
contract D { }
",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r#"pragma solidity ^0.8.10;

// src/C.sol

contract C { }

// src/D.sol

contract D { }

// src/B.sol

contract B { }

// src/A.sol

contract A { }
"#
        );
    });
}

#[test]
fn can_flatten_with_duplicates() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "Foo.sol",
            r#"
pragma solidity ^0.8.10;

contract Foo {
    function foo() public pure returns (uint256) {
        return 1;
    }
}

contract Bar is Foo {}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "Bar.sol",
            r#"
pragma solidity ^0.8.10;
import {Foo} from "./Foo.sol";

contract Bar is Foo {}
"#,
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/Foo.sol

contract Foo {
    function foo() public pure returns (uint256) {
        return 1;
    }
}

contract Bar_0 is Foo {}

// src/Bar.sol

contract Bar_1 is Foo {}
"
    );
}

#[test]
fn can_flatten_complex_aliases_setup_with_duplicates() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A.sol",
            r#"
pragma solidity ^0.8.10;

contract A {
    type SomeCustomValue is uint256;

    struct SomeStruct {
        uint256 field;
    }

    enum SomeEnum { VALUE1, VALUE2 }

    function foo() public pure returns (uint256) {
        return 1;
    }
}
"#,
        )
        .unwrap();

    project
        .add_source(
            "B.sol",
            r#"
pragma solidity ^0.8.10;
import "./A.sol" as A_File;

contract A is A_File.A {}
"#,
        )
        .unwrap();

    project
        .add_source(
            "C.sol",
            r#"
pragma solidity ^0.8.10;
import "./B.sol" as B_File;

contract A is B_File.A_File.A {}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "D.sol",
            r#"
pragma solidity ^0.8.10;
import "./C.sol" as C_File;

C_File.B_File.A_File.A.SomeCustomValue constant fileLevelValue = C_File.B_File.A_File.A.SomeCustomValue.wrap(1);

contract D is C_File.B_File.A_File.A {
    C_File.B_File.A_File.A.SomeStruct public someStruct;
    C_File.B_File.A_File.A.SomeEnum public someEnum = C_File.B_File.A_File.A.SomeEnum.VALUE1;

    constructor() C_File.B_File.A_File.A() {
        someStruct = C_File.B_File.A_File.A.SomeStruct(1);
        someEnum = C_File.B_File.A_File.A.SomeEnum.VALUE2;
    }

    function getSelector() public pure returns (bytes4) {
        return C_File.B_File.A_File.A.foo.selector;
    }

    function getEnumValue1() public pure returns (C_File.B_File.A_File.A.SomeEnum) {
        return C_File.B_File.A_File.A.SomeEnum.VALUE1;
    }

    function getStruct() public pure returns (C_File.B_File.A_File.A.SomeStruct memory) {
        return C_File.B_File.A_File.A.SomeStruct(1);
    }
}
"#,).unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/A.sol

contract A_0 {
    type SomeCustomValue is uint256;

    struct SomeStruct {
        uint256 field;
    }

    enum SomeEnum { VALUE1, VALUE2 }

    function foo() public pure returns (uint256) {
        return 1;
    }
}

// src/B.sol

contract A_1 is A_0 {}

// src/C.sol

contract A_2 is A_0 {}

// src/D.sol

A_0.SomeCustomValue constant fileLevelValue = A_0.SomeCustomValue.wrap(1);

contract D is A_0 {
    A_0.SomeStruct public someStruct;
    A_0.SomeEnum public someEnum = A_0.SomeEnum.VALUE1;

    constructor() A_0() {
        someStruct = A_0.SomeStruct(1);
        someEnum = A_0.SomeEnum.VALUE2;
    }

    function getSelector() public pure returns (bytes4) {
        return A_0.foo.selector;
    }

    function getEnumValue1() public pure returns (A_0.SomeEnum) {
        return A_0.SomeEnum.VALUE1;
    }

    function getStruct() public pure returns (A_0.SomeStruct memory) {
        return A_0.SomeStruct(1);
    }
}
"
    );
}

// https://github.com/foundry-rs/compilers/issues/34
#[test]
fn can_flatten_34_repro() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();
    let target = project
        .add_source(
            "FlieA.sol",
            r#"pragma solidity ^0.8.10;
import {B} from "./FileB.sol";

interface FooBar {
    function foo() external;
}
contract A {
    function execute() external {
        FooBar(address(0)).foo();
    }
}"#,
        )
        .unwrap();

    project
        .add_source(
            "FileB.sol",
            r#"pragma solidity ^0.8.10;

interface FooBar {
    function bar() external;
}
contract B {
    function execute() external {
        FooBar(address(0)).bar();
    }
}"#,
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r#"pragma solidity ^0.8.10;

// src/FileB.sol

interface FooBar_0 {
    function bar() external;
}
contract B {
    function execute() external {
        FooBar_0(address(0)).bar();
    }
}

// src/FlieA.sol

interface FooBar_1 {
    function foo() external;
}
contract A {
    function execute() external {
        FooBar_1(address(0)).foo();
    }
}
"#
    );
}

#[test]
fn can_flatten_experimental_in_other_file() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A.sol",
            r#"
pragma solidity 0.6.12;
pragma experimental ABIEncoderV2;

contract A {}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "B.sol",
            r#"
pragma solidity 0.6.12;

import "./A.sol";

contract B is A {}
"#,
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity =0.6.12;
pragma experimental ABIEncoderV2;

// src/A.sol

contract A {}

// src/B.sol

contract B is A {}
"
    );
}

#[test]
fn can_detect_type_error() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "Contract",
            r#"
    pragma solidity ^0.8.10;

   contract Contract {
        function xyz() public {
            require(address(0), "Error");
        }
   }
   "#,
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    assert!(compiled.has_compiler_errors());
}

#[test]
fn can_flatten_aliases_with_pragma_and_license_after_source() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A",
            r#"pragma solidity ^0.8.10;
contract A { }
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "B",
            r#"contract B is AContract {}
import {A as AContract} from "./A.sol";
pragma solidity ^0.8.10;"#,
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity ^0.8.10;

// src/A.sol

contract A { }

// src/B.sol
contract B is A {}
"
        );
    });
}

#[test]
fn can_flatten_rename_inheritdocs() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "DuplicateA",
            r#"pragma solidity ^0.8.10;
contract A {}
"#,
        )
        .unwrap();

    project
        .add_source(
            "A",
            r#"pragma solidity ^0.8.10;
import {A as OtherName} from "./DuplicateA.sol";

contract A {
    /// Documentation
    function foo() public virtual {}
}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "B",
            r#"pragma solidity ^0.8.10;
import {A} from "./A.sol";

contract B is A {
    /// @inheritdoc A
    function foo() public override {}
}"#,
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/DuplicateA.sol

contract A_0 {}

// src/A.sol

contract A_1 {
    /// Documentation
    function foo() public virtual {}
}

// src/B.sol

contract B is A_1 {
    /// @inheritdoc A_1
    function foo() public override {}
}
"
    );
}

#[test]
fn can_flatten_rename_inheritdocs_alias() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A",
            r#"pragma solidity ^0.8.10;

contract A {
    /// Documentation
    function foo() public virtual {}
}
"#,
        )
        .unwrap();

    let target = project
        .add_source(
            "B",
            r#"pragma solidity ^0.8.10;
import {A as Alias} from "./A.sol";

contract B is Alias {
    /// @inheritdoc Alias
    function foo() public override {}
}"#,
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/A.sol

contract A {
    /// Documentation
    function foo() public virtual {}
}

// src/B.sol

contract B is A {
    /// @inheritdoc A
    function foo() public override {}
}
"
    );
}

#[test]
fn can_flatten_rename_user_defined_functions() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "CustomUint",
            r"
pragma solidity ^0.8.10;

type CustomUint is uint256;

function mul(CustomUint a, CustomUint b) pure returns(CustomUint) {
    return CustomUint.wrap(CustomUint.unwrap(a) * CustomUint.unwrap(b));
}

using {mul} for CustomUint global;",
        )
        .unwrap();

    project
        .add_source(
            "CustomInt",
            r"pragma solidity ^0.8.10;

type CustomInt is int256;

function mul(CustomInt a, CustomInt b) pure returns(CustomInt) {
    return CustomInt.wrap(CustomInt.unwrap(a) * CustomInt.unwrap(b));
}

using {mul} for CustomInt global;",
        )
        .unwrap();

    let target = project
        .add_source(
            "Target",
            r"pragma solidity ^0.8.10;

import {CustomInt} from './CustomInt.sol';
import {CustomUint} from './CustomUint.sol';

contract Foo {
    function mul(CustomUint a, CustomUint b) public returns(CustomUint) {
        return a.mul(b);
    }

    function mul(CustomInt a, CustomInt b) public returns(CustomInt) {
        return a.mul(b);
    }
}",
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/CustomInt.sol

type CustomInt is int256;

function mul_0(CustomInt a, CustomInt b) pure returns(CustomInt) {
    return CustomInt.wrap(CustomInt.unwrap(a) * CustomInt.unwrap(b));
}

using {mul_0} for CustomInt global;

// src/CustomUint.sol

type CustomUint is uint256;

function mul_1(CustomUint a, CustomUint b) pure returns(CustomUint) {
    return CustomUint.wrap(CustomUint.unwrap(a) * CustomUint.unwrap(b));
}

using {mul_1} for CustomUint global;

// src/Target.sol

contract Foo {
    function mul(CustomUint a, CustomUint b) public returns(CustomUint) {
        return a.mul_1(b);
    }

    function mul(CustomInt a, CustomInt b) public returns(CustomInt) {
        return a.mul_0(b);
    }
}
"
    );
}

#[test]
fn can_flatten_rename_global_functions() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "func1",
            r"pragma solidity ^0.8.10;

function func() view {}",
        )
        .unwrap();

    project
        .add_source(
            "func2",
            r"pragma solidity ^0.8.10;

function func(uint256 x) view returns(uint256) {
    return x + 1;
}",
        )
        .unwrap();

    let target = project
        .add_source(
            "Target",
            r"pragma solidity ^0.8.10;

import {func as func1} from './func1.sol';
import {func as func2} from './func2.sol';

contract Foo {
    constructor(uint256 x) {
        func1();
        func2(x);
    }
}",
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/func1.sol

function func_0() view {}

// src/func2.sol

function func_1(uint256 x) view returns(uint256) {
    return x + 1;
}

// src/Target.sol

contract Foo {
    constructor(uint256 x) {
        func_0();
        func_1(x);
    }
}
"
    );
}

#[test]
fn can_flatten_rename_in_assembly() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A",
            r"pragma solidity ^0.8.10;

uint256 constant a = 1;",
        )
        .unwrap();

    project
        .add_source(
            "B",
            r"pragma solidity ^0.8.10;

uint256 constant a = 2;",
        )
        .unwrap();

    let target = project
        .add_source(
            "Target",
            r"pragma solidity ^0.8.10;

import {a as a1} from './A.sol';
import {a as a2} from './B.sol';

contract Foo {
    function test() public returns(uint256 x) {
        assembly {
            x := mul(a1, a2)
        }
    }
}",
        )
        .unwrap();

    let result = Flattener::new(project.project().clone(), &target).unwrap().flatten();
    assert_eq!(
        result,
        r"pragma solidity ^0.8.10;

// src/A.sol

uint256 constant a_0 = 1;

// src/B.sol

uint256 constant a_1 = 2;

// src/Target.sol

contract Foo {
    function test() public returns(uint256 x) {
        assembly {
            x := mul(a_0, a_1)
        }
    }
}
"
    );
}

#[test]
fn can_flatten_combine_pragmas() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A",
            r"pragma solidity >=0.5.0;

contract A {}",
        )
        .unwrap();

    let target = project
        .add_source(
            "B",
            r"pragma solidity <0.9.0;
import './A.sol';

contract B {}",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity <0.9.0 >=0.5.0;

// src/A.sol

contract A {}

// src/B.sol

contract B {}
"
        );
    });
}

#[test]
fn can_flatten_with_assembly_reference_suffix() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r"pragma solidity >=0.5.0;

contract A {
    uint256 val;

    function useSuffix() public {
        bytes32 slot;
        assembly {
            slot := val.slot
        }
    }
}",
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity >=0.5.0;

// src/A.sol

contract A {
    uint256 val;

    function useSuffix() public {
        bytes32 slot;
        assembly {
            slot := val.slot
        }
    }
}
"
        );
    });
}

#[test]
fn can_compile_single_files() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    let f = tmp
        .add_contract(
            "examples/Foo",
            r"
    pragma solidity ^0.8.10;

    contract Foo {}
   ",
        )
        .unwrap();

    let compiled = tmp.project().compile_file(f.clone()).unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Foo").is_some());

    let bar = tmp
        .add_contract(
            "examples/Bar",
            r"
    pragma solidity ^0.8.10;

    contract Bar {}
   ",
        )
        .unwrap();

    let compiled = tmp.project().compile_files(vec![f, bar]).unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Foo").is_some());
    assert!(compiled.find_first("Bar").is_some());
}

#[test]
fn consistent_bytecode() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    tmp.add_source(
        "LinkTest",
        r"
// SPDX-License-Identifier: MIT
library LibTest {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}
contract LinkTest {
    function foo() public returns (uint256) {
        return LibTest.foobar(1);
    }
}
",
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(bytecode.is_unlinked());
    let s = bytecode.as_str().unwrap();
    assert!(!s.starts_with("0x"));

    let s = serde_json::to_string(&bytecode).unwrap();
    assert_eq!(bytecode.clone(), serde_json::from_str(&s).unwrap());
}

#[test]
fn can_apply_libraries() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    tmp.add_source(
        "LinkTest",
        r#"
// SPDX-License-Identifier: MIT
import "./MyLib.sol";
contract LinkTest {
    function foo() public returns (uint256) {
        return MyLib.foobar(1);
    }
}
"#,
    )
    .unwrap();

    let lib = tmp
        .add_source(
            "MyLib",
            r"
// SPDX-License-Identifier: MIT
library MyLib {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}
",
        )
        .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    assert!(compiled.find_first("MyLib").is_some());
    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(bytecode.is_unlinked());

    // provide the library settings to let solc link
    tmp.project_mut().settings.solc.libraries = BTreeMap::from([(
        lib,
        BTreeMap::from([("MyLib".to_string(), format!("{:?}", Address::ZERO))]),
    )])
    .into();
    tmp.project_mut().settings.solc.libraries.slash_paths();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    assert!(compiled.find_first("MyLib").is_some());
    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(!bytecode.is_unlinked());

    let libs = Libraries::parse(&[format!("./src/MyLib.sol:MyLib:{:?}", Address::ZERO)]).unwrap();
    // provide the library settings to let solc link
    tmp.project_mut().settings.solc.libraries =
        libs.apply(|libs| tmp.paths().apply_lib_remappings(libs));

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    assert!(compiled.find_first("MyLib").is_some());
    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(!bytecode.is_unlinked());
}

#[test]
fn can_ignore_warning_from_paths() {
    let setup_and_compile = |ignore_paths: Option<Vec<PathBuf>>| {
        let tmp = match ignore_paths {
            Some(paths) => TempProject::dapptools_with_ignore_paths(paths).unwrap(),
            None => TempProject::<MultiCompiler>::dapptools().unwrap(),
        };

        tmp.add_source(
            "LinkTest",
            r#"
                // SPDX-License-Identifier: MIT
                import "./MyLib.sol";
                contract LinkTest {
                    function foo() public returns (uint256) {
                    }
                }
            "#,
        )
        .unwrap();

        tmp.add_source(
            "MyLib",
            r"
                // SPDX-License-Identifier: MIT
                library MyLib {
                    function foobar(uint256 a) public view returns (uint256) {
                        return a * 100;
                    }
                }
            ",
        )
        .unwrap();

        tmp.compile().unwrap()
    };

    // Test without ignoring paths
    let compiled_without_ignore = setup_and_compile(None);
    compiled_without_ignore.assert_success();
    assert!(compiled_without_ignore.has_compiler_warnings());

    // Test with ignoring paths
    let paths_to_ignore = vec![Path::new("src").to_path_buf()];
    let compiled_with_ignore = setup_and_compile(Some(paths_to_ignore));
    compiled_with_ignore.assert_success();
    assert!(!compiled_with_ignore.has_compiler_warnings());
}
#[test]
fn can_apply_libraries_with_remappings() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    let remapping = tmp.paths().libraries[0].join("remapping");
    tmp.paths_mut()
        .remappings
        .push(Remapping::from_str(&format!("remapping/={}/", remapping.display())).unwrap());

    tmp.add_source(
        "LinkTest",
        r#"
// SPDX-License-Identifier: MIT
import "remapping/MyLib.sol";
contract LinkTest {
    function foo() public returns (uint256) {
        return MyLib.foobar(1);
    }
}
"#,
    )
    .unwrap();

    tmp.add_lib(
        "remapping/MyLib",
        r"
// SPDX-License-Identifier: MIT
library MyLib {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}
",
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    assert!(compiled.find_first("MyLib").is_some());
    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(bytecode.is_unlinked());

    let libs =
        Libraries::parse(&[format!("remapping/MyLib.sol:MyLib:{:?}", Address::ZERO)]).unwrap(); // provide the library settings to let solc link
    tmp.project_mut().settings.solc.libraries =
        libs.apply(|libs| tmp.paths().apply_lib_remappings(libs));
    tmp.project_mut().settings.solc.libraries.slash_paths();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    assert!(compiled.find_first("MyLib").is_some());
    let contract = compiled.find_first("LinkTest").unwrap();
    let bytecode = &contract.bytecode.as_ref().unwrap().object;
    assert!(!bytecode.is_unlinked());
}

#[test]
fn can_detect_invalid_version() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();
    let content = r"
    pragma solidity ^0.100.10;
    contract A {}
   ";
    tmp.add_source("A", content).unwrap();

    let out = tmp.compile().unwrap_err();
    match out {
        SolcError::Message(err) => {
            assert_eq!(err, format!("Encountered invalid solc version in src{MAIN_SEPARATOR}A.sol: No solc version exists that matches the version requirement: ^0.100.10"));
        }
        _ => {
            unreachable!()
        }
    }
}

#[test]
fn test_severity_warnings() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();
    // also treat warnings as error
    tmp.project_mut().compiler_severity_filter = Severity::Warning;

    let content = r"
    pragma solidity =0.8.13;
    contract A {}
   ";
    tmp.add_source("A", content).unwrap();

    let out = tmp.compile().unwrap();
    assert!(out.output().has_error(&[], &[], &Severity::Warning));

    let content = r"
    // SPDX-License-Identifier: MIT OR Apache-2.0
    pragma solidity =0.8.13;
    contract A {}
   ";
    tmp.add_source("A", content).unwrap();

    let out = tmp.compile().unwrap();
    assert!(!out.output().has_error(&[], &[], &Severity::Warning));

    let content = r"
    // SPDX-License-Identifier: MIT OR Apache-2.0
    pragma solidity =0.8.13;
    contract A {
      function id(uint111 value) external pure returns (uint256) {
        return 0;
      }
    }
   ";
    tmp.add_source("A", content).unwrap();

    let out = tmp.compile().unwrap();
    assert!(out.output().has_error(&[], &[], &Severity::Warning));
}

#[test]
fn can_recompile_with_changes() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();
    tmp.project_mut().paths.allowed_paths = BTreeSet::from([tmp.root().join("modules")]);

    let content = r#"
    pragma solidity ^0.8.10;
    import "../modules/B.sol";
    contract A {}
   "#;
    tmp.add_source("A", content).unwrap();

    tmp.add_contract(
        "modules/B",
        r"
    pragma solidity ^0.8.10;
    contract B {}
   ",
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("B").is_some());

    let compiled = tmp.compile().unwrap();
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("B").is_some());
    assert!(compiled.is_unchanged());

    // modify A.sol
    tmp.add_source("A", format!("{content}\n")).unwrap();
    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("B").is_some());
}

#[test]
fn can_recompile_with_lowercase_names() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    tmp.add_source(
        "deployProxy.sol",
        r"
    pragma solidity =0.8.12;
    contract DeployProxy {}
   ",
    )
    .unwrap();

    let upgrade = r#"
    pragma solidity =0.8.12;
    import "./deployProxy.sol";
    import "./ProxyAdmin.sol";
    contract UpgradeProxy {}
   "#;
    tmp.add_source("upgradeProxy.sol", upgrade).unwrap();

    tmp.add_source(
        "ProxyAdmin.sol",
        r"
    pragma solidity =0.8.12;
    contract ProxyAdmin {}
   ",
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("DeployProxy").is_some());
    assert!(compiled.find_first("UpgradeProxy").is_some());
    assert!(compiled.find_first("ProxyAdmin").is_some());

    let artifacts = tmp.artifacts_snapshot().unwrap();
    assert_eq!(artifacts.artifacts.as_ref().len(), 3);
    artifacts.assert_artifacts_essentials_present();

    let compiled = tmp.compile().unwrap();
    assert!(compiled.find_first("DeployProxy").is_some());
    assert!(compiled.find_first("UpgradeProxy").is_some());
    assert!(compiled.find_first("ProxyAdmin").is_some());
    assert!(compiled.is_unchanged());

    // modify upgradeProxy.sol
    tmp.add_source("upgradeProxy.sol", format!("{upgrade}\n")).unwrap();
    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert!(compiled.find_first("DeployProxy").is_some());
    assert!(compiled.find_first("UpgradeProxy").is_some());
    assert!(compiled.find_first("ProxyAdmin").is_some());

    let artifacts = tmp.artifacts_snapshot().unwrap();
    assert_eq!(artifacts.artifacts.as_ref().len(), 3);
    artifacts.assert_artifacts_essentials_present();
}

#[test]
fn can_recompile_unchanged_with_empty_files() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    tmp.add_source(
        "A",
        r#"
    pragma solidity ^0.8.10;
    import "./B.sol";
    contract A {}
   "#,
    )
    .unwrap();

    tmp.add_source(
        "B",
        r#"
    pragma solidity ^0.8.10;
    import "./C.sol";
   "#,
    )
    .unwrap();

    let c = r"
    pragma solidity ^0.8.10;
    contract C {}
   ";
    tmp.add_source("C", c).unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("C").is_some());

    let compiled = tmp.compile().unwrap();
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("C").is_some());
    assert!(compiled.is_unchanged());

    // modify C.sol
    tmp.add_source("C", format!("{c}\n")).unwrap();
    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert!(compiled.find_first("A").is_some());
    assert!(compiled.find_first("C").is_some());
}

#[test]
fn can_emit_empty_artifacts() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    let top_level = tmp
        .add_source(
            "top_level",
            r"
    function test() {}
   ",
        )
        .unwrap();

    tmp.add_source(
        "Contract",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;

import "./top_level.sol";

contract Contract {
    function a() public{
        test();
    }
}
   "#,
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Contract").is_some());
    assert!(compiled.find_first("top_level").is_some());
    let mut artifacts = tmp.artifacts_snapshot().unwrap();

    assert_eq!(artifacts.artifacts.as_ref().len(), 2);

    let mut top_level = artifacts.artifacts.as_mut().remove(&top_level).unwrap();

    assert_eq!(top_level.len(), 1);

    let artifact = top_level.remove("top_level").unwrap().remove(0);
    assert!(artifact.artifact.ast.is_some());

    // recompile
    let compiled = tmp.compile().unwrap();
    assert!(compiled.is_unchanged());

    // modify standalone file

    tmp.add_source(
        "top_level",
        r"
    error MyError();
    function test() {}
   ",
    )
    .unwrap();
    let compiled = tmp.compile().unwrap();
    assert!(!compiled.is_unchanged());
}

#[test]
fn can_detect_contract_def_source_files() {
    let tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    let mylib = tmp
        .add_source(
            "MyLib",
            r"
        pragma solidity 0.8.10;
        library MyLib {
        }
   ",
        )
        .unwrap();

    let myinterface = tmp
        .add_source(
            "MyInterface",
            r"
        pragma solidity 0.8.10;
        interface MyInterface {}
   ",
        )
        .unwrap();

    let mycontract = tmp
        .add_source(
            "MyContract",
            r"
        pragma solidity 0.8.10;
        contract MyContract {}
   ",
        )
        .unwrap();

    let myabstract_contract = tmp
        .add_source(
            "MyAbstractContract",
            r"
        pragma solidity 0.8.10;
        contract MyAbstractContract {}
   ",
        )
        .unwrap();

    let myerr = tmp
        .add_source(
            "MyError",
            r"
        pragma solidity 0.8.10;
       error MyError();
   ",
        )
        .unwrap();

    let myfunc = tmp
        .add_source(
            "MyFunction",
            r"
        pragma solidity 0.8.10;
        function abc(){}
   ",
        )
        .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();

    let mut sources = compiled.into_output().sources;
    let myfunc = sources.remove_by_path(&myfunc).unwrap();
    assert!(!myfunc.contains_contract_definition());

    let myerr = sources.remove_by_path(&myerr).unwrap();
    assert!(!myerr.contains_contract_definition());

    let mylib = sources.remove_by_path(&mylib).unwrap();
    assert!(mylib.contains_contract_definition());

    let myabstract_contract = sources.remove_by_path(&myabstract_contract).unwrap();
    assert!(myabstract_contract.contains_contract_definition());

    let myinterface = sources.remove_by_path(&myinterface).unwrap();
    assert!(myinterface.contains_contract_definition());

    let mycontract = sources.remove_by_path(&mycontract).unwrap();
    assert!(mycontract.contains_contract_definition());
}

#[test]
fn can_compile_sparse_with_link_references() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();

    tmp.add_source(
        "ATest.t.sol",
        r#"
    pragma solidity =0.8.12;
    import {MyLib} from "./mylib.sol";
    contract ATest {
      function test_mylib() public returns (uint256) {
         return MyLib.doStuff();
      }
    }
   "#,
    )
    .unwrap();

    let my_lib_path = tmp
        .add_source(
            "mylib.sol",
            r"
    pragma solidity =0.8.12;
    library MyLib {
       function doStuff() external pure returns (uint256) {return 1337;}
    }
   ",
        )
        .unwrap();

    tmp.project_mut().sparse_output = Some(Box::<TestFileFilter>::default());
    let mut compiled = tmp.compile().unwrap();
    compiled.assert_success();

    let mut output = compiled.clone().into_output();

    assert!(compiled.find_first("ATest").is_some());
    assert!(compiled.find_first("MyLib").is_some());
    let lib = compiled.remove_first("MyLib").unwrap();
    assert!(lib.bytecode.is_some());
    let lib = compiled.remove_first("MyLib");
    assert!(lib.is_none());

    let mut dup = output.clone();
    let lib = dup.remove_first("MyLib");
    assert!(lib.is_some());
    let lib = dup.remove_first("MyLib");
    assert!(lib.is_none());

    dup = output.clone();
    let lib = dup.remove(&my_lib_path, "MyLib");
    assert!(lib.is_some());
    let lib = dup.remove(&my_lib_path, "MyLib");
    assert!(lib.is_none());

    #[cfg(not(windows))]
    let info = ContractInfo::new(&format!("{}:{}", my_lib_path.display(), "MyLib"));
    #[cfg(windows)]
    let info = {
        use path_slash::PathBufExt;
        ContractInfo {
            path: Some(my_lib_path.to_slash_lossy().to_string()),
            name: "MyLib".to_string(),
        }
    };
    let lib = output.remove_contract(&info);
    assert!(lib.is_some());
    let lib = output.remove_contract(&info);
    assert!(lib.is_none());
}

#[test]
fn can_sanitize_bytecode_hash() {
    let mut tmp = TempProject::<MultiCompiler>::dapptools().unwrap();
    tmp.project_mut().settings.solc.metadata = Some(BytecodeHash::Ipfs.into());

    tmp.add_source(
        "A",
        r"
    pragma solidity =0.5.17;
    contract A {}
   ",
    )
    .unwrap();

    let compiled = tmp.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("A").is_some());
}

// https://github.com/foundry-rs/foundry/issues/5307
#[test]
fn can_create_standard_json_input_with_external_file() {
    // File structure:
    // .
    // ├── verif
    // │   └── src
    // │       └── Counter.sol
    // └── remapped
    //     ├── Child.sol
    //     └── Parent.sol

    let dir = tempfile::tempdir().unwrap();
    let verif_dir = utils::canonicalize(dir.path()).unwrap().join("verif");
    let remapped_dir = utils::canonicalize(dir.path()).unwrap().join("remapped");
    fs::create_dir_all(verif_dir.join("src")).unwrap();
    fs::create_dir(&remapped_dir).unwrap();

    let mut verif_project = ProjectBuilder::<SolcCompiler>::new(Default::default())
        .paths(ProjectPathsConfig::dapptools(&verif_dir).unwrap())
        .build(Default::default())
        .unwrap();

    verif_project.paths.remappings.push(Remapping {
        context: None,
        name: "@remapped/".into(),
        path: "../remapped/".into(),
    });
    verif_project.paths.allowed_paths.insert(remapped_dir.clone());

    fs::write(remapped_dir.join("Parent.sol"), "pragma solidity >=0.8.0; import './Child.sol';")
        .unwrap();
    fs::write(remapped_dir.join("Child.sol"), "pragma solidity >=0.8.0;").unwrap();
    fs::write(
        verif_dir.join("src/Counter.sol"),
        "pragma solidity >=0.8.0; import '@remapped/Parent.sol'; contract Counter {}",
    )
    .unwrap();

    // solc compiles using the host file system; therefore, this setup is considered valid
    let compiled = verif_project.compile().unwrap();
    compiled.assert_success();

    // can create project root based paths
    let std_json = verif_project.standard_json_input(&verif_dir.join("src/Counter.sol")).unwrap();
    assert_eq!(
        std_json.sources.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        vec![
            PathBuf::from("src/Counter.sol"),
            PathBuf::from("../remapped/Parent.sol"),
            PathBuf::from("../remapped/Child.sol")
        ]
    );

    let solc = Solc::find_or_install(&Version::new(0, 8, 24)).unwrap();

    // can compile using the created json
    let compiler_errors = solc
        .compile(&std_json)
        .unwrap()
        .errors
        .into_iter()
        .filter_map(|e| if e.severity.is_error() { Some(e.message) } else { None })
        .collect::<Vec<_>>();
    assert!(compiler_errors.is_empty(), "{compiler_errors:?}");
}

#[test]
fn can_compile_std_json_input() {
    let tmp = TempProject::<MultiCompiler>::dapptools_init().unwrap();
    tmp.assert_no_errors();
    let source = tmp.list_source_files().into_iter().find(|p| p.ends_with("Dapp.t.sol")).unwrap();
    let input = tmp.project().standard_json_input(&source).unwrap();

    assert!(input.settings.remappings.contains(&"ds-test/=lib/ds-test/src/".parse().unwrap()));
    let input: SolcInput = input.into();
    assert!(input.sources.contains_key(Path::new("lib/ds-test/src/test.sol")));

    // should be installed
    if let Ok(solc) = Solc::find_or_install(&Version::new(0, 8, 24)) {
        let out = solc.compile(&input).unwrap();
        assert!(out.errors.is_empty());
        assert!(out.sources.contains_key(Path::new("lib/ds-test/src/test.sol")));
    }
}

// This test is exclusive to unix because creating a symlink is a privileged action on windows.
// https://doc.rust-lang.org/std/os/windows/fs/fn.symlink_dir.html#limitations
#[test]
#[cfg(unix)]
fn can_create_standard_json_input_with_symlink() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    let dependency = TempProject::<MultiCompiler>::dapptools().unwrap();

    // File structure:
    //
    // project
    // ├── node_modules
    // │   └── dependency -> symlink to actual 'dependency' directory
    // └── src
    //     └── A.sol
    //
    // dependency
    // └── src
    //     ├── B.sol
    //     └── C.sol

    fs::create_dir(project.root().join("node_modules")).unwrap();

    std::os::unix::fs::symlink(dependency.root(), project.root().join("node_modules/dependency"))
        .unwrap();
    project.project_mut().paths.remappings.push(Remapping {
        context: None,
        name: "@dependency/".into(),
        path: "node_modules/dependency/".into(),
    });

    project
        .add_source(
            "A",
            r"pragma solidity >=0.8.0; import '@dependency/src/B.sol'; contract A is B {}",
        )
        .unwrap();
    dependency
        .add_source("B", r"pragma solidity >=0.8.0; import './C.sol'; contract B is C {}")
        .unwrap();
    dependency.add_source("C", r"pragma solidity >=0.8.0; contract C {}").unwrap();

    // solc compiles using the host file system; therefore, this setup is considered valid
    project.assert_no_errors();

    // can create project root based paths
    let std_json =
        project.project().standard_json_input(&project.sources_path().join("A.sol")).unwrap();
    assert_eq!(
        std_json.sources.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        vec![
            PathBuf::from("src/A.sol"),
            PathBuf::from("node_modules/dependency/src/B.sol"),
            PathBuf::from("node_modules/dependency/src/C.sol")
        ]
    );

    let solc = Solc::find_or_install(&Version::new(0, 8, 24)).unwrap();

    // can compile using the created json
    let compiler_errors = solc
        .compile(&std_json)
        .unwrap()
        .errors
        .into_iter()
        .filter_map(|e| if e.severity.is_error() { Some(e.message) } else { None })
        .collect::<Vec<_>>();
    assert!(compiler_errors.is_empty(), "{compiler_errors:?}");
}

#[test]
fn can_compile_model_checker_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/model-checker-sample");
    let paths = ProjectPathsConfig::builder().sources(root);

    let mut project = TempProject::<MultiCompiler, ConfigurableArtifacts>::new(paths).unwrap();
    project.project_mut().settings.solc.settings.model_checker = Some(ModelCheckerSettings {
        engine: Some(CHC),
        timeout: Some(10000),
        ..Default::default()
    });
    let compiled = project.compile().unwrap();

    assert!(compiled.find_first("Assert").is_some());
    compiled.assert_success();
    assert!(compiled.has_compiler_warnings());
}

#[test]
fn test_compiler_severity_filter() {
    fn gen_test_data_warning_path() -> ProjectPathsConfig {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/test-contract-warnings");

        ProjectPathsConfig::builder().sources(root).build().unwrap()
    }

    let project = Project::builder()
        .no_artifacts()
        .paths(gen_test_data_warning_path())
        .ephemeral()
        .build(Default::default())
        .unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.has_compiler_warnings());
    compiled.assert_success();

    let project = Project::builder()
        .no_artifacts()
        .paths(gen_test_data_warning_path())
        .ephemeral()
        .set_compiler_severity_filter(foundry_compilers_artifacts::Severity::Warning)
        .build(Default::default())
        .unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.has_compiler_warnings());
    assert!(compiled.has_compiler_errors());
}

fn gen_test_data_licensing_warning() -> ProjectPathsConfig {
    let root = canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-data/test-contract-warnings/LicenseWarning.sol"),
    )
    .unwrap();

    ProjectPathsConfig::builder().sources(root).build().unwrap()
}

fn compile_project_with_options(
    severity_filter: Option<foundry_compilers_artifacts::Severity>,
    ignore_paths: Option<Vec<PathBuf>>,
    ignore_error_code: Option<u64>,
) -> ProjectCompileOutput<MultiCompiler> {
    let mut builder =
        Project::builder().no_artifacts().paths(gen_test_data_licensing_warning()).ephemeral();

    if let Some(paths) = ignore_paths {
        builder = builder.ignore_paths(paths);
    }
    if let Some(code) = ignore_error_code {
        builder = builder.ignore_error_code(code);
    }
    if let Some(severity) = severity_filter {
        builder = builder.set_compiler_severity_filter(severity);
    }

    let project = builder.build(Default::default()).unwrap();
    project.compile().unwrap()
}

#[test]
fn test_compiler_ignored_file_paths() {
    let compiled = compile_project_with_options(None, None, None);
    // no ignored paths set, so the warning should be present
    assert!(compiled.has_compiler_warnings());
    compiled.assert_success();

    let testdata =
        canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data")).unwrap();
    let compiled = compile_project_with_options(
        Some(foundry_compilers_artifacts::Severity::Warning),
        Some(vec![testdata]),
        None,
    );

    // ignored paths set, so the warning shouldnt be present
    assert!(!compiled.has_compiler_warnings());
    compiled.assert_success();
}

#[test]
fn test_compiler_severity_filter_and_ignored_error_codes() {
    let missing_license_error_code = 1878;

    let compiled = compile_project_with_options(None, None, None);
    assert!(compiled.has_compiler_warnings());

    let compiled = compile_project_with_options(None, None, Some(missing_license_error_code));
    assert!(!compiled.has_compiler_warnings());
    compiled.assert_success();

    let compiled = compile_project_with_options(
        Some(foundry_compilers_artifacts::Severity::Warning),
        None,
        Some(missing_license_error_code),
    );
    assert!(!compiled.has_compiler_warnings());
    compiled.assert_success();
}

fn remove_solc_if_exists(version: &Version) {
    if Solc::find_svm_installed_version(version).unwrap().is_some() {
        svm::remove_version(version).expect("failed to remove version")
    }
}

#[test]
fn can_install_solc_and_compile_version() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();
    let version = Version::new(0, 8, 10);

    project
        .add_source(
            "Contract",
            format!(
                r#"
pragma solidity {version};
contract Contract {{ }}
"#
            ),
        )
        .unwrap();

    remove_solc_if_exists(&version);

    let compiled = project.compile().unwrap();
    compiled.assert_success();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_install_solc_and_compile_std_json_input_async() {
    let tmp = TempProject::<MultiCompiler>::dapptools_init().unwrap();
    tmp.assert_no_errors();
    let source = tmp.list_source_files().into_iter().find(|p| p.ends_with("Dapp.t.sol")).unwrap();
    let input = tmp.project().standard_json_input(&source).unwrap();
    let solc = Solc::find_or_install(&Version::new(0, 8, 24)).unwrap();

    assert!(input.settings.remappings.contains(&"ds-test/=lib/ds-test/src/".parse().unwrap()));
    let input: SolcInput = input.into();
    assert!(input.sources.contains_key(Path::new("lib/ds-test/src/test.sol")));

    let out = solc.async_compile(&input).await.unwrap();
    assert!(!out.has_error());
    assert!(out.sources.contains_key(&PathBuf::from("lib/ds-test/src/test.sol")));
}

#[test]
fn can_purge_obsolete_artifacts() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    project.set_solc("0.8.10");
    project
        .add_source(
            "Contract",
            r"
    pragma solidity >=0.8.10;

   contract Contract {
        function xyz() public {
        }
   }
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert_eq!(compiled.into_artifacts().count(), 1);

    project.set_solc("0.8.13");

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert_eq!(compiled.into_artifacts().count(), 1);
}

#[test]
fn can_parse_notice() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    project.project_mut().artifacts.additional_values.userdoc = true;
    project.project_mut().settings.solc.settings = project.project_mut().artifacts.solc_settings();

    let contract = r"
    pragma solidity $VERSION;

   contract Contract {
      string greeting;

        /**
         * @notice hello
         */
         constructor(string memory _greeting) public {
            greeting = _greeting;
        }

        /**
         * @notice hello
         */
        function xyz() public {
        }

        /// @notice hello
        function abc() public {
        }
   }
   ";
    project.add_source("Contract", contract.replace("$VERSION", "=0.5.17")).unwrap();

    let mut compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert!(compiled.find_first("Contract").is_some());
    let userdoc = compiled.remove_first("Contract").unwrap().userdoc;

    assert_eq!(
        userdoc,
        Some(UserDoc {
            version: None,
            kind: None,
            methods: BTreeMap::from([
                ("abc()".to_string(), UserDocNotice::Notice { notice: "hello".to_string() }),
                ("xyz()".to_string(), UserDocNotice::Notice { notice: "hello".to_string() }),
                ("constructor".to_string(), UserDocNotice::Constructor("hello".to_string())),
            ]),
            events: BTreeMap::new(),
            errors: BTreeMap::new(),
            notice: None
        })
    );

    project.add_source("Contract", contract.replace("$VERSION", "^0.8.10")).unwrap();

    let mut compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    assert!(compiled.find_first("Contract").is_some());
    let userdoc = compiled.remove_first("Contract").unwrap().userdoc;

    assert_eq!(
        userdoc,
        Some(UserDoc {
            version: Some(1),
            kind: Some("user".to_string()),
            methods: BTreeMap::from([
                ("abc()".to_string(), UserDocNotice::Notice { notice: "hello".to_string() }),
                ("xyz()".to_string(), UserDocNotice::Notice { notice: "hello".to_string() }),
                ("constructor".to_string(), UserDocNotice::Notice { notice: "hello".to_string() }),
            ]),
            events: BTreeMap::new(),
            errors: BTreeMap::new(),
            notice: None
        })
    );
}

#[test]
fn can_parse_doc() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    project.project_mut().artifacts.additional_values.userdoc = true;
    project.project_mut().artifacts.additional_values.devdoc = true;
    project.project_mut().settings.solc.settings = project.project_mut().artifacts.solc_settings();

    let contract = r"
// SPDX-License-Identifier: GPL-3.0-only
pragma solidity 0.8.17;

/// @title Not an ERC20.
/// @author Notadev
/// @notice Do not use this.
/// @dev This is not an ERC20 implementation.
/// @custom:experimental This is an experimental contract.
interface INotERC20 {
    /// @notice Transfer tokens.
    /// @dev Transfer `amount` tokens to account `to`.
    /// @param to Target account.
    /// @param amount Transfer amount.
    /// @return A boolean value indicating whether the operation succeeded.
    function transfer(address to, uint256 amount) external returns (bool);

    /// @notice Transfer some tokens.
    /// @dev Emitted when transfer.
    /// @param from Source account.
    /// @param to Target account.
    /// @param value Transfer amount.
    event Transfer(address indexed from, address indexed to, uint256 value);

    /// @notice Insufficient balance for transfer.
    /// @dev Needed `required` but only `available` available.
    /// @param available Balance available.
    /// @param required Requested amount to transfer.
    error InsufficientBalance(uint256 available, uint256 required);
}

contract NotERC20 is INotERC20 {
    /// @inheritdoc INotERC20
    function transfer(address to, uint256 amount) external returns (bool) {
        return false;
    }
}
    ";
    project.add_source("Contract", contract).unwrap();

    let mut compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());

    assert!(compiled.find_first("INotERC20").is_some());
    let contract = compiled.remove_first("INotERC20").unwrap();
    assert_eq!(
        contract.userdoc,
        Some(UserDoc {
            version: Some(1),
            kind: Some("user".to_string()),
            notice: Some("Do not use this.".to_string()),
            methods: BTreeMap::from([(
                "transfer(address,uint256)".to_string(),
                UserDocNotice::Notice { notice: "Transfer tokens.".to_string() }
            ),]),
            events: BTreeMap::from([(
                "Transfer(address,address,uint256)".to_string(),
                UserDocNotice::Notice { notice: "Transfer some tokens.".to_string() }
            ),]),
            errors: BTreeMap::from([(
                "InsufficientBalance(uint256,uint256)".to_string(),
                vec![UserDocNotice::Notice {
                    notice: "Insufficient balance for transfer.".to_string()
                }]
            ),]),
        })
    );
    assert_eq!(
        contract.devdoc,
        Some(DevDoc {
            version: Some(1),
            kind: Some("dev".to_string()),
            author: Some("Notadev".to_string()),
            details: Some("This is not an ERC20 implementation.".to_string()),
            custom_experimental: Some("This is an experimental contract.".to_string()),
            methods: BTreeMap::from([(
                "transfer(address,uint256)".to_string(),
                MethodDoc {
                    details: Some("Transfer `amount` tokens to account `to`.".to_string()),
                    params: BTreeMap::from([
                        ("to".to_string(), "Target account.".to_string()),
                        ("amount".to_string(), "Transfer amount.".to_string())
                    ]),
                    returns: BTreeMap::from([(
                        "_0".to_string(),
                        "A boolean value indicating whether the operation succeeded.".to_string()
                    ),])
                }
            ),]),
            events: BTreeMap::from([(
                "Transfer(address,address,uint256)".to_string(),
                EventDoc {
                    details: Some("Emitted when transfer.".to_string()),
                    params: BTreeMap::from([
                        ("from".to_string(), "Source account.".to_string()),
                        ("to".to_string(), "Target account.".to_string()),
                        ("value".to_string(), "Transfer amount.".to_string()),
                    ]),
                }
            ),]),
            errors: BTreeMap::from([(
                "InsufficientBalance(uint256,uint256)".to_string(),
                vec![ErrorDoc {
                    details: Some("Needed `required` but only `available` available.".to_string()),
                    params: BTreeMap::from([
                        ("available".to_string(), "Balance available.".to_string()),
                        ("required".to_string(), "Requested amount to transfer.".to_string())
                    ]),
                }]
            ),]),
            title: Some("Not an ERC20.".to_string())
        })
    );

    assert!(compiled.find_first("NotERC20").is_some());
    let contract = compiled.remove_first("NotERC20").unwrap();
    assert_eq!(
        contract.userdoc,
        Some(UserDoc {
            version: Some(1),
            kind: Some("user".to_string()),
            notice: None,
            methods: BTreeMap::from([(
                "transfer(address,uint256)".to_string(),
                UserDocNotice::Notice { notice: "Transfer tokens.".to_string() }
            ),]),
            events: BTreeMap::from([(
                "Transfer(address,address,uint256)".to_string(),
                UserDocNotice::Notice { notice: "Transfer some tokens.".to_string() }
            ),]),
            errors: BTreeMap::from([(
                "InsufficientBalance(uint256,uint256)".to_string(),
                vec![UserDocNotice::Notice {
                    notice: "Insufficient balance for transfer.".to_string()
                }]
            ),]),
        })
    );
    assert_eq!(
        contract.devdoc,
        Some(DevDoc {
            version: Some(1),
            kind: Some("dev".to_string()),
            author: None,
            details: None,
            custom_experimental: None,
            methods: BTreeMap::from([(
                "transfer(address,uint256)".to_string(),
                MethodDoc {
                    details: Some("Transfer `amount` tokens to account `to`.".to_string()),
                    params: BTreeMap::from([
                        ("to".to_string(), "Target account.".to_string()),
                        ("amount".to_string(), "Transfer amount.".to_string())
                    ]),
                    returns: BTreeMap::from([(
                        "_0".to_string(),
                        "A boolean value indicating whether the operation succeeded.".to_string()
                    ),])
                }
            ),]),
            events: BTreeMap::new(),
            errors: BTreeMap::from([(
                "InsufficientBalance(uint256,uint256)".to_string(),
                vec![ErrorDoc {
                    details: Some("Needed `required` but only `available` available.".to_string()),
                    params: BTreeMap::from([
                        ("available".to_string(), "Balance available.".to_string()),
                        ("required".to_string(), "Requested amount to transfer.".to_string())
                    ]),
                }]
            ),]),
            title: None
        })
    );
}

#[test]
fn test_relative_cache_entries() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();
    let _a = project
        .add_source(
            "A",
            r"
pragma solidity ^0.8.10;
contract A { }
",
        )
        .unwrap();
    let _b = project
        .add_source(
            "B",
            r"
pragma solidity ^0.8.10;
contract B { }
",
        )
        .unwrap();
    let _c = project
        .add_source(
            "C",
            r"
pragma solidity ^0.8.10;
contract C { }
",
        )
        .unwrap();
    let _d = project
        .add_source(
            "D",
            r"
pragma solidity ^0.8.10;
contract D { }
",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    let entries = vec![
        PathBuf::from("src/A.sol"),
        PathBuf::from("src/B.sol"),
        PathBuf::from("src/C.sol"),
        PathBuf::from("src/D.sol"),
    ];
    assert_eq!(entries, cache.files.keys().cloned().collect::<Vec<_>>());

    let cache = CompilerCache::<SolcSettings>::read_joined(project.paths()).unwrap();

    assert_eq!(
        entries.into_iter().map(|p| project.root().join(p)).collect::<Vec<_>>(),
        cache.files.keys().cloned().collect::<Vec<_>>()
    );
}

#[test]
fn test_failure_after_removing_file() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();
    project
        .add_source(
            "A",
            r#"
pragma solidity ^0.8.10;
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"
pragma solidity ^0.8.10;
import "./C.sol";
contract B { }
"#,
        )
        .unwrap();

    let c = project
        .add_source(
            "C",
            r"
pragma solidity ^0.8.10;
contract C { }
",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    fs::remove_file(c).unwrap();
    let compiled = project.compile().unwrap();
    assert!(compiled.has_compiler_errors());
}

#[test]
fn can_handle_conflicting_files() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "Greeter",
            r"
    pragma solidity ^0.8.10;

    contract Greeter {}
   ",
        )
        .unwrap();

    project
        .add_source(
            "tokens/Greeter",
            r"
    pragma solidity ^0.8.10;

    contract Greeter {}
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.is_unchanged());
    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    let mut source_files = cache.files.keys().cloned().collect::<Vec<_>>();
    source_files.sort_unstable();

    assert_eq!(
        source_files,
        vec![PathBuf::from("src/Greeter.sol"), PathBuf::from("src/tokens/Greeter.sol"),]
    );

    let mut artifacts = project.artifacts_snapshot().unwrap().artifacts;
    artifacts.strip_prefix_all(&project.paths().artifacts);

    assert_eq!(artifacts.len(), 2);
    let mut artifact_files = artifacts.artifact_files().map(|f| f.file.clone()).collect::<Vec<_>>();
    artifact_files.sort_unstable();

    assert_eq!(
        artifact_files,
        vec![
            PathBuf::from("Greeter.sol/Greeter.json"),
            PathBuf::from("tokens/Greeter.sol/Greeter.json"),
        ]
    );
}

// <https://github.com/foundry-rs/foundry/issues/2843>
#[test]
fn can_handle_conflicting_files_recompile() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "A",
            r"
    pragma solidity ^0.8.10;

    contract A {
            function foo() public{}
    }
   ",
        )
        .unwrap();

    project
        .add_source(
            "inner/A",
            r"
    pragma solidity ^0.8.10;

    contract A {
            function bar() public{}
    }
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.is_unchanged());
    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    let mut source_files = cache.files.keys().cloned().collect::<Vec<_>>();
    source_files.sort_unstable();

    assert_eq!(source_files, vec![PathBuf::from("src/A.sol"), PathBuf::from("src/inner/A.sol"),]);

    let mut artifacts =
        project.artifacts_snapshot().unwrap().artifacts.into_stripped_file_prefixes(project.root());
    artifacts.strip_prefix_all(&project.paths().artifacts);

    assert_eq!(artifacts.len(), 2);
    let mut artifact_files = artifacts.artifact_files().map(|f| f.file.clone()).collect::<Vec<_>>();
    artifact_files.sort_unstable();

    let expected_files = vec![PathBuf::from("A.sol/A.json"), PathBuf::from("inner/A.sol/A.json")];
    assert_eq!(artifact_files, expected_files);

    // overwrite conflicting nested file, effectively changing it
    project
        .add_source(
            "inner/A",
            r"
    pragma solidity ^0.8.10;
    contract A {
    function bar() public{}
    function baz() public{}
    }
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let mut recompiled_artifacts =
        project.artifacts_snapshot().unwrap().artifacts.into_stripped_file_prefixes(project.root());
    recompiled_artifacts.strip_prefix_all(&project.paths().artifacts);

    assert_eq!(recompiled_artifacts.len(), 2);
    let mut artifact_files =
        recompiled_artifacts.artifact_files().map(|f| f.file.clone()).collect::<Vec<_>>();
    artifact_files.sort_unstable();
    assert_eq!(artifact_files, expected_files);

    // ensure that `a.sol/A.json` is unchanged
    let outer = artifacts.find("src/A.sol".as_ref(), "A").unwrap();
    let outer_recompiled = recompiled_artifacts.find("src/A.sol".as_ref(), "A").unwrap();
    assert_eq!(outer, outer_recompiled);

    let inner_recompiled = recompiled_artifacts.find("src/inner/A.sol".as_ref(), "A").unwrap();
    assert!(inner_recompiled.get_abi().unwrap().functions.contains_key("baz"));
}

// <https://github.com/foundry-rs/foundry/issues/2843>
#[test]
fn can_handle_conflicting_files_case_sensitive_recompile() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "a",
            r"
    pragma solidity ^0.8.10;

    contract A {
            function foo() public{}
    }
   ",
        )
        .unwrap();

    project
        .add_source(
            "inner/A",
            r"
    pragma solidity ^0.8.10;

    contract A {
            function bar() public{}
    }
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.is_unchanged());
    let artifacts = compiled.artifacts().count();
    assert_eq!(artifacts, 2);

    let cache = CompilerCache::<SolcSettings>::read(project.cache_path()).unwrap();

    let mut source_files = cache.files.keys().cloned().collect::<Vec<_>>();
    source_files.sort_unstable();

    assert_eq!(source_files, vec![PathBuf::from("src/a.sol"), PathBuf::from("src/inner/A.sol"),]);

    let mut artifacts =
        project.artifacts_snapshot().unwrap().artifacts.into_stripped_file_prefixes(project.root());
    artifacts.strip_prefix_all(&project.paths().artifacts);

    assert_eq!(artifacts.len(), 2);
    let mut artifact_files = artifacts.artifact_files().map(|f| f.file.clone()).collect::<Vec<_>>();
    artifact_files.sort_unstable();

    let expected_files = vec![PathBuf::from("a.sol/A.json"), PathBuf::from("inner/A.sol/A.json")];
    assert_eq!(artifact_files, expected_files);

    // overwrite conflicting nested file, effectively changing it
    project
        .add_source(
            "inner/A",
            r"
    pragma solidity ^0.8.10;
    contract A {
    function bar() public{}
    function baz() public{}
    }
   ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let mut recompiled_artifacts =
        project.artifacts_snapshot().unwrap().artifacts.into_stripped_file_prefixes(project.root());
    recompiled_artifacts.strip_prefix_all(&project.paths().artifacts);

    assert_eq!(recompiled_artifacts.len(), 2);
    let mut artifact_files =
        recompiled_artifacts.artifact_files().map(|f| f.file.clone()).collect::<Vec<_>>();
    artifact_files.sort_unstable();
    assert_eq!(artifact_files, expected_files);

    // ensure that `a.sol/A.json` is unchanged
    let outer = artifacts.find("src/a.sol".as_ref(), "A").unwrap();
    let outer_recompiled = recompiled_artifacts.find("src/a.sol".as_ref(), "A").unwrap();
    assert_eq!(outer, outer_recompiled);

    let inner_recompiled = recompiled_artifacts.find("src/inner/A.sol".as_ref(), "A").unwrap();
    assert!(inner_recompiled.get_abi().unwrap().functions.contains_key("baz"));
}

#[test]
fn can_checkout_repo() {
    let project = TempProject::checkout("transmissions11/solmate").unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    let _artifacts = project.artifacts_snapshot().unwrap();
}

#[test]
fn can_detect_config_changes() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let remapping = project.paths().libraries[0].join("remapping");
    project
        .paths_mut()
        .remappings
        .push(Remapping::from_str(&format!("remapping/={}/", remapping.display())).unwrap());

    project
        .add_source(
            "Foo",
            r#"
    pragma solidity ^0.8.10;
    import "remapping/Bar.sol";

    contract Foo {}
   "#,
        )
        .unwrap();
    project
        .add_lib(
            "remapping/Bar",
            r"
    pragma solidity ^0.8.10;

    contract Bar {}
    ",
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();

    let cache_before =
        CompilerCache::<MultiCompilerSettings>::read(&project.paths().cache).unwrap();
    assert_eq!(cache_before.files.len(), 2);

    // nothing to compile
    let compiled = project.compile().unwrap();
    assert!(compiled.is_unchanged());

    project.project_mut().settings.solc.settings.optimizer.enabled = Some(true);

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());

    let cache_after = CompilerCache::<MultiCompilerSettings>::read(&project.paths().cache).unwrap();
    assert_ne!(cache_before, cache_after);
}

#[test]
fn can_add_basic_contract_and_library() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let remapping = project.paths().libraries[0].join("remapping");
    project
        .paths_mut()
        .remappings
        .push(Remapping::from_str(&format!("remapping/={}/", remapping.display())).unwrap());

    let src = project.add_basic_source("Foo.sol", "^0.8.0").unwrap();

    let lib = project.add_basic_source("Bar", "^0.8.0").unwrap();

    let graph = Graph::<MultiCompilerParsedSource>::resolve(project.paths()).unwrap();
    assert_eq!(graph.files().len(), 2);
    assert!(graph.files().contains_key(&src));
    assert!(graph.files().contains_key(&lib));

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Foo").is_some());
    assert!(compiled.find_first("Bar").is_some());
}

// <https://github.com/foundry-rs/foundry/issues/2706>
#[test]
fn can_handle_nested_absolute_imports() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let remapping = project.paths().libraries[0].join("myDepdendency");
    project
        .paths_mut()
        .remappings
        .push(Remapping::from_str(&format!("myDepdendency/={}/", remapping.display())).unwrap());

    project
        .add_lib(
            "myDepdendency/src/interfaces/IConfig.sol",
            r"
    pragma solidity ^0.8.10;

    interface IConfig {}
   ",
        )
        .unwrap();

    project
        .add_lib(
            "myDepdendency/src/Config.sol",
            r#"
    pragma solidity ^0.8.10;
    import "src/interfaces/IConfig.sol";

    contract Config {}
   "#,
        )
        .unwrap();

    project
        .add_source(
            "Greeter",
            r#"
    pragma solidity ^0.8.10;
    import "myDepdendency/src/Config.sol";

    contract Greeter {}
   "#,
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Greeter").is_some());
    assert!(compiled.find_first("Config").is_some());
    assert!(compiled.find_first("IConfig").is_some());
}

#[test]
fn can_handle_nested_test_absolute_imports() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "Contract.sol",
            r"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

library Library {
    function f(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}

contract Contract {
    uint256 c;

    constructor() {
        c = Library.f(1, 2);
    }
}
   ",
        )
        .unwrap();

    project
        .add_test(
            "Contract.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

import "src/Contract.sol";

contract ContractTest {
    function setUp() public {
    }

    function test() public {
        new Contract();
    }
}
   "#,
        )
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Contract").is_some());
}

// This is a repro and a regression test for https://github.com/foundry-rs/compilers/pull/45
#[test]
fn dirty_files_discovery() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    project
        .add_source(
            "D.sol",
            r"
pragma solidity 0.8.23;
contract D {
    function foo() internal pure returns (uint256) {
        return 1;
    }
}
   ",
        )
        .unwrap();

    project
        .add_source("A.sol", "pragma solidity ^0.8.10; import './C.sol'; contract A is D {}")
        .unwrap();
    project
        .add_source("B.sol", "pragma solidity ^0.8.10; import './A.sol'; contract B is D {}")
        .unwrap();
    project
        .add_source("C.sol", "pragma solidity ^0.8.10; import './D.sol'; contract C is D {}")
        .unwrap();

    project.compile().unwrap();

    // Change D.sol so it becomes dirty
    project
        .add_source(
            "D.sol",
            r"
pragma solidity 0.8.23;
contract D {
    function foo() internal pure returns (uint256) {
        return 2;
    }
}
   ",
        )
        .unwrap();

    let output = project.compile().unwrap();

    // Check that all contracts were recompiled
    assert_eq!(output.compiled_artifacts().len(), 4);
}

#[test]
fn test_deterministic_metadata() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let root = tmp_dir.path();
    let orig_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
    copy_dir_all(&orig_root, tmp_dir.path()).unwrap();

    let compiler = MultiCompiler {
        solc: Some(SolcCompiler::Specific(
            Solc::find_svm_installed_version(&Version::new(0, 8, 18)).unwrap().unwrap(),
        )),
        vyper: None,
    };
    let paths = ProjectPathsConfig::builder().root(root).build().unwrap();
    let project = Project::builder().paths(paths).build(compiler).unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    let artifact = compiled.find_first("DappTest").unwrap();

    let bytecode = artifact.bytecode.as_ref().unwrap().bytes().unwrap().clone();
    let expected_bytecode = Bytes::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-test-bytecode.txt"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(bytecode, expected_bytecode);
}

#[test]
fn can_compile_vyper_with_cache() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let root = tmp_dir.path();
    let cache = root.join("cache").join(SOLIDITY_FILES_CACHE_FILENAME);

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let orig_root = manifest_dir.join("../../test-data/vyper-sample");
    copy_dir_all(&orig_root, tmp_dir.path()).unwrap();

    let paths = ProjectPathsConfig::builder()
        .cache(cache)
        .sources(root.join("src"))
        .artifacts(root.join("out"))
        .root(root)
        .build::<VyperLanguage>()
        .unwrap();

    let settings = VyperSettings {
        output_selection: OutputSelection::default_output_selection(),
        ..Default::default()
    };

    // first compile
    let project = ProjectBuilder::<Vyper>::new(Default::default())
        .settings(settings)
        .paths(paths)
        .build(VYPER.clone())
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Counter").is_some());
    compiled.assert_success();

    // cache is used when nothing to compile
    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Counter").is_some());
    assert!(compiled.is_unchanged());

    // deleted artifacts cause recompile even with cache
    std::fs::remove_dir_all(project.artifacts_path()).unwrap();
    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find_first("Counter").is_some());
    assert!(!compiled.is_unchanged());
}

#[test]
fn yul_remappings_ignored() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/yul-sample");
    // Add dummy remapping.
    let paths = ProjectPathsConfig::builder().sources(root.clone()).remapping(Remapping {
        context: None,
        name: "@openzeppelin".to_string(),
        path: root.to_string_lossy().to_string(),
    });
    let project = TempProject::<MultiCompiler, ConfigurableArtifacts>::new(paths).unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
}

#[test]
fn test_vyper_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/vyper-imports");

    let paths = ProjectPathsConfig::builder()
        .sources(root.join("src"))
        .root(root)
        .build::<VyperLanguage>()
        .unwrap();

    let settings = VyperSettings {
        output_selection: OutputSelection::default_output_selection(),
        ..Default::default()
    };

    let project = ProjectBuilder::<Vyper>::new(Default::default())
        .settings(settings)
        .paths(paths)
        .no_artifacts()
        .build(VYPER.clone())
        .unwrap();

    project.compile().unwrap().assert_success();
}

#[test]
fn test_can_compile_multi() {
    let root =
        canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/multi-sample"))
            .unwrap();

    let paths = ProjectPathsConfig::builder()
        .sources(root.join("src"))
        .root(&root)
        .build::<MultiCompilerLanguage>()
        .unwrap();

    let settings = MultiCompilerSettings {
        vyper: VyperSettings {
            output_selection: OutputSelection::default_output_selection(),
            ..Default::default()
        },
        solc: Default::default(),
    };

    let compiler =
        MultiCompiler { solc: Some(SolcCompiler::default()), vyper: Some(VYPER.clone()) };

    let project = ProjectBuilder::<MultiCompiler>::new(Default::default())
        .settings(settings)
        .paths(paths)
        .no_artifacts()
        .build(compiler)
        .unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(compiled.find(&root.join("src/Counter.sol"), "Counter").is_some());
    assert!(compiled.find(&root.join("src/Counter.vy"), "Counter").is_some());
}

// This is a reproduction of https://github.com/foundry-rs/compilers/issues/47
#[test]
fn remapping_trailing_slash_issue47() {
    use std::sync::Arc;

    use foundry_compilers_artifacts::{EvmVersion, Source, Sources};

    let mut sources = Sources::new();
    sources.insert(
        PathBuf::from("./C.sol"),
        Source {
            content: Arc::new(r#"import "@project/D.sol"; contract C {}"#.to_string()),
            kind: Default::default(),
        },
    );
    sources.insert(
        PathBuf::from("./D.sol"),
        Source { content: Arc::new(r#"contract D {}"#.to_string()), kind: Default::default() },
    );

    let mut settings = Settings { evm_version: Some(EvmVersion::Byzantium), ..Default::default() };
    settings.remappings.push(Remapping {
        context: None,
        name: "@project".into(),
        path: ".".into(),
    });
    let input = SolcInput { language: SolcLanguage::Solidity, sources, settings };
    let compiler = Solc::find_or_install(&Version::new(0, 6, 8)).unwrap();
    let output = compiler.compile_exact(&input).unwrap();
    assert!(!output.has_error());
}

#[test]
fn test_settings_restrictions() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    // default EVM version is Paris, Cancun contract won't compile
    project.project_mut().settings.solc.evm_version = Some(EvmVersion::Paris);

    let common_path = project.add_source("Common.sol", "").unwrap();

    let cancun_path = project
        .add_source(
            "Cancun.sol",
            r#"
import "./Common.sol";

contract TransientContract {
    function lock()public {
        assembly {
            tstore(0, 1)
        }
    }
}"#,
        )
        .unwrap();

    let cancun_importer_path =
        project.add_source("CancunImporter.sol", "import \"./Cancun.sol\";").unwrap();
    let simple_path = project
        .add_source(
            "Simple.sol",
            r#"
import "./Common.sol";

contract SimpleContract {}
"#,
        )
        .unwrap();

    // Add config with Cancun enabled
    let mut cancun_settings = project.project().settings.clone();
    cancun_settings.solc.evm_version = Some(EvmVersion::Cancun);
    project.project_mut().additional_settings.insert("cancun".to_string(), cancun_settings);

    let cancun_restriction = RestrictionsWithVersion {
        restrictions: MultiCompilerRestrictions {
            solc: SolcRestrictions {
                evm_version: Restriction { min: Some(EvmVersion::Cancun), ..Default::default() },
                ..Default::default()
            },
            ..Default::default()
        },
        version: None,
    };

    // Restrict compiling Cancun contract to Cancun EVM version
    project.project_mut().restrictions.insert(cancun_path.clone(), cancun_restriction);

    let output = project.compile().unwrap();

    output.assert_success();

    let artifacts = output
        .artifact_ids()
        .map(|(id, _)| (id.profile, id.source))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(
        artifacts,
        vec![
            ("cancun".to_string(), cancun_path),
            ("cancun".to_string(), cancun_importer_path),
            ("cancun".to_string(), common_path.clone()),
            ("default".to_string(), common_path),
            ("default".to_string(), simple_path),
        ]
    );
}

// <https://github.com/foundry-rs/foundry/issues/9876>
#[test]
fn can_flatten_top_level_event_declaration() {
    let project = TempProject::<MultiCompiler>::dapptools().unwrap();

    let target = project
        .add_source(
            "A",
            r#"pragma solidity ^0.8.10;
import "./B.sol";
contract A { }
"#,
        )
        .unwrap();

    project
        .add_source(
            "B",
            r#"
event TestEvent();
"#,
        )
        .unwrap();

    test_flatteners(&project, &target, |result| {
        assert_eq!(
            result,
            r"pragma solidity ^0.8.10;

// src/B.sol

event TestEvent();

// src/A.sol

contract A { }
"
        );
    });
}
