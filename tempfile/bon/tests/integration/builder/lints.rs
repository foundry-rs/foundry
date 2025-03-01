use crate::prelude::*;

// This used to trigger the `unused_parens` lint
#[test]
fn func_with_skipped_generic_arg() {
    #[builder]
    fn sut(arg: &(impl Clone + Default)) -> impl Clone {
        arg.clone()
    }

    sut().arg(&32).call();
}
