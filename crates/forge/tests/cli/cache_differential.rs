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

#[derive(Clone, Copy, Debug)]
enum GeneratedEdge {
    Direct,
    FreeFunction,
}

impl GeneratedEdge {
    const fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::FreeFunction => "free-function",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum GeneratedRequest {
    A,
    B,
}

impl GeneratedRequest {
    const fn args(self) -> &'static [&'static str] {
        match self {
            Self::A => &["test", "--match-path", "test/A.t.sol"],
            Self::B => &["test", "--match-path", "test/B.t.sol"],
        }
    }

    const fn suite(self) -> &'static str {
        match self {
            Self::A => "test/A.t.sol:ATest",
            Self::B => "test/B.t.sol:BTest",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum GeneratedAction {
    ProbeA,
    ProbeB,
    ToggleDependencyForA,
    ToggleDependencyForB,
    ToggleAEdge,
    ToggleBEdge,
    ToggleAEdgeRequestB,
}

impl GeneratedAction {
    const ALL: [Self; 6] = [
        Self::ProbeA,
        Self::ProbeB,
        Self::ToggleDependencyForA,
        Self::ToggleDependencyForB,
        Self::ToggleAEdge,
        Self::ToggleBEdge,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::ProbeA => "probe-a",
            Self::ProbeB => "probe-b",
            Self::ToggleDependencyForA => "toggle-dependency-request-a",
            Self::ToggleDependencyForB => "toggle-dependency-request-b",
            Self::ToggleAEdge => "toggle-a-edge-request-a",
            Self::ToggleBEdge => "toggle-b-edge-request-b",
            Self::ToggleAEdgeRequestB => "toggle-a-edge-request-b",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GeneratedModel {
    dependency_revision: bool,
    a_edge: GeneratedEdge,
    b_edge: GeneratedEdge,
}

impl Default for GeneratedModel {
    fn default() -> Self {
        Self {
            dependency_revision: false,
            a_edge: GeneratedEdge::FreeFunction,
            b_edge: GeneratedEdge::FreeFunction,
        }
    }
}

impl GeneratedModel {
    const fn value(self) -> u64 {
        if self.dependency_revision { 101 } else { 100 }
    }

    const fn apply(&mut self, action: GeneratedAction) -> GeneratedRequest {
        match action {
            GeneratedAction::ProbeA => GeneratedRequest::A,
            GeneratedAction::ProbeB => GeneratedRequest::B,
            GeneratedAction::ToggleDependencyForA => {
                self.dependency_revision = !self.dependency_revision;
                GeneratedRequest::A
            }
            GeneratedAction::ToggleDependencyForB => {
                self.dependency_revision = !self.dependency_revision;
                GeneratedRequest::B
            }
            GeneratedAction::ToggleAEdge => {
                self.a_edge = match self.a_edge {
                    GeneratedEdge::Direct => GeneratedEdge::FreeFunction,
                    GeneratedEdge::FreeFunction => GeneratedEdge::Direct,
                };
                GeneratedRequest::A
            }
            GeneratedAction::ToggleBEdge => {
                self.b_edge = match self.b_edge {
                    GeneratedEdge::Direct => GeneratedEdge::FreeFunction,
                    GeneratedEdge::FreeFunction => GeneratedEdge::Direct,
                };
                GeneratedRequest::B
            }
            GeneratedAction::ToggleAEdgeRequestB => {
                self.a_edge = match self.a_edge {
                    GeneratedEdge::Direct => GeneratedEdge::FreeFunction,
                    GeneratedEdge::FreeFunction => GeneratedEdge::Direct,
                };
                GeneratedRequest::B
            }
        }
    }
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

const DYNAMIC_NATIVE_MUTATIONS: &[Mutation] = &[
    Mutation {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
function make() returns (Impl) { return new Impl(); }
contract ImplTest {
    function test_value() public { require(make().value() == 111, "changed value"); }
}
"#,
        expected_compiled_files: 1,
    },
    Mutation {
        path: "src/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 222; }
}
"#,
        expected_compiled_files: 2,
    },
    Mutation {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "changed value"); }
}
"#,
        expected_compiled_files: 1,
    },
    Mutation {
        path: "src/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 111; }
}
"#,
        expected_compiled_files: 1,
    },
];

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

const REPLACED_DEPENDENCY_FILES: &[FileSpec] = &[
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
        path: "src/Keeper.sol",
        contents: r#"import {A} from "../external/A.sol";
contract Keeper { function keep(A) external pure {} }
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {A} from "../external/A.sol";
contract ImplTest {
    function test_value() public { require(new A().value() == 111, "changed A"); }
}
"#,
    },
];

const REPLACED_DEPENDENCY_MUTATIONS: &[Mutation] = &[
    Mutation {
        path: "test/Impl.t.sol",
        contents: r#"import {B} from "../external/B.sol";
contract ImplTest {
    function test_value() public { require(new B().value() == 333, "changed B"); }
}
"#,
        // Adding B to the active source context conservatively invalidates every source unit.
        expected_compiled_files: 4,
    },
    Mutation {
        path: "external/A.sol",
        contents: r#"contract A {
    function value() external pure returns (uint256) { return 222; }
}
"#,
        expected_compiled_files: 2,
    },
    Mutation {
        path: "external/B.sol",
        contents: r#"contract B {
    function value() external pure returns (uint256) { return 444; }
}
"#,
        expected_compiled_files: 2,
    },
];

const CLEARED_DEPENDENCY_FILES: &[FileSpec] = &[
    FileSpec {
        path: "external/A.sol",
        contents: r#"contract A {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "src/Keeper.sol",
        contents: r#"import {A} from "../external/A.sol";
contract Keeper { function keep(A) external pure {} }
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {A} from "../external/A.sol";
contract ImplTest {
    function test_value() public { require(new A().value() == 111, "changed A"); }
}
"#,
    },
];

const CLEARED_DEPENDENCY_MUTATIONS: &[Mutation] = &[
    Mutation {
        path: "test/Impl.t.sol",
        contents: r#"contract ImplTest {
    function test_value() public pure { require(1 + 1 == 2, "arithmetic"); }
}
"#,
        expected_compiled_files: 1,
    },
    Mutation {
        path: "external/A.sol",
        contents: r#"contract A {
    function value() external pure returns (uint256) { return 222; }
}
"#,
        expected_compiled_files: 2,
    },
];

const REMAPPING_FILES: &[FileSpec] = &[
    FileSpec {
        path: "foundry.toml",
        contents: r#"[profile.default]
dynamic_test_linking = true
solc = "0.8.37"
bytecode_hash = "none"
cbor_metadata = false
remappings = ["@dep/=vendor/a/"]
"#,
    },
    FileSpec {
        path: "vendor/a/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "vendor/b/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 333; }
}
"#,
    },
    FileSpec {
        path: "src/KeepA.sol",
        contents: r#"import {Impl as A} from "../vendor/a/Impl.sol";
contract KeepA { function keep(A) external pure {} }
"#,
    },
    FileSpec {
        path: "src/KeepB.sol",
        contents: r#"import {Impl as B} from "../vendor/b/Impl.sol";
contract KeepB { function keep(B) external pure {} }
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "@dep/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "wrong target"); }
}
"#,
    },
];

const REMAPPING_MUTATIONS: &[Mutation] = &[
    Mutation {
        path: "foundry.toml",
        contents: r#"[profile.default]
dynamic_test_linking = true
solc = "0.8.37"
bytecode_hash = "none"
cbor_metadata = false
remappings = ["@dep/=vendor/b/"]
"#,
        expected_compiled_files: 1,
    },
    Mutation {
        path: "vendor/a/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 222; }
}
"#,
        expected_compiled_files: 2,
    },
    Mutation {
        path: "vendor/b/Impl.sol",
        contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 444; }
}
"#,
        expected_compiled_files: 3,
    },
];

const LIBRARY_FILES: &[FileSpec] = &[
    FileSpec {
        path: "src/Lib.sol",
        contents: r#"library Lib {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "src/Impl.sol",
        contents: r#"import {Lib} from "./Lib.sol";
contract Impl {
    function value() external view returns (uint256) { return Lib.value(); }
}
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "changed library"); }
}
"#,
    },
];

const LIBRARY_MUTATIONS: &[Mutation] = &[Mutation {
    path: "src/Lib.sol",
    contents: r#"library Lib {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 3,
}];

const INHERITANCE_FILES: &[FileSpec] = &[
    FileSpec {
        path: "src/Base.sol",
        contents: r#"contract Base {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "src/Impl.sol",
        contents: r#"import {Base} from "./Base.sol";
contract Impl is Base {}
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl().value() == 111, "changed base"); }
}
"#,
    },
];

const INHERITANCE_MUTATIONS: &[Mutation] = &[Mutation {
    path: "src/Base.sol",
    contents: r#"contract Base {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 2,
}];

const IMMUTABLE_FILES: &[FileSpec] = &[
    FileSpec {
        path: "src/Impl.sol",
        contents: r#"contract Impl {
    uint256 immutable stored;
    constructor(uint256 value) { stored = value; }
    function read() external view returns (uint256) { return stored; }
}
"#,
    },
    FileSpec {
        path: "test/Impl.t.sol",
        contents: r#"import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_value() public { require(new Impl(111).read() == 111, "changed immutable"); }
}
"#,
    },
];

const IMMUTABLE_MUTATIONS: &[Mutation] = &[Mutation {
    path: "src/Impl.sol",
    contents: r#"contract Impl {
    uint256 immutable stored;
    constructor(uint256 value) { stored = value + 1; }
    function read() external view returns (uint256) { return stored; }
}
"#,
    expected_compiled_files: 1,
}];

const NATIVE_CREATION_CODE_FILES: &[FileSpec] = &[
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
    function creationCode() internal pure returns (bytes memory) { return type(Impl).creationCode; }
    function test_value() public {
        bytes memory code = creationCode();
        address deployed;
        assembly { deployed := create(0, add(code, 32), mload(code)) }
        require(Impl(deployed).value() == 111, "changed creation code");
    }
}
"#,
    },
];

const NATIVE_CREATION_CODE_MUTATIONS: &[Mutation] = &[Mutation {
    path: "src/Impl.sol",
    contents: r#"contract Impl {
    function value() external pure returns (uint256) { return 222; }
}
"#,
    expected_compiled_files: 2,
}];

const SHARED_FILTER_FILES: &[FileSpec] = &[
    FileSpec {
        path: "external/Dep.sol",
        contents: r#"contract Dep {
    function value() external pure returns (uint256) { return 111; }
}
"#,
    },
    FileSpec {
        path: "test/A.t.sol",
        contents: r#"import {Dep} from "../external/Dep.sol";
contract ATest {
    function test_value_a() public { require(new Dep().value() == 111, "changed A"); }
}
"#,
    },
    FileSpec {
        path: "test/B.t.sol",
        contents: r#"import {Dep} from "../external/Dep.sol";
contract BTest {
    function test_value_b() public { require(new Dep().value() == 111, "changed B"); }
}
"#,
    },
];

const SCENARIOS: &[Scenario] = &[
    Scenario { name: "dynamic", files: DYNAMIC_FILES, mutations: DYNAMIC_MUTATIONS },
    Scenario {
        name: "dynamic-native-dynamic",
        files: DYNAMIC_FILES,
        mutations: DYNAMIC_NATIVE_MUTATIONS,
    },
    Scenario { name: "external-native", files: EXTERNAL_FILES, mutations: EXTERNAL_MUTATIONS },
    Scenario {
        name: "independent-native",
        files: INDEPENDENT_FILES,
        mutations: INDEPENDENT_MUTATIONS,
    },
    Scenario {
        name: "replace-native-dependency",
        files: REPLACED_DEPENDENCY_FILES,
        mutations: REPLACED_DEPENDENCY_MUTATIONS,
    },
    Scenario {
        name: "clear-native-dependency",
        files: CLEARED_DEPENDENCY_FILES,
        mutations: CLEARED_DEPENDENCY_MUTATIONS,
    },
    Scenario { name: "linked-library", files: LIBRARY_FILES, mutations: LIBRARY_MUTATIONS },
    Scenario {
        name: "inherited-implementation",
        files: INHERITANCE_FILES,
        mutations: INHERITANCE_MUTATIONS,
    },
    Scenario {
        name: "constructor-immutable",
        files: IMMUTABLE_FILES,
        mutations: IMMUTABLE_MUTATIONS,
    },
    Scenario {
        name: "native-creation-code",
        files: NATIVE_CREATION_CODE_FILES,
        mutations: NATIVE_CREATION_CODE_MUTATIONS,
    },
    Scenario { name: "retarget-remapping", files: REMAPPING_FILES, mutations: REMAPPING_MUTATIONS },
];

#[test]
fn cache_matches_clean_builds() {
    for scenario in SCENARIOS {
        run_scenario(scenario);
    }
}

#[test]
fn dynamic_linking_matches_standard_cache() {
    for scenario in SCENARIOS.iter().filter(|scenario| scenario.name != "retarget-remapping") {
        let dynamic = TestProject::new(
            &format!("cache-differential-{}-dynamic", scenario.name),
            PathStyle::Dapptools,
        );
        let standard = TestProject::new(
            &format!("cache-differential-{}-standard", scenario.name),
            PathStyle::Dapptools,
        );
        materialize(&dynamic, scenario.files);
        materialize_with_linking(&standard, scenario.files, false);
        let mut current = scenario
            .files
            .iter()
            .map(|file| (file.path, file.contents))
            .collect::<BTreeMap<_, _>>();

        compare_linking_modes(scenario, 0, &dynamic, &standard, &current);
        for (index, mutation) in scenario.mutations.iter().enumerate() {
            current.insert(mutation.path, mutation.contents);
            write_file(dynamic.root(), mutation.path, mutation.contents);
            write_file(standard.root(), mutation.path, mutation.contents);
            compare_linking_modes(scenario, index + 1, &dynamic, &standard, &current);
        }
    }
}

fn compare_linking_modes(
    scenario: &Scenario,
    checkpoint: usize,
    dynamic: &TestProject,
    standard: &TestProject,
    current: &BTreeMap<&'static str, &'static str>,
) {
    let dynamic_observation = observe(dynamic, &run_forge_json(dynamic, &["test"]));
    let standard_observation = observe(standard, &run_forge_json(standard, &["test"]));
    let clean = TestProject::new(
        &format!("cache-differential-{}-standard-clean-{checkpoint}", scenario.name),
        PathStyle::Dapptools,
    );
    let files =
        current.iter().map(|(&path, &contents)| FileSpec { path, contents }).collect::<Vec<_>>();
    materialize_with_linking(&clean, &files, false);
    let clean_observation = observe(&clean, &run_forge_json(&clean, &["test"]));
    let mut dynamic_results = dynamic_observation.results.clone();
    let mut standard_results = standard_observation.results.clone();
    remove_unit_test_gas(&mut dynamic_results);
    remove_unit_test_gas(&mut standard_results);
    if dynamic_observation.success != standard_observation.success
        || dynamic_results != standard_results
        || !observations_match(&standard_observation, &clean_observation)
    {
        let saved = preserve_failure(scenario, checkpoint, dynamic, standard, &clean);
        panic!(
            "{} checkpoint {checkpoint}: cached execution differs from its oracle\n\
             replay artifacts: {}\n\
             dynamic: {dynamic_observation:#?}\n\
             standard: {standard_observation:#?}\nstandard clean: {clean_observation:#?}",
            scenario.name,
            saved.display(),
        );
    }
}

/// Runs the exhaustive generated campaign locally without adding its subprocess budget to CI.
#[test]
#[ignore = "brute-force cache campaign"]
fn generated_partial_request_histories_match_oracles() {
    let mut case = 0;
    for first in GeneratedAction::ALL {
        for second in GeneratedAction::ALL {
            for third in GeneratedAction::ALL {
                run_generated_history(case, &[first, second, third]);
                case += 1;
            }
        }
    }
    assert_eq!(case, 216);
}

/// Exercises classification changes to an unselected consumer.
#[test]
#[ignore = "brute-force cache campaign"]
fn generated_unselected_consumer_history_matches_oracles() {
    run_generated_history(
        216,
        &[
            GeneratedAction::ToggleAEdge,
            GeneratedAction::ToggleAEdgeRequestB,
            GeneratedAction::ToggleDependencyForB,
            GeneratedAction::ProbeA,
        ],
    );
}

fn run_generated_history(case: usize, history: &[GeneratedAction]) {
    let dynamic = TestProject::new(
        &format!("cache-differential-generated-{case}-dynamic"),
        PathStyle::Dapptools,
    );
    let standard = TestProject::new(
        &format!("cache-differential-generated-{case}-standard"),
        PathStyle::Dapptools,
    );
    let mut model = GeneratedModel::default();
    let mut files = render_generated_files(model);
    materialize_generated(&dynamic, &files, true);
    materialize_generated(&standard, &files, false);
    run_generated_checkpoint(
        case,
        0,
        &history[..0],
        model,
        GeneratedRequest::A,
        &dynamic,
        &standard,
    );

    for (index, &action) in history.iter().enumerate() {
        let request = model.apply(action);
        let next_files = render_generated_files(model);
        apply_generated_file_diff(&dynamic, &files, &next_files);
        apply_generated_file_diff(&standard, &files, &next_files);
        files = next_files;
        run_generated_checkpoint(
            case,
            index + 1,
            &history[..=index],
            model,
            request,
            &dynamic,
            &standard,
        );
    }
}

fn run_generated_checkpoint(
    case: usize,
    checkpoint: usize,
    history: &[GeneratedAction],
    model: GeneratedModel,
    request: GeneratedRequest,
    dynamic: &TestProject,
    standard: &TestProject,
) {
    let dynamic_output = run_forge_json(dynamic, request.args());
    let standard_output = run_forge_json(standard, request.args());
    let dynamic_observation = try_observe(dynamic, &dynamic_output);
    let standard_observation = try_observe(standard, &standard_output);

    let clean = TestProject::new(
        &format!("cache-differential-generated-{case}-clean-{checkpoint}"),
        PathStyle::Dapptools,
    );
    let files = render_generated_files(model);
    materialize_generated(&clean, &files, false);
    let clean_output = run_forge_json(&clean, request.args());
    let clean_observation = try_observe(&clean, &clean_output);

    let parse_errors = [
        ("dynamic", dynamic_observation.as_ref().err()),
        ("standard", standard_observation.as_ref().err()),
        ("clean", clean_observation.as_ref().err()),
    ]
    .into_iter()
    .filter_map(|(lane, error)| error.map(|error| format!("{lane}: {error}")))
    .collect::<Vec<_>>();
    if !parse_errors.is_empty() {
        let fresh_dynamic = TestProject::new(
            &format!("cache-differential-generated-{case}-fresh-dynamic-{checkpoint}"),
            PathStyle::Dapptools,
        );
        materialize_generated(&fresh_dynamic, &files, true);
        let fresh_dynamic_output = run_forge_json(&fresh_dynamic, request.args());
        let saved = preserve_generated_failure(
            case,
            checkpoint,
            history,
            model,
            request,
            dynamic,
            standard,
            &clean,
            &fresh_dynamic,
            [
                ("dynamic", &dynamic_output),
                ("standard", &standard_output),
                ("clean", &clean_output),
                ("fresh-dynamic", &fresh_dynamic_output),
            ],
        );
        panic!(
            "generated case {case} checkpoint {checkpoint} emitted malformed output\n\
             history: {history:?}\nrequest: {request:?}\n\
             errors: {parse_errors:?}\nreplay artifacts: {}",
            saved.display(),
        );
    }
    let dynamic_observation = dynamic_observation.unwrap();
    let standard_observation = standard_observation.unwrap();
    let clean_observation = clean_observation.unwrap();

    let validation_errors = [
        ("dynamic", generated_observation_error(&dynamic_observation, model, request)),
        ("standard", generated_observation_error(&standard_observation, model, request)),
        ("clean", generated_observation_error(&clean_observation, model, request)),
    ]
    .into_iter()
    .filter_map(|(lane, error)| error.map(|error| format!("{lane}: {error}")))
    .collect::<Vec<_>>();
    let mut dynamic_results = dynamic_observation.results.clone();
    let mut clean_results = clean_observation.results.clone();
    remove_unit_test_gas(&mut dynamic_results);
    remove_unit_test_gas(&mut clean_results);
    let dynamic_matches_clean = dynamic_observation.success == clean_observation.success
        && dynamic_results == clean_results;
    let standard_matches_clean = observations_match(&standard_observation, &clean_observation);

    if validation_errors.is_empty() && dynamic_matches_clean && standard_matches_clean {
        return;
    }

    let fresh_dynamic = TestProject::new(
        &format!("cache-differential-generated-{case}-fresh-dynamic-{checkpoint}"),
        PathStyle::Dapptools,
    );
    materialize_generated(&fresh_dynamic, &files, true);
    let fresh_dynamic_output = run_forge_json(&fresh_dynamic, request.args());
    let saved = preserve_generated_failure(
        case,
        checkpoint,
        history,
        model,
        request,
        dynamic,
        standard,
        &clean,
        &fresh_dynamic,
        [
            ("dynamic", &dynamic_output),
            ("standard", &standard_output),
            ("clean", &clean_output),
            ("fresh-dynamic", &fresh_dynamic_output),
        ],
    );
    let fresh_dynamic_comparison = try_observe(&fresh_dynamic, &fresh_dynamic_output)
        .map(|fresh| observations_match(&dynamic_observation, &fresh));
    panic!(
        "generated case {case} checkpoint {checkpoint} diverged\n\
         history: {history:?}\nrequest: {request:?}\n\
         validation: {validation_errors:?}\n\
         dynamic matches clean: {dynamic_matches_clean}\n\
         standard matches clean: {standard_matches_clean}\n\
         dynamic matches fresh dynamic: {fresh_dynamic_comparison:?}\n\
         replay artifacts: {}",
        saved.display(),
    );
}

fn generated_observation_error(
    observation: &Observation,
    model: GeneratedModel,
    request: GeneratedRequest,
) -> Option<String> {
    if !observation.success {
        return Some("command failed".to_string());
    }
    let Some(suites) = observation.results.as_object() else {
        return Some("result is not an object".to_string());
    };
    if suites.len() != 1 || !suites.contains_key(request.suite()) {
        return Some(format!("unexpected suites: {:?}", suites.keys().collect::<Vec<_>>()));
    }
    let Some(tests) = suites[request.suite()].get("test_results").and_then(Value::as_object) else {
        return Some("test_results is not an object".to_string());
    };
    if tests.len() != 1 || !tests.contains_key("test_value()") {
        return Some(format!("unexpected tests: {:?}", tests.keys().collect::<Vec<_>>()));
    };
    let test = &tests["test_value()"];
    if test.get("status") != Some(&Value::String("Success".to_string())) {
        return Some(format!("unexpected test status: {:?}", test.get("status")));
    }
    let expected_data = format!("0x{:064x}", model.value());
    let logs = test.get("logs").and_then(Value::as_array);
    if logs.is_none_or(|logs| {
        logs.len() != 1 || logs[0].get("data").and_then(Value::as_str) != Some(&expected_data)
    }) {
        return Some(format!("expected one scalar observation with data {expected_data}"));
    }
    None
}

#[test]
fn filtered_request_history_matches_clean_builds() {
    const A_ARGS: &[&str] = &["test", "--match-path", "test/A.t.sol"];
    const B_ARGS: &[&str] = &["test", "--match-path", "test/B.t.sol"];
    const SCENARIO: Scenario =
        Scenario { name: "filtered-history", files: INDEPENDENT_FILES, mutations: &[] };

    let semantic = TestProject::new("cache-differential-filtered-semantic", PathStyle::Dapptools);
    let reuse = TestProject::new("cache-differential-filtered-reuse", PathStyle::Dapptools);
    materialize(&semantic, INDEPENDENT_FILES);
    materialize(&reuse, INDEPENDENT_FILES);

    assert!(run_forge_json(&semantic, A_ARGS).status.success());
    assert!(run_forge_plain(&reuse, A_ARGS).status.success());
    let cached = run_forge_plain(&reuse, A_ARGS);
    assert_eq!(reported_compiled_files(&cached).unwrap(), 0);

    let mut current =
        INDEPENDENT_FILES.iter().map(|file| (file.path, file.contents)).collect::<BTreeMap<_, _>>();
    run_checkpoint(&SCENARIO, 0, &semantic, &reuse, &current, B_ARGS, 2);

    let changed_a = r#"contract A {
    function value() external pure returns (uint256) { return 222; }
}
"#;
    current.insert("external/A.sol", changed_a);
    write_file(semantic.root(), "external/A.sol", changed_a);
    write_file(reuse.root(), "external/A.sol", changed_a);

    run_checkpoint(&SCENARIO, 1, &semantic, &reuse, &current, B_ARGS, 0);
    run_checkpoint(&SCENARIO, 2, &semantic, &reuse, &current, A_ARGS, 2);

    let changed_b = r#"contract B {
    function value() external pure returns (uint256) { return 444; }
}
"#;
    current.insert("external/B.sol", changed_b);
    write_file(semantic.root(), "external/B.sol", changed_b);
    write_file(reuse.root(), "external/B.sol", changed_b);

    run_checkpoint(&SCENARIO, 3, &semantic, &reuse, &current, A_ARGS, 0);
    run_checkpoint(&SCENARIO, 4, &semantic, &reuse, &current, B_ARGS, 2);
}

#[test]
fn shared_dependency_partial_jobs_match_clean_builds() {
    const A_ARGS: &[&str] = &["test", "--match-path", "test/A.t.sol"];
    const B_ARGS: &[&str] = &["test", "--match-path", "test/B.t.sol"];
    const SCENARIO: Scenario =
        Scenario { name: "shared-partial-jobs", files: SHARED_FILTER_FILES, mutations: &[] };

    let semantic = TestProject::new("cache-differential-shared-semantic", PathStyle::Dapptools);
    let reuse = TestProject::new("cache-differential-shared-reuse", PathStyle::Dapptools);
    materialize(&semantic, SHARED_FILTER_FILES);
    materialize(&reuse, SHARED_FILTER_FILES);

    assert!(run_forge_json(&semantic, A_ARGS).status.success());
    assert!(run_forge_plain(&reuse, A_ARGS).status.success());
    let mut current = SHARED_FILTER_FILES
        .iter()
        .map(|file| (file.path, file.contents))
        .collect::<BTreeMap<_, _>>();
    run_checkpoint(&SCENARIO, 0, &semantic, &reuse, &current, B_ARGS, 2);

    let changed_dep = r#"contract Dep {
    function value() external pure returns (uint256) { return 222; }
}
"#;
    let changed_a = r#"import {Dep} from "../external/Dep.sol";
contract ATest {
    function test_value_a() public { require(new Dep().value() == 222, "changed A"); }
}
"#;
    for (path, contents) in [("external/Dep.sol", changed_dep), ("test/A.t.sol", changed_a)] {
        current.insert(path, contents);
        write_file(semantic.root(), path, contents);
        write_file(reuse.root(), path, contents);
    }
    run_checkpoint(&SCENARIO, 1, &semantic, &reuse, &current, A_ARGS, 2);
    run_checkpoint(&SCENARIO, 2, &semantic, &reuse, &current, B_ARGS, 1);

    let changed_b = r#"import {Dep} from "../external/Dep.sol";
contract BTest {
    function test_value_b() public { require(new Dep().value() == 222, "changed B"); }
}
"#;
    current.insert("test/B.t.sol", changed_b);
    write_file(semantic.root(), "test/B.t.sol", changed_b);
    write_file(reuse.root(), "test/B.t.sol", changed_b);
    run_checkpoint(&SCENARIO, 3, &semantic, &reuse, &current, B_ARGS, 1);
}

#[test]
fn file_lifecycle_matches_clean_builds() {
    const TEST_ARGS: &[&str] = &["test"];
    const SCENARIO: Scenario =
        Scenario { name: "file-lifecycle", files: INDEPENDENT_FILES, mutations: &[] };

    let semantic = TestProject::new("cache-differential-lifecycle-semantic", PathStyle::Dapptools);
    let reuse = TestProject::new("cache-differential-lifecycle-reuse", PathStyle::Dapptools);
    materialize(&semantic, INDEPENDENT_FILES);
    materialize(&reuse, INDEPENDENT_FILES);
    assert!(run_forge_json(&semantic, TEST_ARGS).status.success());
    assert!(run_forge_plain(&reuse, TEST_ARGS).status.success());

    let mut current =
        INDEPENDENT_FILES.iter().map(|file| (file.path, file.contents)).collect::<BTreeMap<_, _>>();
    for path in ["test/B.t.sol", "external/B.sol"] {
        current.remove(path);
        fs::remove_file(semantic.root().join(path)).unwrap();
        fs::remove_file(reuse.root().join(path)).unwrap();
    }
    run_checkpoint(&SCENARIO, 0, &semantic, &reuse, &current, TEST_ARGS, 2);

    let dependency_c = r#"contract C {
    function value() external pure returns (uint256) { return 555; }
}
"#;
    let test_c = r#"import {C} from "../external/C.sol";
contract CTest {
    function test_value_c() public { require(new C().value() == 555, "changed C"); }
}
"#;
    for (path, contents) in [("external/C.sol", dependency_c), ("test/C.t.sol", test_c)] {
        current.insert(path, contents);
        write_file(semantic.root(), path, contents);
        write_file(reuse.root(), path, contents);
    }
    run_checkpoint(&SCENARIO, 1, &semantic, &reuse, &current, TEST_ARGS, 4);

    for path in ["test/A.t.sol", "external/A.sol"] {
        current.remove(path);
        fs::remove_file(semantic.root().join(path)).unwrap();
        fs::remove_file(reuse.root().join(path)).unwrap();
    }
    run_checkpoint(&SCENARIO, 2, &semantic, &reuse, &current, TEST_ARGS, 2);

    let dependency_a = r#"contract A {
    function value() external pure returns (uint256) { return 777; }
}
"#;
    let test_a = r#"import {A} from "../external/A.sol";
contract ATest {
    function test_value_a() public { require(new A().value() == 777, "changed A"); }
}
"#;
    for (path, contents) in [("external/A.sol", dependency_a), ("test/A2.t.sol", test_a)] {
        current.insert(path, contents);
        write_file(semantic.root(), path, contents);
        write_file(reuse.root(), path, contents);
    }
    run_checkpoint(&SCENARIO, 3, &semantic, &reuse, &current, TEST_ARGS, 4);
}

fn run_checkpoint(
    scenario: &Scenario,
    index: usize,
    semantic: &TestProject,
    reuse: &TestProject,
    current: &BTreeMap<&'static str, &'static str>,
    args: &[&str],
    expected_compiled_files: usize,
) {
    let incremental_output = run_forge_json(semantic, args);
    let incremental_compile = run_forge_plain(reuse, args);
    let actual_compiled_files = reported_compiled_files(&incremental_compile).unwrap();

    let clean = TestProject::new(
        &format!("cache-differential-{}-clean-{index}", scenario.name),
        PathStyle::Dapptools,
    );
    let files =
        current.iter().map(|(&path, &contents)| FileSpec { path, contents }).collect::<Vec<_>>();
    materialize(&clean, &files);
    let clean_output = run_forge_json(&clean, args);

    let incremental_observation = observe(semantic, &incremental_output);
    let clean_observation = observe(&clean, &clean_output);
    if actual_compiled_files != expected_compiled_files
        || !observations_match(&incremental_observation, &clean_observation)
    {
        let saved = preserve_failure(scenario, index, semantic, reuse, &clean);
        panic!(
            "{} checkpoint {index}: incremental result differs from clean result\n\
             replay artifacts: {}\n\
             command: {args:?}\n\
             compiled files: {actual_compiled_files}, expected: {expected_compiled_files}\n\
             incremental: {incremental_observation:#?}\nclean: {clean_observation:#?}",
            scenario.name,
            saved.display(),
        );
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

    let baseline = run_forge_json(&semantic, &["test"]);
    assert!(baseline.status.success(), "{}: warmup failed: {baseline:?}", scenario.name);
    let reuse_baseline = run_forge_plain(&reuse, &["test"]);
    assert!(reuse_baseline.status.success(), "{}: reuse warmup failed", scenario.name);
    let cached = run_forge_plain(&reuse, &["test"]);
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

        let incremental_output = run_forge_json(&semantic, &["test"]);
        let incremental_compile = run_forge_plain(&reuse, &["test"]);
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
        let clean_output = run_forge_json(&clean, &["test"]);

        let incremental_observation = observe(&semantic, &incremental_output);
        let clean_observation = observe(&clean, &clean_output);
        if actual_compiled_files != mutation.expected_compiled_files
            || !observations_match(&incremental_observation, &clean_observation)
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
    materialize_with_linking(project, files, true);
}

fn materialize_with_linking(project: &TestProject, files: &[FileSpec], dynamic_test_linking: bool) {
    write_file(
        project.root(),
        "foundry.toml",
        &format!(
            r#"[profile.default]
dynamic_test_linking = {dynamic_test_linking}
solc = "{SOLC_VERSION}"
bytecode_hash = "none"
cbor_metadata = false
cache = true
force = false
src = "src"
test = "test"
out = "out"
cache_path = "cache"
libs = []
"#,
        ),
    );
    for file in files {
        write_file(project.root(), file.path, file.contents);
    }
}

fn render_generated_files(model: GeneratedModel) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        (
            "src/X.sol",
            format!(
                "contract X {{ function value() external pure returns (uint256) {{ return {}; }} }}\n",
                model.value()
            ),
        ),
        ("test/A.t.sol", render_generated_test("A", model.a_edge)),
        ("test/B.t.sol", render_generated_test("B", model.b_edge)),
    ])
}

fn render_generated_test(name: &str, edge: GeneratedEdge) -> String {
    let (helper, deployment) = match edge {
        GeneratedEdge::Direct => (String::new(), "new X()".to_string()),
        GeneratedEdge::FreeFunction => (
            format!("function make{name}() returns (X) {{ return new X(); }}\n"),
            format!("make{name}()"),
        ),
    };
    format!(
        "import {{X}} from \"../src/X.sol\";\n\
         {helper}\
         contract {name}Test {{\n\
             event Observed(uint256 value);\n\
             function test_value() public {{ emit Observed({deployment}.value()); }}\n\
         }}\n"
    )
}

fn materialize_generated(
    project: &TestProject,
    files: &BTreeMap<&'static str, String>,
    dynamic_test_linking: bool,
) {
    materialize_with_linking(project, &[], dynamic_test_linking);
    for (&path, contents) in files {
        write_file(project.root(), path, contents);
    }
}

fn apply_generated_file_diff(
    project: &TestProject,
    before: &BTreeMap<&'static str, String>,
    after: &BTreeMap<&'static str, String>,
) {
    for &path in before.keys().filter(|path| !after.contains_key(*path)) {
        fs::remove_file(project.root().join(path)).unwrap();
    }
    for (&path, contents) in after {
        if before.get(path) != Some(contents) {
            write_file(project.root(), path, contents);
        }
    }
}

fn write_file(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn run_forge_json(project: &TestProject, args: &[&str]) -> Output {
    forge_command(project)
        .args(args)
        .args(["--profile", "default", "--config-path"])
        .arg(project.root().join("foundry.toml"))
        .args(["--json", "-vv"])
        .output()
        .unwrap()
}

fn run_forge_plain(project: &TestProject, args: &[&str]) -> Output {
    forge_command(project)
        .args(args)
        .args(["--profile", "default", "--config-path"])
        .arg(project.root().join("foundry.toml"))
        .output()
        .unwrap()
}

fn forge_command(project: &TestProject) -> std::process::Command {
    let mut command = project.forge_bin();
    for (key, _) in std::env::vars_os() {
        let key_text = key.to_string_lossy();
        if key_text.starts_with("FOUNDRY_") || key_text.starts_with("DAPP_") {
            command.env_remove(key);
        }
    }
    command
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
    try_observe(project, output).unwrap_or_else(|error| panic!("{error}"))
}

fn try_observe(project: &TestProject, output: &Output) -> Result<Observation, String> {
    let mut results = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "forge did not emit JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    })?;
    remove_result_durations(&mut results);
    Ok(Observation {
        success: output.status.success(),
        results,
        artifacts: collect_artifacts(project.artifacts()),
    })
}

fn observations_match(incremental: &Observation, clean: &Observation) -> bool {
    incremental.success == clean.success
        && incremental.results == clean.results
        && clean
            .artifacts
            .iter()
            .all(|(path, artifact)| incremental.artifacts.get(path) == Some(artifact))
}

fn remove_result_durations(value: &mut Value) {
    let Some(suites) = value.as_object_mut() else { return };
    for suite in suites.values_mut().filter_map(Value::as_object_mut) {
        suite.remove("duration");
        if let Some(tests) = suite.get_mut("test_results").and_then(Value::as_object_mut) {
            for test in tests.values_mut().filter_map(Value::as_object_mut) {
                test.remove("duration");
            }
        }
    }
}

fn remove_unit_test_gas(value: &mut Value) {
    let Some(suites) = value.as_object_mut() else { return };
    for suite in suites.values_mut().filter_map(Value::as_object_mut) {
        let Some(tests) = suite.get_mut("test_results").and_then(Value::as_object_mut) else {
            continue;
        };
        for test in tests.values_mut().filter_map(Value::as_object_mut) {
            if let Some(unit) = test
                .get_mut("kind")
                .and_then(Value::as_object_mut)
                .and_then(|kind| kind.get_mut("Unit"))
                .and_then(Value::as_object_mut)
            {
                unit.remove("gas");
            }
        }
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
        "immutableReferences": normalized_immutable_references(
            get(&["deployedBytecode", "immutableReferences"]),
        ),
    })
}

fn normalized_immutable_references(references: Value) -> Value {
    let Value::Object(references) = references else { return references };
    let mut groups = references.into_values().collect::<Vec<_>>();
    groups.sort_by_key(Value::to_string);
    Value::Array(groups)
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

#[allow(clippy::too_many_arguments)]
fn preserve_generated_failure<const N: usize>(
    case: usize,
    checkpoint: usize,
    history: &[GeneratedAction],
    model: GeneratedModel,
    request: GeneratedRequest,
    dynamic: &TestProject,
    standard: &TestProject,
    clean: &TestProject,
    fresh_dynamic: &TestProject,
    outputs: [(&str, &Output); N],
) -> PathBuf {
    let root = dynamic
        .foundry_bin_path("forge")
        .parent()
        .unwrap()
        .join("cache-differential")
        .join(format!("generated-{case}-{checkpoint}"));
    let _ = fs::remove_dir_all(&root);
    copy_tree(dynamic.root(), &root.join("dynamic"));
    copy_tree(standard.root(), &root.join("standard"));
    copy_tree(clean.root(), &root.join("clean"));
    copy_tree(fresh_dynamic.root(), &root.join("fresh-dynamic"));
    let output_dir = root.join("outputs");
    fs::create_dir_all(&output_dir).unwrap();
    for (lane, output) in outputs {
        fs::write(output_dir.join(format!("{lane}.stdout")), &output.stdout).unwrap();
        fs::write(output_dir.join(format!("{lane}.stderr")), &output.stderr).unwrap();
    }
    fs::write(
        root.join("replay.json"),
        serde_json::to_vec_pretty(&json!({
            "version": 1,
            "generator": "partial-request-core-v1",
            "case": case,
            "checkpoint": checkpoint,
            "history": history.iter().map(|action| action.name()).collect::<Vec<_>>(),
            "model": {
                "dependencyValue": model.value(),
                "aEdge": model.a_edge.name(),
                "bEdge": model.b_edge.name(),
            },
            "request": request.suite(),
            "forge": dynamic.foundry_bin_path("forge"),
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("realized-inputs.json"),
        serde_json::to_vec_pretty(&generated_realized_inputs(history)).unwrap(),
    )
    .unwrap();
    root
}

fn generated_realized_inputs(history: &[GeneratedAction]) -> Value {
    let mut model = GeneratedModel::default();
    let initial = render_generated_files(model);
    let mut before = initial.clone();
    let steps = history
        .iter()
        .map(|&action| {
            let request = model.apply(action);
            let after = render_generated_files(model);
            let writes = after
                .iter()
                .filter(|(path, contents)| before.get(*path) != Some(*contents))
                .map(|(&path, contents)| (path, contents.clone()))
                .collect::<BTreeMap<_, _>>();
            let deletes = before
                .keys()
                .filter(|path| !after.contains_key(*path))
                .copied()
                .collect::<Vec<_>>();
            before = after;
            json!({
                "action": action.name(),
                "request": request.suite(),
                "writes": writes,
                "deletes": deletes,
            })
        })
        .collect::<Vec<_>>();
    json!({"initial": initial, "steps": steps})
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

#[test]
fn normalizes_only_execution_metrics() {
    let mut results = json!({
        "test/Impl.t.sol:ImplTest": {
            "duration": "1ms",
            "test_results": {
                "test_value()": {
                    "duration": "2ms",
                    "logs": ["observable"],
                    "kind": {"Unit": {"gas": 123}},
                    "gas_snapshots": {
                        "gas": {"duration": "user value"}
                    }
                }
            }
        }
    });
    remove_result_durations(&mut results);
    remove_unit_test_gas(&mut results);

    assert_eq!(
        results,
        json!({
            "test/Impl.t.sol:ImplTest": {
                "test_results": {
                    "test_value()": {
                        "logs": ["observable"],
                        "kind": {"Unit": {}},
                        "gas_snapshots": {
                            "gas": {"duration": "user value"}
                        }
                    }
                }
            }
        })
    );
}

#[test]
fn generated_observations_reject_extra_tests() {
    let observation = Observation {
        success: true,
        results: json!({
            "test/A.t.sol:ATest": {
                "test_results": {
                    "test_value()": {
                        "status": "Success",
                        "logs": [{
                            "data": "0x0000000000000000000000000000000000000000000000000000000000000064"
                        }]
                    },
                    "test_unexpected()": {"status": "Success", "logs": []}
                }
            }
        }),
        artifacts: BTreeMap::new(),
    };

    assert!(
        generated_observation_error(&observation, GeneratedModel::default(), GeneratedRequest::A,)
            .unwrap()
            .starts_with("unexpected tests:")
    );
}
