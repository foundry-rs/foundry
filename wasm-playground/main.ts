import * as Cast from './dist/wasm_playground.ts'

// await Cast.__wasmReady

const url = 'https://reth-ethereum.ithaca.xyz/rpc'

async function main() {
  const outputs: Array<string> = []

  // get block number
  const blockNumber = await Cast.rpc(url, 'eth_blockNumber', [])
  const log1 = `BLOCK NUMBER: ${parseInt(blockNumber.result, 16)}`
  console.log(log1)
  outputs.push(log1)

  const token = '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48'
  const holder = '0xd8da6bf26964af9d7eed9e03e53415d37aa96045'

  // balanceOf call encoding
  const data = Cast.calldata_encode('balanceOf(address)', [holder])
  const ethCall = await Cast.rpc(url, 'eth_call', [
    { to: token, data },
    'latest',
  ])
  const log2 = `USDC BALANCE OF: ${parseInt(ethCall.result, 16) / 1e6}`
  console.info(log2)
  outputs.push(log2)

  const amount = '1000000000000000000'
  const to = '0xd8da6bf26964af9d7eed9e03e53415d37aa96045'

  // transfer call encoding
  const transferData = Cast.calldata_encode('transfer(address,uint256)', [
    to,
    amount,
  ])
  const transferResult = await Cast.rpc(url, 'eth_call', [{
    to: token,
    data: transferData,
  }, 'latest'])
  const log3 =
    `ERC20 TRANSFER SIMULATION RESULT:: ${transferResult?.error.message}`
  console.info(transferResult)
  outputs.push(log3)

  if (typeof document !== 'undefined') {
    const output = document.getElementById('output')
    if (output) {
      output.innerText = JSON.stringify(outputs, null, 2)
    }
  }
}

main().catch(console.error)
