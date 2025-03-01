#![forbid(unsafe_code)]

use condtype::*;

#[test]
fn condval_true() {
    let x = condval!(if true { "a" } else { 1 });
    assert_eq!(x, "a");
}

#[test]
fn condval_false() {
    let x = condval!(if false { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_const() {
    pub const COND: bool = false;

    let x = condval!(if COND { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_path() {
    mod cond {
        pub const COND: bool = false;
    }

    let x = condval!(if cond::COND { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_not_true() {
    let x = condval!(if !true { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_and() {
    let x = condval!(if true && false { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_paren() {
    #[allow(unused_parens)]
    let x = condval!(if (true && false) { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_struct_method() {
    struct Foo {}

    impl Foo {
        const fn foo(self) -> bool {
            false
        }
    }

    // Although this expression isn't allowed in normal `if`, we opt to parse
    // any condition expressions.
    let x = condval!(if Foo {}.foo() { "a" } else { 1 });
    assert_eq!(x, 1);
}

#[test]
fn condval_hygiene() {
    // "te" is used by the `match` in `condval!`.
    let te = 1;

    let x = condval!(if false { "a" } else { te });
    assert_eq!(x, 1);
}

#[test]
fn condval_empty() {
    let _: () = condval!(if true {
    } else {
        1
    });

    let _: () = condval!(if false {
        "a"
    } else {
    });

    let _: () = condval!(if false {
        "a"
    } else if false {
        1
    } else {
    });
}

#[test]
fn condval_else_if1() {
    let x = condval!(if true {
        "a"
    } else if false {
        1
    } else {
        42.0
    });
    assert_eq!(x, "a");
}

#[test]
fn condval_else_if2() {
    let x = condval!(if false {
        "a"
    } else if false {
        1
    } else {
        42.0
    });
    assert_eq!(x, 42.0);
}

#[test]
fn condval_else_if3() {
    let x = condval!(if false {
        "a"
    } else if false {
        1
    } else if true {
        42.0
    } else {
        [1, 2, 3]
    });
    assert_eq!(x, 42.0);
}

#[test]
fn condval_else_if4() {
    let x = condval!(if false {
        "a"
    } else if false {
        1
    } else if false {
        42.0
    } else {
        [1, 2, 3]
    });
    assert_eq!(x, [1, 2, 3]);
}

#[test]
fn condval_let1() {
    const VAL: Option<&str> = Some("a");

    let x = condval!(if let Some(val) = VAL { val } else { 1 });
    assert_eq!(x, "a");
}

#[test]
fn condval_let2() {
    const VAL1: Option<i32> = None;
    const VAL2: Option<&str> = Some("a");

    let x = condval!(if let Some(val1) = VAL1 {
        val1
    } else if let Some(val2) = VAL2 {
        val2
    } else {
        42.0
    });
    assert_eq!(x, "a");
}

#[test]
fn condval_let3() {
    const VAL1: Option<i32> = None;
    const VAL2: Option<&str> = None;

    let x = condval!(if let Some(val1) = VAL1 {
        val1
    } else if let Some(val2) = VAL2 {
        val2
    } else {
        42.0
    });
    assert_eq!(x, 42.0);
}
