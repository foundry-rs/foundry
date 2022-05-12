pragma solidity >=0.4.24;

contract SimpleStorage {

    event ValueChanged(address indexed author, address indexed oldAuthor, string oldValue, string newValue);

    address public lastSender;
    string _value;
    string _otherValue;

    constructor(string memory value) public {
        emit ValueChanged(msg.sender, address(0), _value, value);
        _value = value;
    }

    function getValue() view public returns (string memory) {
        return _value;
    }

    function setValue(string memory value) public {
        emit ValueChanged(msg.sender, lastSender, _value, value);
        _value = value;
        lastSender = msg.sender;
    }

    function setValues(string memory value, string memory value2) public {
        _value = value;
        _otherValue = value2;
        lastSender = msg.sender;
    }

    function _hashPuzzle() public view returns (uint256) {
        return 100;
    }
}
