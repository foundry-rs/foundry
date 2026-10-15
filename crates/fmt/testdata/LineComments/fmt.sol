contract LineComments {
    mapping(
        address account // Account key.
            => uint256 balance
    ) named;
    mapping(
        address // Unnamed key.
            => uint256
    ) unnamed;
    mapping(
        address // Value type.
            => uint256
    ) afterArrow;
    mapping(
        address
            => mapping(
            uint256 id // Nested key.
                => uint256
        )
    ) nested;
    mapping(address /* Block key. */ => uint256) blockComment;

    function tuples() external pure returns (uint256 x, uint256 y) {
        (
            // Omitted first result.
            ,
            x
        ) = (1, 2);
        (
            x,
            // Omitted middle result.
            ,
            y
        ) = (1, 2, 3);
        (
            x,
            // Consecutive omissions.
            ,,
            y
        ) = (1, 2, 3, 4);
        (
            x,
            // Omitted last result.
        ) = (1, 2);
        (/* Block omission. */, x) = (1, 2);
    }
}
