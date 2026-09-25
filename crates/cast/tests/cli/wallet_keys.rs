//! CLI tests for wallet keys commands.

use super::*;

// tests that `cast wallet new-mnemonic --entropy` outputs the expected mnemonic
casttest!(wallet_mnemonic_from_entropy, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
Generating mnemonic from provided entropy...
Successfully generated a new mnemonic.
Phrase:
test test test test test test test test test test test junk

Accounts:
- Account 0:
Address:     0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

- Account 1:
Address:     0x70997970C51812dc3A010C7d01b50e0d17dc79C8
Private key: 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

- Account 2:
Address:     0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
Private key: 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a


"#]]
        .raw(),
    );
});

// tests that `cast wallet new-mnemonic --entropy` outputs the expected mnemonic (verbose variant)
casttest!(wallet_mnemonic_from_entropy_verbose, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "-v",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
Generating mnemonic from provided entropy...
Successfully generated a new mnemonic.
Phrase:
test test test test test test test test test test test junk

Accounts:
- Account 0:
Address:     0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Public key:  0x8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5
Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

- Account 1:
Address:     0x70997970C51812dc3A010C7d01b50e0d17dc79C8
Public key:  0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4
Private key: 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

- Account 2:
Address:     0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
Public key:  0x9d9031e97dd78ff8c15aa86939de9b1e791066a0224e331bc962a2099a7b1f0464b8bbafe1535f2301c72c2cb3535b172da30b02686ab0393d348614f157fbdb
Private key: 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a


"#]]
        .raw(),
    );
});

// tests that `cast wallet new-mnemonic --json` outputs the expected mnemonic
casttest!(wallet_mnemonic_from_entropy_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "--json",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": {
    "mnemonic": "test test test test test test test test test test test junk",
    "accounts": [
      {
        "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
      },
      {
        "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "private_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
      },
      {
        "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "private_key": "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
      }
    ]
  },
  "errors": [],
  "warnings": []
}

"#]]
        .is_json(),
    );
});

// tests that `cast wallet new-mnemonic --json` outputs the expected mnemonic (verbose variant)
casttest!(wallet_mnemonic_from_entropy_json_verbose, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "--json",
        "-v",
    ])
.assert_success()
.stdout_eq(str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": {
    "mnemonic": "test test test test test test test test test test test junk",
    "accounts": [
      {
        "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "public_key": "0x8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
        "private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
      },
      {
        "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "public_key": "0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4",
        "private_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
      },
      {
        "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "public_key": "0x9d9031e97dd78ff8c15aa86939de9b1e791066a0224e331bc962a2099a7b1f0464b8bbafe1535f2301c72c2cb3535b172da30b02686ab0393d348614f157fbdb",
        "private_key": "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
      }
    ]
  },
  "errors": [],
  "warnings": []
}

"#]]
.is_json());
});

// tests that `cast wallet derive` outputs the addresses of the accounts derived from the mnemonic
casttest!(wallet_derive_mnemonic, |_prj, cmd| {
    cmd.args([
        "wallet",
        "derive",
        "--accounts",
        "3",
        "test test test test test test test test test test test junk",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
- Account 0:
[ADDRESS]

- Account 1:
[ADDRESS]

- Account 2:
[ADDRESS]


"#]]);
});

// tests that `cast wallet derive` with insecure flag outputs the addresses and private keys of the
// accounts derived from the mnemonic
casttest!(wallet_derive_mnemonic_insecure, |_prj, cmd| {
    cmd.args([
        "wallet",
        "derive",
        "--accounts",
        "3",
        "--insecure",
        "test test test test test test test test test test test junk",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
- Account 0:
[ADDRESS]
[PRIVATE_KEY]

- Account 1:
[ADDRESS]
[PRIVATE_KEY]

- Account 2:
[ADDRESS]
[PRIVATE_KEY]


"#]]);
});

// tests that `cast wallet derive` with json flag outputs the addresses of the accounts derived from
// the mnemonic in JSON format
casttest!(wallet_derive_mnemonic_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "derive",
        "--accounts",
        "3",
        "--json",
        "test test test test test test test test test test test junk",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    },
    {
      "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
    },
    {
      "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
        .is_json(),
    );
});

// tests that `cast wallet derive` with insecure and json flag outputs the addresses and private
// keys of the accounts derived from the mnemonic in JSON format
casttest!(wallet_derive_mnemonic_insecure_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "derive",
        "--accounts",
        "3",
        "--insecure",
        "--json",
        "test test test test test test test test test test test junk",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": [
    {
      "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      "private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    },
    {
      "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
      "private_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    },
    {
      "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
      "private_key": "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    }
  ],
  "errors": [],
  "warnings": []
}

"#]]
        .is_json(),
    );
});

// tests that `cast wallet private-key` with arguments outputs the private key
casttest!(wallet_private_key_from_mnemonic_arg, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "test test test test test test test test test test test junk",
        "1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});

// tests that `cast wallet private-key` with options outputs the private key
casttest!(wallet_private_key_from_mnemonic_option, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "--mnemonic",
        "test test test test test test test test test test test junk",
        "--mnemonic-index",
        "1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});

// tests that `cast wallet public-key` correctly derives and outputs the public key
casttest!(wallet_public_key_with_private_key, |_prj, cmd| {
    cmd.args([
        "wallet",
        "public-key",
        "--raw-private-key",
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4

"#]]);
});

// tests that `cast wallet private-key` with derivation path outputs the private key
casttest!(wallet_private_key_with_derivation_path, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "--mnemonic",
        "test test test test test test test test test test test junk",
        "--mnemonic-derivation-path",
        "m/44'/60'/0'/0/1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});
