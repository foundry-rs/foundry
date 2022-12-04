// config: line_length = 40
// config: wrap_comments = true
pragma solidity ^0.8.13;

/// @title A Hello world example
contract HelloWorld {
    /// Some example struct
    struct Person {
        uint256 age;
        address wallet;
    }

    /**
     * Here's a more double asterix
     * comment
     */
    Person public theDude;

    /// Constructs the dude
    /// @param age The dude's age
    constructor(uint256 age) {
        theDude = Person({
            age: age,
            wallet: msg.sender
        });
    }

    /**
     * @dev does nothing
     */
    function example() public {
        /**
         * Does this add a whitespace
         * error?
         *
         * Let's find out.
         */
    }

    /**
     * @dev Calculates a rectangle's
     * surface and perimeter.
     * @param w Width of the rectangle.
     * @param h Height of the rectangle.
     * @return s The calculated surface.
     * @return p The calculated
     * perimeter.
     */
    function rectangle(
        uint256 w,
        uint256 h
    )
        public
        pure
        returns (uint256 s, uint256 p)
    {
        s = w * h;
        p = 2 * (w + h);
    }

    /// A long doc line comment that
    /// will be wrapped
    function docLineOverflow()
        external
    {}

    function docLinePostfixOveflow()
        external
    {}

    /// A long doc line comment that
    /// will be wrapped

    /**
     * @notice Here is my comment
     *       - item 1
     *       - item 2
     * Some equations:
     *     y = mx + b
     */
    function anotherExample()
        external
    {}

    /**
     * contract A {
     *     function foo() public {
     *         // does nothing.
     *     }
     * }
     */
    function multilineIndent()
        external
    {}

    /**
     * contract A {
     * function foo() public {
     *             // does nothing.
     *   }
     * }
     */
    function multilineMalformedIndent()
        external
    {}

    /**
     * contract A {
     * function
     * withALongNameThatWillCauseCommentWrap()
     * public {
     *             // does nothing.
     *   }
     * }
     */
    function malformedIndentOveflow()
        external
    {}
}

/**
 * contract A {
 *     function foo() public {
 *         // does nothing.
 *     }
 * }
 */
function freeFloatingMultilineIndent() {}

/// @notice A proof-of-concept
/// implementation demonstrating how
/// Flexible Voting can be used to allow
/// holders of governance tokens to use
/// them in DeFi but still participate
/// in governance. The FractionalPool
/// simulates a lending protocol, such
/// as Compound Finance or Aave, in
/// that:
///
/// - Tokens can be deposited into the
///   Pool by suppliers
/// - Tokens can be withdrawn from the
///   Pool by borrowers
/// - Depositors are able to express
///   their vote preferences on
///   individual governance proposals
/// - The vote preferences of all
///   Depositors are expressed
///   proportionally across any tokens
///   held by the pool, and rolled up
///   into a single delegated vote made
///   by the pool before the proposal is
///   completed
contract FractionalPool {
    /// @notice A proof-of-concept
    /// implementation demonstrating how
    /// Flexible Voting can be used to
    /// allow holders of governance
    /// tokens to use them in DeFi but
    /// still participate in governance.
    /// The FractionalPool simulates a
    /// lending protocol, such as
    /// Compound Finance or Aave, in
    /// that:
    function testDocWrap() external {}

    // @notice A proof-of-concept
    // implementation demonstrating how
    // Flexible Voting can be used to
    // allow holders of governance
    // tokens to use them in DeFi but
    // still participate in governance.
    // The FractionalPool simulates a
    // lending protocol, such as
    // Compound Finance or Aave, in
    // that:
    function testLineWrap() external {}
}

// @notice A proof-of-concept
// implementation demonstrating how
// Flexible Voting can be used to allow
// holders of governance tokens to use
// them in DeFi but still participate in
// governance. The FractionalPool
// simulates a lending protocol, such as
// Compound Finance or Aave, in that:
//
// - Tokens can be deposited into the
//   Pool by suppliers
// - Tokens can be withdrawn from the
//   Pool by borrowers
// - Depositors are able to express
//   their vote preferences on
//   individual governance proposals
// - The vote preferences of all
//   Depositors are expressed
//   proportionally across any tokens
//   held by the pool, and rolled up
//   into a single delegated vote made
//   by the pool before the proposal is
//   completed
contract FractionalPool2 {}

/// Some contract specification that:
/// * satisfies the first long
///   constraint
/// * must uphold to the second
///   constraint under every condition
/// * correctly implements
///   the interface required for
///   interacting with other contracts
contract ContractWithBulletItems {}
