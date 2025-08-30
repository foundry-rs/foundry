forgetest_init!(call, |prj, cmd| {
    prj.add_test(
        "call.t.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                vm.ffi(inputs);
            }
        }
    "#,
    );

    cmd.arg("geiger").assert_success().stderr_eq(str![[r#"
...
note[unsafe-cheatcode]: usage of unsafe cheatcodes that can perform dangerous operations
 [FILE]:9:20
  |
9 |                 vm.ffi(inputs);
  |                    ---
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#unsafe-cheatcode
...
"#]]);
});

forgetest_init!(assignment, |prj, cmd| {
    prj.add_test(
        "assignment.t.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                bytes stuff = vm.ffi(inputs);
            }
        }
    "#,
    );

    cmd.arg("geiger").assert_success().stderr_eq(str![[r#"
...
note[unsafe-cheatcode]: usage of unsafe cheatcodes that can perform dangerous operations
 [FILE]:9:34
  |
9 |                 bytes stuff = vm.ffi(inputs);
  |                                  ---
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#unsafe-cheatcode
...
"#]]);
});

forgetest_init!(exit_code, |prj, cmd| {
    prj.add_test(
        "multiple.t.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                vm.ffi(inputs);
                vm.ffi(inputs);
                vm.ffi(inputs);
            }
        }
    "#,
    );

    cmd.arg("geiger").assert_success().stderr_eq(str![[r#"
...
note[unsafe-cheatcode]: usage of unsafe cheatcodes that can perform dangerous operations
 [FILE]:9:20
  |
9 |                 vm.ffi(inputs);
  |                    ---
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#unsafe-cheatcode

note[unsafe-cheatcode]: usage of unsafe cheatcodes that can perform dangerous operations
  [FILE]:10:20
   |
10 |                 vm.ffi(inputs);
   |                    ---
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#unsafe-cheatcode

note[unsafe-cheatcode]: usage of unsafe cheatcodes that can perform dangerous operations
  [FILE]:11:20
   |
11 |                 vm.ffi(inputs);
   |                    ---
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#unsafe-cheatcode
...
"#]]);
});
