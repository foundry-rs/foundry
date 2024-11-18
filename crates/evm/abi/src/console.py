#!/usr/bin/env python3

import json
import re
import subprocess
import sys


def main():
    if len(sys.argv) < 4:
        print(
            f"Usage: {sys.argv[0]} <console.sol> <HardhatConsole.abi.json> <patches.rs>"
        )
        sys.exit(1)
    [console_file, abi_file, patches_file] = sys.argv[1:4]

    # Parse signatures from `console.sol`'s string literals
    console_sol = open(console_file).read()
    sig_strings = re.findall(
        r'"(log.*?)"',
        console_sol,
    )
    raw_sigs = [s.strip().strip('"') for s in sig_strings]
    sigs = [
        s.replace("string", "string memory").replace("bytes)", "bytes memory)")
        for s in raw_sigs
    ]
    sigs = list(set(sigs))

    # Get HardhatConsole ABI
    s = "interface HardhatConsole{\n"
    for sig in sigs:
        s += f"function {sig} external pure;\n"
    s += "\n}"
    r = subprocess.run(
        ["solc", "-", "--combined-json", "abi"],
        input=s.encode("utf8"),
        capture_output=True,
    )
    combined = json.loads(r.stdout.strip())
    abi = combined["contracts"]["<stdin>:HardhatConsole"]["abi"]
    open(abi_file, "w").write(json.dumps(abi, separators=(",", ":"), indent=None))

    # Make patches
    patches = []
    for raw_sig in raw_sigs:
        patched = raw_sig.replace("int", "int256")
        if raw_sig != patched:
            patches.append([raw_sig, patched])

    # Generate the Rust patches map
    codegen = "[\n"
    for [original, patched] in patches:
        codegen += f"    // `{original}` -> `{patched}`\n"

        original_selector = selector(original)
        patched_selector = selector(patched)
        codegen += f"    // `{original_selector.hex()}` -> `{patched_selector.hex()}`\n"

        codegen += (
            f"    ({list(iter(original_selector))}, {list(iter(patched_selector))}),\n"
        )
    codegen += "]\n"
    open(patches_file, "w").write(codegen)


def keccak256(s):
    r = subprocess.run(["cast", "keccak256", s], capture_output=True)
    return bytes.fromhex(r.stdout.decode("utf8").strip()[2:])


def selector(s):
    return keccak256(s)[:4]


if __name__ == "__main__":
    main()
