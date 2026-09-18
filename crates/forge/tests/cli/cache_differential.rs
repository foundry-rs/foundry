//! Differential tests for incremental compilation cache correctness.

use foundry_compilers::PathStyle;
use foundry_test_utils::{TestProject, util::SOLC_VERSION};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{ExitStatus, Output},
};

#[derive(Clone, Copy, Debug)]
struct FileSpec {
    path: &'static str,
    contents: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct Mutation {
    path: &'static str,
    contents: &'static str,
    expected_compiled_files: usize,
}

#[derive(Debug)]
struct Scenario {
    name: &'static str,
    files: &'static [FileSpec],
    mutations: &'static [Mutation],
}

#[derive(Debug, PartialEq)]
struct Observation {
    success: bool,
    results: Value,
    artifacts: BTreeMap<PathBuf, Value>,
}

const DYNAMIC_FILES: &[FileSpec] = &[
    FileSpec {
        path: "src/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "changed value"); }
}
"#,
    },
];

const DYNAMIC_MUTATIONS: &[Mutation] = &[Mutation {
    path: "src/Impl.sol",
    contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 1,
}];

const EXTERNAL_FILES: &[FileSpec] = &[
    FileSpec {
        path: "external/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../external/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "changed value"); }
}
"#,
    },
];

const EXTERNAL_MUTATIONS: &[Mutation] = &[Mutation {
    path: "external/Impl.sol",
    contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 2,
}];

const INDEPENDENT_FILES: &[FileSpec] = &[
    FileSpec {
        path: "external/A.sol",
        contents: r#"contract A {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "external/B.sol",
        contents: r#"contract B {
    function value() external pure returns (uint256) { return 333; }
}
"#,
    },
    FileSpec {
        path: "test/A.t.sol",
        contents: r#"import {A} from "../external/A.sol";
contract ATest {
    function test_value_a() public { require(new A().value() == 111, "changed A"); }
}
"#,
    },
    FileSpec {
        path: "test/B.t.sol",
        contents: r#"import {B} from "../external/B.sol";
contract BTest {
    function test_value_b() public { require(new B().value() == 333, "changed B"); }
}
"#,
    },
];

const INDEPENDENT_MUTATIONS: &[Mutation] = &[Mutation {
    path: "external/A.sol",
    contents: r#"contract A {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 2,
}];

const SCENARIOS: &[Scenario] = &[
    Scenario { name: "dynamic", files: DYNAMIC_FILES, mutations: DYNAMIC_MUTATIONS },
    Scenario { name: "external-native", files: EXTERNAL_FILES, mutations: EXTERNAL_MUTATIONS },
    Scenario {
        name: "independent-native",
        files: INDEPENDENT_FILES,
        mutations: INDEPENDENT_MUTATIONS,
    },
];

#[test]
fn cache_matches_clean_builds() {
    for scenario in SCENARIOS {
        run_scenario(scenario);
    }
}

fn run_scenario(scenario: &Scenario) {
    let semantic = TestProject::new(
        &format!("cache-differential-{}-semantic", scenario.name),
        PathStyle::Dapptools,
    );
    let reuse = TestProject::new(
        &format!("cache-differential-{}-reuse", scenario.name),
        PathStyle::Dapptools,
    );
    materialize(&semantic, scenario.files);
    materialize(&reuse, scenario.files);

    let baseline = run_forge_json(&semantic);
    assert!(baseline.status.success(), "{}: warmup failed: {baseline:?}", scenario.name);
    let reuse_baseline = run_forge_plain(&reuse);
    assert!(reuse_baseline.status.success(), "{}: reuse warmup failed", scenario.name);
    let cached = run_forge_plain(&reuse);
    assert!(cached.status.success(), "{}: cached warmup failed: {cached:?}", scenario.name);
    assert_eq!(
        reported_compiled_files(&cached).unwrap(),
        0,
        "{}: unchanged run compiled files",
        scenario.name
    );

    let mut current =
        scenario.files.iter().map(|file| (file.path, file.contents)).collect::<BTreeMap<_, _>>();

    for (index, mutation) in scenario.mutations.iter().enumerate() {
        current.insert(mutation.path, mutation.contents);
        write_file(semantic.root(), mutation.path, mutation.contents);
        write_file(reuse.root(), mutation.path, mutation.contents);

        let incremental_output = run_forge_json(&semantic);
        let incremental_compile = run_forge_plain(&reuse);
        let actual_compiled_files = reported_compiled_files(&incremental_compile).unwrap();

        let clean = TestProject::new(
            &format!("cache-differential-{}-clean-{index}", scenario.name),
            PathStyle::Dapptools,
        );
        let files = current
            .iter()
            .map(|(&path, &contents)| FileSpec { path, contents })
            .collect::<Vec<_>>();
        materialize(&clean, &files);
        let clean_output = run_forge_json(&clean);

        let incremental_observation = observe(&semantic, &incremental_output);
        let clean_observation = observe(&clean, &clean_output);
        if actual_compiled_files != mutation.expected_compiled_files
            || incremental_observation != clean_observation
        {
            let saved = preserve_failure(scenario, index, &semantic, &reuse, &clean);
            panic!(
                "{} mutation {index}: incremental result differs from clean result\n\
                 replay artifacts: {}\n\
                 compiled files: {actual_compiled_files}, expected: {}\n\
                 incremental: {incremental_observation:#?}\nclean: {clean_observation:#?}",
                scenario.name,
                saved.display(),
                mutation.expected_compiled_files,
            );
        }
    }
}

fn materialize(project: &TestProject, files: &[FileSpec]) {
    write_file(
        project.root(),
        "foundry.toml",
        &format!(
            r#"[profile.default]
dynamic_test_linking = true
solc = "{SOLC_VERSION}"
bytecode_hash = "none"
cbor_metadata = false
"#,
        ),
    );
    for file in files {
        write_file(project.root(), file.path, file.contents);
    }
}

fn write_file(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn run_forge_json(project: &TestProject) -> Output {
    project.forge_bin().args(["test", "--json"]).output().unwrap()
}

fn run_forge_plain(project: &TestProject) -> Output {
    project.forge_bin().arg("test").output().unwrap()
}

fn reported_compiled_files(output: &Output) -> Result<usize, String> {
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut counts = output.lines().filter_map(|line| line.strip_prefix("Compiling ")).peekable();
    if counts.peek().is_some() {
        counts
            .map(|line| {
                line.split_whitespace()
                    .next()
                    .ok_or_else(|| format!("malformed compiler report: {line}"))?
                    .parse::<usize>()
                    .map_err(|error| format!("malformed compiler report `{line}`: {error}"))
            })
            .sum()
    } else if output.lines().any(|line| line == "No files changed, compilation skipped") {
        Ok(0)
    } else {
        Err(format!("missing compiler report:\n{output}"))
    }
}

fn observe(project: &TestProject, output: &Output) -> Observation {
    let mut results = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "forge did not emit JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    });
    remove_durations(&mut results);
    Observation {
        success: output.status.success(),
        results,
        artifacts: collect_artifacts(project.artifacts()),
    }
}

fn remove_durations(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("duration");
            for value in object.values_mut() {
                remove_durations(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(remove_durations),
        _ => {}
    }
}

fn collect_artifacts(root: &Path) -> BTreeMap<PathBuf, Value> {
    let mut artifacts = BTreeMap::new();
    collect_artifacts_in(root, root, &mut artifacts);
    artifacts
}

fn collect_artifacts_in(root: &Path, dir: &Path, artifacts: &mut BTreeMap<PathBuf, Value>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            if entry.file_name() != "build-info" {
                collect_artifacts_in(root, &path, artifacts);
            }
        } else if path.extension().is_some_and(|extension| extension == "json") {
            let artifact = serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap();
            artifacts.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                artifact_semantics(&artifact),
            );
        }
    }
}

fn artifact_semantics(artifact: &Value) -> Value {
    let get = |path: &[&str]| {
        path.iter().try_fold(artifact, |value, key| value.get(key)).cloned().unwrap_or(Value::Null)
    };
    json!({
        "abi": get(&["abi"]),
        "bytecode": get(&["bytecode", "object"]),
        "bytecodeLinkReferences": get(&["bytecode", "linkReferences"]),
        "runtimeBytecode": get(&["deployedBytecode", "object"]),
        "runtimeLinkReferences": get(&["deployedBytecode", "linkReferences"]),
        "immutableReferences": get(&["deployedBytecode", "immutableReferences"]),
    })
}

fn preserve_failure(
    scenario: &Scenario,
    mutation: usize,
    semantic: &TestProject,
    reuse: &TestProject,
    clean: &TestProject,
) -> PathBuf {
    let root = semantic
        .foundry_bin_path("forge")
        .parent()
        .unwrap()
        .join("cache-differential")
        .join(format!("{}-{mutation}", scenario.name));
    let _ = fs::remove_dir_all(&root);
    copy_tree(semantic.root(), &root.join("semantic"));
    copy_tree(reuse.root(), &root.join("reuse"));
    copy_tree(clean.root(), &root.join("clean"));
    fs::write(
        root.join("scenario.json"),
        serde_json::to_vec_pretty(&json!({
            "version": 1,
            "name": scenario.name,
            "mutation": mutation,
            "forge": semantic.foundry_bin_path("forge"),
        }))
        .unwrap(),
    )
    .unwrap();
    root
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn parses_compilation_reports() {
    fn output(stdout: &str) -> Output {
        Output { status: ExitStatus::default(), stdout: stdout.as_bytes().to_vec(), stderr: vec![] }
    }

    assert_eq!(
        reported_compiled_files(&output(
            "Compiling 2 files with Solc 0.8.37\nCompiling 3 files with Solc 0.7.6\n"
        )),
        Ok(5)
    );
    assert_eq!(reported_compiled_files(&output("No files changed, compilation skipped\n")), Ok(0));
    assert!(reported_compiled_files(&output("Compiler run successful!\n")).is_err());
    assert!(reported_compiled_files(&output("Compiling many files\n")).is_err());
}
