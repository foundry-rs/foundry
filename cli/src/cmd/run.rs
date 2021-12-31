use ethers::prelude::artifacts::SourceFile;

use crate::{
    cmd::Cmd,
    opts::forge::{CompilerArgs, EvmOpts},
};
use ethers::{abi::Abi, prelude::artifacts::DeployedBytecode};
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use structopt::StructOpt;
use ui::{TUIExitReason, Tui, Ui};

use ethers::{
    prelude::artifacts::CompactContract,
    solc::{
        artifacts::{Optimizer, Settings},
        Project, ProjectPathsConfig, SolcConfig,
    },
};

use evm_adapters::Evm;

use ansi_term::Colour;

#[derive(Debug, Clone, StructOpt)]
pub struct RunArgs {
    #[structopt(help = "the path to the contract to run")]
    pub path: PathBuf,

    #[structopt(flatten)]
    pub compiler: CompilerArgs,

    #[structopt(flatten)]
    pub evm_opts: EvmOpts,

    #[structopt(
        long,
        short,
        help = "the function you want to call on the script contract, defaults to run()"
    )]
    pub sig: Option<String>,

    #[structopt(
        long,
        short,
        help = "the contract you want to call and deploy, only necessary if there are more than 1 contract (Interfaces do not count) definitions on the script"
    )]
    pub contract: Option<String>,

    #[structopt(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        // Keeping it like this for simplicity.
        #[cfg(not(feature = "sputnik-evm"))]
        unimplemented!("`run` does not work with EVMs other than Sputnik yet");

        let func = IntoFunction::into(self.sig.as_deref().unwrap_or("run()"));
        let (contract, highlevel_known_contracts, sources) = self.build()?;
        let known_contracts: BTreeMap<String, (Abi, Vec<u8>)> = highlevel_known_contracts
            .iter()
            .map(|(name, (abi, deployed_b))| {
                (
                    name.clone(),
                    (
                        abi.clone(),
                        deployed_b
                            .clone()
                            .bytecode
                            .expect("no bytes")
                            .object
                            .into_bytes()
                            .expect("not bytecode")
                            .to_vec(),
                    ),
                )
            })
            .collect();
        let (abi, bytecode, runtime_bytecode) = contract.into_parts();

        // this should never fail if compilation was successful
        let abi = abi.unwrap();
        let bytecode = bytecode.unwrap();
        let _runtime_bytecode = runtime_bytecode.unwrap();

        // 2. instantiate the EVM w forked backend if needed / pre-funded account(s)
        let mut cfg = crate::utils::sputnik_cfg(self.compiler.evm_version);
        let vicinity = self.evm_opts.vicinity()?;
        let mut evm = crate::utils::sputnik_helpers::evm(&self.evm_opts, &mut cfg, &vicinity)?;

        // 3. deploy the contract
        let (addr, _, _, logs) = evm.deploy(self.evm_opts.sender, bytecode, 0u32.into())?;

        // 4. set up the runner
        let mut runner =
            ContractRunner::new(&mut evm, &abi, addr, Some(self.evm_opts.sender), &logs);

        // 5. run the test function
        let result = runner.run_test(&func, false, Some(&known_contracts))?;

        if self.evm_opts.debug {
            // 6. Boot up debugger

            let source_code: BTreeMap<u32, String> = sources
                .iter()
                .map(|(id, path)| {
                    (
                        *id,
                        std::fs::read_to_string(path)
                            .expect("Something went wrong reading the file"),
                    )
                })
                .collect();

            println!("{:?}", source_code);

            let calls = evm.debug_calls();
            println!("debugging {}", calls.len());
            let mut flattened = Vec::new();
            calls[0].flatten(0, &mut flattened);
            flattened = flattened[1..].to_vec();
            // flattened.iter().for_each(|flat| {println!("{:?}", flat.1[0..5].iter().map(|step|
            // step.pretty_opcode()).collect::<Vec<String>>().join(", "))});
            let tui = Tui::new(
                flattened,
                0,
                result.identified_contracts.expect("debug but not verbosity"),
                highlevel_known_contracts,
                source_code,
            )?;
            match tui.start().expect("Failed to start tui") {
                TUIExitReason::CharExit => return Ok(()),
            }
        } else {
            // 6. print the result nicely
            if result.success {
                println!("{}", Colour::Green.paint("Script ran successfully."));
            } else {
                println!("{}", Colour::Red.paint("Script failed."));
            }

            println!("Gas Used: {}", result.gas_used);
            println!("== Logs == ");
            result.logs.iter().for_each(|log| println!("{}", log));
        }

        Ok(())
    }
}

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    // TODO: This is too verbose. We definitely want an easier way to do "take this file, detect
    // its solc version and give me all its ABIs & Bytecodes in memory w/o touching disk".
    pub fn build(
        &self,
    ) -> eyre::Result<(
        CompactContract,
        BTreeMap<String, (Abi, DeployedBytecode)>,
        BTreeMap<u32, String>,
    )> {
        let paths = ProjectPathsConfig::builder().root(&self.path).sources(&self.path).build()?;

        let optimizer = Optimizer {
            enabled: Some(self.compiler.optimize),
            runs: Some(self.compiler.optimize_runs as usize),
        };

        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.compiler.evm_version),
            ..Default::default()
        };
        let solc_cfg = SolcConfig::builder().settings(solc_settings).build()?;

        // setup the compiler
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&self.path)
            .solc_config(solc_cfg)
            // we do not want to generate any compilation artifacts in the script run mode
            .no_artifacts()
            // no cache
            .ephemeral();
        if self.no_auto_detect {
            builder = builder.no_auto_detect();
        }
        let project = builder.build()?;

        println!("compiling...");
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        };

        // get the contracts
        let contracts = output.output();
        let sources = contracts
            .sources
            .iter()
            .map(|(path, source_file)| (source_file.id, path.clone()))
            .collect();

        // deployed bytecode one for
        let mut highlevel_known_contracts: BTreeMap<String, (Abi, DeployedBytecode)> =
            Default::default();

        // get the specific contract
        let contract = if let Some(ref contract_name) = self.contract {
            let (_name, contract) = contracts
                .contracts_into_iter()
                .find(|(name, _contract)| name == contract_name)
                .ok_or_else(|| {
                    eyre::Error::msg("contract not found, did you type the name wrong?")
                })?;
            highlevel_known_contracts.insert(
                contract_name.to_string(),
                (
                    contract.abi.clone().expect("no abi"),
                    contract
                        .evm
                        .clone()
                        .expect("no evm")
                        .deployed_bytecode
                        .expect("no deployed bytecode"),
                ),
            );
            CompactContract::from(contract)
        } else {
            let mut contracts = contracts.contracts_into_iter().filter(|(_fname, contract)| {
                // TODO: Should have a helper function for finding if a contract's bytecode is
                // empty or not.
                match contract.evm {
                    Some(ref evm) => match evm.bytecode {
                        Some(ref bytecode) => bytecode
                            .object
                            .as_bytes()
                            .map(|x| !x.as_ref().is_empty())
                            .unwrap_or(false),
                        _ => false,
                    },
                    _ => false,
                }
            });
            let (contract_name, contract) =
                contracts.next().ok_or_else(|| eyre::Error::msg("no contract found"))?;
            highlevel_known_contracts.insert(
                contract_name,
                (
                    contract.abi.clone().expect("no abi"),
                    contract
                        .evm
                        .clone()
                        .expect("no evm")
                        .deployed_bytecode
                        .expect("no deployed bytecode"),
                ),
            );
            if contracts.peekable().peek().is_some() {
                eyre::bail!(
                    ">1 contracts found, please provide a contract name to choose one of them"
                )
            }
            CompactContract::from(contract)
        };
        Ok((contract, highlevel_known_contracts, sources))
    }
}

// sources: {
//     "./run_test.sol": SourceFile {
//         id: 0,
//         ast: Object({
//             "absolutePath": String(
//                 "./run_test.sol",
//             ),
//             "exportedSymbols": Object({
//                 "C": Array([
//                     Number(
//                         89,
//                     ),
//                 ]),
//                 "ERC20": Array([
//                     Number(
//                         12,
//                     ),
//                 ]),
//                 "VM": Array([
//                     Number(
//                         18,
//                     ),
//                 ]),
//             }),
//             "id": Number(
//                 90,
//             ),
//             "nodeType": String(
//                 "SourceUnit",
//             ),
//             "nodes": Array([
//                 Object({
//                     "id": Number(
//                         1,
//                     ),
//                     "literals": Array([
//                         String(
//                             "solidity",
//                         ),
//                         String(
//                             "^",
//                         ),
//                         String(
//                             "0.7",
//                         ),
//                         String(
//                             ".6",
//                         ),
//                     ]),
//                     "nodeType": String(
//                         "PragmaDirective",
//                     ),
//                     "src": String(
//                         "0:23:0",
//                     ),
//                 }),
//                 Object({
//                     "abstract": Bool(
//                         false,
//                     ),
//                     "baseContracts": Array([]),
//                     "contractDependencies": Array([]),
//                     "contractKind": String(
//                         "interface",
//                     ),
//                     "fullyImplemented": Bool(
//                         false,
//                     ),
//                     "id": Number(
//                         12,
//                     ),
//                     "linearizedBaseContracts": Array([
//                         Number(
//                             12,
//                         ),
//                     ]),
//                     "name": String(
//                         "ERC20",
//                     ),
//                     "nodeType": String(
//                         "ContractDefinition",
//                     ),
//                     "nodes": Array([
//                         Object({
//                             "functionSelector": String(
//                                 "70a08231",
//                             ),
//                             "id": Number(
//                                 8,
//                             ),
//                             "implemented": Bool(
//                                 false,
//                             ),
//                             "kind": String(
//                                 "function",
//                             ),
//                             "modifiers": Array([]),
//                             "name": String(
//                                 "balanceOf",
//                             ),
//                             "nodeType": String(
//                                 "FunctionDefinition",
//                             ),
//                             "parameters": Object({
//                                 "id": Number(
//                                     4,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([
//                                     Object({
//                                         "constant": Bool(
//                                             false,
//                                         ),
//                                         "id": Number(
//                                             3,
//                                         ),
//                                         "mutability": String(
//                                             "mutable",
//                                         ),
//                                         "name": String(
//                                             "",
//                                         ),
//                                         "nodeType": String(
//                                             "VariableDeclaration",
//                                         ),
//                                         "scope": Number(
//                                             8,
//                                         ),
//                                         "src": String(
//                                             "66:7:0",
//                                         ),
//                                         "stateVariable": Bool(
//                                             false,
//                                         ),
//                                         "storageLocation": String(
//                                             "default",
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_address",
//                                             ),
//                                             "typeString": String(
//                                                 "address",
//                                             ),
//                                         }),
//                                         "typeName": Object({
//                                             "id": Number(
//                                                 2,
//                                             ),
//                                             "name": String(
//                                                 "address",
//                                             ),
//                                             "nodeType": String(
//                                                 "ElementaryTypeName",
//                                             ),
//                                             "src": String(
//                                                 "66:7:0",
//                                             ),
//                                             "stateMutability": String(
//                                                 "nonpayable",
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_address",
//                                                 ),
//                                                 "typeString": String(
//                                                     "address",
//                                                 ),
//                                             }),
//                                         }),
//                                         "visibility": String(
//                                             "internal",
//                                         ),
//                                     }),
//                                 ]),
//                                 "src": String(
//                                     "65:9:0",
//                                 ),
//                             }),
//                             "returnParameters": Object({
//                                 "id": Number(
//                                     7,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([
//                                     Object({
//                                         "constant": Bool(
//                                             false,
//                                         ),
//                                         "id": Number(
//                                             6,
//                                         ),
//                                         "mutability": String(
//                                             "mutable",
//                                         ),
//                                         "name": String(
//                                             "",
//                                         ),
//                                         "nodeType": String(
//                                             "VariableDeclaration",
//                                         ),
//                                         "scope": Number(
//                                             8,
//                                         ),
//                                         "src": String(
//                                             "98:7:0",
//                                         ),
//                                         "stateVariable": Bool(
//                                             false,
//                                         ),
//                                         "storageLocation": String(
//                                             "default",
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_uint256",
//                                             ),
//                                             "typeString": String(
//                                                 "uint256",
//                                             ),
//                                         }),
//                                         "typeName": Object({
//                                             "id": Number(
//                                                 5,
//                                             ),
//                                             "name": String(
//                                                 "uint256",
//                                             ),
//                                             "nodeType": String(
//                                                 "ElementaryTypeName",
//                                             ),
//                                             "src": String(
//                                                 "98:7:0",
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_uint256",
//                                                 ),
//                                                 "typeString": String(
//                                                     "uint256",
//                                                 ),
//                                             }),
//                                         }),
//                                         "visibility": String(
//                                             "internal",
//                                         ),
//                                     }),
//                                 ]),
//                                 "src": String(
//                                     "97:9:0",
//                                 ),
//                             }),
//                             "scope": Number(
//                                 12,
//                             ),
//                             "src": String(
//                                 "47:60:0",
//                             ),
//                             "stateMutability": String(
//                                 "view",
//                             ),
//                             "virtual": Bool(
//                                 false,
//                             ),
//                             "visibility": String(
//                                 "external",
//                             ),
//                         }),
//                         Object({
//                             "functionSelector": String(
//                                 "d0e30db0",
//                             ),
//                             "id": Number(
//                                 11,
//                             ),
//                             "implemented": Bool(
//                                 false,
//                             ),
//                             "kind": String(
//                                 "function",
//                             ),
//                             "modifiers": Array([]),
//                             "name": String(
//                                 "deposit",
//                             ),
//                             "nodeType": String(
//                                 "FunctionDefinition",
//                             ),
//                             "parameters": Object({
//                                 "id": Number(
//                                     9,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([]),
//                                 "src": String(
//                                     "128:2:0",
//                                 ),
//                             }),
//                             "returnParameters": Object({
//                                 "id": Number(
//                                     10,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([]),
//                                 "src": String(
//                                     "147:0:0",
//                                 ),
//                             }),
//                             "scope": Number(
//                                 12,
//                             ),
//                             "src": String(
//                                 "112:36:0",
//                             ),
//                             "stateMutability": String(
//                                 "payable",
//                             ),
//                             "virtual": Bool(
//                                 false,
//                             ),
//                             "visibility": String(
//                                 "external",
//                             ),
//                         }),
//                     ]),
//                     "scope": Number(
//                         90,
//                     ),
//                     "src": String(
//                         "25:125:0",
//                     ),
//                 }),
//                 Object({
//                     "abstract": Bool(
//                         false,
//                     ),
//                     "baseContracts": Array([]),
//                     "contractDependencies": Array([]),
//                     "contractKind": String(
//                         "interface",
//                     ),
//                     "fullyImplemented": Bool(
//                         false,
//                     ),
//                     "id": Number(
//                         18,
//                     ),
//                     "linearizedBaseContracts": Array([
//                         Number(
//                             18,
//                         ),
//                     ]),
//                     "name": String(
//                         "VM",
//                     ),
//                     "nodeType": String(
//                         "ContractDefinition",
//                     ),
//                     "nodes": Array([
//                         Object({
//                             "functionSelector": String(
//                                 "06447d56",
//                             ),
//                             "id": Number(
//                                 17,
//                             ),
//                             "implemented": Bool(
//                                 false,
//                             ),
//                             "kind": String(
//                                 "function",
//                             ),
//                             "modifiers": Array([]),
//                             "name": String(
//                                 "startPrank",
//                             ),
//                             "nodeType": String(
//                                 "FunctionDefinition",
//                             ),
//                             "parameters": Object({
//                                 "id": Number(
//                                     15,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([
//                                     Object({
//                                         "constant": Bool(
//                                             false,
//                                         ),
//                                         "id": Number(
//                                             14,
//                                         ),
//                                         "mutability": String(
//                                             "mutable",
//                                         ),
//                                         "name": String(
//                                             "",
//                                         ),
//                                         "nodeType": String(
//                                             "VariableDeclaration",
//                                         ),
//                                         "scope": Number(
//                                             17,
//                                         ),
//                                         "src": String(
//                                             "191:7:0",
//                                         ),
//                                         "stateVariable": Bool(
//                                             false,
//                                         ),
//                                         "storageLocation": String(
//                                             "default",
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_address",
//                                             ),
//                                             "typeString": String(
//                                                 "address",
//                                             ),
//                                         }),
//                                         "typeName": Object({
//                                             "id": Number(
//                                                 13,
//                                             ),
//                                             "name": String(
//                                                 "address",
//                                             ),
//                                             "nodeType": String(
//                                                 "ElementaryTypeName",
//                                             ),
//                                             "src": String(
//                                                 "191:7:0",
//                                             ),
//                                             "stateMutability": String(
//                                                 "nonpayable",
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_address",
//                                                 ),
//                                                 "typeString": String(
//                                                     "address",
//                                                 ),
//                                             }),
//                                         }),
//                                         "visibility": String(
//                                             "internal",
//                                         ),
//                                     }),
//                                 ]),
//                                 "src": String(
//                                     "190:9:0",
//                                 ),
//                             }),
//                             "returnParameters": Object({
//                                 "id": Number(
//                                     16,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([]),
//                                 "src": String(
//                                     "208:0:0",
//                                 ),
//                             }),
//                             "scope": Number(
//                                 18,
//                             ),
//                             "src": String(
//                                 "171:38:0",
//                             ),
//                             "stateMutability": String(
//                                 "nonpayable",
//                             ),
//                             "virtual": Bool(
//                                 false,
//                             ),
//                             "visibility": String(
//                                 "external",
//                             ),
//                         }),
//                     ]),
//                     "scope": Number(
//                         90,
//                     ),
//                     "src": String(
//                         "152:59:0",
//                     ),
//                 }),
//                 Object({
//                     "abstract": Bool(
//                         false,
//                     ),
//                     "baseContracts": Array([]),
//                     "contractDependencies": Array([]),
//                     "contractKind": String(
//                         "contract",
//                     ),
//                     "fullyImplemented": Bool(
//                         true,
//                     ),
//                     "id": Number(
//                         89,
//                     ),
//                     "linearizedBaseContracts": Array([
//                         Number(
//                             89,
//                         ),
//                     ]),
//                     "name": String(
//                         "C",
//                     ),
//                     "nodeType": String(
//                         "ContractDefinition",
//                     ),
//                     "nodes": Array([
//                         Object({
//                             "constant": Bool(
//                                 false,
//                             ),
//                             "id": Number(
//                                 23,
//                             ),
//                             "mutability": String(
//                                 "mutable",
//                             ),
//                             "name": String(
//                                 "weth",
//                             ),
//                             "nodeType": String(
//                                 "VariableDeclaration",
//                             ),
//                             "scope": Number(
//                                 89,
//                             ),
//                             "src": String(
//                                 "230:62:0",
//                             ),
//                             "stateVariable": Bool(
//                                 true,
//                             ),
//                             "storageLocation": String(
//                                 "default",
//                             ),
//                             "typeDescriptions": Object({
//                                 "typeIdentifier": String(
//                                     "t_contract$_ERC20_$12",
//                                 ),
//                                 "typeString": String(
//                                     "contract ERC20",
//                                 ),
//                             }),
//                             "typeName": Object({
//                                 "id": Number(
//                                     19,
//                                 ),
//                                 "name": String(
//                                     "ERC20",
//                                 ),
//                                 "nodeType": String(
//                                     "UserDefinedTypeName",
//                                 ),
//                                 "referencedDeclaration": Number(
//                                     12,
//                                 ),
//                                 "src": String(
//                                     "230:5:0",
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_contract$_ERC20_$12",
//                                     ),
//                                     "typeString": String(
//                                         "contract ERC20",
//                                     ),
//                                 }),
//                             }),
//                             "value": Object({
//                                 "arguments": Array([
//                                     Object({
//                                         "hexValue": String(
//
// "307843303261614133396232323346453844304130653543344632376541443930383343373536436332",
//                                         ),
//                                         "id": Number(
//                                             21,
//                                         ),
//                                         "isConstant": Bool(
//                                             false,
//                                         ),
//                                         "isLValue": Bool(
//                                             false,
//                                         ),
//                                         "isPure": Bool(
//                                             true,
//                                         ),
//                                         "kind": String(
//                                             "number",
//                                         ),
//                                         "lValueRequested": Bool(
//                                             false,
//                                         ),
//                                         "nodeType": String(
//                                             "Literal",
//                                         ),
//                                         "src": String(
//                                             "249:42:0",
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_address_payable",
//                                             ),
//                                             "typeString": String(
//                                                 "address payable",
//                                             ),
//                                         }),
//                                         "value": String(
//                                             "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
//                                         ),
//                                     }),
//                                 ]),
//                                 "expression": Object({
//                                     "argumentTypes": Array([
//                                         Object({
//                                             "typeIdentifier": String(
//                                                 "t_address_payable",
//                                             ),
//                                             "typeString": String(
//                                                 "address payable",
//                                             ),
//                                         }),
//                                     ]),
//                                     "id": Number(
//                                         20,
//                                     ),
//                                     "name": String(
//                                         "ERC20",
//                                     ),
//                                     "nodeType": String(
//                                         "Identifier",
//                                     ),
//                                     "overloadedDeclarations": Array([]),
//                                     "referencedDeclaration": Number(
//                                         12,
//                                     ),
//                                     "src": String(
//                                         "243:5:0",
//                                     ),
//                                     "typeDescriptions": Object({
//                                         "typeIdentifier": String(
//                                             "t_type$_t_contract$_ERC20_$12_$",
//                                         ),
//                                         "typeString": String(
//                                             "type(contract ERC20)",
//                                         ),
//                                     }),
//                                 }),
//                                 "id": Number(
//                                     22,
//                                 ),
//                                 "isConstant": Bool(
//                                     false,
//                                 ),
//                                 "isLValue": Bool(
//                                     false,
//                                 ),
//                                 "isPure": Bool(
//                                     true,
//                                 ),
//                                 "kind": String(
//                                     "typeConversion",
//                                 ),
//                                 "lValueRequested": Bool(
//                                     false,
//                                 ),
//                                 "names": Array([]),
//                                 "nodeType": String(
//                                     "FunctionCall",
//                                 ),
//                                 "src": String(
//                                     "243:49:0",
//                                 ),
//                                 "tryCall": Bool(
//                                     false,
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_contract$_ERC20_$12",
//                                     ),
//                                     "typeString": String(
//                                         "contract ERC20",
//                                     ),
//                                 }),
//                             }),
//                             "visibility": String(
//                                 "internal",
//                             ),
//                         }),
//                         Object({
//                             "constant": Bool(
//                                 true,
//                             ),
//                             "id": Number(
//                                 42,
//                             ),
//                             "mutability": String(
//                                 "constant",
//                             ),
//                             "name": String(
//                                 "vm",
//                             ),
//                             "nodeType": String(
//                                 "VariableDeclaration",
//                             ),
//                             "scope": Number(
//                                 89,
//                             ),
//                             "src": String(
//                                 "298:86:0",
//                             ),
//                             "stateVariable": Bool(
//                                 true,
//                             ),
//                             "storageLocation": String(
//                                 "default",
//                             ),
//                             "typeDescriptions": Object({
//                                 "typeIdentifier": String(
//                                     "t_contract$_VM_$18",
//                                 ),
//                                 "typeString": String(
//                                     "contract VM",
//                                 ),
//                             }),
//                             "typeName": Object({
//                                 "id": Number(
//                                     24,
//                                 ),
//                                 "name": String(
//                                     "VM",
//                                 ),
//                                 "nodeType": String(
//                                     "UserDefinedTypeName",
//                                 ),
//                                 "referencedDeclaration": Number(
//                                     18,
//                                 ),
//                                 "src": String(
//                                     "298:2:0",
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_contract$_VM_$18",
//                                     ),
//                                     "typeString": String(
//                                         "contract VM",
//                                     ),
//                                 }),
//                             }),
//                             "value": Object({
//                                 "arguments": Array([
//                                     Object({
//                                         "arguments": Array([
//                                             Object({
//                                                 "arguments": Array([
//                                                     Object({
//                                                         "arguments": Array([
//                                                             Object({
//                                                                 "arguments": Array([
//                                                                     Object({
//                                                                         "arguments": Array([
//                                                                             Object({
//                                                                                 "hexValue":
// String(
// "6865766d20636865617420636f6465",
// ),                                                                                 "id": Number(
//                                                                                     35,
//                                                                                 ),
//                                                                                 "isConstant":
// Bool(                                                                                     false,
//                                                                                 ),
//                                                                                 "isLValue": Bool(
//                                                                                     false,
//                                                                                 ),
//                                                                                 "isPure": Bool(
//                                                                                     true,
//                                                                                 ),
//                                                                                 "kind": String(
//                                                                                     "string",
//                                                                                 ),
//
// "lValueRequested": Bool(
// false,                                                                                 ),
//                                                                                 "nodeType":
// String(
// "Literal",                                                                                 ),
//                                                                                 "src": String(
//                                                                                     "361:17:0",
//                                                                                 ),
//
// "typeDescriptions": Object({
// "typeIdentifier": String(
// "t_stringliteral_885cb69240a935d632d79c317109709ecfa91a80626ff3989d68f67f5b1dd12d",
// ),
// "typeString": String(
// "literal_string \"hevm cheat code\"",
// ),                                                                                 }),
//                                                                                 "value": String(
//                                                                                     "hevm cheat
// code",                                                                                 ),
//                                                                             }),
//                                                                         ]),
//                                                                         "expression": Object({
//                                                                             "argumentTypes":
// Array([                                                                                 Object({
//
// "typeIdentifier": String(
// "t_stringliteral_885cb69240a935d632d79c317109709ecfa91a80626ff3989d68f67f5b1dd12d",
// ),
// "typeString": String(
// "literal_string \"hevm cheat code\"",
// ),                                                                                 }),
//                                                                             ]),
//                                                                             "id": Number(
//                                                                                 34,
//                                                                             ),
//                                                                             "name": String(
//                                                                                 "keccak256",
//                                                                             ),
//                                                                             "nodeType": String(
//                                                                                 "Identifier",
//                                                                             ),
//
// "overloadedDeclarations": Array([]),
// "referencedDeclaration": Number(
// -8,                                                                             ),
//                                                                             "src": String(
//                                                                                 "351:9:0",
//                                                                             ),
//                                                                             "typeDescriptions":
// Object({
// "typeIdentifier": String(
// "t_function_keccak256_pure$_t_bytes_memory_ptr_$returns$_t_bytes32_$",
// ),                                                                                 "typeString":
// String(
// "function (bytes memory) pure returns (bytes32)",
// ),                                                                             }),
//                                                                         }),
//                                                                         "id": Number(
//                                                                             36,
//                                                                         ),
//                                                                         "isConstant": Bool(
//                                                                             false,
//                                                                         ),
//                                                                         "isLValue": Bool(
//                                                                             false,
//                                                                         ),
//                                                                         "isPure": Bool(
//                                                                             true,
//                                                                         ),
//                                                                         "kind": String(
//                                                                             "functionCall",
//                                                                         ),
//                                                                         "lValueRequested": Bool(
//                                                                             false,
//                                                                         ),
//                                                                         "names": Array([]),
//                                                                         "nodeType": String(
//                                                                             "FunctionCall",
//                                                                         ),
//                                                                         "src": String(
//                                                                             "351:28:0",
//                                                                         ),
//                                                                         "tryCall": Bool(
//                                                                             false,
//                                                                         ),
//                                                                         "typeDescriptions":
// Object({
// "typeIdentifier": String(
// "t_bytes32",                                                                             ),
//                                                                             "typeString": String(
//                                                                                 "bytes32",
//                                                                             ),
//                                                                         }),
//                                                                     }),
//                                                                 ]),
//                                                                 "expression": Object({
//                                                                     "argumentTypes": Array([
//                                                                         Object({
//                                                                             "typeIdentifier":
// String(
// "t_bytes32",                                                                             ),
//                                                                             "typeString": String(
//                                                                                 "bytes32",
//                                                                             ),
//                                                                         }),
//                                                                     ]),
//                                                                     "id": Number(
//                                                                         33,
//                                                                     ),
//                                                                     "isConstant": Bool(
//                                                                         false,
//                                                                     ),
//                                                                     "isLValue": Bool(
//                                                                         false,
//                                                                     ),
//                                                                     "isPure": Bool(
//                                                                         true,
//                                                                     ),
//                                                                     "lValueRequested": Bool(
//                                                                         false,
//                                                                     ),
//                                                                     "nodeType": String(
//
// "ElementaryTypeNameExpression",
// ),                                                                     "src": String(
//                                                                         "343:7:0",
//                                                                     ),
//                                                                     "typeDescriptions": Object({
//                                                                         "typeIdentifier": String(
//
// "t_type$_t_uint256_$",                                                                         ),
//                                                                         "typeString": String(
//                                                                             "type(uint256)",
//                                                                         ),
//                                                                     }),
//                                                                     "typeName": Object({
//                                                                         "id": Number(
//                                                                             32,
//                                                                         ),
//                                                                         "name": String(
//                                                                             "uint256",
//                                                                         ),
//                                                                         "nodeType": String(
//                                                                             "ElementaryTypeName",
//                                                                         ),
//                                                                         "src": String(
//                                                                             "343:7:0",
//                                                                         ),
//                                                                         "typeDescriptions":
// Object({}),                                                                     }),
//                                                                 }),
//                                                                 "id": Number(
//                                                                     37,
//                                                                 ),
//                                                                 "isConstant": Bool(
//                                                                     false,
//                                                                 ),
//                                                                 "isLValue": Bool(
//                                                                     false,
//                                                                 ),
//                                                                 "isPure": Bool(
//                                                                     true,
//                                                                 ),
//                                                                 "kind": String(
//                                                                     "typeConversion",
//                                                                 ),
//                                                                 "lValueRequested": Bool(
//                                                                     false,
//                                                                 ),
//                                                                 "names": Array([]),
//                                                                 "nodeType": String(
//                                                                     "FunctionCall",
//                                                                 ),
//                                                                 "src": String(
//                                                                     "343:37:0",
//                                                                 ),
//                                                                 "tryCall": Bool(
//                                                                     false,
//                                                                 ),
//                                                                 "typeDescriptions": Object({
//                                                                     "typeIdentifier": String(
//                                                                         "t_uint256",
//                                                                     ),
//                                                                     "typeString": String(
//                                                                         "uint256",
//                                                                     ),
//                                                                 }),
//                                                             }),
//                                                         ]),
//                                                         "expression": Object({
//                                                             "argumentTypes": Array([
//                                                                 Object({
//                                                                     "typeIdentifier": String(
//                                                                         "t_uint256",
//                                                                     ),
//                                                                     "typeString": String(
//                                                                         "uint256",
//                                                                     ),
//                                                                 }),
//                                                             ]),
//                                                             "id": Number(
//                                                                 31,
//                                                             ),
//                                                             "isConstant": Bool(
//                                                                 false,
//                                                             ),
//                                                             "isLValue": Bool(
//                                                                 false,
//                                                             ),
//                                                             "isPure": Bool(
//                                                                 true,
//                                                             ),
//                                                             "lValueRequested": Bool(
//                                                                 false,
//                                                             ),
//                                                             "nodeType": String(
//                                                                 "ElementaryTypeNameExpression",
//                                                             ),
//                                                             "src": String(
//                                                                 "335:7:0",
//                                                             ),
//                                                             "typeDescriptions": Object({
//                                                                 "typeIdentifier": String(
//                                                                     "t_type$_t_uint160_$",
//                                                                 ),
//                                                                 "typeString": String(
//                                                                     "type(uint160)",
//                                                                 ),
//                                                             }),
//                                                             "typeName": Object({
//                                                                 "id": Number(
//                                                                     30,
//                                                                 ),
//                                                                 "name": String(
//                                                                     "uint160",
//                                                                 ),
//                                                                 "nodeType": String(
//                                                                     "ElementaryTypeName",
//                                                                 ),
//                                                                 "src": String(
//                                                                     "335:7:0",
//                                                                 ),
//                                                                 "typeDescriptions": Object({}),
//                                                             }),
//                                                         }),
//                                                         "id": Number(
//                                                             38,
//                                                         ),
//                                                         "isConstant": Bool(
//                                                             false,
//                                                         ),
//                                                         "isLValue": Bool(
//                                                             false,
//                                                         ),
//                                                         "isPure": Bool(
//                                                             true,
//                                                         ),
//                                                         "kind": String(
//                                                             "typeConversion",
//                                                         ),
//                                                         "lValueRequested": Bool(
//                                                             false,
//                                                         ),
//                                                         "names": Array([]),
//                                                         "nodeType": String(
//                                                             "FunctionCall",
//                                                         ),
//                                                         "src": String(
//                                                             "335:46:0",
//                                                         ),
//                                                         "tryCall": Bool(
//                                                             false,
//                                                         ),
//                                                         "typeDescriptions": Object({
//                                                             "typeIdentifier": String(
//                                                                 "t_uint160",
//                                                             ),
//                                                             "typeString": String(
//                                                                 "uint160",
//                                                             ),
//                                                         }),
//                                                     }),
//                                                 ]),
//                                                 "expression": Object({
//                                                     "argumentTypes": Array([
//                                                         Object({
//                                                             "typeIdentifier": String(
//                                                                 "t_uint160",
//                                                             ),
//                                                             "typeString": String(
//                                                                 "uint160",
//                                                             ),
//                                                         }),
//                                                     ]),
//                                                     "id": Number(
//                                                         29,
//                                                     ),
//                                                     "isConstant": Bool(
//                                                         false,
//                                                     ),
//                                                     "isLValue": Bool(
//                                                         false,
//                                                     ),
//                                                     "isPure": Bool(
//                                                         true,
//                                                     ),
//                                                     "lValueRequested": Bool(
//                                                         false,
//                                                     ),
//                                                     "nodeType": String(
//                                                         "ElementaryTypeNameExpression",
//                                                     ),
//                                                     "src": String(
//                                                         "327:7:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_type$_t_bytes20_$",
//                                                         ),
//                                                         "typeString": String(
//                                                             "type(bytes20)",
//                                                         ),
//                                                     }),
//                                                     "typeName": Object({
//                                                         "id": Number(
//                                                             28,
//                                                         ),
//                                                         "name": String(
//                                                             "bytes20",
//                                                         ),
//                                                         "nodeType": String(
//                                                             "ElementaryTypeName",
//                                                         ),
//                                                         "src": String(
//                                                             "327:7:0",
//                                                         ),
//                                                         "typeDescriptions": Object({}),
//                                                     }),
//                                                 }),
//                                                 "id": Number(
//                                                     39,
//                                                 ),
//                                                 "isConstant": Bool(
//                                                     false,
//                                                 ),
//                                                 "isLValue": Bool(
//                                                     false,
//                                                 ),
//                                                 "isPure": Bool(
//                                                     true,
//                                                 ),
//                                                 "kind": String(
//                                                     "typeConversion",
//                                                 ),
//                                                 "lValueRequested": Bool(
//                                                     false,
//                                                 ),
//                                                 "names": Array([]),
//                                                 "nodeType": String(
//                                                     "FunctionCall",
//                                                 ),
//                                                 "src": String(
//                                                     "327:55:0",
//                                                 ),
//                                                 "tryCall": Bool(
//                                                     false,
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//                                                         "t_bytes20",
//                                                     ),
//                                                     "typeString": String(
//                                                         "bytes20",
//                                                     ),
//                                                 }),
//                                             }),
//                                         ]),
//                                         "expression": Object({
//                                             "argumentTypes": Array([
//                                                 Object({
//                                                     "typeIdentifier": String(
//                                                         "t_bytes20",
//                                                     ),
//                                                     "typeString": String(
//                                                         "bytes20",
//                                                     ),
//                                                 }),
//                                             ]),
//                                             "id": Number(
//                                                 27,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 true,
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "nodeType": String(
//                                                 "ElementaryTypeNameExpression",
//                                             ),
//                                             "src": String(
//                                                 "319:7:0",
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_type$_t_address_$",
//                                                 ),
//                                                 "typeString": String(
//                                                     "type(address)",
//                                                 ),
//                                             }),
//                                             "typeName": Object({
//                                                 "id": Number(
//                                                     26,
//                                                 ),
//                                                 "name": String(
//                                                     "address",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "ElementaryTypeName",
//                                                 ),
//                                                 "src": String(
//                                                     "319:7:0",
//                                                 ),
//                                                 "typeDescriptions": Object({}),
//                                             }),
//                                         }),
//                                         "id": Number(
//                                             40,
//                                         ),
//                                         "isConstant": Bool(
//                                             false,
//                                         ),
//                                         "isLValue": Bool(
//                                             false,
//                                         ),
//                                         "isPure": Bool(
//                                             true,
//                                         ),
//                                         "kind": String(
//                                             "typeConversion",
//                                         ),
//                                         "lValueRequested": Bool(
//                                             false,
//                                         ),
//                                         "names": Array([]),
//                                         "nodeType": String(
//                                             "FunctionCall",
//                                         ),
//                                         "src": String(
//                                             "319:64:0",
//                                         ),
//                                         "tryCall": Bool(
//                                             false,
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_address_payable",
//                                             ),
//                                             "typeString": String(
//                                                 "address payable",
//                                             ),
//                                         }),
//                                     }),
//                                 ]),
//                                 "expression": Object({
//                                     "argumentTypes": Array([
//                                         Object({
//                                             "typeIdentifier": String(
//                                                 "t_address_payable",
//                                             ),
//                                             "typeString": String(
//                                                 "address payable",
//                                             ),
//                                         }),
//                                     ]),
//                                     "id": Number(
//                                         25,
//                                     ),
//                                     "name": String(
//                                         "VM",
//                                     ),
//                                     "nodeType": String(
//                                         "Identifier",
//                                     ),
//                                     "overloadedDeclarations": Array([]),
//                                     "referencedDeclaration": Number(
//                                         18,
//                                     ),
//                                     "src": String(
//                                         "316:2:0",
//                                     ),
//                                     "typeDescriptions": Object({
//                                         "typeIdentifier": String(
//                                             "t_type$_t_contract$_VM_$18_$",
//                                         ),
//                                         "typeString": String(
//                                             "type(contract VM)",
//                                         ),
//                                     }),
//                                 }),
//                                 "id": Number(
//                                     41,
//                                 ),
//                                 "isConstant": Bool(
//                                     false,
//                                 ),
//                                 "isLValue": Bool(
//                                     false,
//                                 ),
//                                 "isPure": Bool(
//                                     true,
//                                 ),
//                                 "kind": String(
//                                     "typeConversion",
//                                 ),
//                                 "lValueRequested": Bool(
//                                     false,
//                                 ),
//                                 "names": Array([]),
//                                 "nodeType": String(
//                                     "FunctionCall",
//                                 ),
//                                 "src": String(
//                                     "316:68:0",
//                                 ),
//                                 "tryCall": Bool(
//                                     false,
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_contract$_VM_$18",
//                                     ),
//                                     "typeString": String(
//                                         "contract VM",
//                                     ),
//                                 }),
//                             }),
//                             "visibility": String(
//                                 "internal",
//                             ),
//                         }),
//                         Object({
//                             "constant": Bool(
//                                 false,
//                             ),
//                             "id": Number(
//                                 45,
//                             ),
//                             "mutability": String(
//                                 "mutable",
//                             ),
//                             "name": String(
//                                 "who",
//                             ),
//                             "nodeType": String(
//                                 "VariableDeclaration",
//                             ),
//                             "scope": Number(
//                                 89,
//                             ),
//                             "src": String(
//                                 "390:56:0",
//                             ),
//                             "stateVariable": Bool(
//                                 true,
//                             ),
//                             "storageLocation": String(
//                                 "default",
//                             ),
//                             "typeDescriptions": Object({
//                                 "typeIdentifier": String(
//                                     "t_address",
//                                 ),
//                                 "typeString": String(
//                                     "address",
//                                 ),
//                             }),
//                             "typeName": Object({
//                                 "id": Number(
//                                     43,
//                                 ),
//                                 "name": String(
//                                     "address",
//                                 ),
//                                 "nodeType": String(
//                                     "ElementaryTypeName",
//                                 ),
//                                 "src": String(
//                                     "390:7:0",
//                                 ),
//                                 "stateMutability": String(
//                                     "nonpayable",
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_address",
//                                     ),
//                                     "typeString": String(
//                                         "address",
//                                     ),
//                                 }),
//                             }),
//                             "value": Object({
//                                 "hexValue": String(
//
// "307864386441364246323639363461463944376545643965303345353334313544333761413936303435",
//                                 ),
//                                 "id": Number(
//                                     44,
//                                 ),
//                                 "isConstant": Bool(
//                                     false,
//                                 ),
//                                 "isLValue": Bool(
//                                     false,
//                                 ),
//                                 "isPure": Bool(
//                                     true,
//                                 ),
//                                 "kind": String(
//                                     "number",
//                                 ),
//                                 "lValueRequested": Bool(
//                                     false,
//                                 ),
//                                 "nodeType": String(
//                                     "Literal",
//                                 ),
//                                 "src": String(
//                                     "404:42:0",
//                                 ),
//                                 "typeDescriptions": Object({
//                                     "typeIdentifier": String(
//                                         "t_address_payable",
//                                     ),
//                                     "typeString": String(
//                                         "address payable",
//                                     ),
//                                 }),
//                                 "value": String(
//                                     "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
//                                 ),
//                             }),
//                             "visibility": String(
//                                 "internal",
//                             ),
//                         }),
//                         Object({
//                             "anonymous": Bool(
//                                 false,
//                             ),
//                             "id": Number(
//                                 49,
//                             ),
//                             "name": String(
//                                 "log_uint",
//                             ),
//                             "nodeType": String(
//                                 "EventDefinition",
//                             ),
//                             "parameters": Object({
//                                 "id": Number(
//                                     48,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([
//                                     Object({
//                                         "constant": Bool(
//                                             false,
//                                         ),
//                                         "id": Number(
//                                             47,
//                                         ),
//                                         "indexed": Bool(
//                                             false,
//                                         ),
//                                         "mutability": String(
//                                             "mutable",
//                                         ),
//                                         "name": String(
//                                             "",
//                                         ),
//                                         "nodeType": String(
//                                             "VariableDeclaration",
//                                         ),
//                                         "scope": Number(
//                                             49,
//                                         ),
//                                         "src": String(
//                                             "468:7:0",
//                                         ),
//                                         "stateVariable": Bool(
//                                             false,
//                                         ),
//                                         "storageLocation": String(
//                                             "default",
//                                         ),
//                                         "typeDescriptions": Object({
//                                             "typeIdentifier": String(
//                                                 "t_uint256",
//                                             ),
//                                             "typeString": String(
//                                                 "uint256",
//                                             ),
//                                         }),
//                                         "typeName": Object({
//                                             "id": Number(
//                                                 46,
//                                             ),
//                                             "name": String(
//                                                 "uint256",
//                                             ),
//                                             "nodeType": String(
//                                                 "ElementaryTypeName",
//                                             ),
//                                             "src": String(
//                                                 "468:7:0",
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_uint256",
//                                                 ),
//                                                 "typeString": String(
//                                                     "uint256",
//                                                 ),
//                                             }),
//                                         }),
//                                         "visibility": String(
//                                             "internal",
//                                         ),
//                                     }),
//                                 ]),
//                                 "src": String(
//                                     "467:9:0",
//                                 ),
//                             }),
//                             "src": String(
//                                 "453:24:0",
//                             ),
//                         }),
//                         Object({
//                             "body": Object({
//                                 "id": Number(
//                                     87,
//                                 ),
//                                 "nodeType": String(
//                                     "Block",
//                                 ),
//                                 "src": String(
//                                     "507:295:0",
//                                 ),
//                                 "statements": Array([
//                                     Object({
//                                         "expression": Object({
//                                             "arguments": Array([
//                                                 Object({
//                                                     "id": Number(
//                                                         55,
//                                                     ),
//                                                     "name": String(
//                                                         "who",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         45,
//                                                     ),
//                                                     "src": String(
//                                                         "566:3:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 }),
//                                             ]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([
//                                                     Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "expression": Object({
//                                                     "id": Number(
//                                                         52,
//                                                     ),
//                                                     "name": String(
//                                                         "vm",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         42,
//                                                     ),
//                                                     "src": String(
//                                                         "552:2:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_contract$_VM_$18",
//                                                         ),
//                                                         "typeString": String(
//                                                             "contract VM",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "id": Number(
//                                                     54,
//                                                 ),
//                                                 "isConstant": Bool(
//                                                     false,
//                                                 ),
//                                                 "isLValue": Bool(
//                                                     false,
//                                                 ),
//                                                 "isPure": Bool(
//                                                     false,
//                                                 ),
//                                                 "lValueRequested": Bool(
//                                                     false,
//                                                 ),
//                                                 "memberName": String(
//                                                     "startPrank",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "MemberAccess",
//                                                 ),
//                                                 "referencedDeclaration": Number(
//                                                     17,
//                                                 ),
//                                                 "src": String(
//                                                     "552:13:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_external_nonpayable$_t_address_$returns$__$",
// ),                                                     "typeString": String(
//                                                         "function (address) external",
//                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 56,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "552:18:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_tuple$__$",
//                                                 ),
//                                                 "typeString": String(
//                                                     "tuple()",
//                                                 ),
//                                             }),
//                                         }),
//                                         "id": Number(
//                                             57,
//                                         ),
//                                         "nodeType": String(
//                                             "ExpressionStatement",
//                                         ),
//                                         "src": String(
//                                             "552:18:0",
//                                         ),
//                                     }),
//                                     Object({
//                                         "assignments": Array([
//                                             Number(
//                                                 59,
//                                             ),
//                                         ]),
//                                         "declarations": Array([
//                                             Object({
//                                                 "constant": Bool(
//                                                     false,
//                                                 ),
//                                                 "id": Number(
//                                                     59,
//                                                 ),
//                                                 "mutability": String(
//                                                     "mutable",
//                                                 ),
//                                                 "name": String(
//                                                     "balanceBefore",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "VariableDeclaration",
//                                                 ),
//                                                 "scope": Number(
//                                                     87,
//                                                 ),
//                                                 "src": String(
//                                                     "581:21:0",
//                                                 ),
//                                                 "stateVariable": Bool(
//                                                     false,
//                                                 ),
//                                                 "storageLocation": String(
//                                                     "default",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//                                                         "t_uint256",
//                                                     ),
//                                                     "typeString": String(
//                                                         "uint256",
//                                                     ),
//                                                 }),
//                                                 "typeName": Object({
//                                                     "id": Number(
//                                                         58,
//                                                     ),
//                                                     "name": String(
//                                                         "uint256",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "ElementaryTypeName",
//                                                     ),
//                                                     "src": String(
//                                                         "581:7:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "visibility": String(
//                                                     "internal",
//                                                 ),
//                                             }),
//                                         ]),
//                                         "id": Number(
//                                             64,
//                                         ),
//                                         "initialValue": Object({
//                                             "arguments": Array([
//                                                 Object({
//                                                     "id": Number(
//                                                         62,
//                                                     ),
//                                                     "name": String(
//                                                         "who",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         45,
//                                                     ),
//                                                     "src": String(
//                                                         "620:3:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 }),
//                                             ]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([
//                                                     Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "expression": Object({
//                                                     "id": Number(
//                                                         60,
//                                                     ),
//                                                     "name": String(
//                                                         "weth",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         23,
//                                                     ),
//                                                     "src": String(
//                                                         "605:4:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_contract$_ERC20_$12",
//                                                         ),
//                                                         "typeString": String(
//                                                             "contract ERC20",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "id": Number(
//                                                     61,
//                                                 ),
//                                                 "isConstant": Bool(
//                                                     false,
//                                                 ),
//                                                 "isLValue": Bool(
//                                                     false,
//                                                 ),
//                                                 "isPure": Bool(
//                                                     false,
//                                                 ),
//                                                 "lValueRequested": Bool(
//                                                     false,
//                                                 ),
//                                                 "memberName": String(
//                                                     "balanceOf",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "MemberAccess",
//                                                 ),
//                                                 "referencedDeclaration": Number(
//                                                     8,
//                                                 ),
//                                                 "src": String(
//                                                     "605:14:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_external_view$_t_address_$returns$_t_uint256_$",
// ),                                                     "typeString": String(
//                                                         "function (address) view external returns
// (uint256)",                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 63,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "605:19:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_uint256",
//                                                 ),
//                                                 "typeString": String(
//                                                     "uint256",
//                                                 ),
//                                             }),
//                                         }),
//                                         "nodeType": String(
//                                             "VariableDeclarationStatement",
//                                         ),
//                                         "src": String(
//                                             "581:43:0",
//                                         ),
//                                     }),
//                                     Object({
//                                         "eventCall": Object({
//                                             "arguments": Array([
//                                                 Object({
//                                                     "id": Number(
//                                                         66,
//                                                     ),
//                                                     "name": String(
//                                                         "balanceBefore",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         59,
//                                                     ),
//                                                     "src": String(
//                                                         "648:13:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 }),
//                                             ]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([
//                                                     Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "id": Number(
//                                                     65,
//                                                 ),
//                                                 "name": String(
//                                                     "log_uint",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "Identifier",
//                                                 ),
//                                                 "overloadedDeclarations": Array([]),
//                                                 "referencedDeclaration": Number(
//                                                     49,
//                                                 ),
//                                                 "src": String(
//                                                     "639:8:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_event_nonpayable$_t_uint256_$returns$__$",
// ),                                                     "typeString": String(
//                                                         "function (uint256)",
//                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 67,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "639:23:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_tuple$__$",
//                                                 ),
//                                                 "typeString": String(
//                                                     "tuple()",
//                                                 ),
//                                             }),
//                                         }),
//                                         "id": Number(
//                                             68,
//                                         ),
//                                         "nodeType": String(
//                                             "EmitStatement",
//                                         ),
//                                         "src": String(
//                                             "634:28:0",
//                                         ),
//                                     }),
//                                     Object({
//                                         "expression": Object({
//                                             "arguments": Array([]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([]),
//                                                 "expression": Object({
//                                                     "argumentTypes": Array([]),
//                                                     "expression": Object({
//                                                         "id": Number(
//                                                             69,
//                                                         ),
//                                                         "name": String(
//                                                             "weth",
//                                                         ),
//                                                         "nodeType": String(
//                                                             "Identifier",
//                                                         ),
//                                                         "overloadedDeclarations": Array([]),
//                                                         "referencedDeclaration": Number(
//                                                             23,
//                                                         ),
//                                                         "src": String(
//                                                             "673:4:0",
//                                                         ),
//                                                         "typeDescriptions": Object({
//                                                             "typeIdentifier": String(
//                                                                 "t_contract$_ERC20_$12",
//                                                             ),
//                                                             "typeString": String(
//                                                                 "contract ERC20",
//                                                             ),
//                                                         }),
//                                                     }),
//                                                     "id": Number(
//                                                         71,
//                                                     ),
//                                                     "isConstant": Bool(
//                                                         false,
//                                                     ),
//                                                     "isLValue": Bool(
//                                                         false,
//                                                     ),
//                                                     "isPure": Bool(
//                                                         false,
//                                                     ),
//                                                     "lValueRequested": Bool(
//                                                         false,
//                                                     ),
//                                                     "memberName": String(
//                                                         "deposit",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "MemberAccess",
//                                                     ),
//                                                     "referencedDeclaration": Number(
//                                                         11,
//                                                     ),
//                                                     "src": String(
//                                                         "673:12:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//
// "t_function_external_payable$__$returns$__$",
// ),                                                         "typeString": String(
//                                                             "function () payable external",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "id": Number(
//                                                     73,
//                                                 ),
//                                                 "isConstant": Bool(
//                                                     false,
//                                                 ),
//                                                 "isLValue": Bool(
//                                                     false,
//                                                 ),
//                                                 "isPure": Bool(
//                                                     false,
//                                                 ),
//                                                 "lValueRequested": Bool(
//                                                     false,
//                                                 ),
//                                                 "names": Array([
//                                                     String(
//                                                         "value",
//                                                     ),
//                                                 ]),
//                                                 "nodeType": String(
//                                                     "FunctionCallOptions",
//                                                 ),
//                                                 "options": Array([
//                                                     Object({
//                                                         "hexValue": String(
//                                                             "3135",
//                                                         ),
//                                                         "id": Number(
//                                                             72,
//                                                         ),
//                                                         "isConstant": Bool(
//                                                             false,
//                                                         ),
//                                                         "isLValue": Bool(
//                                                             false,
//                                                         ),
//                                                         "isPure": Bool(
//                                                             true,
//                                                         ),
//                                                         "kind": String(
//                                                             "number",
//                                                         ),
//                                                         "lValueRequested": Bool(
//                                                             false,
//                                                         ),
//                                                         "nodeType": String(
//                                                             "Literal",
//                                                         ),
//                                                         "src": String(
//                                                             "693:8:0",
//                                                         ),
//                                                         "subdenomination": String(
//                                                             "ether",
//                                                         ),
//                                                         "typeDescriptions": Object({
//                                                             "typeIdentifier": String(
//
// "t_rational_15000000000000000000_by_1",
// ),                                                             "typeString": String(
//                                                                 "int_const 15000000000000000000",
//                                                             ),
//                                                         }),
//                                                         "value": String(
//                                                             "15",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "src": String(
//                                                     "673:29:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_external_payable$__$returns$__$value",
// ),                                                     "typeString": String(
//                                                         "function () payable external",
//                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 74,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "673:31:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_tuple$__$",
//                                                 ),
//                                                 "typeString": String(
//                                                     "tuple()",
//                                                 ),
//                                             }),
//                                         }),
//                                         "id": Number(
//                                             75,
//                                         ),
//                                         "nodeType": String(
//                                             "ExpressionStatement",
//                                         ),
//                                         "src": String(
//                                             "673:31:0",
//                                         ),
//                                     }),
//                                     Object({
//                                         "assignments": Array([
//                                             Number(
//                                                 77,
//                                             ),
//                                         ]),
//                                         "declarations": Array([
//                                             Object({
//                                                 "constant": Bool(
//                                                     false,
//                                                 ),
//                                                 "id": Number(
//                                                     77,
//                                                 ),
//                                                 "mutability": String(
//                                                     "mutable",
//                                                 ),
//                                                 "name": String(
//                                                     "balanceAfter",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "VariableDeclaration",
//                                                 ),
//                                                 "scope": Number(
//                                                     87,
//                                                 ),
//                                                 "src": String(
//                                                     "715:20:0",
//                                                 ),
//                                                 "stateVariable": Bool(
//                                                     false,
//                                                 ),
//                                                 "storageLocation": String(
//                                                     "default",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//                                                         "t_uint256",
//                                                     ),
//                                                     "typeString": String(
//                                                         "uint256",
//                                                     ),
//                                                 }),
//                                                 "typeName": Object({
//                                                     "id": Number(
//                                                         76,
//                                                     ),
//                                                     "name": String(
//                                                         "uint256",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "ElementaryTypeName",
//                                                     ),
//                                                     "src": String(
//                                                         "715:7:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "visibility": String(
//                                                     "internal",
//                                                 ),
//                                             }),
//                                         ]),
//                                         "id": Number(
//                                             82,
//                                         ),
//                                         "initialValue": Object({
//                                             "arguments": Array([
//                                                 Object({
//                                                     "id": Number(
//                                                         80,
//                                                     ),
//                                                     "name": String(
//                                                         "who",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         45,
//                                                     ),
//                                                     "src": String(
//                                                         "753:3:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 }),
//                                             ]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([
//                                                     Object({
//                                                         "typeIdentifier": String(
//                                                             "t_address",
//                                                         ),
//                                                         "typeString": String(
//                                                             "address",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "expression": Object({
//                                                     "id": Number(
//                                                         78,
//                                                     ),
//                                                     "name": String(
//                                                         "weth",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         23,
//                                                     ),
//                                                     "src": String(
//                                                         "738:4:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_contract$_ERC20_$12",
//                                                         ),
//                                                         "typeString": String(
//                                                             "contract ERC20",
//                                                         ),
//                                                     }),
//                                                 }),
//                                                 "id": Number(
//                                                     79,
//                                                 ),
//                                                 "isConstant": Bool(
//                                                     false,
//                                                 ),
//                                                 "isLValue": Bool(
//                                                     false,
//                                                 ),
//                                                 "isPure": Bool(
//                                                     false,
//                                                 ),
//                                                 "lValueRequested": Bool(
//                                                     false,
//                                                 ),
//                                                 "memberName": String(
//                                                     "balanceOf",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "MemberAccess",
//                                                 ),
//                                                 "referencedDeclaration": Number(
//                                                     8,
//                                                 ),
//                                                 "src": String(
//                                                     "738:14:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_external_view$_t_address_$returns$_t_uint256_$",
// ),                                                     "typeString": String(
//                                                         "function (address) view external returns
// (uint256)",                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 81,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "738:19:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_uint256",
//                                                 ),
//                                                 "typeString": String(
//                                                     "uint256",
//                                                 ),
//                                             }),
//                                         }),
//                                         "nodeType": String(
//                                             "VariableDeclarationStatement",
//                                         ),
//                                         "src": String(
//                                             "715:42:0",
//                                         ),
//                                     }),
//                                     Object({
//                                         "eventCall": Object({
//                                             "arguments": Array([
//                                                 Object({
//                                                     "id": Number(
//                                                         84,
//                                                     ),
//                                                     "name": String(
//                                                         "balanceAfter",
//                                                     ),
//                                                     "nodeType": String(
//                                                         "Identifier",
//                                                     ),
//                                                     "overloadedDeclarations": Array([]),
//                                                     "referencedDeclaration": Number(
//                                                         77,
//                                                     ),
//                                                     "src": String(
//                                                         "781:12:0",
//                                                     ),
//                                                     "typeDescriptions": Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 }),
//                                             ]),
//                                             "expression": Object({
//                                                 "argumentTypes": Array([
//                                                     Object({
//                                                         "typeIdentifier": String(
//                                                             "t_uint256",
//                                                         ),
//                                                         "typeString": String(
//                                                             "uint256",
//                                                         ),
//                                                     }),
//                                                 ]),
//                                                 "id": Number(
//                                                     83,
//                                                 ),
//                                                 "name": String(
//                                                     "log_uint",
//                                                 ),
//                                                 "nodeType": String(
//                                                     "Identifier",
//                                                 ),
//                                                 "overloadedDeclarations": Array([]),
//                                                 "referencedDeclaration": Number(
//                                                     49,
//                                                 ),
//                                                 "src": String(
//                                                     "772:8:0",
//                                                 ),
//                                                 "typeDescriptions": Object({
//                                                     "typeIdentifier": String(
//
// "t_function_event_nonpayable$_t_uint256_$returns$__$",
// ),                                                     "typeString": String(
//                                                         "function (uint256)",
//                                                     ),
//                                                 }),
//                                             }),
//                                             "id": Number(
//                                                 85,
//                                             ),
//                                             "isConstant": Bool(
//                                                 false,
//                                             ),
//                                             "isLValue": Bool(
//                                                 false,
//                                             ),
//                                             "isPure": Bool(
//                                                 false,
//                                             ),
//                                             "kind": String(
//                                                 "functionCall",
//                                             ),
//                                             "lValueRequested": Bool(
//                                                 false,
//                                             ),
//                                             "names": Array([]),
//                                             "nodeType": String(
//                                                 "FunctionCall",
//                                             ),
//                                             "src": String(
//                                                 "772:22:0",
//                                             ),
//                                             "tryCall": Bool(
//                                                 false,
//                                             ),
//                                             "typeDescriptions": Object({
//                                                 "typeIdentifier": String(
//                                                     "t_tuple$__$",
//                                                 ),
//                                                 "typeString": String(
//                                                     "tuple()",
//                                                 ),
//                                             }),
//                                         }),
//                                         "id": Number(
//                                             86,
//                                         ),
//                                         "nodeType": String(
//                                             "EmitStatement",
//                                         ),
//                                         "src": String(
//                                             "767:27:0",
//                                         ),
//                                     }),
//                                 ]),
//                             }),
//                             "functionSelector": String(
//                                 "c0406226",
//                             ),
//                             "id": Number(
//                                 88,
//                             ),
//                             "implemented": Bool(
//                                 true,
//                             ),
//                             "kind": String(
//                                 "function",
//                             ),
//                             "modifiers": Array([]),
//                             "name": String(
//                                 "run",
//                             ),
//                             "nodeType": String(
//                                 "FunctionDefinition",
//                             ),
//                             "parameters": Object({
//                                 "id": Number(
//                                     50,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([]),
//                                 "src": String(
//                                     "495:2:0",
//                                 ),
//                             }),
//                             "returnParameters": Object({
//                                 "id": Number(
//                                     51,
//                                 ),
//                                 "nodeType": String(
//                                     "ParameterList",
//                                 ),
//                                 "parameters": Array([]),
//                                 "src": String(
//                                     "507:0:0",
//                                 ),
//                             }),
//                             "scope": Number(
//                                 89,
//                             ),
//                             "src": String(
//                                 "483:319:0",
//                             ),
//                             "stateMutability": String(
//                                 "nonpayable",
//                             ),
//                             "virtual": Bool(
//                                 false,
//                             ),
//                             "visibility": String(
//                                 "external",
//                             ),
//                         }),
//                     ]),
//                     "scope": Number(
//                         90,
//                     ),
//                     "src": String(
//                         "213:591:0",
//                     ),
//                 }),
//             ]),
//             "src": String(
//                 "0:804:0",
//             ),
//         }),
//     },
// },
