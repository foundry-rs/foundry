pragma solidity ^0.8.13;

/// @title A Hello world example
contract HelloWorld {

        /// Some example struct
    struct Person {
        uint age;
        address wallet;
    }

            /**
        Here's a more double asterix comment
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

    /** @dev Calculates a rectangle's surface and perimeter.
      * @param w Width of the rectangle.
        * @param h Height of the rectangle.
                * @return s The calculated surface.
* @return p The calculated perimeter.
      */
    function rectangle(uint256 w, uint256 h) public pure returns (uint256 s, uint256 p) {
        s = w * h;
        p = 2 * (w + h);
    }
}
