#!/usr/bin/env -S deno run --allow-read --allow-net

// Test script for wasm-playground functions
import * as wasm from './dist/wasm_playground.ts'

const url = 'https://reth-ethereum.ithaca.xyz/rpc'

console.log('Testing wasm-playground functions...\n')

// Test keccak256
console.log('=== keccak256 ===')
const hash1 = wasm.keccak256('hello world')
console.log(`keccak256("hello world") = ${hash1}`)
const hash2 = wasm.keccak256('0x68656c6c6f20776f726c64')
console.log(`keccak256("0x68656c6c6f20776f726c64") = ${hash2}`)

// Test hex conversions
console.log('\n=== Hex Conversions ===')
const hex1 = wasm.to_hex('123456')
console.log(`to_hex("123456") = ${hex1}`)
const utf8_1 = wasm.from_utf8('hello')
console.log(`from_utf8("hello") = ${utf8_1}`)
const utf8_2 = wasm.to_utf8('0x68656c6c6f')
console.log(`to_utf8("0x68656c6c6f") = ${utf8_2}`)

// Test uint256/int256
console.log('\n=== Number Conversions ===')
const uint256 = wasm.to_uint256('12345')
console.log(`to_uint256("12345") = ${uint256}`)
const int256 = wasm.to_int256('-12345')
console.log(`to_int256("-12345") = ${int256}`)

// Test bytes32 string functions
console.log('\n=== Bytes32 Strings ===')
const bytes32 = wasm.format_bytes32_string('hello')
console.log(`format_bytes32_string("hello") = ${bytes32}`)
const parsed = wasm.parse_bytes32_string(
  '0x68656c6c6f000000000000000000000000000000000000000000000000000000',
)
console.log(`parse_bytes32_string("0x68656c6c6f00...") = ${parsed}`)

// Test selector
console.log('\n=== Function Selector ===')
const selector = wasm.selector('transfer(address,uint256)')
console.log(`selector("transfer(address,uint256)") = ${selector}`)

// Test shift operations
console.log('\n=== Bit Shifts ===')
const leftShift = wasm.left_shift('0x1', '8')
console.log(`left_shift("0x1", "8") = ${leftShift}`)
const rightShift = wasm.right_shift('0x100', '8')
console.log(`right_shift("0x100", "8") = ${rightShift}`)

// Test padding
console.log('\n=== Padding ===')
const padLeft = wasm.pad_left('0x1234', 32)
console.log(`pad_left("0x1234", 32) = ${padLeft}`)
const padRight = wasm.pad_right('0x1234', 32)
console.log(`pad_right("0x1234", 32) = ${padRight}`)

// Test calldata encoding
console.log('\n=== Calldata Encoding ===')
const calldata = wasm.calldata_encode('transfer(address,uint256)', [
  '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3',
  '1000000000000000000',
])
console.log(`calldata_encode("transfer(address,uint256)", [...]) = ${calldata}`)

// Test ABI encoding
console.log('\n=== ABI Encoding ===')
const abiEncoded = wasm.abi_encode('uint256,address', [
  '123456',
  '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3',
])
console.log(`abi_encode("uint256,address", [...]) = ${abiEncoded}`)

// Test concat_hex
console.log('\n=== Concat Hex ===')
const concatenated = wasm.concat_hex(['0x1234', '0x5678', '0xabcd'])
console.log(`concat_hex(["0x1234", "0x5678", "0xabcd"]) = ${concatenated}`)

// Test RPC calls from main.ts
console.log('\n=== RPC Calls ===')
try {
  // Get block number
  const blockNumber = await wasm.rpc(url, 'eth_blockNumber', [])
  console.log(`eth_blockNumber: ${parseInt(blockNumber.result, 16)}`)

  const token = '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48' // USDC
  const holder = '0xd8da6bf26964af9d7eed9e03e53415d37aa96045'

  // Test balanceOf call encoding and RPC
  const balanceOfData = wasm.calldata_encode('balanceOf(address)', [holder])
  console.log(`balanceOf calldata: ${balanceOfData}`)

  const ethCall = await wasm.rpc(url, 'eth_call', [
    { to: token, data: balanceOfData },
    'latest',
  ])
  if (ethCall.result) {
    const balance = parseInt(ethCall.result, 16) / 1e6
    console.log(`USDC balance of ${holder}: ${balance}`)
  }

  // Test transfer call encoding
  const amount = '1000000000000000000'
  const to = '0xd8da6bf26964af9d7eed9e03e53415d37aa96045'
  const transferData = wasm.calldata_encode('transfer(address,uint256)', [
    to,
    amount,
  ])
  console.log(`transfer calldata: ${transferData}`)

  // Simulate transfer (will likely fail as expected)
  const transferResult = await wasm.rpc(url, 'eth_call', [{
    to: token,
    data: transferData,
  }, 'latest'])
  if (transferResult?.error) {
    console.log(
      `Transfer simulation error (expected): ${transferResult.error.message}`,
    )
  } else if (transferResult?.result) {
    console.log(`Transfer simulation result: ${transferResult.result}`)
  }
} catch (error) {
  console.log(`RPC error: ${error}`)
}

// Test all new functions from test-new.ts
console.log('\n=== Unit Conversions ===')
const wei1 = wasm.to_wei('1.5', 'ether')
console.log(`to_wei("1.5", "ether") = ${wei1}`)
const eth1 = wasm.from_wei('1500000000000000000', 'ether')
console.log(`from_wei("1500000000000000000", "ether") = ${eth1}`)
const parsedUnits = wasm.parse_units('1.234', 6)
console.log(`parse_units("1.234", 6) = ${parsedUnits}`)
const formatted = wasm.format_units('1234000', 6)
console.log(`format_units("1234000", 6) = ${formatted}`)

// Test base conversions
console.log('\n=== Base Conversions ===')
const bin1 = wasm.to_base('255', 'bin')
console.log(`to_base("255", "bin") = ${bin1}`)
const hex2 = wasm.to_base('0b11111111', 'hex')
console.log(`to_base("0b11111111", "hex") = ${hex2}`)
const oct1 = wasm.to_base('0x100', 'oct')
console.log(`to_base("0x100", "oct") = ${oct1}`)

// Test integer bounds
console.log('\n=== Integer Bounds ===')
const max256 = wasm.max_int('256')
console.log(`max_int("256") = ${max256}`)
const min256 = wasm.min_int('256')
console.log(`min_int("256") = ${min256}`)
const max8 = wasm.max_int('8')
console.log(`max_int("8") = ${max8}`)

// Test bytes32 operations
console.log('\n=== Additional Bytes32 Operations ===')
const bytes32_1 = wasm.to_bytes32('hello')
console.log(`to_bytes32("hello") = ${bytes32_1}`)
const addr = wasm.parse_bytes32_address(
  '0x000000000000000000000000742d35cc6634c0532925a3b844bc9e7595f0beb3',
)
console.log(`parse_bytes32_address("0x00...742d35cc...") = ${addr}`)

// Test ABI packed encoding
console.log('\n=== ABI Packed Encoding ===')
const packed = wasm.abi_encode_packed('address,uint256', [
  '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3',
  '123456',
])
console.log(`abi_encode_packed("address,uint256", [...]) = ${packed}`)

// Test storage index
console.log('\n=== Storage Index ===')
const storageIdx = wasm.storage_index(
  '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3',
  '1',
)
console.log(`storage_index("0x742d35Cc...", "1") = ${storageIdx}`)

console.log('\n✅ All tests completed!')
