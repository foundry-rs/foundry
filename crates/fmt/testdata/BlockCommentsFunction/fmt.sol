contract A {
    Counter public counter;
    /**
     *  TODO: this fuzz use too much time to execute
     * function testGetFuzz(bytes[2][] memory kvs) public {
     *     for (uint256 i = 0; i < kvs.length; i++) {
     *         bytes32 root = trie.update(kvs[i][0], kvs[i][1]);
     *         console.logBytes32(root);
     *     }
     *
     *     for (uint256 i = 0; i < kvs.length; i++) {
     *         (bool exist, bytes memory value) = trie.get(kvs[i][0]);
     *         console.logBool(exist);
     *         console.logBytes(value);
     *         require(exist);
     *         require(BytesSlice.equal(value, trie.getRaw(kvs[i][0])));
     *     }
     * }
     */
}
