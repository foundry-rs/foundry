use crate::mutation::mutators::{tests::helper::mutator_tests, unary_op_mutator::UnaryOpMutator};

mutator_tests!(UnaryOpMutator;
    pre_inc:    "++x"       => Some(vec!["--x", "~x", "-x", "x++", "x--"]);
    pre_dec:    "--x"       => Some(vec!["++x", "~x", "-x", "x++", "x--"]);
    neg:        "-x"        => Some(vec!["++x", "--x", "~x", "x++", "x--"]);
    bit_not:    "~x"        => Some(vec!["++x", "--x", "-x", "x++", "x--"]);
    post_inc:   "x++"       => Some(vec!["++x", "--x", "~x", "-x", "x--"]);
    post_dec:   "x--"       => Some(vec!["++x", "--x", "~x", "-x", "x++"]);
    bool_not:   "!x"        => Some(vec!["x"]);
    indexed_post_inc: "arr[i]++" => Some(vec![
        "++arr[i]",
        "--arr[i]",
        "~arr[i]",
        "-arr[i]",
        "arr[i]--",
    ]);
    member_post_inc: "boxValue.value++" => Some(vec![
        "++boxValue.value",
        "--boxValue.value",
        "~boxValue.value",
        "-boxValue.value",
        "boxValue.value--",
    ]);
    chained_member_post_inc: "foo().bar++" => Some(vec![
        "++foo().bar",
        "--foo().bar",
        "~foo().bar",
        "-foo().bar",
        "foo().bar--",
    ]);
    not_parenthesized_binary: "!(a == b)" => Some(vec!["(a == b)"]);
    non_unary:  "a = b + c" => None;
);
