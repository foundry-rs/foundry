contract Contract0 {
    function arithmetic() public {
        (1 / 2) * 3; // Unsafe
        (1 * 2) / 3; // Safe
        (1 / 2) * 3; // Unsafe
        (1 * 2) / 3; // Safe
        ((1 / 2) * 3) * 4; // Unsafe
        ((1 * 2) / 3) * 4; // Unsafe
        (1 / 2 / 3) * 4; // Unsafe
        (1 / (2 + 3)) * 4; // Unsafe
        (1 / 2 + 3) * 4; // Safe
        (1 / 2 - 3) * 4; // Safe
        (1 + 2 / 3) * 4; // Safe
        (1 / 2 - 3) * 4; // Safe
        ((1 / 2) % 3) * 4; // Safe
        1 / (2 * 3 + 3); // Safe
        1 / ((2 / 3) * 3); // Unsafe
        1 / ((2 * 3) + 3); // Safe

        // uint256 x = 5;
        // x /= 2 * 3; // Unsafe
        // x /= (2 * 3); // Unsafe
        // x /= 2 * 3 - 4; // Unsafe
        // x /= (2 * 3) % 4; // Unsafe
        // x /= (2 * 3) | 4; // Unsafe
        // x /= (2 * 3) & 4; // Unsafe
        // x /= (2 * 3) ^ 4; // Unsafe
        // x /= (2 * 3) << 4; // Unsafe
        // x /= (2 * 3) >> 4; // Unsafe
        // x /= 3 % 4; // Safe
        // x /= 3 | 4; // Safe
        // x /= 3 & 4; // Safe
        // x /= 3 ^ 4; // Safe
        // x /= 3 << 4; // Safe
        // x /= 3 >> 4; // Safe
    }
}
