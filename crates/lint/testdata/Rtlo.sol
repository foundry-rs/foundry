// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for the `rtlo` lint, which detects "Trojan Source" bidirectional
// formatting characters (CVE-2021-42574). These have no legitimate use in
// Solidity source and can be used to hide malicious code.
//
// Note: solc itself rejects unbalanced directional override markers (error
// 8936), so each test uses a balanced opening/closing pair. Our lint flags
// each occurrence individually regardless of balance.

contract Rtlo {
    // SHOULD FAIL: every codepoint in the Trojan-Source set is flagged.
    // Each line below contains two bidi characters (an opener and its closer)
    // and produces two diagnostics.

    string public lre = unicode"‪_‬";
    //~^WARN: U+202A (Left-to-Right Embedding) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    string public rle = unicode"‫_‬";
    //~^WARN: U+202B (Right-to-Left Embedding) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    string public pdf = unicode"‪‬";
    //~^WARN: U+202A (Left-to-Right Embedding) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    string public lro = unicode"‭_‬";
    //~^WARN: U+202D (Left-to-Right Override) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    string public rlo = unicode"‮_‬";
    //~^WARN: U+202E (Right-to-Left Override) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    string public lri = unicode"⁦_⁩";
    //~^WARN: U+2066 (Left-to-Right Isolate) detected
    //~|WARN: U+2069 (Pop Directional Isolate) detected

    string public rli = unicode"⁧_⁩";
    //~^WARN: U+2067 (Right-to-Left Isolate) detected
    //~|WARN: U+2069 (Pop Directional Isolate) detected

    string public fsi = unicode"⁨_⁩";
    //~^WARN: U+2068 (First Strong Isolate) detected
    //~|WARN: U+2069 (Pop Directional Isolate) detected

    string public pdi = unicode"⁦⁩";
    //~^WARN: U+2066 (Left-to-Right Isolate) detected
    //~|WARN: U+2069 (Pop Directional Isolate) detected

    // SHOULD FAIL: bidi controls inside a block comment are also detected.
    /* hidden‮ /* text ‬ */ uint256 inBlockComment;
    //~^WARN: U+202E (Right-to-Left Override) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    // SHOULD FAIL: bidi controls inside a line comment are also detected. The
    // expectation markers must come on separate lines because the ui-test
    // parser only treats the first comment on a line as a marker.
    // sneaky‮ payload ‬ trailing
    //~^WARN: U+202E (Right-to-Left Override) detected
    //~|WARN: U+202C (Pop Directional Formatting) detected

    // SHOULD PASS: inline-config disable suppresses the diagnostic.
    // forge-lint: disable-next-line(rtlo)
    string public suppressedLine = unicode"‮_‬";

    // forge-lint: disable-start(rtlo)
    string public suppressedA = unicode"‮_‬";
    string public suppressedB = unicode"⁦_⁩";
    // forge-lint: disable-end(rtlo)

    // SHOULD PASS: plain ASCII source, no bidi controls.
    string public clean = "no bidi here";

    // SHOULD FAIL: LRM/RLM marks (U+200E/U+200F) are also flagged.
    string public marks = unicode"left‎right‏end";
    //~^WARN: U+200E (Left-to-Right Mark) detected
    //~|WARN: U+200F (Right-to-Left Mark) detected
}
