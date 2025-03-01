# EVMole

[![try it online](https://img.shields.io/badge/Try_It_Online-evmole.xyz-brightgreen)](https://evmole.xyz/)
[![npm](https://img.shields.io/npm/v/evmole)](https://www.npmjs.com/package/evmole)
[![Crates.io](https://img.shields.io/crates/v/evmole?color=e9b44f)](https://crates.io/crates/evmole)
[![PyPI](https://img.shields.io/pypi/v/evmole?color=006dad)](https://pypi.org/project/evmole)

EVMole is a powerful library that extracts information from Ethereum Virtual Machine (EVM) bytecode, including [function selectors](https://docs.soliditylang.org/en/latest/abi-spec.html#function-selector), arguments, [state mutability](https://docs.soliditylang.org/en/latest/contracts.html#state-mutability), and storage layout, even for unverified contracts.


## Key Features

- Multi-language support: Available as [JavaScript](#javascript), [Rust](#rust), and [Python](#python) libraries.
- High accuracy and performance: [Outperforms](#benchmark) existing tools.
- Broad compatibility: Tested with both Solidity and Vyper compiled contracts.
- Lightweight: Clean codebase with minimal external dependencies.
- Unverified contract analysis: Extracts information even from unverified bytecode.


## Usage
### JavaScript
[API documentation](./javascript/#api) and [usage examples](./javascript#usage) (node, vite, webpack, parcel, esbuild)
```sh
$ npm i evmole
```
```javascript
import { contractInfo } from 'evmole'

const code = '0x6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256'

console.log( contractInfo(code, {selectors:true, arguments:true, stateMutability:true}) )
// {
//   functions: [
//     {
//       selector: '2125b65b',
//       bytecodeOffset: 52,
//       arguments: 'uint32,address,uint224',
//       stateMutability: 'pure'
//     },
//     ...
```

### Rust
Documentation is available on [docs.rs](https://docs.rs/evmole/latest/evmole/)
```rust
let code = hex::decode("6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256").unwrap();

println!("{:?}", evmole::contract_info(
    evmole::ContractInfoArgs::new(&code)
        .with_selectors()
        .with_arguments()
        .with_state_mutability()
    )
);
// Contract {
//     functions: Some([
//         Function {
//             selector: [33, 37, 182, 91],
//             bytecode_offset: 52,
//             arguments: Some([Uint(32), Address, Uint(224)]),
//             state_mutability: Some(Pure)
//         },
//         ...
```

### Python
[API documentation](./python/#api)
```sh
$ pip install evmole --upgrade
```
```python
from evmole import contract_info

code = '0x6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256'

print( contract_info(code, selectors=True, arguments=True, state_mutability=True) )
# Contract(
#     functions=[
#     Function(
#             selector=2125b65b,
#             bytecode_offset=52,
#             arguments=uint32,address,uint224,
#             state_mutability=pure),
#     ...
```

### Foundry
<a href="https://getfoundry.sh/">Foundy's cast</a> uses the Rust implementation of EVMole
```sh

$ cast selectors $(cast code 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2)
0x06fdde03                           view
0x095ea7b3  address,uint256          nonpayable
0x18160ddd                           view
0x23b872dd  address,address,uint256  nonpayable
...

$ cast selectors --resolve $(cast code 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2)
0x06fdde03                           view        name()
0x095ea7b3  address,uint256          nonpayable  approve(address,uint256)
0x18160ddd                           view        totalSupply()
0x23b872dd  address,address,uint256  nonpayable  transferFrom(address,address,uint256)
...
```

## Benchmark

### function selectors
<i>FP/FN</i> - [False Positive/False Negative](https://en.wikipedia.org/wiki/False_positives_and_false_negatives) errors; <b>smaller is better</b>

<table>
 <tr>
  <td>Dataset</td>
  <td></td>
  <td><b><i>evmole</i><b> <a href="benchmark/providers/evmole-rs/"><b><i>rs</i></b></a> · <a href="benchmark/providers/evmole-js/"><b><i>js</i></b></a> · <a href="benchmark/providers/evmole-py/"><b><i>py</i></b></a></td>
  <td><a href="benchmark/providers/whatsabi/"><b><i>whatsabi</i></b></a></td>
  <td><a href="benchmark/providers/sevm/"><b><i>sevm</i></b></a></td>
  <td><a href="benchmark/providers/evm-hound-rs/"><b><i>evmhound</i></b></a></td>
  <td><a href="benchmark/providers/heimdall-rs/"><b><i>heimdall</i></b></a></td>
  <td><a href="benchmark/providers/simple/"><b><i>smpl</i></b></a></td>
 </tr>
 <tr>
  <td rowspan="5"><b>largest1k</b><br><sub>1000<br>addresses<br><br>24427<br>functions</sub></td>
  <td><i>FP <sub>addrs</sub></i></td>
  <td>1 🥈</td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>75</td>
  <td>18</td>
  <td>95</td>
 </tr>
 <tr>
  <td><i>FN <sub>addrs</sub></i></td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>40</td>
  <td>111</td>
  <td>9</td>
 </tr>
 <tr>
  <td><i>FP <sub>funcs</sub></i></td>
  <td>192 🥈</td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>720</td>
  <td>600</td>
  <td>749</td>
 </tr>
 <tr>
  <td><i>FN <sub>funcs</sub></i></td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>0 🥇</td>
  <td>191</td>
  <td>147</td>
  <td>12</td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>0.4s · 0.6s · 0.4s</td>
  <td>2.8s</td>
  <td>35s<sup>(*)</sup></td>
  <td>0.4s</td>
  <td>343s<sup>(*)</sup></td>
  <td>1.6s</td>
 </tr>
 <tr><td colspan="8"></td></tr>
 <tr>
  <td rowspan="5"><b>random50k</b><br><sub>50000<br>addresses<br><br>1171102<br>functions</sub></td>
  <td><i>FP <sub>addrs</sub></i></td>
  <td>1 🥇</td>
  <td>43</td>
  <td>1</td>
  <td>693</td>
  <td>3</td>
  <td>4136</td>
 </tr>
 <tr>
  <td><i>FN <sub>addrs</sub></i></td>
  <td>9 🥇</td>
  <td>11</td>
  <td>10</td>
  <td>2903</td>
  <td>4708</td>
  <td>77</td>
 </tr>
 <tr>
  <td><i>FP <sub>funcs</sub></i></td>
  <td>3 🥇</td>
  <td>51</td>
  <td>3</td>
  <td>10798</td>
  <td>29</td>
  <td>14652</td>
 </tr>
 <tr>
  <td><i>FN <sub>funcs</sub></i></td>
  <td>10 🥇</td>
  <td>12</td>
  <td>11</td>
  <td>3538</td>
  <td>6098</td>
  <td>96</td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>4.3s · 9.0s · 7.2s</td>
  <td>53s</td>
  <td>1124s<sup>(*)</sup></td>
  <td>5.8s</td>
  <td>8572s<sup>(*)</sup></td>
  <td>44s</td>
 </tr>
 <tr><td colspan="8"></td></tr>
 <tr>
  <td rowspan="5"><b>vyper</b><br><sub>780<br>addresses<br><br>21244<br>functions</sub></td>
  <td><i>FP <sub>addrs</sub></i></td>
  <td>0 🥇</td>
  <td>30</td>
  <td>0</td>
  <td>19</td>
  <td>0</td>
  <td>185</td>
 </tr>
 <tr>
  <td><i>FN <sub>addrs</sub></i></td>
  <td>0 🥇</td>
  <td>780</td>
  <td>0</td>
  <td>300</td>
  <td>780</td>
  <td>480</td>
 </tr>
 <tr>
  <td><i>FP <sub>funcs</sub></i></td>
  <td>0 🥇</td>
  <td>30</td>
  <td>0</td>
  <td>19</td>
  <td>0</td>
  <td>197</td>
 </tr>
 <tr>
  <td><i>FN <sub>funcs</sub></i></td>
  <td>0 🥇</td>
  <td>21244</td>
  <td>0</td>
  <td>8273</td>
  <td>21244</td>
  <td>12971</td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>0.3s · 0.6s · 0.4s</td>
  <td>2.0s</td>
  <td>46s<sup>(*)</sup></td>
  <td>0.3s</td>
  <td>28s<sup>(*)</sup></td>
  <td>1.0s</td>
 </tr>
</table>

### function arguments
<i>Errors</i> - when at least 1 argument is incorrect: `(uint256,string)` ≠ `(uint256,bytes)`

<table>
 <tr>
  <td>Dataset</td>
  <td></td>
  <td><b><i>evmole</i><b> <a href="benchmark/providers/evmole-rs/"><b><i>rs</i></b></a> · <a href="benchmark/providers/evmole-js/"><b><i>js</i></b></a> · <a href="benchmark/providers/evmole-py/"><b><i>py</i></b></a></td>
  <td><a href="benchmark/providers/heimdall-rs/"><b><i>heimdall</i></b></a></td>
  <td><a href="benchmark/providers/simple/"><b><i>smpl</i></b></a></td>
 </tr>
 <tr>
  <td rowspan="2"><b>largest1k</b><br><sub>24427<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>14.0% 🥇<br><sub>3410</sub></td>
  <td>31.1%<br><sub>7593</sub></td>
  <td>58.3%<br><sub>14242</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>1.0s · 2.1s · 1.1s</td>
  <td>344s<sup>(*)</sup></td>
  <td>0.5s</td>
 </tr>
 <tr><td colspan="5"></td></tr>
 <tr>
  <td rowspan="2"><b>random50k</b><br><sub>1171102<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>4.5% 🥇<br><sub>52670</sub></td>
  <td>19.4%<br><sub>227612</sub></td>
  <td>54.9%<br><sub>643213</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>20s · 49s · 24s</td>
  <td>8517s<sup>(*)</sup></td>
  <td>8.1s</td>
 </tr>
 <tr><td colspan="5"></td></tr>
 <tr>
  <td rowspan="2"><b>vyper</b><br><sub>21244<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>48.5% 🥇<br><sub>10299</sub></td>
  <td>100.0%<br><sub>21244</sub></td>
  <td>56.8%<br><sub>12077</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>0.7s · 1.4s · 0.8s</td>
  <td>28s<sup>(*)</sup></td>
  <td>0.5s</td>
 </tr>
</table>

### function state mutability

<i>Errors</i> - Results are not equal (treating `view` and `pure` as equivalent to `nonpayable`)

<i>Errors strict</i> - Results are strictly unequal (`nonpayable` ≠ `view`). Some ABIs mark `pure`/`view` functions as `nonpayable`, so not all strict errors indicate real issues.

<table>
 <tr>
  <td>Dataset</td>
  <td></td>
  <td><b><i>evmole</i><b> <a href="benchmark/providers/evmole-rs/"><b><i>rs</i></b></a> · <a href="benchmark/providers/evmole-js/"><b><i>js</i></b></a> · <a href="benchmark/providers/evmole-py/"><b><i>py</i></b></a></td>
  <td><a href="benchmark/providers/whatsabi/"><b><i>whatsabi</i></b></a></td>
  <td><a href="benchmark/providers/sevm/"><b><i>sevm</i></b></a></td>
  <td><a href="benchmark/providers/heimdall-rs/"><b><i>heimdall</i></b></a></td>
  <td><a href="benchmark/providers/simple/"><b><i>smpl</i></b></a></td>
 </tr>
 <tr>
  <td rowspan="3"><b>largest1k</b><br><sub>24427<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>0.0% 🥇<br><sub>0</sub></td>
  <td>68.1%<br><sub>16623</sub></td>
  <td>2.1%<br><sub>501</sub></td>
  <td>25.4%<br><sub>6201</sub></td>
  <td>2.6%<br><sub>643</sub></td>
 </tr>
 <tr>
  <td><i>Errors strict</i></td>
  <td>18.6% 🥇<br><sub>4555</sub></td>
  <td>79.4%<br><sub>19393</sub></td>
  <td>59.0%<br><sub>14417</sub></td>
  <td>54.9%<br><sub>13403</sub></td>
  <td>60.9%<br><sub>14864</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>8.9s · 16s · 8.9s</td>
  <td>3.8s</td>
  <td>33s<sup>(*)</sup></td>
  <td>339s<sup>(*)</sup></td>
  <td>0.5s</td>
 </tr>
 <tr><td colspan="6"></td></tr>
 <tr>
  <td rowspan="3"><b>random50k</b><br><sub>1160861<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>0.0% 🥇<br><sub>44</sub></td>
  <td>30.2%<br><sub>351060</sub></td>
  <td>0.3%<br><sub>3370</sub></td>
  <td>11.6%<br><sub>134196</sub></td>
  <td>2.2%<br><sub>24961</sub></td>
 </tr>
 <tr>
  <td><i>Errors strict</i></td>
  <td>6.8% 🥇<br><sub>78923</sub></td>
  <td>58.2%<br><sub>675111</sub></td>
  <td>55.7%<br><sub>646831</sub></td>
  <td>27.7%<br><sub>321495</sub></td>
  <td>57.7%<br><sub>670318</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>169s · 334s · 172s</td>
  <td>88s</td>
  <td>1131s<sup>(*)</sup></td>
  <td>8267s<sup>(*)</sup></td>
  <td>8.2s</td>
 </tr>
 <tr><td colspan="6"></td></tr>
 <tr>
  <td rowspan="3"><b>vyper</b><br><sub>21166<br>functions</sub></td>
  <td><i>Errors</i></td>
  <td>0.5% 🥇<br><sub>110</sub></td>
  <td>100.0%<br><sub>21166</sub></td>
  <td>76.3%<br><sub>16150</sub></td>
  <td>100.0%<br><sub>21166</sub></td>
  <td>1.8%<br><sub>390</sub></td>
 </tr>
 <tr>
  <td><i>Errors strict</i></td>
  <td>4.0% 🥇<br><sub>854</sub></td>
  <td>100.0%<br><sub>21166</sub></td>
  <td>90.2%<br><sub>19092</sub></td>
  <td>100.0%<br><sub>21166</sub></td>
  <td>59.6%<br><sub>12610</sub></td>
 </tr>
 <tr>
  <td><i>Time</i></td>
  <td>9.5s · 13s · 9.7s</td>
  <td>2.1s</td>
  <td>38s<sup>(*)</sup></td>
  <td>28s<sup>(*)</sup></td>
  <td>0.5s</td>
 </tr>
</table>

See [benchmark/README.md](./benchmark/) for the methodology and commands to reproduce these results

<i>versions: evmole v0.6.2; <a href="https://github.com/shazow/whatsabi">whatsabi</a> v0.17.0; <a href="https://github.com/acuarica/evm">sevm</a> v0.7.3; <a href="https://github.com/g00dv1n/evm-hound-rs">evm-hound-rs</a> v0.1.4; <a href="https://github.com/Jon-Becker/heimdall-rs">heimdall-rs</a> v0.8.4</i>

<sup>(*)</sup>: <b>sevm</b> and <b>heimdall-rs</b> are full decompilers, not limited to extracting function selectors

## How it works

Short: Executes code with a custom EVM and traces CALLDATA usage.

Long: TODO

## License
MIT
