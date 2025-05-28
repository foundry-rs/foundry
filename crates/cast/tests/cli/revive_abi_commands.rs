use foundry_test_utils::casttest;

casttest!(test_abi_encode, async |_prj, cmd| {
    cmd.cast_fuse().args(["abi-encode", "foo(uint256)", "1"]).assert_success().stdout_eq(str![[
        r#"
0x0000000000000000000000000000000000000000000000000000000000000001

"#
    ]]);
});

casttest!(test_cast_sig, |_prj, cmd| {
    cmd.cast_fuse().args(["sig", "transfer(address,uint256)"]).assert_success().stdout_eq(str![[
        r#"
0xa9059cbb

"#
    ]]);
});

casttest!(test_cast_4byte_lookup, |_prj, cmd| {
    cmd.cast_fuse().args(["4byte", "0xa9059cbb"]).assert_success().stdout_eq(str![[r#"
transfer(address,uint256)

"#]]);
});

casttest!(test_cast_4byte_calldata, |_prj, cmd| {
    let calldata = "0xa9059cbb000000000000000000000000e78388b4ce79068e89bf8aa7f218ef6b9ab0e9d00000000000000000000000000000000000000000000000000174b37380cea000";
    cmd.cast_fuse().args(["4byte-calldata", calldata]).assert_success().stdout_eq(str![[r#"
1) "transfer(address,uint256)"
0xE78388b4CE79068e89Bf8aA7f218eF6b9AB0e9d0
104906000000000000 [1.049e17]

"#]]);
});

casttest!(test_cast_4byte_event, |_prj, cmd| {
    let topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
    cmd.cast_fuse().args(["4byte-event", topic]).assert_success().stdout_eq(str![[r#"
Transfer(address,address,uint256)

"#]]);
});

casttest!(test_cast_calldata_named, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["calldata", "balanceOf(address)", "0xa16081f360e3847006db660bae1c6d1b2e17ec2a"])
        .assert_success()
        .stdout_eq(str![[r#"
0x70a08231000000000000000000000000a16081f360e3847006db660bae1c6d1b2e17ec2a

"#]]);
});

casttest!(test_cast_decode_abi, |_prj, cmd| {
    let types = "transfer(address,uint256)";
    let data = "0xa9059cbb000000000000000000000000e78388b4ce79068e89bf8aa7f218ef6b9ab0e9d0000000000000000000000000000000000000000000000000008a8e4b1a3d8000";
    cmd.cast_fuse().args(["decode-abi", "--input", types, data]).assert_success().stdout_eq(str![
        [r#"
0x00000000E78388B4CE79068E89Bf8AA7F218ef6B
69968757479791728420194130618274621873781963541191066330122034508882829282891 [6.996e76]

"#]
    ]);
});

casttest!(test_cast_decode_calldata, |_prj, cmd| {
    let sig = "transfer(address,uint256)";
    let data = "0xa9059cbb000000000000000000000000e78388b4ce79068e89bf8aa7f218ef6b9ab0e9d0000000000000000000000000000000000000000000000000008a8e4b1a3d8000";
    cmd.cast_fuse().args(["decode-calldata", sig, data]).assert_success().stdout_eq(str![[r#"
0xE78388b4CE79068e89Bf8aA7f218eF6b9AB0e9d0
39000000000000000 [3.9e16]

"#]]);
});

casttest!(test_cast_upload_signature, |_prj, cmd| {
    cmd.cast_fuse().args(["upload-signature", "spam(uint256,address)"]).assert_success().stdout_eq(
        str![[r#"
Duplicated: Function spam(uint256,address): 0xd6b056cc
Selectors successfully uploaded to OpenChain

"#]],
    );
});
