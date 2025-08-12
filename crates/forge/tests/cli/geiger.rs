forgetest!(call, |prj, cmd| {
    prj.add_source(
        "call.sol",
        r#"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                vm.ffi(inputs);
            }
        }
    "#,
    )
    .unwrap();

    cmd.arg("geiger").assert_code(1).stderr_eq(str![[r#"
error: usage of unsafe cheatcode `vm.ffi`
 [FILE]:7:20
  |
7 |                 vm.ffi(inputs);
  |                    ^^^
  |


"#]]);
});

forgetest!(assignment, |prj, cmd| {
    prj.add_source(
        "assignment.sol",
        r#"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                bytes stuff = vm.ffi(inputs);
            }
        }
    "#,
    )
    .unwrap();

    cmd.arg("geiger").assert_code(1).stderr_eq(str![[r#"
error: usage of unsafe cheatcode `vm.ffi`
 [FILE]:7:34
  |
7 |                 bytes stuff = vm.ffi(inputs);
  |                                  ^^^
  |


"#]]);
});

forgetest!(exit_code, |prj, cmd| {
    prj.add_source(
        "multiple.sol",
        r#"
        contract A is Test {
            function do_ffi() public {
                vm.ffi(inputs);
                vm.ffi(inputs);
                vm.ffi(inputs);
            }
        }
    "#,
    )
    .unwrap();

    cmd.arg("geiger").assert_code(3).stderr_eq(str![[r#"
error: usage of unsafe cheatcode `vm.ffi`
 [FILE]:6:20
  |
6 |                 vm.ffi(inputs);
  |                    ^^^
  |

error: usage of unsafe cheatcode `vm.ffi`
 [FILE]:7:20
  |
7 |                 vm.ffi(inputs);
  |                    ^^^
  |

error: usage of unsafe cheatcode `vm.ffi`
 [FILE]:8:20
  |
8 |                 vm.ffi(inputs);
  |                    ^^^
  |


"#]]);
});
