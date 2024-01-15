use std::fmt::Display;

use itertools::Itertools;

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};

impl Cheatcode for assertTrue_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        assert_true(self.condition, None)
    }
}

impl Cheatcode for assertTrue_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        assert_true(self.condition, Some(&self.error))
    }
}

impl Cheatcode for assertEq_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(a, b, None)
    }
}

impl Cheatcode for assertEq_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq(&hex::encode_prefixed(a), &hex::encode_prefixed(b), None)
    }
}

impl Cheatcode for assertEq_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq(&hex::encode_prefixed(a), &hex::encode_prefixed(b), Some(error))
    }
}

impl Cheatcode for assertEq_14Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_15Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_16Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_17Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_18Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_19Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_20Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_21Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_22Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_23Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_24Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_eq_array(a, b, None)
    }
}

impl Cheatcode for assertEq_25Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertEq_26Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        let a = a.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        let b = b.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        assert_eq_array(&a, &b, None)
    }
}

impl Cheatcode for assertEq_27Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        let a = a.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        let b = b.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        assert_eq_array(&a, &b, Some(error))
    }
}

impl Cheatcode for assertNotEq_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(a, b, None)
    }
}

impl Cheatcode for assertNotEq_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq(&hex::encode_prefixed(a), &hex::encode_prefixed(b), None)
    }
}

impl Cheatcode for assertNotEq_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq(&hex::encode_prefixed(a), &hex::encode_prefixed(b), Some(error))
    }
}

impl Cheatcode for assertNotEq_14Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_15Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_16Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_17Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_18Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_19Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_20Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_21Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_22Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_23Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_24Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_not_eq_array(a, b, None)
    }
}

impl Cheatcode for assertNotEq_25Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_not_eq_array(a, b, Some(error))
    }
}

impl Cheatcode for assertNotEq_26Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        let a = a.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        let b = b.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        assert_not_eq_array(&a, &b, None)
    }
}

impl Cheatcode for assertNotEq_27Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        let a = a.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        let b = b.iter().map(|x| hex::encode_prefixed(x)).collect::<Vec<_>>();
        assert_not_eq_array(&a, &b, Some(error))
    }
}

impl Cheatcode for assertGt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_gt(a, b, None)
    }
}

impl Cheatcode for assertGt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_gt(a, b, Some(error))
    }
}

impl Cheatcode for assertGt_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_gt(a, b, None)
    }
}

impl Cheatcode for assertGt_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_gt(a, b, Some(error))
    }
}

impl Cheatcode for assertGe_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_ge(a, b, None)
    }
}

impl Cheatcode for assertGe_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_ge(a, b, Some(error))
    }
}

impl Cheatcode for assertGe_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_ge(a, b, None)
    }
}

impl Cheatcode for assertGe_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_ge(a, b, Some(error))
    }
}

impl Cheatcode for assertLt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_lt(a, b, None)
    }
}

impl Cheatcode for assertLt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_lt(a, b, Some(error))
    }
}

impl Cheatcode for assertLt_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_lt(a, b, None)
    }
}

impl Cheatcode for assertLt_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_lt(a, b, Some(error))
    }
}

impl Cheatcode for assertLe_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_le(a, b, None)
    }
}

impl Cheatcode for assertLe_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_le(a, b, Some(error))
    }
}

impl Cheatcode for assertLe_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b } = self;
        assert_le(a, b, None)
    }
}

impl Cheatcode for assertLe_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { a, b, error } = self;
        assert_le(a, b, Some(error))
    }
}

fn get_revert_string(error_message: Option<&str>, failed_assertion_info: String) -> String {
    let mut error = String::new();
    if let Some(message) = error_message {
        error.push_str(&format!("{message}. "));
    }
    error.push_str(&format!("Assertion failed: {}", failed_assertion_info));
    error
}

fn assert_true(condition: bool, error_message: Option<&str>) -> Result {
    if condition {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, "condition is false".to_string()))
    }
}

fn assert_eq<T: Display + PartialEq>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a == b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} != {}", a, b)))
    }
}

fn assert_not_eq<T: Display + PartialEq>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a != b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} == {}", a, b)))
    }
}

fn assert_eq_array<T: Display + PartialEq>(a: &Vec<T>, b: &Vec<T>, error_message: Option<&str>) -> Result {
    if a == b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(
            error_message,
            format!("[{}] != [{}]", a.into_iter().join(", "), b.into_iter().join(", "))
        ))
    }
}

fn assert_not_eq_array<T: Display + PartialEq>(
    a: &Vec<T>,
    b: &Vec<T>,
    error_message: Option<&str>,
) -> Result {
    if a != b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(
            error_message,
            format!("[{}] == [{}]", a.into_iter().join(", "), b.into_iter().join(", "))
        ))
    }
}

fn assert_gt<T: Display + PartialOrd>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a > b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} <= {}", a, b)))
    }
}

fn assert_ge<T: Display + PartialOrd>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a >= b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} < {}", a, b)))
    }
}

fn assert_lt<T: Display + PartialOrd>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a < b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} >= {}", a, b)))
    }
}

fn assert_le<T: Display + PartialOrd>(a: &T, b: &T, error_message: Option<&str>) -> Result {
    if a <= b {
        Ok(Default::default())
    } else {
        bail!(get_revert_string(error_message, format!("{} > {}", a, b)))
    }
}