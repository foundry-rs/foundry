use foundry_test_utils::util::{RemoteProject, setup_forge_remote};

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc", "--build"]).assert_success();
}
