contract C {
    mapping(uint nameSame => mapping(uint name1 => mapping(uint nameSame => uint name3) name6) name4) map;

    function main() external {
        mapping(uint nameSame => mapping(uint name1 => mapping(uint nameSame => uint name3) name6) name4) storage _map = map;
    }
}
