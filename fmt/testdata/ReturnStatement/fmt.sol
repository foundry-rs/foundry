contract ReturnStatement {
    function value() internal returns (uint256) {
        return type(uint256).max;
    }

    function returnEmpty() external {
        if (true) {
            return;
        }

        if (false) {
            // return empty 1
            /* return empty 2 */
            return; // return empty 3
        }

        /* return empty 4 */
        return;
        // return empty 5
    }

    function returnSingleValue(uint256 val) external returns (uint256) {
        if (val == 0) {
            return 0x00; // return single value 1
        }

        if (val == 1) {
            return 1;
        }

        if (val == 2) {
            return 3 - 1;
        }

        if (val == 4) {
            /* return single value 2 */
            return 2** // return single value 3
                3 // return single value 4
                /* return single value 5 */;
        }

        return value(); // return single value 6
    }
}
