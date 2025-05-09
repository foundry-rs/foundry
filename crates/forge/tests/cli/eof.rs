use foundry_compilers::artifacts::ConfigurableContractArtifact;

// Ensure we can build and decode EOF bytecode.
forgetest_init!(test_build_with_eof, |prj, cmd| {
    cmd.forge_fuse()
        .args(["build", "src/Counter.sol", "--eof", "--use", "0.8.29"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // get artifact bytecode
    let artifact_path = prj.paths().artifacts.join("Counter.sol/Counter.json");
    let artifact: ConfigurableContractArtifact =
        foundry_compilers::utils::read_json_file(&artifact_path).unwrap();
    assert!(artifact.metadata.is_some());
    let bytecode = format!("{}", artifact.bytecode.unwrap().object.into_bytes().unwrap());

    cmd.cast_fuse()
        .args(["decode-eof", bytecode.as_str()])
        .assert_success().stdout_eq(str![[r#"
Header:
╭------------------------+-------╮
| type_size              | 4     |
|------------------------+-------|
| num_code_sections      | 1     |
|------------------------+-------|
| code_sizes             | [17]  |
|------------------------+-------|
| num_container_sections | 1     |
|------------------------+-------|
| container_sizes        | [257] |
|------------------------+-------|
| data_size              | 0     |
╰------------------------+-------╯

Code sections:
╭---+--------+---------+------------------+--------------------------------------╮
|   | Inputs | Outputs | Max stack height | Code                                 |
+================================================================================+
| 0 | 0      | 128     | 2                | 0x608060405234e100055f6080ee005f80fd |
╰---+--------+---------+------------------+--------------------------------------╯

Container sections:
╭---+--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------╮
| 0 | 0xef000101000402000100ab04004300008000056080806040526004361015e100035f80fd5f3560e01c9081633fb5c1cb14e1006d81638381f58a14e100475063d09de08a14e100045fe0ffd534e100325f600319360112e100255f545f198114e10009600190015f555f80f3634e487b7160e01b5f52601160045260245ffd5f80fd5f80fd34e100155f600319360112e100086020905f548152f35f80fd5f80fd34e100166020600319360112e100086004355f555f80f35f80fd5f80fda364697066735822122030195051c5939201983e86f52c88296e7ba03945a054b4e413a7b16cafb76bd96c6578706572696d656e74616cf564736f6c634300081d0041 |
╰---+--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------╯

"#]]);
});

// Ensure compiler fails if doesn't support EOFs but eof flag used.
forgetest_init!(test_unsupported_compiler, |prj, cmd| {
    cmd.forge_fuse()
        .args(["build", "src/Counter.sol", "--eof", "--use", "0.8.27"])
        .assert_failure()
        .stderr_eq(str![[r#"
...
Error: Compiler run failed:
...

"#]]);
});
