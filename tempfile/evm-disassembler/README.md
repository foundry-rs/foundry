# Disassemble EVM bytecode
Lightweight library with the sole purpose of decoding evm bytecode into individual Opcodes and formatting them in a human readable way

# Inspiration
This library was inspired by the [pyevmasm](https://github.com/crytic/pyevmasm). When formatting the decoded operations using the inbuilt function the output should be equivalent to that of `pyevasm`, which is tested on the bytecode of several large evm contracts.

# Installation
`cargo add evm-disassembler`

# Documentation
See the API reference [here](https://docs.rs/evm-disassembler/).

# Example
 ```rust
 use evm_disassembler::{disassemble_str, disassemble_bytes, format_operations};
 
 fn main() {
    
   let bytecode = "608060405260043610603f57600035";
   // Decode from string directly
   let instructions = disassemble_str(bytecode).unwrap();
   println!("{}", format_operations(instructions));

   let bytes = hex::decode(bytecode).unwrap();
   // Decode from Vec<u8> with identical output as above
   let instructions_from_bytes = disassemble_bytes(bytes).unwrap();
   println!("{}", format_operations(instructions_from_bytes));

 }
 ```

# Tests
You can run the tests as usual with `cargo test`.
The main tests compare the output of this library when decoding contract bytecode against the output from `pyevasm`. The input and reference files for these tests are saved in `testdata`. 
To generate new testdata for these tests you can run the `generate_testdata.sh` script with an array of ethereum mainnet addresses. (Requires prior installation of [`foundry`](https://book.getfoundry.sh/) and `pyevasm`).










