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
    negated_call: "-foo()" => Some(vec!["~foo()"]);
    negated_storage_push: "-values.push()" => Some(vec![
        "++values.push()",
        "--values.push()",
        "~values.push()",
        "values.push()++",
        "values.push()--",
    ]);
    negated_method_named_push: "-producer.push(1)" => Some(vec!["~producer.push(1)"]);
    parenthesized_storage_push: "-(values.push())" => Some(vec![
        "++(values.push())",
        "--(values.push())",
        "~(values.push())",
        "(values.push())++",
        "(values.push())--",
    ]);
    indexed_call: "-getStorageArray()[i]" => Some(vec![
        "++getStorageArray()[i]",
        "--getStorageArray()[i]",
        "~getStorageArray()[i]",
        "getStorageArray()[i]++",
        "getStorageArray()[i]--",
    ]);
    bit_not_binary: "~(a + b)" => Some(vec!["-(a + b)"]);
    negated_cast: "-uint256(x)" => Some(vec!["~uint256(x)"]);
    negated_literal: "-1" => Some(vec!["~1"]);
    parenthesized_identifier: "-(x)" => Some(vec![
        "++(x)",
        "--(x)",
        "~(x)",
        "(x)++",
        "(x)--",
    ]);
    non_unary:  "a = b + c" => None;
);
