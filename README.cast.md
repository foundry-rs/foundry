# Polkadot Foundry Supported Cast Commands Documentation with Examples

## Documentation Format and Color Scheme

This documentation is structured to provide a clear overview of the supported `cast` commands. Each command is presented in the following format:

- **Command Name**: The name of the command, colored to indicate its status (**<span style="color: green;">green</span>** for working, **<span style="color: red;">red</span>** for non-working).
- **Command**: The full command syntax with required parameters.
- **Required Parameters**: Parameters that must be provided for the command to execute, as specified in the help files.
- **Example**: A collapsible dropdown containing the complete command with its output or error message, ensuring all relevant details are included.

This format ensures clarity and ease of navigation, with the color scheme providing an immediate visual cue for command reliability.

## Rule of Thumb

- If the command is not listed, it is not supported.
- If the command is listed with a **<span style="color: red;">red</span>** color, it is not supported.
- If the command is listed with a **<span style="color: green;">green</span>** color, it is supported.

## Known Issues

## Cast Commands

### Working Commands

#### ✅ <span style="color: green;">4byte</span>
- **Command**: `cast 4byte [OPTIONS] [TOPIC_0]`
- **Required Parameters**: `TOPIC_0`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast 4byte 0xd09de08a
  increment()
  ```
  </details>

#### ✅ <span style="color: green;">4byte-event</span>
- **Command**: `cast 4byte-event [OPTIONS] [TOPIC_0]`
- **Required Parameters**: `TOPIC_0`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast 4byte-event 0xb68ce3d4f35f8b562c4caf11012045e29a80cc1082438f785646ec651416c8d6
  CounterChanged(int256)
  ```
  </details>

#### ✅ <span style="color: green;">abi-encode</span>
- **Command**: `cast abi-encode [OPTIONS] <SIG> [ARGS]...`
- **Required Parameters**: `SIG`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast abi-encode "increment()"
  ```
  </details>

#### ✅ <span style="color: green;">address-zero</span>
- **Command**: `cast address-zero`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast address-zero
  0x0000000000000000000000000000000000000000
  ```
  </details>

#### ✅ <span style="color: green;">age</span>
- **Command**: `cast age [OPTIONS] [BLOCK]`
- **Required Parameters**: `BLOCK`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast age latest --rpc-url https://westend-asset-hub-eth-rpc.polkadot.io
  Tue Jun 10 14:04:30 2025 UTC
  ```
  </details>

#### ✅ <span style="color: green;">balance</span>
- **Command**: `cast balance [OPTIONS] <WHO>`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast balance 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  99922331840422000000
  ```
  </details>

#### ✅ <span style="color: green;">base-fee</span>
- **Command**: `cast base-fee [OPTIONS] [BLOCK]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast base-fee latest --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  1000
  ```
  </details>

#### ✅ <span style="color: green;">block</span>
- **Command**: `cast block [OPTIONS] [BLOCK]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast block latest --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  baseFeePerGas        1000
  difficulty           0
  extraData            0x
  gasLimit             1966080000000
  gasUsed              0
  hash                 0x54bb013653c8c9f6e76e4e48666bc8451e7d139e0a1e71af2319b9e17abdb889
  logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
  miner                0x0000000000000000000000000000000000000000
  mixHash              0x0000000000000000000000000000000000000000000000000000000000000000
  nonce                0x0000000000000000
  number               179802
  parentHash           0x8d25cc4503a87e575d677e22383bb2121d736b50425b35b9c7d74e1a8cd18b58
  parentBeaconRoot     
  transactionsRoot     0x0a594e257f74a87f5659ba3a4612b6b4fc44319a303934907db20f4e3fc050f8
  receiptsRoot         0x0a594e257f74a87f5659ba3a4612b6b4fc44319a303934907db20f4e3fc050f8
  sha3Uncles           0x0000000000000000000000000000000000000000000000000000000000000000
  size                 0
  stateRoot            0x8a4948ed507366d31a68675659d7a83c3a959460e784eb1a8786ca767d61d6eb
  timestamp            1749564276 (Tue, 10 Jun 2025 14:04:36 +0000)
  withdrawalsRoot      
  totalDifficulty      
  blobGasUsed          
  excessBlobGas        
  requestsHash         
  transactions:        []
  ```
  </details>

#### ✅ <span style="color: green;">block-number</span>
- **Command**: `cast block-number [OPTIONS] [BLOCK]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast block-number latest --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  179802
  ```
  </details>

#### ✅ <span style="color: green;">call</span>
- **Command**: `cast call`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast call 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 increment() --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  0x
  ```
  </details>

#### ✅ <span style="color: green;">chain</span>
- **Command**: `cast chain [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast chain --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  ```
  </details>

#### ✅ <span style="color: green;">chain-id</span>
- **Command**: `cast chain-id [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast chain-id --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  ```
  </details>

#### ✅ <span style="color: green;">client</span>
- **Command**: `cast client [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast client --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  ```
  </details>

#### ✅ <span style="color: green;">code</span>
- **Command**: `cast code [OPTIONS] <WHO>`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast code 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  ```
  </details>

#### ✅ <span style="color: green;">codesize</span>
- **Command**: `cast codesize [OPTIONS] <WHO>`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast codesize 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  4994
  ```
  </details>

#### ✅ <span style="color: green;">compute-address</span>
- **Command**: `cast compute-address [OPTIONS] <WHO>`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast compute-address 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  Computed Address: 0xcbED7f469a9F2d5580E3fE09DFcd971e108D3a02
  ```
  </details>

#### ✅ <span style="color: green;">decode-abi</span>
- **Command**: `cast decode-abi [OPTIONS] <SIG> <CALLDATA>`
- **Required Parameters**: `SIG`, `CALLDATA`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast decode-abi balanceOf(address)(uint256) 0x000000000000000000000000000000000000000000000000000000000000000a
  10
  ```
  </details>

#### ✅ <span style="color: green;">decode-calldata</span>
- **Command**: `cast decode-calldata [OPTIONS] <SIG> <CALLDATA>`
- **Required Parameters**: `SIG`, `CALLDATA`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast decode-calldata transfer(address,uint256) 0xa9059cbb000000000000000000000000e78388b4ce79068e89bf8aa7f218ef6b9ab0e9d0000000000000000000000000000000000000000000000000008a8e4b1a3d8000
  0xE78388b4CE79068e89Bf8aA7f218eF6b9AB0e9d0
  39000000000000000 [3.9e16]
  ```
  </details>

#### ✅ <span style="color: green;">decode-error</span>
- **Command**: `cast decode-error [OPTIONS] <DATA>`
- **Required Parameters**: `DATA`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast decode-error 0x4e487b710000000000000000000000000000000000000000000000000000000000000011 --sig Panic(uint256)
  17
  ```
  </details>

#### ✅ <span style="color: green;">decode-event</span>
- **Command**: `cast decode-event [OPTIONS] <DATA>`
- **Required Parameters**: `DATA`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast decode-event 0x000000000000000000000000000000000000000000000000000000000000002a --sig CounterChanged(int256)
  42
  ```
  </details>

#### ✅ <span style="color: green;">estimate</span>
- **Command**: `cast estimate [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]`
- **Required Parameters**: `TO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast estimate --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io --from 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 increment()
  53772291631
  ```
  </details>

#### ✅ <span style="color: green;">find-block</span>
- **Command**: `cast find-block [OPTIONS] <TIMESTAMP>`
- **Required Parameters**: `TIMESTAMP`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast find-block 1749564284 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  179803
  ```
  </details>

#### ✅ <span style="color: green;">gas-price</span>
- **Command**: `cast gas-price [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast gas-price --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  1000
  ```
  </details>

#### ✅ <span style="color: green;">generate-fig-spec</span>
- **Command**: `cast generate-fig-spec`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast generate-fig-spec
  ```
  </details>

#### ✅ <span style="color: green;">index-string</span>
- **Command**: `cast index-string`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast index string increment 1
  0xc7728314374610455dba288d68795a0a1f4e297598fadddf5234bb036cb803cc
  ```
  </details>

#### ✅ <span style="color: green;">index-erc7201</span>
- **Command**: `cast index-erc7201`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast index-erc7201 1
  0x37b41b5c3e40b7905ecb7bb8b5e767c8863eb94bcacb56e4ec2e4884d425e400
  ```
  </details>

#### ✅ <span style="color: green;">logs</span>
- **Command**: `cast logs [OPTIONS] [SIG_OR_TOPIC] [TOPICS_OR_ARGS]...`
- **Required Parameters**: `SIG_OR_TOPIC`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast logs --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io --address 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 --from-block 78016
  ```
  </details>

#### ✅ <span style="color: green;">max-int</span>
- **Command**: `cast max-int`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast max-int
  57896044618658097711785492504343953926634992332820282019728792003956564819967
  ```
  </details>

#### ✅ <span style="color: green;">max-uint</span>
- **Command**: `cast max-uint`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast max-uint
  115792089237316195423570985008687907853269984665640564039457584007913129639935
  ```
  </details>

#### ✅ <span style="color: green;">min-int</span>
- **Command**: `cast min-int`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast min-int
  -57896044618658097711785492504343953926634992332820282019728792003956564819968
  ```
  </details>

#### ✅ <span style="color: green;">mktx</span>
- **Command**: `cast mktx [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]`
- **Required Parameters**: `TO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast mktx 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 increment() --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io --private-key 0xf88c374c84378042e20927119c4c8b6ed2d57508c9b8a4f05fe2868ab8f8b73e --from 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997
  0x02f86f84190f1b452d018207d1850c85140e2f9421de540df1c0fac44e9504fbf43046d1656ed0c78084d09de08ac080a0e12bf64571adf9c44e84de68e09b9e99167e40087674745f7b91bbffe6895dcea00c330054988ce9ddead827efb8a22bbf2634bc196b5bfa7f9d71c5de7794aaac
  ```
  </details>

#### ✅ <span style="color: green;">decode-transaction</span>
- **Command**: `cast decode-transaction [OPTIONS] [TX]`
- **Required Parameters**: `TX`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast decode-transaction 0x02f86f84190f1b452d018207d1850c85140e2f9421de540df1c0fac44e9504fbf43046d1656ed0c78084d09de08ac080a0e12bf64571adf9c44e84de68e09b9e99167e40087674745f7b91bbffe6895dcea00c330054988ce9ddead827efb8a22bbf2634bc196b5bfa7f9d71c5de7794aaac
  {
    "signer": "0x2fcdc5f0799accb67008aab9d4aaa08994897997",
    "type": "0x2",
    "chainId": "0x190f1b45",
    "nonce": "0x2d",
    "gas": "0xc85140e2f",
    "maxFeePerGas": "0x7d1",
    "maxPriorityFeePerGas": "0x1",
    "to": "0x21de540df1c0fac44e9504fbf43046d1656ed0c7",
    "value": "0x0",
    "accessList": [],
    "input": "0xd09de08a",
    "r": "0xe12bf64571adf9c44e84de68e09b9e99167e40087674745f7b91bbffe6895dce",
    "s": "0xc330054988ce9ddead827efb8a22bbf2634bc196b5bfa7f9d71c5de7794aaac",
    "yParity": "0x0",
    "v": "0x0",
    "hash": "0x350fa5032a64c14e78fceb50769cc361edefab8c92bb966610bb7eae709db332"
  }
  ```
  </details>

#### ✅ <span style="color: green;">namehash increment</span>
- **Command**: `cast namehash increment`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast namehash increment
  0x8afe645ccddb43a658dfe445605688ca833db88c87e218a157afdc04976d5462
  ```
  </details>

#### ✅ <span style="color: green;">nonce</span>
- **Command**: `cast nonce [OPTIONS] <WHO>`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast nonce 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  45
  ```
  </details>

#### ✅ <span style="color: green;">parse-bytes32-address</span>
- **Command**: `cast parse-bytes32-address`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast parse-bytes32-address 0x000000000000000000000000000000000000000000000000000000000000000a
  0x000000000000000000000000000000000000000A
  ```
  </details>

#### ✅ <span style="color: green;">parse-bytes32-string</span>
- **Command**: `cast parse-bytes32-string`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast parse-bytes32-string 0x696e6372656d656e740000000000000000000000000000000000000000000000
  increment
  ```
  </details>

#### ✅ <span style="color: green;">parse-units</span>
- **Command**: `cast parse-units`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast parse-units 1
  1000000000000000000
  ```
  </details>

#### ✅ <span style="color: green;">pretty-calldata</span>
- **Command**: `cast pretty-calldata [OPTIONS] <DATA>`
- **Required Parameters**: `DATA`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast pretty-calldata 0xd09de08a
  Possible methods:
  - increment()
  ------------
  ```
  </details>

#### ✅ <span style="color: green;">publish</span>
- **Command**: `cast publish [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast publish 0x02f86f84190f1b452d018207d1850c85140e2f9421de540df1c0fac44e9504fbf43046d1656ed0c78084d09de08ac080a0e12bf64571adf9c44e84de68e09b9e99167e40087674745f7b91bbffe6895dcea00c330054988ce9ddead827efb8a22bbf2634bc196b5bfa7f9d71c5de7794aaac --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  {"status":"0x1","cumulativeGasUsed":"0x0","logs":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","type":"0x2","transactionHash":"0x350fa5032a64c14e78fceb50769cc361edefab8c92bb966610bb7eae709db332","transactionIndex":"0x2","blockHash":"0x8338e28e7535066fe289f80b46cae381ad591a6609637059f7866dc6a7eadde0","blockNumber":"0x2be5d","gasUsed":"0x2227833c87","effectiveGasPrice":"0x3e9","from":"0x2fcdc5f0799accb67008aab9d4aaa08994897997","to":"0x21de540df1c0fac44e9504fbf43046d1656ed0c7","contractAddress":null}
  ```
  </details>

#### ✅ <span style="color: green;">receipt</span>
- **Command**: `cast receipt`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast receipt 0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  blockHash            0x8d25cc4503a87e575d677e22383bb2121d736b50425b35b9c7d74e1a8cd18b58
  blockNumber          179801
  contractAddress      0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7
  cumulativeGasUsed    0
  effectiveGasPrice    1001
  from                 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997
  gasUsed              1552638953046
  logs                 []
  logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
  root                 
  status               1 (success)
  transactionHash      0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6
  transactionIndex     2
  type                 2
  blobGasPrice         
  blobGasUsed          
  ```
  </details>

#### ✅ <span style="color: green;">rpc</span>
- **Command**: `cast rpc [OPTIONS] <METHOD> [PARAMS]...`
- **Required Parameters**: `METHOD`, `RPC`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast rpc eth_getTransactionByHash 0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  {"blockHash":"0x8d25cc4503a87e575d677e22383bb2121d736b50425b35b9c7d74e1a8cd18b58","blockNumber":"0x2be59","from":"0x2fcdc5f0799accb67008aab9d4aaa08994897997","hash":"0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6","transactionIndex":"0x2","accessList":[],"chainId":"0x190f1b45","gas":"0x78931d5ab1","gasPrice":"0x0","input":"","maxFeePerGas":"0x7d1","maxPriorityFeePerGas":"0x1","nonce":"0x2b","to":null,"type":"0x2","value":"0x0","r":"0x49922d3ebb211a638efd05bf5bc30160e054e809d4966f348f391bf16466ce5b","s":"0x65547942a3c8aac5f74b34a7268ccd89b39a8908b921bc754bbb408622ec3acf","yParity":"0x1"}
  ```
  </details>

#### ✅ <span style="color: green;">send</span>
- **Command**: `cast send [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]`
- **Required Parameters**: `TO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast send 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 increment() --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io --private-key 0xf88c374c84378042e20927119c4c8b6ed2d57508c9b8a4f05fe2868ab8f8b73e
  blockHash            0xd01aad373b99bf4ec44674658e96f523a072c1554758101d6acf5ae4ace8932f
  blockNumber          179806
  contractAddress      
  cumulativeGasUsed    0
  effectiveGasPrice    1001
  from                 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997
  gasUsed              146739752247
  logs                 []
  logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
  root                 
  status               1 (success)
  transactionHash      0x9ee2f302231bd559a1176d9117f0910e6239b318e35c68bbc34e5445319f87f2
  transactionIndex     2
  type                 2
  blobGasPrice         
  blobGasUsed          
  to                   0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7
  ```
  </details>

#### ✅ <span style="color: green;">sig</span>
- **Command**: `cast sig [OPTIONS] [EVENT_STRING]`
- **Required Parameters**: `EVENT_STRING`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast sig increment()
  0xd09de08a
  ```
  </details>

#### ✅ <span style="color: green;">sig-event</span>
- **Command**: `cast sig-event [OPTIONS] [EVENT_STRING]`
- **Required Parameters**: `EVENT_STRING`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast sig-event increment()
  0xd09de08ab1a974aadf0a76e6f99a2ec20e431f22bbc101a6c3f718e53646ed8d
  ```
  </details>

#### ✅ <span style="color: green;">storage</span>
- **Command**: `cast storage [OPTIONS] <ADDRESS> [SLOT]`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Required Parameters**: `ADDRESS`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast storage 0x21dE540Df1C0FAC44E9504fbf43046d1656ED0c7 0xc7728314374610455dba288d68795a0a1f4e297598fadddf5234bb036cb803cc --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  0x0000000000000000000000000000000000000000000000000000000000000000
  ```
  </details>

#### ✅ <span style="color: green;">tx</span>
- **Command**: `cast tx`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast tx 0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6 --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io
  blockHash            0x8d25cc4503a87e575d677e22383bb2121d736b50425b35b9c7d74e1a8cd18b58
  blockNumber          179801
  from                 0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997
  transactionIndex     2
  effectiveGasPrice    0

  accessList           []
  chainId              420420421
  gasLimit             517864250033
  hash                 0x0cb04feb40b754ab47334d65df175f16972a72c7613c6747310138235b09d5c6
  input                
  maxFeePerGas         2001
  maxPriorityFeePerGas 1
  nonce                43
  r                    0x49922d3ebb211a638efd05bf5bc30160e054e809d4966f348f391bf16466ce5b
  s                    0x65547942a3c8aac5f74b34a7268ccd89b39a8908b921bc754bbb408622ec3acf
  to                   
  type                 2
  value                0
  yParity              1
  ```
  </details>

#### ✅ <span style="color: green;">upload-signature</span>
- **Command**: `cast upload-signature [OPTIONS] <SIGNATURE_STRING>`
- **Required Parameters**: `SIGNATURE_STRING`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast upload-signature transfer(uint256)
  Duplicated: Function transfer(uint256): 0x12514bba
  Selectors successfully uploaded to OpenChain
  ```
  </details>

#### ✅ <span style="color: green;">wallet</span>
- **Command**: `cast wallet`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast wallet new
  Successfully created new keypair.
  Address:     0x0d69fBC8e33b77De1BC8F67A44696E0Ec29aD176
  Private key: 0x992a0e0c3200d9d6fa62138a96de0869d1c279eb52ef5878437dcd5c29cd3760
  ```
  </details>

#### ✅ <span style="color: green;">wallet new-mnemonic</span>
- **Command**: `cast wallet new-mnemonic`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast wallet new-mnemonic
  Generating mnemonic from provided entropy...
  Successfully generated a new mnemonic.
  Phrase:
  common puppy load skill south minor struggle black invest entry save outdoor

  Accounts:
  - Account 0:
  Address:     0xB685D21617C57dc9387A54210C0E2F1D26492b9C
  Private key: 0x0eef219f21f0f4df582a8b94ba333d731532287ca3b1e0426c39f8cffbee39a3
  ```
  </details>

#### ✅ <span style="color: green;">wallet address</span>
- **Command**: `cast wallet address [OPTIONS]`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast wallet address --private-key 0xf88c374c84378042e20927119c4c8b6ed2d57508c9b8a4f05fe2868ab8f8b73e
  ```
  </details>

### Not Working Commands

#### ❌ <span style="color: red;">proof</span>
- **Command**: `cast proof [OPTIONS] <ADDRESS> [SLOTS]...`
- **Required Parameters**: `ADDRESS`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast proof
  ```
  </details>

#### ❌ <span style="color: red;">storage-root</span>
- **Command**: `cast storage-root [OPTIONS] <WHO> [SLOTS]...`
- **Required Parameters**: `WHO`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > cast storage-root
  ```
  </details>
