//! Batch transaction tests for cast batch-send, batch-sign, and batch-mktx commands

use anvil::NodeConfig;
use foundry_test_utils::str;

// Test batch-send with multiple ETH transfers (5 transactions)
casttest!(batch_send_multiple_transfers, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-send",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1",
        "--tx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC:2",
        "--tx",
        "0x90F79bf6EB2c4f870365E785982E1f101E93b906:3",
        "--tx",
        "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65:4",
        "--tx",
        "0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc:5",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
        "--async",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Processing 5 transactions...
Starting nonce: 0
Building transaction 1 of 5 (nonce: 0)...
Transaction 1 sent: 0x[..]
Building transaction 2 of 5 (nonce: 1)...
Transaction 2 sent: 0x[..]
Building transaction 3 of 5 (nonce: 2)...
Transaction 3 sent: 0x[..]
Building transaction 4 of 5 (nonce: 3)...
Transaction 4 sent: 0x[..]
Building transaction 5 of 5 (nonce: 4)...
Transaction 5 sent: 0x[..]
Batch complete! 5 transactions sent:
  1. 0x[..]
  2. 0x[..]
  3. 0x[..]
  4. 0x[..]
  5. 0x[..]

"#]]);
});

// Test batch-send with 8 transactions using semicolon-delimited input
casttest!(batch_send_large_semicolon_delimited, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-send",
        "--tx", 
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1;0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC:2;0x90F79bf6EB2c4f870365E785982E1f101E93b906:3;0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65:4;0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc:5;0x976EA74026E726554dB657fA54763abd0C3a0aa9:6;0x14dC79964da2C08b23698B3D3cc7Ca32193d9955:7;0x23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f:8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
        "--async",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Processing 8 transactions...
Starting nonce: 0
Building transaction 1 of 8 (nonce: 0)...
Transaction 1 sent: 0x[..]
Building transaction 2 of 8 (nonce: 1)...
Transaction 2 sent: 0x[..]
Building transaction 3 of 8 (nonce: 2)...
Transaction 3 sent: 0x[..]
Building transaction 4 of 8 (nonce: 3)...
Transaction 4 sent: 0x[..]
Building transaction 5 of 8 (nonce: 4)...
Transaction 5 sent: 0x[..]
Building transaction 6 of 8 (nonce: 5)...
Transaction 6 sent: 0x[..]
Building transaction 7 of 8 (nonce: 6)...
Transaction 7 sent: 0x[..]
Building transaction 8 of 8 (nonce: 7)...
Transaction 8 sent: 0x[..]
Batch complete! 8 transactions sent:
  1. 0x[..]
  2. 0x[..]
  3. 0x[..]
  4. 0x[..]
  5. 0x[..]
  6. 0x[..]
  7. 0x[..]
  8. 0x[..]

"#]]);
});

// Test batch-sign with 6 transactions demonstrating nonce sequence
casttest!(batch_sign_nonce_sequence, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-sign",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1",
        "--tx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC:2",
        "--tx",
        "0x90F79bf6EB2c4f870365E785982E1f101E93b906:3",
        "--tx",
        "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65:4",
        "--tx",
        "0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc:5",
        "--tx",
        "0x976EA74026E726554dB657fA54763abd0C3a0aa9:6",
        "--start-nonce",
        "42", // Custom starting nonce
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Signing 6 transactions...
Starting nonce: 42
Signing transaction 1 of 6 (nonce: 42)...
Transaction 1 signed
Signing transaction 2 of 6 (nonce: 43)...
Transaction 2 signed
Signing transaction 3 of 6 (nonce: 44)...
Transaction 3 signed
Signing transaction 4 of 6 (nonce: 45)...
Transaction 4 signed
Signing transaction 5 of 6 (nonce: 46)...
Transaction 5 signed
Signing transaction 6 of 6 (nonce: 47)...
Transaction 6 signed
Batch complete! 6 transactions signed:
1. 0x[..]
2. 0x[..]
3. 0x[..]
4. 0x[..]
5. 0x[..]
6. 0x[..]

"#]]);
});

// Test batch-mktx with 7 unsigned transactions
casttest!(batch_mktx_multiple_unsigned, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "batch-mktx",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1",
        "--tx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC:2",
        "--tx",
        "0x90F79bf6EB2c4f870365E785982E1f101E93b906:3",
        "--tx",
        "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65:4",
        "--tx",
        "0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc:5",
        "--tx",
        "0x976EA74026E726554dB657fA54763abd0C3a0aa9:6",
        "--tx",
        "0x14dC79964da2C08b23698B3D3cc7Ca32193d9955:7",
        "--raw-unsigned",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--start-nonce",
        "100",
        "--chain",
        "1",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--rpc-url",
        handle.http_endpoint().as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Building 7 transactions...
Starting nonce: 100
Building transaction 1 of 7 (nonce: 100)...
Transaction 1 built
Building transaction 2 of 7 (nonce: 101)...
Transaction 2 built
Building transaction 3 of 7 (nonce: 102)...
Transaction 3 built
Building transaction 4 of 7 (nonce: 103)...
Transaction 4 built
Building transaction 5 of 7 (nonce: 104)...
Transaction 5 built
Building transaction 6 of 7 (nonce: 105)...
Transaction 6 built
Building transaction 7 of 7 (nonce: 106)...
Transaction 7 built
Batch complete! 7 transactions built:
1. 0x[..]
2. 0x[..]
3. 0x[..]
4. 0x[..]
5. 0x[..]
6. 0x[..]
7. 0x[..]

"#]]);
});

// Test that function signatures with commas work with individual --tx flags
casttest!(batch_send_function_sig_with_commas, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-send",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8::transfer(address,uint256):0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC,1000",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
        "--async",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Processing 1 transactions...
Starting nonce: 0
Building transaction 1 of 1 (nonce: 0)...
Transaction 1 sent: 0x[..]
Batch complete! 1 transactions sent:
  1. 0x[..]

"#]]);
});

// Test multiple complex function signatures in one command
casttest!(batch_sign_complex_signatures, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-sign",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8::transferFrom(address,address,uint256):0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC,0x90F79bf6EB2c4f870365E785982E1f101E93b906,1000",
        "--tx", 
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC::approve(address,uint256):0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65,2000",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Signing 2 transactions...
Starting nonce: 0
Signing transaction 1 of 2 (nonce: 0)...
Transaction 1 signed
Signing transaction 2 of 2 (nonce: 1)...
Transaction 2 signed
Batch complete! 2 transactions signed:
1. 0x[..]
2. 0x[..]

"#]]);
});

// Test semicolon-delimited with function signatures containing commas
casttest!(batch_send_semicolon_with_function_sigs, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "batch-send",
        "--tx",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8::transfer(address,uint256):0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC,1000;0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC::approve(address,uint256):0x90F79bf6EB2c4f870365E785982E1f101E93b906,2000",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        endpoint.as_str(),
        "--async",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Processing 2 transactions...
Starting nonce: 0
Building transaction 1 of 2 (nonce: 0)...
Transaction 1 sent: 0x[..]
Building transaction 2 of 2 (nonce: 1)...
Transaction 2 sent: 0x[..]
Batch complete! 2 transactions sent:
  1. 0x[..]
  2. 0x[..]

"#]]);
});
