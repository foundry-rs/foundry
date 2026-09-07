use foundry_test_utils::util::ExtTester;

/// Test `forge fmt` immutability.
/// TODO: make sure original fmt is not changed after projects format and rev available.
macro_rules! fmt_test {
    ($name:ident, $org:expr, $repo:expr, $commit:expr) => {
        #[test]
        fn $name() {
            let (_, mut cmd) = ExtTester::new($org, $repo, $commit).setup_forge_prj(false);
            cmd.arg("fmt").assert_success();
            cmd.arg("--check").assert_success();
        }
    };
}

fmt_test!(fmt_ithaca_account, "ithacaxyz", "account", "213c04ee1808784c18609607d85feba7730538fd");

fmt_test!(fmt_univ4_core, "Uniswap", "v4-core", "59d3ecf53afa9264a16bba0e38f4c5d2231f80bc");

fmt_test!(
    fmt_evk_periphery,
    "euler-xyz",
    "evk-periphery",
    "e41f2b9b7ed677ca03ff7bd7221a4e2fdd55504f"
);

fmt_test!(fmt_0x_settler, "0xProject", "0x-settler", "a388c8251ab6c4bedce1641b31027d7b1136daef");
