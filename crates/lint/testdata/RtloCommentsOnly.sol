// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Bidi chars that only appear in comments (outside any item) must still be
// reported.

// hidden‮ payload ‬ trailing
//~^WARN: U+202E (Right-to-Left Override) detected
//~|WARN: U+202C (Pop Directional Formatting) detected

/* block‮ comment ‬ end */
//~^WARN: U+202E (Right-to-Left Override) detected
//~|WARN: U+202C (Pop Directional Formatting) detected

contract RtloCommentsOnly {}
