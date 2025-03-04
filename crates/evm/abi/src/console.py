#!/usr/bin/env python3
# Generates the JSON ABI for console.sol.

import json
import re
import subprocess
import sys


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <console.sol> <Console.json>")
        sys.exit(1)
    [console_file, abi_file] = sys.argv[1:3]

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


if __name__ == "__main__":
    main()
