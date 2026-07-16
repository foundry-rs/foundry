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
// user `_mint` override that transitively delegates to one of those, unless every successful path
// proves the recipient code-less or rejects it after the delegation. Calls to `_safeMint`, calls
// made inside the canonical `_safeMint` wrapper, `_mint` functions of other contracts and
// same-name local contracts stay clean.

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

// Calling the hook before ownership is established is not equivalent to `_safeMint`: the
// recipient may accept before the mint but reject after it based on `ownerOf` or its balance.
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
        _mint(to, id); //~WARN: `ERC721._mint` does not check
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

// A reverting refusal branch still runs the callback too early when it precedes the mint.
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
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The account short circuit is safe for accounts, but contract recipients are called before
// ownership is established and can make their decision based on the missing state.
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
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A callback in a modifier prefix still precedes the wrapped mint and cannot replace `_safeMint`.
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
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The guard sits after the placeholder: the body's plain `return` still routes through the
// modifier's tail, so the check runs on every path that keeps the token.
contract ModifierTailGuardNft is ERC721 {
    modifier checkedAfter(address to, uint256 tokenId) {
        _;
        require(
            to.code.length == 0
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        checkedAfter(to, tokenId)
    {
        super._mint(to, tokenId);
        if (tokenId == 0) {
            return;
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The same tail guard over a body holding an assembly block: the EVM `return` it can hold
// leaves the call frame without ever coming back to the modifier, keeping the token unchecked.
contract ModifierTailAssemblyNft is ERC721 {
    modifier checkedAfter(address to, uint256 tokenId) {
        _;
        require(
            to.code.length == 0
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        checkedAfter(to, tokenId)
    {
        super._mint(to, tokenId);
        assembly {
            return(0, 0)
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
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

// `to.code.length >= 1` selects contracts, but their callback still runs before ownership is set.
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
        _mint(to, id); //~WARN: `ERC721._mint` does not check
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

// Named mint arguments preserve the checked identities for a callback that follows the mint.
contract NamedArgumentNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint({tokenId: tokenId, to: to});
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

// `assert` reverts on refusal as `require` does when it follows the mint.
contract CheckedByAssertNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        assert(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The accepting answer may be held by a named constant in a post-mint callback guard.
contract NamedSelectorNft is ERC721 {
    bytes4 private constant RECEIVED = 0x150b7a02;

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "") == RECEIVED,
            "unsafe receiver"
        );
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

// A helper can discharge a mint that preceded it. Later state-changing work in the helper cannot
// retroactively invalidate the completed code-less or callback check.
contract CheckedViaHelperNft is ERC721 {
    function _requireReceiver(address to, uint256 tokenId) private {
        require(
            to.code.length == 0
                || IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        (bool ok,) = to.call("");
        ok;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        _requireReceiver(to, tokenId);
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

// A code-less recipient cannot need the ERC721 receiver callback, so this is safe.
contract AccountOnlyNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A code-less proof is about the recipient, not the token. Remapping the token after the proof
// does not make the recipient require a callback.
contract AccountOnlyRemappedTokenNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        tokenId += 1;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract AccountOnlyComputedTokenNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId + 1);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract AccountOnlyHelperNft is ERC721 {
    function _requireAccount(address to) private view {
        require(to.code.length == 0, "accounts only");
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        _requireAccount(to);
        super._mint(to, tokenId + 1);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract AccountOnlyModifierNft is ERC721 {
    modifier accountOnly(address to) {
        require(to.code.length == 0, "accounts only");
        _;
    }

    function _mint(address to, uint256 tokenId) internal virtual override accountOnly(to) {
        tokenId += 1;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract Create2NonReceiver {}

// A code-length check is only a snapshot. An internal call can deploy a contract at a
// precomputed `to` address before the delegation, so the override still needs `_safeMint` at
// its call site.
contract AccountOnlyThenInternalCallNft is ERC721 {
    function _deployRecipient(uint256 salt) private {
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        _deployRecipient(tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The same invalidation applies to an external call, whose callee may run CREATE2 before the
// delegated mint.
contract AccountOnlyThenExternalCallNft is ERC721 {
    function deployRecipient(uint256 salt) external {
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        this.deployRecipient(tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A call hidden in another `require` argument can run after the code-length expression, so the
// recognized condition does not make the whole guard statement stable.
contract AccountOnlyGuardArgumentCallNft is ERC721 {
    function _deployAndReturnMessage(uint256 salt) private returns (string memory) {
        new Create2NonReceiver{salt: bytes32(salt)}();
        return "accounts only";
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, _deployAndReturnMessage(tokenId));
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A statically known internal helper that only writes storage cannot deploy code and leaves the
// snapshot valid.
contract AccountOnlyThenStorageCallNft is ERC721 {
    uint256 private calls;

    function _incrementCalls() private {
        calls++;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        _incrementCalls();
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// Pure and view calls cannot deploy code, so they do not retire an account-only proof.
contract AccountOnlyThenViewCallNft is ERC721 {
    function _observe() private view returns (bool) {
        return address(this).code.length > 0;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        require(_observe(), "not deployed");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A receiver callback before the mint is not proof of post-mint acceptance, regardless of an
// unrelated state-changing call between the callback and delegation.
contract CallbackThenInternalCallNft is ERC721 {
    function _deployOther(uint256 salt) private {
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        _deployOther(tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Modifier prefixes execute from outer to inner. The inner deployment invalidates the outer
// account-only snapshot before the body delegates.
contract AccountOnlyOuterDeployInnerNft is ERC721 {
    modifier accountOnly(address to) {
        require(to.code.length == 0, "accounts only");
        _;
    }

    modifier deployBefore(uint256 salt) {
        new Create2NonReceiver{salt: bytes32(salt)}();
        _;
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnly(to)
        deployBefore(tokenId)
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A modifier with a nested placeholder is not expanded precisely, but its deployment still
// retires an outer snapshot on the path that reaches the body.
contract AccountOnlyOuterNestedDeployInnerNft is ERC721 {
    bool private enabled;

    modifier accountOnly(address to) {
        require(to.code.length == 0, "accounts only");
        _;
    }

    modifier deployBefore(uint256 salt) {
        new Create2NonReceiver{salt: bytes32(salt)}();
        if (enabled) {
            _;
        }
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnly(to)
        deployBefore(tokenId)
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Modifier arguments are evaluated as their modifier is entered. An inner argument can therefore
// replace the function recipient after an outer modifier checked the original value.
contract AccountOnlyOuterMutatingModifierArgNft is ERC721 {
    modifier accountOnly(address to) {
        require(to.code.length == 0, "accounts only");
        _;
    }

    modifier passthrough(address) {
        _;
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnly(to)
        passthrough(to = address(1))
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// An outer tail guard also captures the original argument. A later modifier-argument assignment
// must retire that deferred proof before the body mints the replacement recipient.
contract AccountOnlyTailOuterMutatingModifierArgNft is ERC721 {
    modifier accountOnlyAfter(address checked) {
        _;
        require(checked.code.length == 0, "accounts only");
    }

    modifier passthrough(address) {
        _;
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnlyAfter(to)
        passthrough(to = address(1))
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A call after the placeholder cannot retroactively invalidate the snapshot that covered the
// delegated mint in the body.
contract AccountOnlyThenModifierTailCallNft is ERC721 {
    modifier accountOnly(address to, uint256 salt) {
        require(to.code.length == 0, "accounts only");
        _;
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnly(to, tokenId)
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// Conversely, a tail code-length guard runs after a state-changing body call and reverts if that
// call deployed the recipient, so every successful mint remains covered.
contract AccountOnlyModifierTailGuardNft is ERC721 {
    modifier accountOnlyAfter(address to) {
        _;
        require(to.code.length == 0, "accounts only");
    }

    function _deployRecipient(uint256 salt) private {
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        accountOnlyAfter(to)
    {
        _deployRecipient(tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
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

// A successful exit sits between the mint and the guard: token zero is credited and the
// function leaves before the hook ever runs, so the guard does not govern that path.
contract MintThenExitGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (tokenId == 0) {
            return;
        }
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Named helper arguments bind by parameter name, not by source position: the helper checks
// `checked`, which the call binds to the guardian, so the recipient is never asked.
contract NamedHelperArgsNft is ERC721 {
    address internal guardian;

    function _check(address checked, address other, uint256 id) internal {
        other;
        require(
            IERC721Receiver(checked).onERC721Received(msg.sender, address(0), id, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        _check({other: to, checked: guardian, id: tokenId});
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The helper holds a real guard shape, but asks about a fixed token, not the delegated one.
contract HelperWrongTokenNft is ERC721 {
    function _check(address to, uint256 id) internal {
        id;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), 0, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        _check(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A mutable state token is not a stable identity: an intervening call can reenter and change it
// after the mint reads the old value but before the callback guard reads the new one.
contract StateTokenNft is ERC721 {
    uint256 internal nextId;

    function _mint(address to, uint256 tokenId) internal virtual override {
        tokenId;
        super._mint(to, nextId);
        this.bumpNextId();
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), nextId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function bumpNextId() external {
        nextId++;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The hook is asked about another token than the one the delegation mints: the recipient may
// accept the former and refuse the latter.
contract WrongTokenHookNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId + 1, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A member spelled `onERC721Received.selector` on another interface is not the accepting
// answer: the one-parameter hook hashes to a different selector, so a recipient answering
// the real one still fails this comparison, and one answering this value never accepted.
contract ForeignSelectorNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == INotAReceiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The delegation truncates the recipient through a lossy cast chain: the minted address is
// usually not the one the guard asked, so the cast does not preserve the recipient.
contract TruncatedRecipientNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(address(uint160(uint8(uint160(to)))), tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A callback modifier prefix is too early to cover the mint; the body also remaps its token.
contract ModifierGuardRemapTokenNft is ERC721 {
    modifier checked(address to, uint256 tokenId) {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        _;
    }

    function _mint(address to, uint256 tokenId) internal virtual override checked(to, tokenId) {
        tokenId = tokenId + 1;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A callback modifier prefix is too early to cover the mint; the body also redirects its
// recipient.
contract ModifierGuardRedirectNft is ERC721 {
    address internal attacker;

    modifier checked(address to, uint256 tokenId) {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        _;
    }

    function _mint(address to, uint256 tokenId) internal virtual override checked(to, tokenId) {
        to = attacker;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A tail-guard modifier checks the entry value on the way out; the body still mints a
// reassigned token the modifier's captured argument never covered.
contract ModifierTailRemapNft is ERC721 {
    modifier checkedAfter(address to, uint256 tokenId) {
        _;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        checkedAfter(to, tokenId)
    {
        tokenId = tokenId + 1;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A post-mint guard reassigns the token inside another hook argument. Conservatively, mutation
// during the guard retires correlation even though the third argument was evaluated first.
contract GuardArgReassignNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(
                msg.sender, address(0), tokenId, abi.encodePacked(tokenId = tokenId + 1)
            ) == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The token is reassigned inside an `if` condition between the mint and callback, so the hook
// acknowledges a different value from the one already credited.
contract CondAssignTokenNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if ((tokenId = tokenId + 1) > 0) {}
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The recipient is redirected inside an `if` condition after the mint, so the hook asks another
// address to accept the already credited token.
contract CondAssignRecipientNft is ERC721 {
    address internal attacker;

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if ((to = attacker) != address(0)) {}
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A post-increment between the mint and callback makes the hook acknowledge the next token.
contract CondIncrementTokenNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (tokenId++ > 0) {}
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A local is reassigned after the mint, so the later callback checks another token despite using
// the same variable name.
contract LocalTokenReassignNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        uint256 id = tokenId;
        super._mint(to, id);
        id = tokenId + 1;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), id, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The recipient parameter is reassigned after the mint, so the callback asks another address.
contract RecipientReassignNft is ERC721 {
    address internal attacker;

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        to = attacker;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The token parameter is remapped after the mint, so the callback acknowledges the offset token.
contract ParamTokenRemapNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        tokenId = tokenId + 1;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The account branch a `to.code.length` test seeds as covered redirects the recipient to a
// contract and mints unchecked: the seed assumed an account, which the reassignment defeats.
contract AccountBranchReassignNft is ERC721 {
    address internal attacker;

    function _mint(address to, uint256 tokenId) internal virtual override {
        if (to.code.length > 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
            super._mint(to, tokenId);
        } else {
            to = attacker;
            super._mint(to, tokenId);
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The check after the mint may be skipped for accounts: an account accepts the token already
// credited as it accepts any other, and the contract branch reverts on refusal.
contract MintThenAccountSkipNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (to.code.length > 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The same skip written as an early return: the path that leaves is the account one, which
// keeps the token rightfully, and the contract path still reaches the guard.
contract MintThenReversedSkipNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (to.code.length == 0) {
            return;
        }
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

// A conditional mint is still covered by the unconditional guard after it: whichever path
// credited the token, the revert undoes it.
contract ConditionalMintNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (tokenId % 2 == 0) {
            super._mint(to, tokenId);
        }
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

// A post-mint callback behind a condition covers the mint when every branch performs one.
contract EitherBranchGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (tokenId % 2 == 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver"
            );
        } else {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                    == IERC721Receiver.onERC721Received.selector,
                "unsafe receiver too"
            );
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A revert between the mint and the guard is not an escape: no path through it keeps the token.
contract RevertBetweenNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (tokenId == 0) {
            revert("zero");
        }
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

// The round trip through `uint160` keeps the address in fact, but the peel does not follow
// numeric intermediates: a conservative report, on the safe side.
contract RoundTripCastNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(address(uint160(to)), tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A `payable` conversion keeps the address: the minted recipient is the guarded one.
contract PayableRecipientNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(payable(to), tokenId);
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

// An attached library function is not the recipient answering: `to.onERC721Received(...)`
// resolves to the library's internal function, which runs in this contract without any
// external call, so the recipient never acknowledged the token.
library AttachedReceiverCheck {
    function onERC721Received(address, address, uint256, bytes memory)
        internal
        pure
        returns (bytes4)
    {
        return 0x150b7a02;
    }
}

contract AttachedHookNft is ERC721 {
    using AttachedReceiverCheck for address;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            to.onERC721Received(address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The same attachment called with named arguments: name binding would land the token on the
// right parameter despite the shifted first argument, so only the resolved declaration itself,
// a library function no external call ever reaches, rules the shape out.
library NamedAttachedCheck {
    function onERC721Received(address self, address from, uint256 tokenId, bytes memory data)
        internal
        pure
        returns (bytes4)
    {
        self;
        from;
        tokenId;
        data;
        return 0x150b7a02;
    }
}

contract NamedAttachedHookNft is ERC721 {
    using NamedAttachedCheck for address;

    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            to.onERC721Received({from: address(0), tokenId: tokenId, data: ""})
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
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

// Recursive unsafe targets preserve both identities, so an outer callback after the recursive
// delegation covers the canonical mint.
contract RecursiveIdentityBaseNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
    }
}

contract RecursiveIdentityCheckedNft is RecursiveIdentityBaseNft {
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

// An intermediate override remaps the token before reaching the canonical mint. The outer
// guard acknowledged the original token, so it cannot exempt callers through this target.
contract RecursiveTokenRemapBaseNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId + 1);
    }
}

contract RecursiveTokenRemapCheckedNft is RecursiveTokenRemapBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The same recursive identity requirement applies to the recipient: the intermediate target
// credits another address even though the outer override checked `to`.
contract RecursiveRecipientRemapBaseNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        to = address(1);
        super._mint(to, tokenId);
    }
}

contract RecursiveRecipientRemapCheckedNft is RecursiveRecipientRemapBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A code-less proof survives the recursive token remap above because only the recipient's
// identity matters.
contract RecursiveCodeLessTokenRemapNft is RecursiveTokenRemapBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// An intermediate override can invalidate an outer code-length proof before it reaches the
// canonical mint, so the stability summary follows the recursive delegation chain too.
contract RecursiveCodeDeployBaseNft is ERC721 {
    function _deployRecipient(uint256 salt) private {
        new Create2NonReceiver{salt: bytes32(salt)}();
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        _deployRecipient(tokenId);
        super._mint(to, tokenId);
    }
}

contract RecursiveCodeLessThenDeployNft is RecursiveCodeDeployBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

contract RecursiveModifierDeployBaseNft is ERC721 {
    modifier deployBefore(uint256 salt) {
        new Create2NonReceiver{salt: bytes32(salt)}();
        _;
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        deployBefore(tokenId)
    {
        super._mint(to, tokenId);
    }
}

// Modifier prefixes are included in recursive code-stability summaries too.
contract RecursiveCodeLessThenModifierDeployNft is RecursiveModifierDeployBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

contract RecursiveModifierArgRedirectBaseNft is ERC721 {
    modifier passthrough(address) {
        _;
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        passthrough(to = address(1))
    {
        super._mint(to, tokenId);
    }
}

// Recursive identity and code-stability summaries include modifier-argument assignments.
contract RecursiveCodeLessThenModifierArgRedirectNft is RecursiveModifierArgRedirectBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// It does not survive a recursive recipient remap: the canonical mint credits an address whose
// code length the outer override never checked.
contract RecursiveCodeLessRecipientRemapNft is RecursiveRecipientRemapBaseNft {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(to.code.length == 0, "accounts only");
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Even in the accepted branch, the callback ran before ownership was established.
contract AcceptedBranchMintNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector
        ) {
            super._mint(to, tokenId);
        } else {
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Recognizing the refusal guard does not skip its accepted branch. It changes the token before
// the later mint, invalidating the coverage established by the comparison.
contract AcceptedBranchTokenRemapNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector
        ) {
            tokenId += 1;
        } else {
            revert("unsafe receiver");
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A helper's body is not guaranteed to run when one of its modifiers can skip the placeholder.
contract ModifiedGuardHelperNft is ERC721 {
    bool internal checksEnabled;

    modifier whenChecksEnabled() {
        if (checksEnabled) {
            _;
        }
    }

    function _check(address to, uint256 tokenId) private whenChecksEnabled {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        _check(to, tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// An inner sibling modifier can leave the whole frame after its placeholder, bypassing an outer
// modifier's tail guard.
contract SiblingModifierTailEscapeNft is ERC721 {
    modifier checkedAfter(address to, uint256 tokenId) {
        _;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    modifier escapeAfter() {
        _;
        assembly {
            return(0, 0)
        }
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        checkedAfter(to, tokenId)
        escapeAfter
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// The reverse order is safe: the inner tail guard runs before the outer modifier can leave the
// frame, so modifier bypass analysis must respect expansion order.
contract InnerModifierTailGuardNft is ERC721 {
    modifier escapeAfter() {
        _;
        assembly {
            return(0, 0)
        }
    }

    modifier checkedAfter(address to, uint256 tokenId) {
        _;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId)
        internal
        virtual
        override
        escapeAfter
        checkedAfter(to, tokenId)
    {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// An ordinary internal helper before the refusal revert does not leave the current frame, so
// the branch still always reverts and the wrapper remains safe.
contract RefusalNoopHelperNft is ERC721 {
    function _noop() private pure {}

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            _noop();
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// An internal helper can transitively execute an assembly return in the caller's frame. The
// later revert is then unreachable and cannot make the refusal branch safe.
contract RefusalHelperAssemblyEscapeNft is ERC721 {
    function _escape() private pure {
        assembly {
            return(0, 0)
        }
    }

    function _escapeIndirectly() private pure {
        _escape();
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            _escapeIndirectly();
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// An internal function-pointer target is unresolved at its call site. It can reach assembly that
// leaves the current frame before the later revert, so the refusal branch is not proven safe.
contract RefusalPointerAssemblyEscapeNft is ERC721 {
    function _escape() private pure {
        assembly {
            return(0, 0)
        }
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        function() internal escape = _escape;
        super._mint(to, tokenId);
        if (
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                != IERC721Receiver.onERC721Received.selector
        ) {
            escape();
            revert("unsafe receiver");
        }
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A same-width selector conversion preserves the accepting answer for a post-mint callback.
contract PreservingSelectorCastNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == bytes4(uint32(0x150b7a02)),
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// The `uint8` conversion truncates the selector to `0x02`; later widening cannot restore it.
contract TruncatedSelectorCastNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == bytes4(uint32(uint8(uint32(0x150b7a02)))),
            "unsafe receiver"
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Helper recursion detection is path scoped. After the first check and mutation, an independent
// second call to the same helper follows the mint and covers the remapped token.
contract RepeatedGuardHelperNft is ERC721 {
    function _check(address to, uint256 tokenId) private {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        _check(to, tokenId);
        tokenId += 1;
        super._mint(to, tokenId);
        _check(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

// A recognized `if` comparison cannot restore coverage after one of its other hook arguments
// reassigns the token. The hook may have seen the old value while the later mint credits the new
// one, just as with a mutating `require` argument.
contract IfGuardArgReassignNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        if (
            IERC721Receiver(to).onERC721Received(
                msg.sender, address(0), tokenId, abi.encodePacked(tokenId = tokenId + 1)
            ) != IERC721Receiver.onERC721Received.selector
        ) {
            revert("unsafe receiver");
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A helper receives value-type copies. Reassigning its token parameter before the check means
// it acknowledges a different value from the unchanged token its caller later mints.
contract RemappedGuardHelperNft is ERC721 {
    function _check(address to, uint256 tokenId) private {
        tokenId += 1;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        _check(to, tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Modifier parameters are copies too: this guard asks about the incremented local token while
// the wrapped body still mints the function parameter passed to the modifier.
contract RemappedGuardModifierNft is ERC721 {
    modifier checked(address to, uint256 tokenId) {
        tokenId += 1;
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        _;
    }

    function _mint(address to, uint256 tokenId) internal virtual override checked(to, tokenId) {
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A public helper called by bare name dispatches internally in the same frame and can cover a
// mint that precedes it.
contract PublicGuardHelperNft is ERC721 {
    function _check(address to, uint256 tokenId) public {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        _check(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract ExternalReceiverGuard {
    function check(address to, uint256 tokenId) external {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
    }
}

// A cross-contract helper calls the hook from its own address. A receiver can accept that caller
// while refusing the NFT contract itself, so this does not prove that the mint is receivable.
contract ExternalGuardHelperNft is ERC721 {
    ExternalReceiverGuard internal guard;

    constructor(ExternalReceiverGuard guard_) {
        guard = guard_;
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        guard.check(to, tokenId);
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A guard's error argument can leave the entire frame through an internal assembly helper before
// `require` reverts, preserving the mint that already ran.
contract GuardArgumentAssemblyEscapeNft is ERC721 {
    function _escapeMessage() private pure returns (string memory) {
        assembly {
            return(0, 0)
        }
    }

    function _mint(address to, uint256 tokenId) internal virtual override {
        super._mint(to, tokenId);
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            _escapeMessage()
        );
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Inline assembly can rewrite a Solidity local without producing an HIR assignment expression.
// Coverage from the earlier guard must not survive that opaque block.
contract AssemblyTokenRemapNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(
            IERC721Receiver(to).onERC721Received(msg.sender, address(0), tokenId, "")
                == IERC721Receiver.onERC721Received.selector,
            "unsafe receiver"
        );
        assembly {
            tokenId := add(tokenId, 1)
        }
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A fresh callback after the opaque assembly block and mint checks the remapped token.
contract AssemblyTokenRemapThenGuardNft is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        assembly {
            tokenId := add(tokenId, 1)
        }
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
