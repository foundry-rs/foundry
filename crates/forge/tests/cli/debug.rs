use itertools::Itertools;
use std::path::Path;

// Sets up a debuggable test case.
forgetest_async!(
    #[ignore = "ran manually"]
    manual_debug_setup,
    |prj, cmd| {
        cmd.args(["init", "--force"]).arg(prj.root()).assert_non_empty_stdout();
        cmd.forge_fuse();

        let script = prj
            .add_script(
                "Counter.s.sol",
                r#"
import "forge-std/Script.sol";

contract A {
  address a;
  uint b;
  int c;
  bytes32 d;
  bool e;
  bytes f;
  string g;

  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
    a = _a;
    b = _b;
    c = _c;
    d = _d;
    e = _e;
    f = _f;
    g = _g;
  }
}

contract B {
  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
    new A(_a, _b, _c, _d, _e, _f, _g);
  }
}

contract Script0 is Script {
  function run() external {
    vm.broadcast();
    new B(msg.sender, 2 ** 32, -1 * (2 ** 32), keccak256(abi.encode(1)), true, "abcdef", "hello");
  }
}
   "#,
            )
            .unwrap();

        cmd.args([
            "script",
            script.to_str().unwrap(),
            "--root",
            prj.root().to_str().unwrap(),
            "--tc=Script0",
            "--sender=0x00a329c0648769A73afAc7F9381E08FB43dBEA72",
            "--debug",
        ]);
        eprintln!("root: {}", prj.root().display());
        let cmd_path = Path::new(cmd.cmd().get_program()).canonicalize().unwrap();
        let args = cmd.cmd().get_args().map(|s| s.to_str().unwrap()).format(" ");
        eprintln!(" cmd: {} {args}", cmd_path.display());
        std::mem::forget(prj);
    }
);
