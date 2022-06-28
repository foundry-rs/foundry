use ethers::{
    abi::Address,
    prelude::{Http, Middleware, NameOrAddress, Provider, U256},
    utils::hex,
};

use std::{collections::BTreeMap, path::Path, str::FromStr};

use crate::TestCommand;

pub const BROADCAST_TEST_PATH: &str = "src/Broadcast.t.sol";

pub enum ScriptOutcome {
    OkSimulation,
    OkBroadcast,
    WarnSpecifyDeployer,
    MissingSender,
    MissingWallet,
    FailedScript,
}

impl ScriptOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScriptOutcome::OkSimulation => "SIMULATION COMPLETE. To broadcast these",
            ScriptOutcome::OkBroadcast => "ONCHAIN EXECUTION COMPLETE & SUCCESSFUL",
            ScriptOutcome::WarnSpecifyDeployer => "You have more than one deployer who could predeploy libraries. Using `--sender` instead.",
            ScriptOutcome::MissingSender => "You seem to be using Foundry's default sender. Be sure to set your own --sender",
            ScriptOutcome::MissingWallet => "No associated wallet",
            ScriptOutcome::FailedScript => "Script failed.",
        }
    }

    pub fn is_err(&self) -> bool {
        match self {
            ScriptOutcome::OkSimulation => false,
            ScriptOutcome::OkBroadcast => false,
            ScriptOutcome::WarnSpecifyDeployer => false,
            ScriptOutcome::MissingSender => true,
            ScriptOutcome::MissingWallet => true,
            ScriptOutcome::FailedScript => true,
        }
    }
}

/// A helper struct to test forge script scenarios
pub struct ScriptTester {
    pub accounts_pub: Vec<Address>,
    pub accounts_priv: Vec<String>,
    pub provider: Provider<Http>,
    pub nonces: BTreeMap<u32, U256>,
    pub cmd: TestCommand,
}

impl ScriptTester {
    pub fn new(mut cmd: TestCommand, endpoint: &str, current_dir: &Path) -> Self {
        ScriptTester::link_testdata(current_dir).unwrap();
        cmd.set_current_dir(current_dir);

        let target_contract = current_dir.join(BROADCAST_TEST_PATH).to_string_lossy().to_string();

        cmd.args([
            "script",
            "-r",
            "ds-test/=lib/",
            target_contract.as_str(),
            "--root",
            current_dir.to_str().unwrap(),
            "--fork-url",
            endpoint,
            "-vvvvv",
        ]);

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
            provider: Provider::<Http>::try_from(endpoint).unwrap(),
            nonces: BTreeMap::default(),
            cmd,
        }
    }

    fn link_testdata(current_dir: &Path) -> eyre::Result<()> {
        let testdata = format!("{}/../../testdata", env!("CARGO_MANIFEST_DIR"));
        std::fs::hard_link(
            testdata.clone() + "/cheats/Cheats.sol",
            current_dir.join("src/Cheats.sol"),
        )?;
        std::fs::hard_link(
            testdata.clone() + "/cheats/Broadcast.t.sol",
            current_dir.join(BROADCAST_TEST_PATH),
        )?;
        std::fs::hard_link(
            testdata + "/lib/ds-test/src/test.sol",
            current_dir.join("lib/test.sol"),
        )?;

        Ok(())
    }

    pub async fn load_private_keys(&mut self, keys_indexes: Vec<u32>) -> &mut Self {
        for index in keys_indexes {
            self.cmd.args(["--private-keys", &self.accounts_priv[index as usize]]);
            let nonce = self
                .provider
                .get_transaction_count(
                    NameOrAddress::Address(self.accounts_pub[index as usize]),
                    None,
                )
                .await
                .unwrap();
            self.nonces.insert(index, nonce);
        }
        self
    }

    pub fn add_deployer(&mut self, index: u32) -> &mut Self {
        self.cmd.args([
            "--sender",
            &format!("0x{}", hex::encode(&self.accounts_pub[index as usize].as_bytes())),
        ]);
        self
    }

    pub fn add_sig(&mut self, contract_name: &str, sig: &str) -> &mut Self {
        self.cmd.args(["--tc", contract_name, "--sig", sig]);
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

    /// In Vec<(private_key_slot, expected increment)>
    pub async fn assert_nonce_increment(&mut self, keys_indexes: Vec<(u32, u32)>) -> &mut Self {
        for (private_key_slot, expected_increment) in keys_indexes {
            let nonce = self
                .provider
                .get_transaction_count(
                    NameOrAddress::Address(self.accounts_pub[private_key_slot as usize]),
                    None,
                )
                .await
                .unwrap();
            let prev_nonce = self.nonces.get(&private_key_slot).unwrap();

            assert_eq!(nonce, prev_nonce + U256::from(expected_increment));
        }
        self
    }

    pub fn run(&mut self, expected: ScriptOutcome) -> &mut Self {
        let output =
            if expected.is_err() { self.cmd.stderr_lossy() } else { self.cmd.stdout_lossy() };

        if !output.contains(expected.as_str()) {
            panic!("OUTPUT: {}\n\nEXPECTED: {}", output, expected.as_str());
        }
        self
    }

    pub fn slow(&mut self) -> &mut Self {
        self.cmd.arg("--slow");
        self
    }
}
