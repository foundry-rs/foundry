//@compile-flags: --only-lint unsafe-oz-erc721-mint
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {
    ERC721,
    ERC721Upgradeable,
    ERC721Consecutive
} from "./auxiliary/openzeppelin-contracts/Erc721Mocks.sol";
import {ERC721 as LocalERC721} from "./auxiliary/not-openzeppelin/Erc721Mocks.sol";

// Tests for `unsafe-oz-erc721-mint`: `ERC721._mint` credits a token without checking that the
// recipient can receive it (no `onERC721Received` call), so minting to a non-receiver contract
// locks the token; `_safeMint` performs the check. A call is flagged when it resolves to a
// function named `_mint` declared in a contract named exactly `ERC721`, `ERC721Upgradeable`,
// `ERC721Consecutive` or `ERC721ConsecutiveUpgradeable` (the v4 Consecutive extensions forward
// to the base without the check) whose source comes from an OpenZeppelin package path, or to a
// user `_mint` override that transitively delegates to one of those, unless the recipient's
// refusal reverts the mint. Calls to `_safeMint`, calls made inside the canonical `_safeMint`
// wrapper, `_mint` functions of other contracts and same-name local contracts stay clean.

// Same `_mint` shape on a fungible token: transfers need no receiver check, out of scope.
contract ERC20 {
    mapping(address => uint256) internal _balances;

    function _mint(address account, uint256 amount) internal {
        _balances[account] += amount;
    }
}

contract DirectNft is ERC721 {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintSuper(address to, uint256 id) external {
        super._mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintQualified(address to, uint256 id) external {
        ERC721._mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintSafe(address to, uint256 id) external {
        _safeMint(to, id);
    }
}

contract BaseNft is ERC721 {}

// The ERC721 base is two levels up: resolution through the linearized bases still finds it.
contract IndirectNft is BaseNft {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

contract UpgradeableNft is ERC721Upgradeable {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A consumer of the extension resolves `_mint` to the Consecutive override: still unchecked.
// The mirror lives under the OpenZeppelin auxiliary path; its own `super._mint` sits inside a
// `_mint` override and stays exempt.
contract ConsecutiveNft is ERC721Consecutive {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A user-defined `_safeMint` override is not the canonical OZ wrapper: calling `_mint`
// directly inside it skips the receiver check and must stay analyzed.
contract BrokenSafeNft is ERC721 {
    function _safeMint(address to, uint256 tokenId) internal override {
        _mint(to, tokenId); //~WARN: `ERC721._mint` does not check
    }
}

// A safe override in a contract whose name happens to contain "ERC721": the call resolves
// to the local override, so nothing fires, since matching is on the exact base name.
contract SafeERC721Override is ERC721 {
    function _mint(address to, uint256 tokenId) internal override {
        _owners[tokenId] = to;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract Token is ERC20 {
    function mint(address account, uint256 amount) external {
        _mint(account, amount);
    }
}

// Overriding `_mint` without calling the base makes the override the dispatch target:
// the plain call is safe, only an explicit `super._mint` still reaches the base.
contract SafeOverride is ERC721 {
    function _mint(address to, uint256 tokenId) internal override {
        _owners[tokenId] = to;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }

    function mintSuper(address to, uint256 id) external {
        super._mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A `_mint` override delegating to the base with its own guard (the capped/pausable
// pattern) is an unsafe call target: the path still reaches the unchecked base. Direct
// calls dispatching to it report; the `super._mint` inside the override itself does not,
// since the override is the mint primitive and `_safeMint` there would re-enter it through
// the virtual dispatch.
contract CappedNft is ERC721 {
    uint256 internal total;
    uint256 internal constant CAP = 10;

    function _mint(address to, uint256 tokenId) internal override {
        require(total < CAP, "cap");
        total++;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The delegation is judged transitively: an override that simply forwards still reaches the
// unchecked base, so its direct callers report, while `_safeMint` stays the clean path.
contract DelegatingOverrideNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
    }

    function unsafeMint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function safeMint(address to, uint256 id) external {
        _safeMint(to, id);
    }
}

contract MiddleSafe is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        _owners[tokenId] = to;
    }
}

// `super` dispatches to the closest declaration in the linearization: MiddleSafe's safe
// override, not the ERC721 base behind it.
contract ChildOfMiddle is MiddleSafe {
    function mintSuper(address to, uint256 id) external {
        super._mint(to, id);
    }
}

// A local overload with a different arity is the only candidate a one-argument call can
// dispatch to: out of scope. The two-argument call still resolves to the base `_mint`.
contract OverloadNft is ERC721 {
    function _mint(address to) internal {
        _owners[0] = to;
    }

    function mintOne(address to) external {
        _mint(to);
    }

    function mintTwo(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A genuine same-arity overload with different parameter types is not an override of the base
// `_mint(address, uint256)`: the two-argument uint call still dispatches to the unsafe base.
contract DataNft is ERC721 {
    function _mint(address to, bytes memory data) internal {
        _owners[uint256(uint160(to))] = to;
        data;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintWithData(address to, bytes memory data) external {
        _mint(to, data);
    }
}

// A library is not the OZ ERC721 contract, even with ERC721 in its name: out of scope.
library ERC721Lib {
    function _mint(address to, uint256 id) internal pure {}
}

contract UsesLib {
    function mint(address to, uint256 id) external pure {
        ERC721Lib._mint(to, id);
    }
}

// Calls in a modifier or a constructor are calls like any other.
contract EagerNft is ERC721 {
    modifier premint(address to, uint256 id) {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
        _;
    }

    constructor(address to) {
        _mint(to, 0); //~WARN: `ERC721._mint` does not check
    }

    function noop(address to, uint256 id) external premint(to, id) {}
}

// A local `_mint` on a contract that is not an ERC721: out of scope.
contract Standalone {
    uint256 internal _supply;

    function _mint(uint256 amount) internal {
        _supply += amount;
    }

    function mint(uint256 amount) external {
        _mint(amount);
    }
}

// A local contract reusing the exact `ERC721ConsecutiveUpgradeable` name, unrelated to
// OpenZeppelin: the provenance check keeps its `_mint` out of scope.
contract ERC721ConsecutiveUpgradeable {
    mapping(uint256 => address) internal _holders;

    function _mint(address account, uint256 amount) internal {
        _holders[amount] = account;
    }
}

contract UsesConsecutiveUpgradeableName is ERC721ConsecutiveUpgradeable {
    function mint(address account, uint256 amount) external {
        _mint(account, amount);
    }
}

interface IERC721Receiver {
    function onERC721Received(address operator, address from, uint256 tokenId, bytes calldata data)
        external
        returns (bytes4);
}

// A same-name hook carrying another shape, for the case below.
interface INotAReceiver {
    function onERC721Received(address operator) external returns (bytes4);
}

// An override whose delegated mint reverts when the recipient refuses is a safe wrapper, like
// the canonical `_safeMint`: its direct callers are not reported. Skipping the hook for an
// account is the one admissible reason not to run it.
contract CheckedOverrideNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The guard may follow the mint, as the canonical `_safeMint` does: the revert undoes it.
contract CheckedAfterMintNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// An `if` whose branch always exits is a guard too.
contract CheckedByRevertNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            revert("unsafe receiver");
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The account short circuit runs the hook for every recipient carrying code.
contract CheckedOrAccountNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            to.code.length == 0
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A modifier carries the guard as well as the body does.
contract CheckedByModifierNft is ERC721 {
    modifier checked(address to, uint256 tokenId) {
        require(
            to.code.length == 0
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        _;
    }

    function _mint(address to, uint256 tokenId) internal virtual override checked(to, tokenId) {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The hook sits in the condition, but a first operand can satisfy it on its own: the call never
// runs for a trusted recipient, which is then minted to unchecked.
contract ShortCircuitGuardNft is ERC721 {
    address internal trusted;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            to == trusted
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The hook rides in the revert message, where its answer decides nothing.
contract MessageArgumentNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            tokenId > 0,
            string(
                abi.encodePacked(
                    IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                )
            )
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The hook is asked of a guardian, so the recipient never answered.
contract GuardianHookNft is ERC721 {
    mapping(address => address) internal guardians;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(guardians[to]).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The exiting branch is the one an acceptance takes, so a refusal falls through to the mint.
contract InvertedGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector
        ) {
            revert("accepted");
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// `to.code.length >= 1` says the recipient carries code, as `> 0` does.
contract CheckedAtLeastOneNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length >= 1) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The refusal only returns, and the token was already credited.
contract ReturnGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            return;
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The guard sits in a `virtual` helper, which an override may replace with an empty body: the
// analyzed declaration is not the one the call dispatches to.
contract VirtualCheckNft is ERC721 {
    function _check(address to, uint256 tokenId) internal virtual {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0) {
            _check(to, tokenId);
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The reverting branch lets a privileged caller `return` first, keeping the token it was already
// credited: a block reverts only when nothing before the revert can leave the function.
contract EarlyReturnGuardNft is ERC721 {
    address internal owner;

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            if (msg.sender == owner) {
                return;
            }
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The accepting answer moved into a constant does not become accepting: a recipient answering
// zero refused the token.
contract WrongConstantNft is ERC721 {
    bytes4 private constant REFUSED = 0x00000000;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") == REFUSED,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The second mint hands the base an address the guard never named.
contract SecondRecipientNft is ERC721 {
    address internal treasury;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
        super._mint(treasury, tokenId + 1);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The mint sits in the accepting branch and a sibling statement reverts. Nothing locks here, but
// reading that takes the order of the statements, so the wrapper reports: a conservative report.
contract FallThroughRevertNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0) {
            if (
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector
            ) {
                super._mint(to, tokenId);
                return;
            }
            revert("unsafe receiver");
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

contract ForwardingBaseOne is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
    }
}

contract ForwardingBaseTwo is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
    }
}

// The two bases reach the same unchecked `_mint`, and the second one mints elsewhere.
contract DiamondNft is ForwardingBaseOne, ForwardingBaseTwo {
    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override(ForwardingBaseOne, ForwardingBaseTwo)
    {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        ForwardingBaseOne._mint(to, tokenId);
        ForwardingBaseTwo._mint(address(1), tokenId + 1);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The assembly block can hold the EVM `return`, so the `revert` after it may never run.
contract AssemblyEscapeNft is ERC721 {
    address internal owner;

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            if (msg.sender == owner) {
                assembly {
                    return(0, 0)
                }
            }
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The accepting answer settled at deployment is unknown here, so it does not exempt.
contract ImmutableAnswerNft is ERC721 {
    bytes4 private immutable answer;

    constructor(bytes4 expected) {
        answer = expected;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") == answer,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Named arguments come in source order, not in parameter order: the token still goes to `to`.
contract NamedArgumentNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint({tokenId: tokenId, to: to});
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// `assert` reverts on refusal as `require` does.
contract CheckedByAssertNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        assert(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The accepting answer may be held by a named constant, as the older OpenZeppelin releases do.
contract NamedSelectorNft is ERC721 {
    bytes4 private constant RECEIVED = 0x150b7a02;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") == RECEIVED,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The answer gates the revert, but against a literal that is not the accepting one: a recipient
// answering zero refused the token and is minted to anyway.
contract WrongAnswerNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") == bytes4(0),
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A loop body may never run, so a guard inside one governs nothing.
contract LoopGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        for (uint256 i = 0; i < tokenId; i++) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A helper returning the answer as a `bool` reverts on refusal through its caller, but the value
// is not followed across the call, so the wrapper reports.
contract BoolHelperNft is ERC721 {
    function _accepts(address to, uint256 tokenId) private returns (bool) {
        return IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
            == IERC721Receiver.onERC721Received.selector;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(_accepts(to, tokenId), "unsafe receiver");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The guard may sit in a helper the recipient is handed to, the way OpenZeppelin factors
// `_checkOnERC721Received` out of `_safeMint`.
contract CheckedViaHelperNft is ERC721 {
    function _requireReceiver(address to, uint256 tokenId) private {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0) {
            _requireReceiver(to, tokenId);
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A `try` hands the refusal to its `catch`, which may swallow it: the mint goes through even
// for a recipient that reverts, so the override is not a safe wrapper.
contract TryCatchOverrideNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        try IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") returns (
            bytes4
        ) {} catch {}
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The hook runs, but its answer is discarded: a recipient refusing the token is still minted to.
contract IgnoredAnswerNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The guard is real but a condition unrelated to the recipient decides whether it runs, so it
// governs nothing: every other token id reaches the unchecked base.
contract ConditionalGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (tokenId == 123456789) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The `.code` comparison is not alone here, so the guard does not govern the mint.
contract PartialGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0 && tokenId == 5) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The answer is checked, but through a local: the value is not followed across statements, so
// the wrapper is reported. A documented limit, on the safe side.
contract StoredAnswerNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        bytes4 answer = IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "");
        require(answer == IERC721Receiver.onERC721Received.selector, "unsafe receiver");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The guarded call lands on a same-name hook of another interface, so the shape rules it out.
contract WrongHookNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            INotAReceiver(to).onERC721Received(msg.sender) == bytes4(0x150b7a02), "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A `.code` guard is a contract check, not a receiver check: direct callers still report.
contract CodeGuardedOverrideNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length > 0, "no receiver");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Admitting only code-less recipients is not a receiver check either: a contract minting to
// itself from its constructor passes it, and never acknowledged the token.
contract AccountOnlyNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A bare read of the recipient's `.code` guards nothing at all: the value is never used to
// gate the delegated mint, so direct callers report.
contract BareCodeReadNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        uint256 len = to.code.length;
        len;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A construction guard on `address(this)` says nothing about the recipient and leaves the path
// to the unchecked base wide open for arbitrary ones, so direct callers of the override report.
contract ConstructionGuardedNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(address(this).code.length > 0, "no construction mint");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A contract named exactly `ERC721` but declared under a `not-openzeppelin/` path: its path
// contains the substring "openzeppelin" but no OpenZeppelin package-root component, so the
// provenance check keeps its `_mint` out of scope.
contract UsesLocalErc721 is LocalERC721 {
    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}
