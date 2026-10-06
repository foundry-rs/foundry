use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness};
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, snapbox::IntoData, str, util::OutputExt};
use std::{env, process::Command};

fn symbolic_conformance_enabled() -> bool {
    env::var_os("SYMBOLIC_CONFORMANCE").is_some()
}

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

fn should_skip() -> bool {
    if !symbolic_conformance_enabled() {
        let _ = sh_eprintln!(
            "skipping symbolic conformance test because SYMBOLIC_CONFORMANCE is not set"
        );
        return true;
    }
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic conformance test because z3 is not available");
        return true;
    }
    false
}

#[forgetest_init]
fn symbolic_conformance_block_cheatcodes(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceBlock.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceBlock is Test {
    function checkBlock(uint256 timestamp, uint256 number, uint256 chain) public {
        vm.warp(timestamp);
        vm.roll(number);
        vm.chainId(chain);

        assert(block.timestamp == timestamp);
        assert(block.number == number);
        assert(block.chainid == chain);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkBlock"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceBlock.t.sol:SymbolicConformanceBlock
[PASS] checkBlock(uint256,uint256,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_halmos_feature_matrix(prj: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosFeatureMatrix.t.sol",
        r#"
import "forge-std/Test.sol";

interface MatrixSvm {
    function createCalldata(string calldata name) external returns (bytes memory);
}

struct MatrixPacket {
    bytes payload;
    string label;
    uint256[] numbers;
}

contract MatrixToken {
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;

    constructor(address owner) {
        totalSupply = 100;
        balanceOf[owner] = 100;
    }

    function transfer(address from, address to, uint256 amount) external {
        if (balanceOf[from] >= amount) {
            unchecked {
                balanceOf[from] -= amount;
            }
            balanceOf[to] += amount;
        }
    }

    function mintBackdoor(address to, uint256 amount) external {
        if (amount == 9) {
            balanceOf[to] += amount;
        }
    }
}

contract MatrixNft {
    mapping(uint256 => address) public ownerOf;
    mapping(uint256 => address) public getApproved;

    constructor(address owner) {
        ownerOf[1] = owner;
    }

    function approve(address spender, uint256 id) external {
        if (msg.sender == ownerOf[id]) {
            getApproved[id] = spender;
        }
    }

    function transferFrom(address from, address to, uint256 id) external {
        if (ownerOf[id] == from && to != address(0) && (msg.sender == from || getApproved[id] == msg.sender)) {
            ownerOf[id] = to;
        }
    }
}

contract MatrixDispatcher {
    bool public hit;

    function trip(uint256 value) external {
        if (value == 42) {
            hit = true;
        }
    }

    fallback() external payable {}
}

contract SymbolicConformanceHalmosFeatureMatrix is Test {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);
    address constant ALICE = address(0xA11CE);
    address constant BOB = address(0xB0B);

    MatrixToken token;
    MatrixNft nft;
    MatrixDispatcher dispatcher;

    function setUp() public {
        token = new MatrixToken(ALICE);
        nft = new MatrixNft(ALICE);
        dispatcher = new MatrixDispatcher();
        vm.deal(address(this), 1 ether);
    }

    /// forge-config: default.symbolic.array_lengths = [2, 2, 2, 2, 2]
    function checkMatrixDynamicAbi(bytes memory data, string memory text, MatrixPacket memory packet) public pure {
        assert(data.length <= 2);
        assert(bytes(text).length <= 2);
        assert(packet.payload.length <= 2);
        assert(bytes(packet.label).length <= 2);
        assert(packet.numbers.length <= 2);
    }

    function checkMatrixErc20SupplyAccounting(uint256 amount, address to) public {
        vm.assume(to != address(0) && to != ALICE && to != BOB);

        token.transfer(ALICE, BOB, amount);
        token.mintBackdoor(to, amount);

        uint256 sum = token.balanceOf(ALICE) + token.balanceOf(BOB) + token.balanceOf(to);
        assert(sum <= token.totalSupply());
    }

    function checkMatrixErc721ClearsApproval(address spender, address buyer) public {
        vm.assume(spender != address(0));
        vm.assume(buyer != address(0) && buyer != ALICE);

        vm.prank(ALICE);
        nft.approve(spender, 1);

        vm.prank(ALICE);
        nft.transferFrom(ALICE, buyer, 1);

        assert(nft.getApproved(1) == address(0));
    }

    /// forge-config: default.symbolic.max_calldata_bytes = 36
    function checkMatrixCreateCalldataDispatch() public {
        bytes memory data = MatrixSvm(SVM_ADDRESS).createCalldata("matrix-dispatch");
        (bool ok,) = address(dispatcher).call(data);
        ok;

        assert(!dispatcher.hit());
    }

    function checkMatrixEmptyUnknownTarget(uint256 value) public {
        address target = address(0xBEEF);
        (bool ok, bytes memory out) = target.call(abi.encodeWithSignature("missing(uint256)", value));

        assert(ok);
        assert(out.length == 0);
    }

    function checkMatrixRevertAll(uint256 x) public pure {
        require(x != x, "impossible");
    }
}
"#,
    );

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkMatrixDynamicAbi|checkMatrixErc20SupplyAccounting|checkMatrixErc721ClearsApproval|checkMatrixCreateCalldataDispatch|checkMatrixEmptyUnknownTarget|checkMatrixRevertAll)\\(",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 6 tests for test/SymbolicConformanceHalmosFeatureMatrix.t.sol:SymbolicConformanceHalmosFeatureMatrix
[PASS] checkMatrixCreateCalldataDispatch() ([METRICS])
[PASS] checkMatrixDynamicAbi(bytes,string,(bytes,string,uint256[])) ([METRICS])
[PASS] checkMatrixEmptyUnknownTarget(uint256) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkMatrixErc20SupplyAccounting(uint256,address) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMatrixErc721ClearsApproval(address,address) ([METRICS])
[FAIL: incomplete symbolic execution (RevertAll): all symbolic paths reverted] checkMatrixRevertAll(uint256) ([METRICS])
Suite result: FAILED. 3 passed; 3 failed; 0 skipped; [ELAPSED]
...
"#]].unordered());
}

#[forgetest_init]
fn symbolic_conformance_halmos_simple_total_price(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosTotalPrice.t.sol",
        r#"
contract SymbolicConformanceHalmosTotalPrice {
    function buggyTotal(uint96 price, uint32 quantity) internal pure returns (uint128) {
        unchecked {
            uint120 wrapped = uint120(price) * uint120(quantity);
            return uint128(wrapped);
        }
    }

    function fixedTotal(uint96 price, uint32 quantity) internal pure returns (uint128) {
        return uint128(price) * uint128(quantity);
    }

    function checkBuggyTotal(uint96 price, uint32 quantity) public pure {
        uint128 total = buggyTotal(price, quantity);
        assert(quantity == 0 || total >= price);
    }

    function checkFixedTotal(uint96 price, uint32 quantity) public pure {
        uint128 total = fixedTotal(price, quantity);
        assert(quantity == 0 || total >= price);
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkBuggyTotal",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosTotalPrice.t.sol:SymbolicConformanceHalmosTotalPrice
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkBuggyTotal(uint96,uint32) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("Stuck"), "{stdout}");
    assert!(!stdout.contains("RevertAll"), "{stdout}");

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkFixedTotal",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosTotalPrice.t.sol:SymbolicConformanceHalmosTotalPrice
[PASS] checkFixedTotal(uint96,uint32) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_halmos_simple_power_of_two_loop(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosPowerOfTwo.t.sol",
        r#"
contract SymbolicConformanceHalmosPowerOfTwo {
    function bitHack(uint256 x) internal pure returns (bool) {
        unchecked {
            return x != 0 && (x & (x - 1)) == 0;
        }
    }

    function loopSpec(uint256 x) internal pure returns (bool) {
        for (uint256 i = 0; i < 256; i++) {
            if (x == (uint256(1) << i)) {
                return true;
            }
        }
        return false;
    }

    /// forge-config: default.symbolic.loop = 256
    function checkPowerOfTwoLoop(uint256 x) public pure {
        assert(bitHack(x) == loopSpec(x));
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkPowerOfTwoLoop",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosPowerOfTwo.t.sol:SymbolicConformanceHalmosPowerOfTwo
[PASS] checkPowerOfTwoLoop(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic loop bound exceeded"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_halmos_simple_vault_share_price(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosVault.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceHalmosVaultTarget {
    uint256 public totalAssets;
    uint256 public totalShares;

    function setTotals(uint256 assets, uint256 shares) external {
        totalAssets = assets;
        totalShares = shares;
    }

    function deposit(uint256 assets) external {
        uint256 shares = (assets * totalShares) / totalAssets;
        totalAssets += assets;
        totalShares += shares;
    }

    function mint(uint256 shares) external {
        uint256 assets = (shares * totalAssets) / totalShares;
        totalAssets += assets;
        totalShares += shares;
    }
}

contract SymbolicConformanceHalmosVault is Test {
    SymbolicConformanceHalmosVaultTarget vault;

    function setUp() public {
        vault = new SymbolicConformanceHalmosVaultTarget();
    }

    function checkDepositPreservesSharePrice() public {
        vault.setTotals(1, 1);

        vault.deposit(7);

        assertEq(vault.totalAssets(), 8);
        assertEq(vault.totalShares(), 8);
    }

    function checkMintCanDiluteSharePrice() public {
        vault.setTotals(3, 2);

        vault.mint(1);

        assert(uint256(3) * vault.totalShares() <= vault.totalAssets() * uint256(2));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkDepositPreservesSharePrice",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosVault.t.sol:SymbolicConformanceHalmosVault
[PASS] checkDepositPreservesSharePrice() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    let stdout = assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMintCanDiluteSharePrice",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosVault.t.sol:SymbolicConformanceHalmosVault
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMintCanDiluteSharePrice() ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("Stuck"), "{stdout}");
    assert!(!stdout.contains("RevertAll"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_halmos_simple_fork_style_setup(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosForkSetup.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceRuntimeCounter {
    uint256 public total;
    mapping(address => uint256) public seen;

    function increment(address user) external {
        seen[user]++;
        total++;
    }
}

contract SymbolicConformanceEmpty {}

contract SymbolicConformanceHalmosForkSetup is Test {
    SymbolicConformanceRuntimeCounter counter;

    function setUp() public {
        counter = SymbolicConformanceRuntimeCounter(address(new SymbolicConformanceEmpty()));
        vm.etch(address(counter), type(SymbolicConformanceRuntimeCounter).runtimeCode);
        vm.store(address(counter), bytes32(uint256(0)), bytes32(uint256(12)));
        vm.store(address(counter), keccak256(abi.encode(address(0x1001), uint256(1))), bytes32(uint256(7)));
        vm.store(address(counter), keccak256(abi.encode(address(0x1002), uint256(1))), bytes32(uint256(5)));
    }

    function checkEtchedStorageInvariant(address user) public view {
        // An unwritten mapping slot read at a symbolic key is a fresh symbolic value, so only the
        // keys written in `setUp` can be proven against `total`.
        vm.assume(user == address(0x1001) || user == address(0x1002));
        assertEq(counter.total(), 12);
        assertEq(counter.seen(address(0x1001)), 7);
        assertEq(counter.seen(address(0x1002)), 5);
        assertLe(counter.seen(user), counter.total());
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkEtchedStorageInvariant",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosForkSetup.t.sol:SymbolicConformanceHalmosForkSetup
[PASS] checkEtchedStorageInvariant(address) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic vm.etch"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_halmos_simple_symbolic_signature_replay(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosElection.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicConformanceSvm {
    function createBytes(uint256 len, string calldata name) external returns (bytes memory);
}

contract SymbolicConformanceElection {
    mapping(uint256 => uint256) public votesFor;
    mapping(bytes32 => bool) public usedSignature;

    function signatureVoter(bytes memory signature) public pure returns (address voter) {
        if (signature.length != 64 && signature.length != 65) {
            return address(0);
        }
        bytes32 word;
        assembly {
            word := mload(add(signature, 32))
        }
        voter = address(uint160(uint256(word >> 96)));
    }

    function vote(uint256 proposalId, bool support, address voter, bytes memory signature) external {
        require(support);
        require(signatureVoter(signature) == voter);
        bytes32 signatureId = keccak256(signature);
        require(!usedSignature[signatureId]);
        usedSignature[signatureId] = true;
        votesFor[proposalId]++;
    }
}

contract SymbolicConformanceHalmosElection is Test {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);
    SymbolicConformanceElection election;

    function setUp() public {
        election = new SymbolicConformanceElection();
    }

    function checkCannotVoteTwiceWithAlternateSignature(uint256 proposalId, address voter) public {
        vm.assume(voter != address(0));

        bytes memory original =
            abi.encodePacked(bytes20(voter), bytes32(uint256(1)), bytes13(uint104(0)));
        bytes memory alternate = SymbolicConformanceSvm(SVM_ADDRESS).createBytes(64, "alternate");

        vm.assume(election.signatureVoter(alternate) == voter);

        election.vote(proposalId, true, voter, original);
        assertEq(election.votesFor(proposalId), 1);

        election.vote(proposalId, true, voter, alternate);
        assertEq(election.votesFor(proposalId), 1);
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkCannotVoteTwiceWithAlternateSignature",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosElection.t.sol:SymbolicConformanceHalmosElection
[FAIL: EvmError: Revert; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkCannotVoteTwiceWithAlternateSignature(uint256,address) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
    assert!(!stdout.contains("Stuck"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_halmos_multicaller_batched_calls(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosMulticaller.t.sol",
        r#"
contract SymbolicConformanceMulticallTarget {
    uint256 public total;

    function add(uint256 value) external returns (uint256) {
        total += value;
        return total;
    }

    function echo(bytes calldata data) external pure returns (bytes memory) {
        return data;
    }

    function maybeRevert(uint256 value) external pure returns (uint256) {
        require(value != 13, "unlucky");
        return value + 1;
    }
}

contract SymbolicConformanceMulticaller {
    struct Call {
        address target;
        uint256 value;
        bytes data;
    }

    function multicall(Call[] memory calls) external returns (bool[] memory successes, bytes[] memory results) {
        successes = new bool[](calls.length);
        results = new bytes[](calls.length);

        for (uint256 i; i < calls.length; i++) {
            (successes[i], results[i]) = calls[i].target.call{value: calls[i].value}(calls[i].data);
        }
    }
}

contract SymbolicConformanceHalmosMulticaller {
    SymbolicConformanceMulticallTarget target;
    SymbolicConformanceMulticaller multicaller;

    function setUp() public {
        target = new SymbolicConformanceMulticallTarget();
        multicaller = new SymbolicConformanceMulticaller();
    }

    /// forge-config: default.symbolic.array_lengths = [2]
    function checkMulticallReturndata(uint64 value, bytes memory payload) public {
        SymbolicConformanceMulticaller.Call[] memory calls = new SymbolicConformanceMulticaller.Call[](2);
        calls[0] = SymbolicConformanceMulticaller.Call({
            target: address(target),
            value: 0,
            data: abi.encodeCall(target.add, (uint256(value)))
        });
        calls[1] = SymbolicConformanceMulticaller.Call({
            target: address(target),
            value: 0,
            data: abi.encodeCall(target.echo, (payload))
        });

        (bool[] memory successes, bytes[] memory results) = multicaller.multicall(calls);

        assert(successes[0]);
        assert(successes[1]);
        assert(abi.decode(results[0], (uint256)) == uint256(value));

        bytes memory echoed = abi.decode(results[1], (bytes));
        assert(echoed.length == 2);
        assert(payload.length == 2);
        assert(echoed[0] == payload[0]);
        assert(echoed[1] == payload[1]);
    }

    function checkMulticallFindsFailedCall(uint64 value) public {
        SymbolicConformanceMulticaller.Call[] memory calls = new SymbolicConformanceMulticaller.Call[](1);
        calls[0] = SymbolicConformanceMulticaller.Call({
            target: address(target),
            value: 0,
            data: abi.encodeCall(target.maybeRevert, (uint256(value)))
        });

        (bool[] memory successes,) = multicaller.multicall(calls);

        assert(successes[0]);
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMulticallReturndata",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosMulticaller.t.sol:SymbolicConformanceHalmosMulticaller
[PASS] checkMulticallReturndata(uint64,bytes) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("unsupported external CALL"), "{stdout}");
    assert!(!stdout.contains("Stuck"), "{stdout}");

    let stdout = assert_symbolic(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMulticallFindsFailedCall",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosMulticaller.t.sol:SymbolicConformanceHalmosMulticaller
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=0xc298ec3b000000000000000000000000000000000000000000000000000000000000000d args=[13]] checkMulticallFindsFailedCall(uint64) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("unsupported external CALL"), "{stdout}");
    assert!(!stdout.contains("RevertAll"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_halmos_invariant_reentrancy_exploit(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceHalmosReentrancy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceVulnerableVault {
    mapping(address => uint256) public balanceOf;

    function deposit() external payable {
        balanceOf[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) external {
        require(balanceOf[msg.sender] >= amount, "balance");

        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "send");

        unchecked {
            balanceOf[msg.sender] -= amount;
        }
    }
}

contract SymbolicConformanceReentrancyAttacker {
    SymbolicConformanceVulnerableVault public vault;
    bool internal entered;

    constructor(SymbolicConformanceVulnerableVault target) {
        vault = target;
    }

    receive() external payable {
        if (!entered) {
            entered = true;
            vault.withdraw(1 ether);
            entered = false;
        }
    }

    function depositOne() external {
        vault.deposit{value: 1 ether}();
    }

    function withdrawOne() external {
        vault.withdraw(1 ether);
    }
}

contract SymbolicConformanceHalmosReentrancy is Test {
    SymbolicConformanceVulnerableVault vault;
    SymbolicConformanceReentrancyAttacker attacker;

    function setUp() public {
        vault = new SymbolicConformanceVulnerableVault();
        attacker = new SymbolicConformanceReentrancyAttacker(vault);
        vm.deal(address(vault), 5 ether);
        vm.deal(address(attacker), 1 ether);
        targetContract(address(attacker));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_vaultBacksAttackerAccounting() public view {
        assertGe(address(vault).balance, vault.balanceOf(address(attacker)));
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_vaultBacksAttackerAccounting",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceHalmosReentrancy.t.sol:SymbolicConformanceHalmosReentrancy
[FAIL: assertion failed: 4000000000000000000 < 115792089237316195423570985008687907853269984665640564039456584007913129639936]
	[Sequence] (original: 2, shrunk: 2)
		[SENDER] [SENDER] calldata=depositOne() [ARGS]
		[SENDER] [SENDER] calldata=withdrawOne() [ARGS]
 invariant_vaultBacksAttackerAccounting() ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("Stuck"), "{stdout}");
}

#[forgetest_init]
fn symbolic_conformance_stateless_arithmetic_and_opcodes(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceArithmetic.t.sol",
        r#"
contract SymbolicConformanceArithmetic {
    function checkArithmetic(uint256 x, int8 signed, bytes32 word) public pure {
        assert(x / 1 == x);
        assert(x % 1 == 0);

        if (x == 9) {
            assert(x ** 2 == 81);
        }

        int256 extended = signed;
        assert(extended >= -128);
        assert(extended <= 127);

        bytes1 high = word[0];
        assert(uint8(high) == uint8(uint256(word >> 248)));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkArithmetic"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceArithmetic.t.sol:SymbolicConformanceArithmetic
[PASS] checkArithmetic(uint256,int8,bytes32) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_dynamic_abi_matrix(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceDynamicAbi.t.sol",
        r#"
contract SymbolicConformanceDynamicAbi {
    struct Pair {
        bytes left;
        uint256[] right;
    }

    /// forge-config: default.symbolic.array_lengths = [3, 2, 4, 1, 2]
    function checkDynamic(bytes memory data, string memory text, uint256[] memory values, Pair memory pair) public pure {
        assert(data.length == 3);
        assert(bytes(text).length == 2);
        assert(values.length == 4);
        assert(pair.left.length == 1);
        assert(pair.right.length == 2);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkDynamic"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceDynamicAbi.t.sol:SymbolicConformanceDynamicAbi
[PASS] checkDynamic(bytes,string,uint256[],(bytes,uint256[])) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_symbolic_selector_backdoor(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceSelector.t.sol",
        r#"
contract SymbolicConformanceSelectorTarget {
    uint256 public drained;

    function safe(uint256) external {}

    function drain(uint256 amount) external {
        if (amount == 42) {
            drained = 1;
        }
    }
}

contract SymbolicConformanceSelector {
    SymbolicConformanceSelectorTarget target;

    function setUp() public {
        target = new SymbolicConformanceSelectorTarget();
    }

    function checkNoBackdoor(bytes4 selector, uint256 amount) public {
        address(target).call(abi.encodeWithSelector(selector, amount));
        assert(target.drained() == 0);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkNoBackdoor"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceSelector.t.sol:SymbolicConformanceSelector
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=0x8267e2eef6b19c7400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a args=[0xf6b19c74, 42]] checkNoBackdoor(bytes4,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_stateful_erc20_invariant(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceErc20Invariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceMiniErc20 {
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply = 100;

    constructor() {
        balanceOf[msg.sender] = 100;
    }

    function transfer(address to, uint256 amount) external {
        if (balanceOf[msg.sender] >= amount) {
            balanceOf[msg.sender] -= amount;
            balanceOf[to] += amount;
        }
    }
}

contract SymbolicConformanceErc20Invariant is Test {
    SymbolicConformanceMiniErc20 token;

    function setUp() public {
        token = new SymbolicConformanceMiniErc20();
        targetContract(address(token));
        targetSender(address(this));
        targetSender(address(0xB0B));
    }

    /// forge-config: default.symbolic.invariant_depth = 3
    function invariant_totalSupplyConstant() public view {
        assert(token.totalSupply() == 100);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_totalSupplyConstant",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceErc20Invariant.t.sol:SymbolicConformanceErc20Invariant
[PASS] invariant_totalSupplyConstant() ([METRICS])

╭------------------------------+----------+--------+---------+----------╮
| Contract                     | Selector | Calls  | Reverts | Discards |
+=======================================================================+
| SymbolicConformanceMiniErc20 | transfer | [..]
╰------------------------------+----------+--------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_stateful_erc721_invariant(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceErc721Invariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceMiniErc721 {
    mapping(uint256 => address) public ownerOf;
    mapping(uint256 => address) public getApproved;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    constructor(address initialOwner) {
        ownerOf[1] = initialOwner;
    }

    function approve(address spender, uint256 id) external {
        address owner = ownerOf[id];
        if (msg.sender == owner || isApprovedForAll[owner][msg.sender]) {
            getApproved[id] = spender;
        }
    }

    function setApprovalForAll(address operator, bool approved) external {
        isApprovedForAll[msg.sender][operator] = approved;
    }

    function transferFrom(address from, address to, uint256 id) external {
        address owner = ownerOf[id];
        if (
            owner == from &&
            to != address(0) &&
            (msg.sender == owner || getApproved[id] == msg.sender || isApprovedForAll[owner][msg.sender])
        ) {
            ownerOf[id] = to;
            getApproved[id] = address(0);
        }
    }
}

contract SymbolicConformanceErc721Handler {
    SymbolicConformanceMiniErc721 internal nft;
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address internal constant OWNER = address(0xA11CE);
    address internal constant BOB = address(0xB0B);
    address internal constant OPERATOR = address(0xCAFE);

    constructor(SymbolicConformanceMiniErc721 target) {
        nft = target;
    }

    function approveBobFromOwner() external {
        vm.prank(OWNER);
        nft.approve(BOB, 1);
    }

    function setOperatorForOwner(bool approved) external {
        vm.prank(OWNER);
        nft.setApprovalForAll(OPERATOR, approved);
    }

    function transferOwnerToBob() external {
        vm.prank(OWNER);
        nft.transferFrom(OWNER, BOB, 1);
    }

    function transferBobToOwner() external {
        vm.prank(BOB);
        nft.transferFrom(BOB, OWNER, 1);
    }
}

contract SymbolicConformanceErc721Invariant is Test {
    SymbolicConformanceMiniErc721 nft;
    SymbolicConformanceErc721Handler handler;
    address constant OWNER = address(0xA11CE);

    function setUp() public {
        nft = new SymbolicConformanceMiniErc721(OWNER);
        handler = new SymbolicConformanceErc721Handler(nft);
        targetContract(address(handler));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_mintedOwnerIsNeverZero() public view {
        assert(nft.ownerOf(1) != address(0));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_mintedOwnerIsNeverZero",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceErc721Invariant.t.sol:SymbolicConformanceErc721Invariant
[PASS] invariant_mintedOwnerIsNeverZero() ([METRICS])

╭----------------------------------+---------------------+-------+---------+----------╮
| Contract                         | Selector            | Calls | Reverts | Discards |
+=====================================================================================+
| SymbolicConformanceErc721Handler | approveBobFromOwner | [..]
|----------------------------------+---------------------+-------+---------+----------|
| SymbolicConformanceErc721Handler | setOperatorForOwner | [..]
|----------------------------------+---------------------+-------+---------+----------|
| SymbolicConformanceErc721Handler | transferBobToOwner  | [..]
|----------------------------------+---------------------+-------+---------+----------|
| SymbolicConformanceErc721Handler | transferOwnerToBob  | [..]
╰----------------------------------+---------------------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_conformance_stateful_reentrancy_sequence(prj: _, cmd: _) {
    if should_skip() {
        return;
    }

    prj.add_test(
        "SymbolicConformanceReentrancyInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConformanceVault {
    mapping(address => uint256) public balanceOf;
    bool internal locked;

    function deposit() external payable {
        balanceOf[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) external {
        if (balanceOf[msg.sender] >= amount && !locked) {
            locked = true;
            balanceOf[msg.sender] -= amount;
            (bool ok,) = msg.sender.call{value: amount}("");
            ok;
            locked = false;
        }
    }
}

contract SymbolicConformanceReentrantHandler {
    SymbolicConformanceVault public vault;
    bool internal entered;

    constructor(SymbolicConformanceVault target) {
        vault = target;
    }

    receive() external payable {
        if (!entered) {
            entered = true;
            vault.withdraw(1 ether);
            entered = false;
        }
    }

    function depositOne() external {
        vault.deposit{value: 1 ether}();
    }

    function withdrawOne() external {
        vault.withdraw(1 ether);
    }
}

contract SymbolicConformanceReentrancyInvariant is Test {
    SymbolicConformanceVault vault;
    SymbolicConformanceReentrantHandler handler;

    function setUp() public {
        vault = new SymbolicConformanceVault();
        handler = new SymbolicConformanceReentrantHandler(vault);
        vm.deal(address(handler), 3 ether);
        targetContract(address(handler));
    }

    /// forge-config: default.symbolic.invariant_depth = 3
    function invariant_vaultBacksHandlerBalance() public view {
        assert(address(vault).balance >= vault.balanceOf(address(handler)));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_vaultBacksHandlerBalance",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicConformanceReentrancyInvariant.t.sol:SymbolicConformanceReentrancyInvariant
[PASS] invariant_vaultBacksHandlerBalance() ([METRICS])

╭-------------------------------------+-------------+-------+---------+----------╮
| Contract                            | Selector    | Calls | Reverts | Discards |
+================================================================================+
| SymbolicConformanceReentrantHandler | depositOne  | [..]
|-------------------------------------+-------------+-------+---------+----------|
| SymbolicConformanceReentrantHandler | withdrawOne | [..]
╰-------------------------------------+-------------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
