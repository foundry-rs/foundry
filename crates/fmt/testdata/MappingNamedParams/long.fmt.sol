// config: line_length = 120
contract C {
    mapping(uint256 nameSame => mapping(uint256 name1 => mapping(uint256 nameSame => uint256 name3) name6) name4) map;

    function main() external {
        mapping(
            uint256 nameSame => mapping(uint256 name1 => mapping(uint256 nameSame => uint256 name3) name6) name4
        ) storage _map = map;
    }
}
