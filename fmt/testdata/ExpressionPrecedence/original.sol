function test() {
    uint256 expr001 = (1 + 2) + 3;
    uint256 expr002 = 1 + (2 + 3);
    uint256 expr003 = 1 * 2 + 3;
    uint256 expr004 = (1 * 2) + 3;
    uint256 expr005 = 1 * (2 + 3);
    uint256 expr006 = 1 + 2 * 3;
    uint256 expr007 = (1 + 2) * 3;
    uint256 expr008 = 1 + (2 * 3);
    uint256 expr009 = 1 ** 2 ** 3;
    uint256 expr010 = 1 ** (2 ** 3);
    uint256 expr011 = (1 ** 2) ** 3;
}
