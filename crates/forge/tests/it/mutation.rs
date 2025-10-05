// use forge::mutation::MutationHandler;
// use forge_script::ScriptArgs;
// use foundry_common::shell::{ColorChoice, OutputFormat, OutputMode, Shell};
// use std::sync::Arc;

// #[tokio::test(flavor = "multi_thread")]
// async fn test_mutation_test_lifecycle() {
//     let contract = r#"
//         // SPDX-License-Identifier: UNLICENSED
//         pragma solidity ^0.8.13;

//         contract Counter {
//             uint256 public number;

//             function increment() public {
//                 number++;
//                 // This should result in 5 mutants: ++number, --number, -number, ~number,
// number--                 // -number should be invalid
//                 // ++number should be alive
//                 // the rest should be dead
//             }
//         }"#;

//     let test = r#"
//         // SPDX-License-Identifier: UNLICENSED
//         pragma solidity ^0.8.13;

//         // Avoid having to manage a libs folder
//         import {Counter} from "../src/Counter.sol";

//         contract CounterTest {
//             Counter public counter;

//             function setUp() public {
//                 counter = new Counter();
//             }

//             function test_Increment() public {
//                 uint256 _countBefore = counter.number();

//                 counter.increment();

//                 assert(counter.number() == _countBefore + 1);
//             }
//         }"#;

//     let temp_dir = tempfile::tempdir().unwrap();

//     let src_dir = temp_dir.path().join("src");
//     std::fs::create_dir_all(&src_dir).expect("Failed to create src directory");

//     let test_dir = temp_dir.path().join("test");
//     std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");

//     let cache_dir = temp_dir.path().join("cache");
//     std::fs::create_dir_all(&cache_dir).expect("Failed to create test directory");

//     let out_dir = temp_dir.path().join("out");
//     std::fs::create_dir_all(&out_dir).expect("Failed to create test directory");

//     std::fs::write(&src_dir.join("Counter.sol"), contract)
//         .unwrap_or_else(|_| panic!("Failed to write to target file {:?}", &src_dir));

//     std::fs::write(&test_dir.join("CounterTest.t.sol"), test)
//         .unwrap_or_else(|_| panic!("Failed to write to target file {:?}", &src_dir));

//     let mut config = foundry_config::Config::default();
//     config.cache_path = cache_dir;
//     config.out = out_dir;
//     config.src = src_dir.clone();
//     config.test = test_dir.clone();

//     let mut mutation_handler = MutationHandler::new(src_dir.join("Counter.sol"),
// Arc::new(config));

//     mutation_handler.read_source_contract();
//     mutation_handler.generate_ast().await;
//     mutation_handler.create_mutation_folders();
//     let mutants = mutation_handler.generate_and_compile().await;

//     // Test if we compile and collect the valid/invalid mutants
//     assert_eq!(mutants.iter().filter(|(_, output)| output.is_none()).count(), 1);
//     assert_eq!(mutants.iter().filter(|(_, output)| output.is_some()).count(), 4);

//     // @todo run the tests
//     let mut invalids = 0;
//     let mut alive = 0;
//     let mut dead = 0;

//     // Create a new shell to suppress any script output
//     let shell = Shell::new_with(OutputFormat::Json, OutputMode::Quiet, ColorChoice::Never, 0);
//     shell.set();

//     // Run the tests as scripts, for convenience
//     for mutant in mutants {
//         if mutant.1.is_some() {
//             let result = ScriptArgs {
//                 path: mutant.0.path.join("test/CounterTest.t.sol").to_string_lossy().to_string(),
//                 sig: "test_Increment".to_string(),
//                 args: vec![],
//                 ..Default::default()
//             }
//             .run_script()
//             .await;

//             if result.is_err() {
//                 dead += 1;
//             } else {
//                 alive += 1;
//             }
//         } else {
//             invalids += 1;
//         }
//     }

//     assert_eq!(invalids, 1);
//     assert_eq!(alive, 1);
//     assert_eq!(dead, 3);
// }
