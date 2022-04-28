//! Contains various tests for checking the `forge create` subcommand

use crate::utils::{self, EnvExternalities};
use ethers::{solc::remappings::Remapping, types::Address};
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};
use foundry_config::Config;
use std::str::FromStr;

/// This will insert _dummy_ contract that uses a library
///
/// **NOTE** This is intended to be linked against a random address and won't actually work. The
/// purpose of this is _only_ to make sure we can deploy contracts linked against addresses.
///
/// This will create a library `remapping/MyLib.sol:MyLib`
fn setup_with_remapping(prj: &mut TestProject) {
    // explicitly set remapping and libraries
    let config = Config {
        remappings: vec![Remapping::from_str("remapping/=lib/remapping/").unwrap().into()],
        libraries: vec![format!("remapping/MyLib.sol:MyLib:{:?}", Address::random())],
        ..Default::default()
    };
    prj.write_config(config);

    prj.inner()
        .add_source(
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

    prj.inner()
        .add_lib(
            "remapping/MyLib",
            r#"
// SPDX-License-Identifier: MIT
library MyLib {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}
"#,
        )
        .unwrap();
}

fn create_on_chain(info: Option<EnvExternalities>, mut prj: TestProject, mut cmd: TestCommand) {
    if let Some(info) = info {
        setup_with_remapping(&mut prj);
        cmd.arg("create");
        cmd.args(info.create_args()).arg("src/LinkTest.sol:LinkTest");

        let out = cmd.stdout_lossy();
        let _address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));
    }
}

// tests `forge` create on goerli if correct env vars are set
forgetest!(can_create_on_goerli, |prj: TestProject, cmd: TestCommand| {
    create_on_chain(EnvExternalities::goerli(), prj, cmd);
});
