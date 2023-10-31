use crate::{init_tracing, TestCommand};
use alloy_primitives::{Address, U256};
use ethers::prelude::{Middleware, NameOrAddress};
use eyre::Result;
use foundry_common::{get_http_provider, RetryProvider};
use foundry_utils::types::{ToAlloy, ToEthers};
use std::{collections::BTreeMap, path::Path, str::FromStr};

pub const BROADCAST_TEST_PATH: &str = "src/Broadcast.t.sol";

/// A helper struct to test forge script scenarios
pub struct ScriptTester {
    pub accounts_pub: Vec<Address>,
    pub accounts_priv: Vec<String>,
    pub provider: Option<RetryProvider>,
    pub nonces: BTreeMap<u32, U256>,
    pub address_nonces: BTreeMap<Address, U256>,
    pub cmd: TestCommand,
}

impl ScriptTester {
    /// Creates a new instance of a Tester for the given contract
    pub fn new(
        mut cmd: TestCommand,
        endpoint: Option<&str>,
        project_root: &Path,
        target_contract: &str,
    ) -> Self {
        init_tracing();
        ScriptTester::copy_testdata(project_root).unwrap();
        cmd.set_current_dir(project_root);

        cmd.args([
            "script",
            "-R",
            "ds-test/=lib/",
            target_contract,
            "--root",
            project_root.to_str().unwrap(),
            "-vvvvv",
        ]);

        let mut provider = None;
        if let Some(endpoint) = endpoint {
            cmd.args(["--fork-url", endpoint]);
            provider = Some(get_http_provider(endpoint))
        }

        ScriptTester {
            accounts_pub: vec![
                Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
                Address::from_str("0x70997970C51812dc3A010C7d01b50e0d17dc79C8").unwrap(),
                Address::from_str("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC").unwrap(),
            ],
            accounts_priv: vec![
                "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string(),
                "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".to_string(),
                "5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a".to_string(),
            ],
            provider,
            nonces: BTreeMap::default(),
            address_nonces: BTreeMap::default(),
            cmd,
        }
    }

    /// Creates a new instance of a Tester for the `broadcast` test at the given `project_root` by
    /// configuring the `TestCommand` with script
    pub fn new_broadcast(cmd: TestCommand, endpoint: &str, project_root: &Path) -> Self {
        let target_contract = project_root.join(BROADCAST_TEST_PATH).to_string_lossy().to_string();

        // copy the broadcast test
        let testdata = Self::testdata_path();
        std::fs::copy(testdata + "/cheats/Broadcast.t.sol", project_root.join(BROADCAST_TEST_PATH))
            .expect("Failed to initialize broadcast contract");

        Self::new(cmd, Some(endpoint), project_root, &target_contract)
    }

    /// Creates a new instance of a Tester for the `broadcast` test at the given `project_root` by
    /// configuring the `TestCommand` with script without an endpoint
    pub fn new_broadcast_without_endpoint(cmd: TestCommand, project_root: &Path) -> Self {
        let target_contract = project_root.join(BROADCAST_TEST_PATH).to_string_lossy().to_string();

        // copy the broadcast test
        let testdata = Self::testdata_path();
        std::fs::copy(testdata + "/cheats/Broadcast.t.sol", project_root.join(BROADCAST_TEST_PATH))
            .expect("Failed to initialize broadcast contract");

        Self::new(cmd, None, project_root, &target_contract)
    }

    /// Returns the path to the dir that contains testdata
    fn testdata_path() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata").into()
    }

    /// Initialises the test contracts by copying them into the workspace
    fn copy_testdata(current_dir: &Path) -> Result<()> {
        let testdata = Self::testdata_path();
        std::fs::copy(testdata.clone() + "/cheats/Vm.sol", current_dir.join("src/Vm.sol"))?;
        std::fs::copy(testdata + "/lib/ds-test/src/test.sol", current_dir.join("lib/test.sol"))?;

        Ok(())
    }

    pub async fn load_private_keys(
        &mut self,
        keys_indexes: impl IntoIterator<Item = u32>,
    ) -> &mut Self {
        for index in keys_indexes {
            self.cmd.args(["--private-keys", &self.accounts_priv[index as usize]]);

            if let Some(provider) = &self.provider {
                let nonce = provider
                    .get_transaction_count(
                        NameOrAddress::Address(self.accounts_pub[index as usize].to_ethers()),
                        None,
                    )
                    .await
                    .unwrap();
                self.nonces.insert(index, nonce.to_alloy());
            }
        }
        self
    }

    pub async fn load_addresses(&mut self, addresses: Vec<Address>) -> &mut Self {
        for address in addresses {
            let nonce = self
                .provider
                .as_ref()
                .unwrap()
                .get_transaction_count(NameOrAddress::Address(address.to_ethers()), None)
                .await
                .unwrap();
            self.address_nonces.insert(address, nonce.to_alloy());
        }
        self
    }

    pub fn add_deployer(&mut self, index: u32) -> &mut Self {
        self.sender(self.accounts_pub[index as usize])
    }

    /// Adds given address as sender
    pub fn sender(&mut self, addr: Address) -> &mut Self {
        self.cmd.args(["--sender", addr.to_string().as_str()]);
        self
    }

    pub fn add_sig(&mut self, contract_name: &str, sig: &str) -> &mut Self {
        self.cmd.args(["--tc", contract_name, "--sig", sig]);
        self
    }

    /// Adds the `--unlocked` flag
    pub fn unlocked(&mut self) -> &mut Self {
        self.cmd.arg("--unlocked");
        self
    }

    pub fn simulate(&mut self, expected: ScriptOutcome) -> &mut Self {
        self.run(expected)
    }

    pub fn broadcast(&mut self, expected: ScriptOutcome) -> &mut Self {
        self.cmd.arg("--broadcast");
        self.run(expected)
    }

    pub fn resume(&mut self, expected: ScriptOutcome) -> &mut Self {
        self.cmd.arg("--resume");
        self.run(expected)
    }

    /// `[(private_key_slot, expected increment)]`
    pub async fn assert_nonce_increment(
        &mut self,
        keys_indexes: impl IntoIterator<Item = (u32, u32)>,
    ) -> &mut Self {
        for (private_key_slot, expected_increment) in keys_indexes {
            let addr = self.accounts_pub[private_key_slot as usize];
            let nonce = self
                .provider
                .as_ref()
                .unwrap()
                .get_transaction_count(NameOrAddress::Address(addr.to_ethers()), None)
                .await
                .unwrap();
            let prev_nonce = self.nonces.get(&private_key_slot).unwrap();

            assert_eq!(
                nonce,
                (prev_nonce + U256::from(expected_increment)).to_ethers(),
                "nonce not incremented correctly for {addr}: \
                 {prev_nonce} + {expected_increment} != {nonce}"
            );
        }
        self
    }

    /// In Vec<(address, expected increment)>
    pub async fn assert_nonce_increment_addresses(
        &mut self,
        address_indexes: Vec<(Address, u32)>,
    ) -> &mut Self {
        for (address, expected_increment) in address_indexes {
            let nonce = self
                .provider
                .as_ref()
                .unwrap()
                .get_transaction_count(NameOrAddress::Address(address.to_ethers()), None)
                .await
                .unwrap();
            let prev_nonce = self.address_nonces.get(&address).unwrap();

            assert_eq!(nonce, (prev_nonce + U256::from(expected_increment)).to_ethers());
        }
        self
    }

    pub fn run(&mut self, expected: ScriptOutcome) -> &mut Self {
        let (stdout, stderr) = self.cmd.unchecked_output_lossy();
        trace!(target: "tests", "STDOUT\n{stdout}\n\nSTDERR\n{stderr}");

        let output = if expected.is_err() { &stderr } else { &stdout };
        if !output.contains(expected.as_str()) {
            let which = if expected.is_err() { "stderr" } else { "stdout" };
            panic!(
                "--STDOUT--\n{stdout}\n\n--STDERR--\n{stderr}\n\n--EXPECTED--\n{:?} in {which}",
                expected.as_str()
            );
        }

        self
    }

    pub fn slow(&mut self) -> &mut Self {
        self.cmd.arg("--slow");
        self
    }

    pub fn args(&mut self, args: Vec<String>) -> &mut Self {
        self.cmd.args(args);
        self
    }
}

/// Various `forge` script results
#[derive(Debug)]
pub enum ScriptOutcome {
    OkNoEndpoint,
    OkSimulation,
    OkBroadcast,
    WarnSpecifyDeployer,
    MissingSender,
    MissingWallet,
    StaticCallNotAllowed,
    FailedScript,
    UnsupportedLibraries,
    ErrorSelectForkOnBroadcast,
}

impl ScriptOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OkNoEndpoint => "If you wish to simulate on-chain transactions pass a RPC URL.",
            Self::OkSimulation => "SIMULATION COMPLETE. To broadcast these",
            Self::OkBroadcast => "ONCHAIN EXECUTION COMPLETE & SUCCESSFUL",
            Self::WarnSpecifyDeployer => "You have more than one deployer who could predeploy libraries. Using `--sender` instead.",
            Self::MissingSender => "You seem to be using Foundry's default sender. Be sure to set your own --sender",
            Self::MissingWallet => "No associated wallet",
            Self::StaticCallNotAllowed => "staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead",
            Self::FailedScript => "Script failed.",
            Self::UnsupportedLibraries => "Multi chain deployment does not support library linking at the moment.",
            Self::ErrorSelectForkOnBroadcast => "cannot select forks during a broadcast",
        }
    }

    pub fn is_err(&self) -> bool {
        match self {
            ScriptOutcome::OkNoEndpoint |
            ScriptOutcome::OkSimulation |
            ScriptOutcome::OkBroadcast |
            ScriptOutcome::WarnSpecifyDeployer => false,
            ScriptOutcome::MissingSender |
            ScriptOutcome::MissingWallet |
            ScriptOutcome::StaticCallNotAllowed |
            ScriptOutcome::UnsupportedLibraries |
            ScriptOutcome::ErrorSelectForkOnBroadcast |
            ScriptOutcome::FailedScript => true,
        }
    }
}
