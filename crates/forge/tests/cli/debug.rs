use itertools::Itertools;
use std::path::Path;

// Sets up a debuggable test case.
// Run with `cargo test-debugger`.
forgetest_async!(
    #[ignore = "ran manually"]
    manual_debug_setup,
    |prj, cmd| {
        cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

        prj.add_source("Counter2.sol", r#"
contract A {
    address public a;
    uint public b;
    int public c;
    bytes32 public d;
    bool public e;
    bytes public f;
    string public g;

    constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
        a = _a;
        b = _b;
        c = _c;
        d = _d;
        e = _e;
        f = _f;
        g = _g;
    }

    function getA() public view returns (address) {
        return a;
    }

    function setA(address _a) public {
        a = _a;
    }
}"#,
        )
        .unwrap();

        let script = prj.add_script("Counter.s.sol", r#"
import "../src/Counter2.sol";
import "forge-std/Script.sol";
import "forge-std/Test.sol";

contract B is A {
    A public other;
    address public self = address(this);

    constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g)
        A(_a, _b, _c, _d, _e, _f, _g)
    {
        other = new A(_a, _b, _c, _d, _e, _f, _g);
    }
}

contract Script0 is Script, Test {
    function run() external {
        assertEq(uint256(1), uint256(1));

        vm.startBroadcast();
        B b = new B(msg.sender, 2 ** 32, -1 * (2 ** 32), keccak256(abi.encode(1)), true, "abcdef", "hello");
        assertEq(b.getA(), msg.sender);
        b.setA(tx.origin);
        assertEq(b.getA(), tx.origin);
        address _b = b.self();
        bytes32 _d = b.d();
        bytes32 _d2 = b.other().d();
    }
}"#,
        )
        .unwrap();

        cmd.forge_fuse().args(["build"]).assert_success();

        cmd.args([
            "script",
            script.to_str().unwrap(),
            "--root",
            prj.root().to_str().unwrap(),
            "--tc=Script0",
            "--debug",
        ]);
        eprintln!("root: {}", prj.root().display());
        let cmd_path = Path::new(cmd.cmd().get_program()).canonicalize().unwrap();
        let args = cmd.cmd().get_args().map(|s| s.to_str().unwrap()).format(" ");
        eprintln!(" cmd: {} {args}", cmd_path.display());
        std::mem::forget(prj);
    }
);
