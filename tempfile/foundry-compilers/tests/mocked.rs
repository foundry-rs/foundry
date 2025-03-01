//! mocked project tests

use foundry_compilers::{
    compilers::multi::MultiCompiler,
    project_util::{
        mock::{MockProjectGenerator, MockProjectSettings, MockProjectSkeleton},
        TempProject,
    },
};
use foundry_compilers_core::error::Result;

// default version to use
const DEFAULT_VERSION: &str = "^0.8.10";

struct MockSettings {
    settings: MockProjectSettings,
    version: &'static str,
}

impl From<MockProjectSettings> for MockSettings {
    fn from(settings: MockProjectSettings) -> Self {
        Self { settings, version: DEFAULT_VERSION }
    }
}
impl From<(MockProjectSettings, &'static str)> for MockSettings {
    fn from(input: (MockProjectSettings, &'static str)) -> Self {
        Self { settings: input.0, version: input.1 }
    }
}

/// Helper function to run a test and report the used generator if the closure failed.
fn run_mock(
    settings: impl Into<MockSettings>,
    f: impl FnOnce(&mut TempProject, &MockProjectGenerator) -> Result<()>,
) -> TempProject {
    let MockSettings { settings, version } = settings.into();
    let gen = MockProjectGenerator::new(&settings);
    let mut project = TempProject::dapptools().unwrap();
    let remappings = gen.remappings_at(project.root());
    project.paths_mut().remappings.extend(remappings);
    project.mock(&gen, version).unwrap();

    if let Err(err) = f(&mut project, &gen) {
        panic!(
            "mock failed: `{}` with mock settings:\n {}",
            err,
            serde_json::to_string(&gen).unwrap()
        );
    }

    project
}

/// Runs a basic set of tests for the given settings
fn run_basic(settings: impl Into<MockSettings>) {
    let settings = settings.into();
    let version = settings.version;
    run_mock(settings, |project, _| {
        project.ensure_no_errors_recompile_unchanged()?;
        project.add_basic_source("Dummy", version)?;
        project.ensure_changed()?;
        Ok(())
    });
}

#[test]
fn can_compile_mocked_random() {
    run_basic(MockProjectSettings::random());
}

// compile a bunch of random projects
#[test]
fn can_compile_mocked_multi() {
    for _ in 0..10 {
        run_basic(MockProjectSettings::random());
    }
}

#[test]
fn can_compile_mocked_large() {
    run_basic(MockProjectSettings::large())
}

#[test]
fn can_compile_mocked_modified() {
    run_mock(MockProjectSettings::random(), |project, gen| {
        project.ensure_no_errors_recompile_unchanged()?;
        // modify a random file
        gen.modify_file(gen.used_file_ids().count() / 2, project.paths(), DEFAULT_VERSION)?;
        project.ensure_changed()?;
        project.artifacts_snapshot()?.assert_artifacts_essentials_present();
        Ok(())
    });
}

#[test]
fn can_compile_mocked_modified_all() {
    run_mock(MockProjectSettings::random(), |project, gen| {
        project.ensure_no_errors_recompile_unchanged()?;
        // modify a random file
        for id in gen.used_file_ids() {
            gen.modify_file(id, project.paths(), DEFAULT_VERSION)?;
            project.ensure_changed()?;
            project.artifacts_snapshot()?.assert_artifacts_essentials_present();
        }
        Ok(())
    });
}

// a test useful to manually debug a serialized skeleton
#[test]
fn can_compile_skeleton() {
    let mut project = TempProject::<MultiCompiler>::dapptools().unwrap();
    let s = r#"{"files":[{"id":0,"name":"SourceFile0","imports":[{"External":[0,1]},{"External":[3,4]}],"lib_id":null,"emit_artifacts":true},{"id":1,"name":"SourceFile1","imports":[],"lib_id":0,"emit_artifacts":true},{"id":2,"name":"SourceFile2","imports":[],"lib_id":1,"emit_artifacts":true},{"id":3,"name":"SourceFile3","imports":[],"lib_id":2,"emit_artifacts":true},{"id":4,"name":"SourceFile4","imports":[],"lib_id":3,"emit_artifacts":true}],"libraries":[{"name":"Lib0","id":0,"offset":1,"num_files":1},{"name":"Lib1","id":1,"offset":2,"num_files":1},{"name":"Lib2","id":2,"offset":3,"num_files":1},{"name":"Lib3","id":3,"offset":4,"num_files":1}]}"#;
    let gen: MockProjectGenerator = serde_json::from_str::<MockProjectSkeleton>(s).unwrap().into();
    let remappings = gen.remappings_at(project.root());
    project.paths_mut().remappings.extend(remappings);
    project.mock(&gen, DEFAULT_VERSION).unwrap();

    // mattsse: helper to show what's being generated
    // gen.write_to(&foundry_compilers::ProjectPathsConfig::dapptools("./skeleton").unwrap(),
    // DEFAULT_VERSION).unwrap();

    let compiled = project.compile().unwrap();
    compiled.assert_success();
    assert!(!compiled.is_unchanged());
    for id in gen.used_file_ids() {
        gen.modify_file(id, project.paths(), DEFAULT_VERSION).unwrap();
        project.ensure_changed().unwrap();
        project.artifacts_snapshot().unwrap().assert_artifacts_essentials_present();
    }
}
