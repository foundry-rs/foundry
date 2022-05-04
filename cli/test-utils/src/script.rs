use ethers::{
    abi::Address,
    prelude::{Http, Middleware, Provider, U256},
};

use std::{collections::BTreeMap, str::FromStr};

use crate::TestCommand;

/// An helper structure to test forge script scenarios
pub struct ScriptTester {
    pub accounts_pub: Vec<String>,
    pub accounts_priv: Vec<String>,
    pub provider: Provider<Http>,
    pub nonces: BTreeMap<u32, U256>,
    pub cmd: TestCommand,
    pub err: bool,
}

impl ScriptTester {
    pub fn new(mut cmd: TestCommand) -> Self {
        let current_dir = std::env::current_dir().unwrap();
        let root_path = current_dir.join("../testdata");
        let root = root_path.to_string_lossy().to_string();
        let target_contract =
            root_path.join("./cheats/Broadcast.t.sol").to_string_lossy().to_string();
        let url = "http://127.0.0.1:8545";

        cmd.args([
            "script",
            target_contract.as_str(),
            "--root",
            root.as_str(),
            "--fork-url",
            url,
            "-vvv",
            "--legacy", // only necessary for ganache
        ]);

        ScriptTester {
            accounts_pub: vec![
                "0x90F8bf6A479f320ead074411a4B0e7944Ea8c9C1".to_string(),
                "0xFFcf8FDEE72ac11b5c542428B35EEF5769C409f0".to_string(),
            ],
            accounts_priv: vec![
                "4f3edf983ac636a65a842ce7c78d9aa706d3b113bce9c46f30d7d21715b23b1d".to_string(),
                "6cbed15c793ce57650b9877cf6fa156fbef513c4e6134f022a85b1ffdd59b2a1".to_string(),
            ],
            provider: Provider::<Http>::try_from(url).unwrap(),
            nonces: BTreeMap::default(),
            err: false,
            cmd,
        }
    }

    pub async fn load_private_keys(&mut self, keys_indexes: Vec<u32>) -> &mut Self {
        for index in keys_indexes {
            self.cmd.args(["--private-keys", &self.accounts_priv[index as usize]]);
            let nonce = self
                .provider
                .get_transaction_count(
                    Address::from_str(&self.accounts_pub[index as usize]).unwrap(),
                    None,
                )
                .await
                .unwrap();
            self.nonces.insert(index, nonce);
        }
        self
    }

    pub fn add_sender(&mut self, index: u32) -> &mut Self {
        self.cmd.args(["--sender", &self.accounts_pub[index as usize]]);
        self
    }

    pub fn add_sig(&mut self, contract_name: &str, sig: &str) -> &mut Self {
        self.cmd.args(["--tc", contract_name, "--sig", sig]);
        self
    }

    pub fn sim(&mut self, expected: &str) -> &mut Self {
        self.run(expected)
    }

    pub fn execute(&mut self, expected: &str) -> &mut Self {
        self.cmd.arg("--execute");
        self.run(expected)
    }

    pub fn resume(&mut self, expected: &str) -> &mut Self {
        self.cmd.arg("--resume");
        self.run(expected)
    }

    pub fn force_resume(&mut self, expected: &str) -> &mut Self {
        self.cmd.arg("--force-resume");
        self.run(expected)
    }

    pub async fn assert_nonce_increment(&mut self, keys_indexes: Vec<(u32, u32)>) -> &mut Self {
        for (index, increment) in keys_indexes {
            let nonce = self
                .provider
                .get_transaction_count(
                    Address::from_str(&self.accounts_pub[index as usize]).unwrap(),
                    None,
                )
                .await
                .unwrap();
            let prev_nonce = self.nonces.get(&index).unwrap();

            assert!(nonce == prev_nonce + U256::from(increment));
        }
        self
    }

    pub fn run(&mut self, expected: &str) -> &mut Self {
        let output = if self.err {
            self.err = false;
            self.cmd.stderr_lossy()
        } else {
            self.cmd.stdout_lossy()
        };
        assert!(output.contains(expected));
        self
    }

    pub fn expect_err(&mut self) -> &mut Self {
        self.err = true;
        self
    }
}
