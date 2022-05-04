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
    pub fn new(mut cmd: TestCommand, port: u16) -> Self {
        let current_dir = std::env::current_dir().unwrap();
        let root_path = current_dir.join("../testdata");
        let root = root_path.to_string_lossy().to_string();
        let target_contract =
            root_path.join("./cheats/Broadcast.t.sol").to_string_lossy().to_string();
        let url = format!("http://127.0.0.1:{port}");

        cmd.args([
            "script",
            target_contract.as_str(),
            "--root",
            root.as_str(),
            "--fork-url",
            url.as_str(),
            "-vvv",
            "--legacy", // only necessary for ganache
        ]);

        ScriptTester {
            accounts_pub: vec![
                "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
                "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string(),
            ],
            accounts_priv: vec![
                "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string(),
                "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".to_string(),
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
