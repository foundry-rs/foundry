# Polkadot Foundry Supported Forge Commands Documentation with Examples

## Documentation Format and Color Scheme

This documentation is structured to provide a clear overview of the supported `forge` commands. Each command is presented in the following format:

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

## [Forge Commands](https://github.com/paritytech/foundry-polkadot/issues/54)

### Project Setup and Installation

#### ✅ <span style="color: green;">init</span>
- **Command**: `forge init`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge init
  Initializing /home/ec2-user/test-foundry/example...
  Installing forge-std in /home/ec2-user/test-foundry/example/lib/forge-std (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
  Cloning into '/home/ec2-user/test-foundry/example/lib/forge-std'...
  remote: Enumerating objects: 2111, done.
  remote: Counting objects: 100% (1042/1042), done.
  remote: Compressing objects: 100% (150/150), done.
  remote: Total 2111 (delta 955), reused 904 (delta 892), pack-reused 1069 (from 1)
  Receiving objects: 100% (2111/2111), 680.96 KiB | 17.92 MiB/s, done.
  Resolving deltas: 100% (1431/1431), done.
      Installed forge-std v1.9.7
      Initialized forge project
  ```
  </details>

#### ✅ <span style="color: green;">inspect</span>
- **Command**: `forge inspect`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler. When running with this flag the output for the bytecode should start with `0x505`.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge inspect Counter bytecode --resolc
  0x50564d00008213000000000000010700c13000c0008004808f08000000000e0000001c0000002a0000003500000040000000520000005d00000063616c6c5f646174615f636f707963616c6c5f646174615f6c6f616463616c6c5f646174615f73697a656765745f73746f726167657365616c5f72657475726e7365745f696d6d757461626c655f646174617365745f73746f7261676576616c75655f7472616e736665727265640511028f110463616c6c8f19066465706c6f790692c34b02902415005d012b028102d202d702030334035d03660379038503ab03030408040d044004730478049005c405c905150673067a067f0688068d069906d706320748075c079707c5072f0849085f089c08d4084e095d09d5096c0a910a4b0bcc0bd50be10b470c4c0c530c580c600c650c710c8d0c920ce50c6c0d820d900daa0dc60dc80ddf0d030e740ea90ed10eff0e190f220f3e0f430f390808000251086b03330730000383770a05285e037c78017c797c7a027c7b03978808d4980897aa1097bb18d4ba0ad4a8087c79057c7a047c7b067c7c07979908d4a90997bb1097cc18d4cb0bd4b909979920d489027c79097c7a087c7b0a7c7c0b979908d4a90997bb1097cc18d4cb0bd4b9097c7a0d7c7b0c7c7c0e7c780f97aa08d4ba0a97cc10978818d4c808d4a808978820d498037c78117c7a107c7b127c7c13978808d4a80897bb1097cc18d4cb0bd4b8087c7a157c7b147c7c167c791797aa08d4ba0a97cc10979918d4c909d4a909979920d4890a7c78197c79187c7b1a7c7c1b978808d4980897bb1097cc18d4cb0bd4b8087c791d7c7b1c7c7c1e7c771f979908d4b90997cc10977718d4c707d49707977720d487076f776fa86f396f2a7b5a187b59107b58087b57821008821595111032009511d87b10207b15187b161082897b19088289087b198285108286183308205010042b016f686f59821a6faa821b086fbb787b18787a10787908787898bc38787c1f98bc30787c1e98bc28787c1d98bc20787c1c98bc18787c1b98bc10787c1a98bb08787b1998ab38787b1798ab30787b1698ab28787b1598ab20787b1498ab18787b1398ab10787b1298aa08787a11989a38787a0f989a30787a0e989a28787a0d989a20787a0c989a18787a0b989a10787a0a98990878790998893878790798893078790698892878790598892078790498891878790398891078790298880878780182102082151882161095112832006f776f996faa6f887b18187b1a107b19087b17491138491130491128491120481140208318831a20831b403309ff33070a03821738821830821928821a206f776f886f996faa7b6a187b69107b68087b675012084b0d32008b7910520931c8780883881f8488e05638000001253309040002390a040002ae8a093d080400020133081000028377c887073200009511f07b10087b158475010a02013d0700000251050750100a0950100cb3009511807b10787b15707b1668951580008411e0491138491130491120800033074095182049112850100e3bfe4911584911504911484911408317400a0701821750821858821948821a40d49808d4a707d4870752072e641750101011018217188218108219088216d49707d48609d47909989920d48707977720d497075107090050101219096467330850101422ff8377330833090a2893fc646733085010160fff8378330733093300180a04019511a07b10587b15509515608411e049111849111049018000330740641849110850101a93fd39070000025317045383172033080a010181173c51478af581834e51478ae09dd0425247cbc1b53f3633001c951120ff7b10d8007b15d0009515e0008411e04921b8004921b0004921a8004921a0008317a0000a0728cb0150101e6e08501020d9073300229511a0fe7b1058017b1550017b164801951560018411e049213801492130014921280149212001831720010a072848059511c07b10387b15307b16289515408411f0647664173308403300249511f07b10087b156489647533082064973300022813fe5012260b0b3200828910828a18828b088288d4ba0ad4980bd4ab0b98bb20d4a909979920d4b9095209449511c07b10387b15307b16289515408411e06476838883170a01821718821810821908821a7b67187b68107b69087b6a9551c0821038821530821628951140320000951150ff7b10a8007b15a0007b1698009515b0008411f0828410828308829608828c7b1c10829b829210d3360a7b1a28d8360ad8cb0c821028da0c0a64308288187b18208298187b1818c94209c9a9087b1828d8a9087b1408d8420a821420821310821918c94909c9a909c98909c90606c9c608c93b0a8e8c88aa2085aa01db8c0a8f968218288e8cdb960cd49808db8c0a510a4e64767b13507b10588217087b17609517709518507b1468501028effe8217800082188800821970821a787b67107b68187b697b6a08955150ff8210a8008215a000821698009511b000320050102ab20650122c810932008217b0008218b8008219a800821aa000d49808d4a707d4870752079f0038070000024911584911504911487b1740491178491170491160049517800095186095194049116850102ecffe821880008217880082199000821a98007b1a387b19307b17289517207b18203300309511b07b10487b15409515508411f08279827808827a108277184911384911304911284911207b17187b1a107b180895172064187b1933007a2824066417501032cbfd5012342b0951080900501036ee05501038fbfb83783307330933003a0a0401827218828318827b10827c088289088274828a828810d3c907d8c909d84a0adb790ac9b807d8a707d8b808c92309c98909c97909570905320050103c9f059511887b10707b15687b16607b174082977b17508297087b17588297107b174882921882861882891082858288088e6a8e9bdb6a0b8e8a885c000185cc01db8a0cd46909db9b0c7b1c388359807b1564267b1218642850103eab087b17307b182083597b192882175082185850104095087b17107b18088d5980008217486468501042a708821508d485058219889a80007b1a08821820daa805821858da9805821610d47606821730daa706821750da970682174882181882192850104446088219089398939782193894969495949794988219407b98187b97107b95087b9682107082156882166095117832008217607b17508217687b17588212708218787b1848821c3898c73d976903d4970798663d821a2897a903d4960698a93d821a4097aa03d4a90997cc037b1c408e7a88cb000185bb01db7a0b8e978e6adb970ad46909db9a0b7b1b2883c98064267b12106427501048ae077b17187b18208217388a79037b193882175082185850104a94077b177b18088218408d898000646782184850104ca407821ad47a0a821940889b80007b1b821718dab70a821750da970a7b1a50821608d48606821720dab706821758da970682171082184882193850104e41078219939793988219289496821a50949a949894978219307b97107b98187b9a7b9608955140ff8210b8008215b0008216a8009511c0003200821750821858821940821a487b67107b68187b697b6a08955180821078821570821668951180003200828a10828b18828c088289d4cb0bd4a908d4b808988820d4ba0a97aa20d4a80852083f9511d07b10287b15209515308411f0827a18827810827b0882777b177b1b087b181064187b1a186497501052f0f79551d08210288215209511303200008218108217087b87088217187b877b861082177b8718955180821078821570821668951180003200821730018218380182192801821a2001d49808d4a707d487075207290238070000024921f8004921f0004921e8007b17e00049211801492110014921000104951700019518e00049210801501056c9fc9517c0003300589511807b10787b15707b1668951580008411f0647649111849111049110849014911384911304911289517409518206419491120330050951140ff7b10b8007b15b0007b16a8009515c0008411e07b17308297187b17408297107b172882960882977b1738828718828910828a0882887b1798007b1990007b1a88007b1880009517609518800033004633020628f4048217c0007b17388217c8007b17308217d0007b17288217d8007b17209517a00050105ab4f98217b8007b17188216b0008218a8007b1810821aa0007b1a088219207b19588219287b19508219307b19488219387b19407b17787b16707b1868951780009518609519407b1a6033005c9511807b10787b15707b1668951580008411f07b171082878292828b08829308957a207b1a18d87a06c86b0a7b1a08d8ba0cda660c828a10828818829410829918c8ca06d8a60cc88c0c7b1c7b18387b1a307b1b287b17207b19587b14507b13489517409518207b124033005428b5fd821918821b10821008d49b07d46008d47808988820d46707977720d4870752075d646482178800821898007b183882138000821a9000d3b706d8b70cd80308da680cc94a06c9c602d8c606d84a0a821c38c99c0cc9ac0cc96c0cc9b707c98707c90306d4c707d42608d47808d42707988820977720d487075107090050105eaa0064076468501060b3f68378836933073300620a0401951160ff7b1098007b1590009515a0008411e04911784911704911684911608317600a0701821770821878821968821a60d49808d4a707d4870752074138070000024911384911304911287b17204911584911504911400495174095182049114850106457fa5010662d026417501068f2f750126a52035108080050106c1550106e23f68378330733093300700a04019511f87b103308100002838833070133093300720a0433027428bb02501274b8023200951160ff7b1098007b1590007b1688009515a0008411e082897b19188289087b19108289107b1908828618827818827910827a0882777b18587b19507b1a487b1740951720951840330076330206287b027b16788217087b17708217107b17688217187b17609517409518603300789511a07b10587b15509515608411e08272827a08827b108277188283828908828c108288186f746fbb6faa6f276f826fcc6f996f387b17187b1a107b1b087b147b18387b19307b1c287b12208318831a203309ff330b2033070a069551a08210588215509511603200955160ff8210980082159000821688009511a00032009551b082104882154095115032007b17387b19307b1a287b1820641795182033007e33020628b6018217108218188219821a087b67107b68187b697b6a08502280000702320049111849111049011133070464184911085020840060f3390804000256183f0b2003040002400133081000028388330701330924330086000a04018289828218828a08828c10959b0188b901c8a909d49b0a88aa01c8ca0ad8ca0cc82c0cd4c902d4ab08d428085108107b7b7b79087b7a107b7c183200330088009511b07b10487b15409515508411f0491130491128491120140700000000717b484e9518207b173833073300820028c8f2951130ff7b10c8007b15c0009515d0008411f04921980049219000492188009517a000951880004921800033008a0033027c28b7008219a0008217a8008218b000821ab8007b1a587b18507b17489517609518407b194050208c0038ff821760821868821970821a787b1a187b19107b18087b17491138491130491128951720641849112050208e009bfd955130ff8210c8008215c0009511d0003200330750209000a5f3330701502092009cf39511c07b10387b15307b16289515408411f0647664175020940008f550229600403200828918828a10828b0882887b79187b7a107b7b087b7832029511a07b10587b15507b16489515608411e06476828718828910828a08828832028217108218188219821a087b67107b68187b697b6a089551c08210388215308216289511403202821818821910821a088217d4a808d4970ad48a0a98aa20d49808978820d4a80832029551a08210588215508216489511603202849a40520a195109148d9a40cfa80ad09808d09707d4a707320032009597c0d0780733083200849a40520a195109148d9a40d0a70acf9707cf9808d4a808320032009598c0cf87083307320021422525499224499224499224499224499224499224499224499224499224499224495295a49492a4a424a12a252949922449922449922449922449922449922449922449aa4a529244492a49aa92144a920421240ca54969488424892421494892344992a490a4244992a1423515aa344992444a08919a402090888844444409211111918888284992549254290925294992244992aa24252949d2888848925292242949922449922449494a49529224091149522222a2108a884812214992882421222249922449499224492a29840ca14a93249524499224852449499224a5522449522924250949a4901449922449928454aa92a42429499224499224495224952429852449124aa2849228499224499250a554494a4444942449499228494992244992249594429234494a4992282222498488888810112192249192942429898888244952921411914812894422112211892449922489244992482a4992244992a424499224499224495244229224499224499224c950a14a23229224699224494892249284500819a14a5328a5241411112949922425499224499224499294a4245555494a9292549214111125494952922425294149528210c150122d499224499294a448922409202912119188888848222292240992244949528288880a12244952412849492a499292545292922429494992a424494992249554922449250500
  ```
  </details>

### Compilation and Testing

#### ✅ <span style="color: green;">build</span>
- **Command**: `forge build`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge build --resolc
  [⠊] Compiling...
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol
  ```
  </details>

#### ❌ <span style="color: red;">test</span>
- **Command**: `forge test`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge test
  [⠊] Compiling...
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol

  Ran 1 test for test/Counter.t.sol:CounterTest
  [FAIL: EvmError: StackUnderflow] constructor() (gas: 0)
  Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in 4.52ms (0.00ns CPU time)

  Ran 1 test suite in 118.49ms (4.52ms CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

  Failing tests:
  Encountered 1 failing test in test/Counter.t.sol:CounterTest
  [FAIL: EvmError: StackUnderflow] constructor() (gas: 0)

  Encountered a total of 1 failing tests, 0 tests succeeded
  ```
  </details>

#### ❌ <span style="color: red;">snapshot</span>
- **Command**: `forge snapshot`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge snapshot --resolc
  [⠃] Compiling...
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol

  Ran 1 test for test/Counter.t.sol:CounterTest
  [FAIL: EvmError: StackUnderflow] constructor() (gas: 0)
  Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in 1.02ms (0.00ns CPU time)

  Ran 1 test suite in 110.19ms (1.02ms CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

  Failing tests:
  Encountered 1 failing test in test/Counter.t.sol:CounterTest
  [FAIL: EvmError: StackUnderflow] constructor() (gas: 0)

  Encountered a total of 1 failing tests, 0 tests succeeded
  ```
  </details>

#### ✅ <span style="color: green;">bind</span>
- **Command**: `forge bind`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge bind --resolc
  [⠒] Compiling...
  Compiler run successful!
  Generating bindings for 2 contracts
  Bindings have been generated to /home/ec2-user/test-foundry/out/bindings
  ```
  </details>

#### ✅ <span style="color: green;">bind</span>
- **Command**: `forge bind-json`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge bind-json --resolc
  [⠒] Compiling...
  Bindings have been generated to /home/ec2-user/test-foundry/utils/JsonBindings.sol
  ```
  </details>

### Contract Deployment

#### ✅ <span style="color: green;">create</span>
- **Command**: `forge create [OPTIONS] <CONTRACT>`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Required Parameters**: `CONTRACT`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge create Counter --resolc --rpc-url https://westend-asset-hub-eth-rpc.polkadot.io --private-key 5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133 --broadcast -vvvvv --constructor-args 5
  [⠊] Compiling...
  No files changed, compilation skipped
  Deployed to: 0x36c09D9A72BE4c2A18dC2537e81A419C8955e223
  ```
  </details>

### Code Manipulation and Documentation

#### ✅ <span style="color: green;">flatten</span>
- **Command**: `forge flatten [OPTIONS] <PATH>`
- **Required Parameters**: `PATH`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge flatten src/Counter.sol
  // SPDX-License-Identifier: UNLICENSED
  pragma solidity ^0.8.13;

  contract Counter {
      uint256 public number;

      function setNumber(uint256 newNumber) public {
          number = newNumber;
      }

      function increment() public {
          number++;
      }
  }
  ```
  </details>

#### ✅ <span style="color: green;">doc</span>
- **Command**: `forge doc`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge doc
  ```
  </details>

#### ✅ <span style="color: green;">cache clean</span>
- **Command**: `forge cache clean`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge cache clean
  ```
  </details>

#### ✅ <span style="color: green;">cache ls</span>
- **Command**: `forge cache ls`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge cache ls
  ```
  </details>

#### ✅ <span style="color: green;">selectors upload</span>
- **Command**: `forge selectors upload`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors upload --all
  [⠃] Compiling...
  Compiler run successful!
  Uploading selectors for Counter...
  Duplicated: Function increment(): 0xd09de08a
  Duplicated: Function number(): 0x8381f58a
  Duplicated: Function setNumber(uint256): 0x3fb5c1cb
  Selectors successfully uploaded to OpenChain
  ```
  </details>

#### ✅ <span style="color: green;">selectors list</span>
- **Command**: `forge selectors list`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors list
  Listing selectors for contracts in the project...
  Counter

  ╭----------+--------------------+------------╮
  | Type     | Signature          | Selector   |
  +============================================+
  | Function | increment()        | 0xd09de08a |
  |----------+--------------------+------------|
  | Function | number()           | 0x8381f58a |
  |----------+--------------------+------------|
  | Function | setNumber(uint256) | 0x3fb5c1cb |
  ╰----------+--------------------+------------╯
  ```
  </details>

#### ✅ <span style="color: green;">selectors find</span>
- **Command**: `forge selectors find`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors find 0xd09de08a
  Searching for selector "0xd09de08a" in the project...

  Found 1 instance(s)...

  ╭----------+-------------+------------+----------╮
  | Type     | Signature   | Selector   | Contract |
  +================================================+
  | Function | increment() | 0xd09de08a | Counter  |
  ╰----------+-------------+------------+----------╯
  ```
  </details>

#### ✅ <span style="color: green;">selectors cache</span>
- **Command**: `forge selectors cache`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors cache
  Caching selectors for contracts in the project...
  ```
  </details>

#### ✅ <span style="colr: green;">cache clean</span>
- **Command**: `forge clean`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge clean
  ```
  </details>

#### ✅ <span style="color: green;">compiler resolve</span>
- **Command**: `forge compiler resolve`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge compiler resolve
  Solidity:
  - 0.8.29
  ```
  </details>

#### ✅ <span style="color: green;">config</span>
- **Command**: `forge config`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge config
  ```
  </details>

#### ❌ <span style="color: red;">coverage</span>
- **Command**: `forge coverage`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge coverage
  ```
  </details>

#### ✅ <span style="color: green;">fmt</span>
- **Command**: `forge fmt`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge fmt
  ```
  </details>

#### ✅ <span style="color: green;">tree</span>
- **Command**: `forge tree`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge tree
  ```
  </details>

#### ✅ <span style="color: green;">update</span>
- **Command**: `forge update`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge update
  ```
  </details>

#### ✅ <span style="color: green;">install</span>
- **Command**: `forge install`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge install --no-git transmissions11/solmate
  Installing solmate in lib/solmate (url: Some("https://github.com/transmissions11/solmate"), tag: None)
  Cloning into 'lib/solmate'...
  remote: Enumerating objects: 90, done.
  remote: Counting objects: 100% (90/90), done.
  remote: Compressing objects: 100% (76/76), done.
  remote: Total 90 (delta 11), reused 43 (delta 8), pack-reused 0 (from 0)
  Receiving objects: 100% (90/90), 220.04 KiB | 1.51 MiB/s, done.
  Resolving deltas: 100% (11/11), done.
  Submodule 'lib/ds-test' (https://github.com/dapphub/ds-test) registered for path 'lib/ds-test'
  Cloning into 'lib/ds-test'...
  remote: Enumerating objects: 15, done.        
  remote: Counting objects: 100% (15/15), done.        
  remote: Compressing objects: 100% (11/11), done.        
  remote: Total 15 (delta 0), reused 11 (delta 0), pack-reused 0 (from 0)        
  Receiving objects: 100% (15/15), 18.34 KiB | 481.00 KiB/s, done.
    Installed solmate
  ```
  </details>
