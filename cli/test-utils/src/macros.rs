/// A macro to generate a new test case
/// accepts a closure with an empty project and the forge executable
#[macro_export]
macro_rules! forgetest {
    ($test:ident, $fun:expr) => {
        #[test]
        fn $test() {
            let (dir, cmd) =
                $crate::util::setup(stringify!($test), $crate::ethers_solc::PathStyle::Dapptools);
            $fun(dir, cmd);
        }
    };
}
