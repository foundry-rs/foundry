contract ReturnStatement {
    function value() internal returns (uint256) {
        return type(uint256).max;
    }

    function returnEmpty() external {
        if (true) {
            return  ;
        }

        if (false) {
              // return empty 1
    return /* return empty 2 */ ; // return empty 3
        }

        /* return empty 4 */ return // return empty 5
        ;
    }

    function returnSingleValue(uint256 val) external returns (uint256) {
        if (val == 0) {
        return // return single 1
        0x00;
        }

        if (val == 1) { return 
        1; }

        if (val == 2) {
                return 3
                -
                    1;
        }

        if (val == 4) {
            /* return single 2 */ return 2** // return single 3
            3 // return single 4
            ;
        }

        return  value() // return single 5
        ;
    }

    function returnMultipleValues(uint256 val) external returns (uint256, uint256, bool) {
        if (val == 0) { return /* return mul 1 */ (0, 1,/* return mul 2 */ false); }

        if (val == 1) { 
    // return mul 3
            return /* return mul 4 */
            (
                987654321, 1234567890,/* return mul 5 */ false); }

        if (val == 2) {
            return /* return mul 6 */ ( 1234567890 + 987654321  + 87654123536, 987654321 + 1234567890  + 124245235235, true);  
        }

        return someFunction().getValue().modifyValue().negate().scaleBySomeFactor(1000).transformToTuple();
    }
}
