use crate::mutation::mutators::{binary_op_mutator::BinaryOpMutator, tests::helper::mutator_tests};

mutator_tests!(BinaryOpMutator;
    add:                       "x + y"   => Some(vec!["x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    sub:                       "x - y"   => Some(vec!["x + y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    mul:                       "x * y"   => Some(vec!["x + y", "x - y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    div:                       "x / y"   => Some(vec!["x + y", "x - y", "x * y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    modulus:                   "x % y"   => Some(vec!["x + y", "x - y", "x * y", "x / y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    pow:                       "x ** y"  => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    bit_shift_left:            "x << y"  => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    bit_shift_right:           "x >> y"  => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >>> y", "x & y", "x | y", "x ^ y"]);
    bit_shift_right_unsigned:  "x >>> y" => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x & y", "x | y", "x ^ y"]);
    bit_and:                   "x & y"   => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x | y", "x ^ y"]);
    bit_or:                    "x | y"   => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x ^ y"]);
    bit_xor:                   "x ^ y"   => Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y"]);
    non_binary:                "a = true" => None;
    // Compound assignments are intentionally skipped: the current textual
    // replacement would rewrite `a += b` to `a - b` (dropping the assignment)
    // instead of `a -= b`, so the mutator must report them as inapplicable.
    compound_assign_add:       "a += b" => None;
    compound_assign_sub:       "a -= b" => None;
    compound_assign_mul:       "a *= b" => None;
);
