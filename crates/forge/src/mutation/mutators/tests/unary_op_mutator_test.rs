use crate::mutation::mutators::{tests::helper::mutator_tests, unary_op_mutator::UnaryOpMutator};

mutator_tests!(UnaryOpMutator;
    pre_inc:    "++x"       => Some(vec!["--x", "~x", "-x", "x++", "x--"]);
    pre_dec:    "--x"       => Some(vec!["++x", "~x", "-x", "x++", "x--"]);
    neg:        "-x"        => Some(vec!["++x", "--x", "~x", "x++", "x--"]);
    bit_not:    "~x"        => Some(vec!["++x", "--x", "-x", "x++", "x--"]);
    post_inc:   "x++"       => Some(vec!["++x", "--x", "~x", "-x", "x--"]);
    post_dec:   "x--"       => Some(vec!["++x", "--x", "~x", "-x", "x++"]);
    bool_not:   "!x"        => Some(vec!["x"]);
    non_unary:  "a = b + c" => None;
);
