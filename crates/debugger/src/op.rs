use alloy_primitives::Bytes;
use revm::interpreter::opcode;

/// Named parameter of an EVM opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpcodeParam {
    /// The name of the parameter.
    pub(crate) name: &'static str,
    /// The index of the parameter on the stack. This is relative to the top of the stack.
    pub(crate) index: usize,
}

impl OpcodeParam {
    /// Returns the list of named parameters for the given opcode, accounts for special opcodes
    /// requiring immediate bytes to determine stack items.
    #[inline]
    pub(crate) fn of(op: u8, immediate: Option<&Bytes>) -> Option<Vec<Self>> {
        match op {
            // Handle special cases requiring immediate bytes
            opcode::DUPN => immediate
                .and_then(|i| i.first().copied())
                .map(|i| vec![Self { name: "dup_value", index: i as usize }]),
            opcode::SWAPN => immediate.and_then(|i| {
                i.first().map(|i| {
                    vec![
                        Self { name: "a", index: 1 },
                        Self { name: "swap_value", index: *i as usize },
                    ]
                })
            }),
            opcode::EXCHANGE => immediate.and_then(|i| {
                i.first().map(|imm| {
                    let n = (imm >> 4) + 1;
                    let m = (imm & 0xf) + 1;
                    vec![
                        Self { name: "value1", index: n as usize },
                        Self { name: "value2", index: m as usize },
                    ]
                })
            }),
            _ => Some(MAP[op as usize].to_vec()),
        }
    }
}

static MAP: [&[OpcodeParam]; 256] = {
    let mut table = [[].as_slice(); 256];
    let mut i = 0;
    while i < 256 {
        table[i] = map_opcode(i as u8);
        i += 1;
    }
    table
};

const fn map_opcode(op: u8) -> &'static [OpcodeParam] {
    macro_rules! map {
        ($($op:literal($($idx:literal : $name:literal),* $(,)?)),* $(,)?) => {
            match op {
                $($op => &[
                    $(OpcodeParam {
                        name: $name,
                        index: $idx,
                    }),*
                ]),*
            }
        };
    }

    // https://www.evm.codes
    // https://github.com/smlxl/evm.codes
    // https://github.com/klkvr/evm.codes
    // https://github.com/klkvr/evm.codes/blob/HEAD/opcodes.json
    // jq -rf opcodes.jq opcodes.json
    /*
    def mkargs(input):
        input | split(" | ") | to_entries | map("\(.key): \"\(.value)\"") | join(", ");

    to_entries[] | "0x\(.key)(\(mkargs(.value.input))),"
    */
    map! {
        0x00(),
        0x01(0: "a", 1: "b"),
        0x02(0: "a", 1: "b"),
        0x03(0: "a", 1: "b"),
        0x04(0: "a", 1: "b"),
        0x05(0: "a", 1: "b"),
        0x06(0: "a", 1: "b"),
        0x07(0: "a", 1: "b"),
        0x08(0: "a", 1: "b", 2: "N"),
        0x09(0: "a", 1: "b", 2: "N"),
        0x0a(0: "a", 1: "exponent"),
        0x0b(0: "b", 1: "x"),
        0x0c(),
        0x0d(),
        0x0e(),
        0x0f(),
        0x10(0: "a", 1: "b"),
        0x11(0: "a", 1: "b"),
        0x12(0: "a", 1: "b"),
        0x13(0: "a", 1: "b"),
        0x14(0: "a", 1: "b"),
        0x15(0: "a"),
        0x16(0: "a", 1: "b"),
        0x17(0: "a", 1: "b"),
        0x18(0: "a", 1: "b"),
        0x19(0: "a"),
        0x1a(0: "i", 1: "x"),
        0x1b(0: "shift", 1: "value"),
        0x1c(0: "shift", 1: "value"),
        0x1d(0: "shift", 1: "value"),
        0x1e(),
        0x1f(),
        0x20(0: "offset", 1: "size"),
        0x21(),
        0x22(),
        0x23(),
        0x24(),
        0x25(),
        0x26(),
        0x27(),
        0x28(),
        0x29(),
        0x2a(),
        0x2b(),
        0x2c(),
        0x2d(),
        0x2e(),
        0x2f(),
        0x30(),
        0x31(0: "address"),
        0x32(),
        0x33(),
        0x34(),
        0x35(0: "i"),
        0x36(),
        0x37(0: "destOffset", 1: "offset", 2: "size"),
        0x38(),
        0x39(0: "destOffset", 1: "offset", 2: "size"),
        0x3a(),
        0x3b(0: "address"),
        0x3c(0: "address", 1: "destOffset", 2: "offset", 3: "size"),
        0x3d(),
        0x3e(0: "destOffset", 1: "offset", 2: "size"),
        0x3f(0: "address"),
        0x40(0: "blockNumber"),
        0x41(),
        0x42(),
        0x43(),
        0x44(),
        0x45(),
        0x46(),
        0x47(),
        0x48(),
        0x49(),
        0x4a(),
        0x4b(),
        0x4c(),
        0x4d(),
        0x4e(),
        0x4f(),
        0x50(0: "y"),
        0x51(0: "offset"),
        0x52(0: "offset", 1: "value"),
        0x53(0: "offset", 1: "value"),
        0x54(0: "key"),
        0x55(0: "key", 1: "value"),
        0x56(0: "counter"),
        0x57(0: "counter", 1: "b"),
        0x58(),
        0x59(),
        0x5a(),
        0x5b(),
        0x5c(),
        0x5d(),
        0x5e(),

        // PUSHN
        0x5f(),
        0x60(),
        0x61(),
        0x62(),
        0x63(),
        0x64(),
        0x65(),
        0x66(),
        0x67(),
        0x68(),
        0x69(),
        0x6a(),
        0x6b(),
        0x6c(),
        0x6d(),
        0x6e(),
        0x6f(),
        0x70(),
        0x71(),
        0x72(),
        0x73(),
        0x74(),
        0x75(),
        0x76(),
        0x77(),
        0x78(),
        0x79(),
        0x7a(),
        0x7b(),
        0x7c(),
        0x7d(),
        0x7e(),
        0x7f(),

        // DUPN
        0x80(0x00: "dup_value"),
        0x81(0x01: "dup_value"),
        0x82(0x02: "dup_value"),
        0x83(0x03: "dup_value"),
        0x84(0x04: "dup_value"),
        0x85(0x05: "dup_value"),
        0x86(0x06: "dup_value"),
        0x87(0x07: "dup_value"),
        0x88(0x08: "dup_value"),
        0x89(0x09: "dup_value"),
        0x8a(0x0a: "dup_value"),
        0x8b(0x0b: "dup_value"),
        0x8c(0x0c: "dup_value"),
        0x8d(0x0d: "dup_value"),
        0x8e(0x0e: "dup_value"),
        0x8f(0x0f: "dup_value"),

        // SWAPN
        0x90(0: "a", 0x01: "swap_value"),
        0x91(0: "a", 0x02: "swap_value"),
        0x92(0: "a", 0x03: "swap_value"),
        0x93(0: "a", 0x04: "swap_value"),
        0x94(0: "a", 0x05: "swap_value"),
        0x95(0: "a", 0x06: "swap_value"),
        0x96(0: "a", 0x07: "swap_value"),
        0x97(0: "a", 0x08: "swap_value"),
        0x98(0: "a", 0x09: "swap_value"),
        0x99(0: "a", 0x0a: "swap_value"),
        0x9a(0: "a", 0x0b: "swap_value"),
        0x9b(0: "a", 0x0c: "swap_value"),
        0x9c(0: "a", 0x0d: "swap_value"),
        0x9d(0: "a", 0x0e: "swap_value"),
        0x9e(0: "a", 0x0f: "swap_value"),
        0x9f(0: "a", 0x10: "swap_value"),

        0xa0(0: "offset", 1: "size"),
        0xa1(0: "offset", 1: "size", 2: "topic"),
        0xa2(0: "offset", 1: "size", 2: "topic1", 3: "topic2"),
        0xa3(0: "offset", 1: "size", 2: "topic1", 3: "topic2", 4: "topic3"),
        0xa4(0: "offset", 1: "size", 2: "topic1", 3: "topic2", 4: "topic3", 5: "topic4"),
        0xa5(),
        0xa6(),
        0xa7(),
        0xa8(),
        0xa9(),
        0xaa(),
        0xab(),
        0xac(),
        0xad(),
        0xae(),
        0xaf(),
        0xb0(),
        0xb1(),
        0xb2(),
        0xb3(),
        0xb4(),
        0xb5(),
        0xb6(),
        0xb7(),
        0xb8(),
        0xb9(),
        0xba(),
        0xbb(),
        0xbc(),
        0xbd(),
        0xbe(),
        0xbf(),
        0xc0(),
        0xc1(),
        0xc2(),
        0xc3(),
        0xc4(),
        0xc5(),
        0xc6(),
        0xc7(),
        0xc8(),
        0xc9(),
        0xca(),
        0xcb(),
        0xcc(),
        0xcd(),
        0xce(),
        0xcf(),
        0xd0(0: "offset"),
        0xd1(),
        0xd2(),
        0xd3(0: "memOffset", 1: "offset", 2: "size"),
        0xd4(),
        0xd5(),
        0xd6(),
        0xd7(),
        0xd8(),
        0xd9(),
        0xda(),
        0xdb(),
        0xdc(),
        0xdd(),
        0xde(),
        0xdf(),
        0xe0(),
        0xe1(0: "condition"),
        0xe2(0: "case"),
        0xe3(),
        0xe4(),
        0xe5(),
        0xe6(),
        0xe7(),
        0xe8(),
        0xe9(),
        0xea(),
        0xeb(),
        0xec(0: "value", 1: "salt", 2: "offset", 3: "size"),
        0xed(),
        0xee(0: "offset", 1: "size"),
        0xef(),
        0xf0(0: "value", 1: "offset", 2: "size"),
        0xf1(0: "gas", 1: "address", 2: "value", 3: "argsOffset", 4: "argsSize", 5: "retOffset", 6: "retSize"),
        0xf2(0: "gas", 1: "address", 2: "value", 3: "argsOffset", 4: "argsSize", 5: "retOffset", 6: "retSize"),
        0xf3(0: "offset", 1: "size"),
        0xf4(0: "gas", 1: "address", 2: "argsOffset", 3: "argsSize", 4: "retOffset", 5: "retSize"),
        0xf5(0: "value", 1: "offset", 2: "size", 3: "salt"),
        0xf6(),
        0xf7(0: "offset"),
        0xf8(0: "address", 1: "argsOffset", 2: "argsSize", 3: "value"),
        0xf9(0: "address", 1: "argsOffset", 2: "argsSize"),
        0xfa(0: "gas", 1: "address", 2: "argsOffset", 3: "argsSize", 4: "retOffset", 5: "retSize"),
        0xfb(0: "address", 1: "argsOffset", 2: "argsSize"),
        0xfc(),
        0xfd(0: "offset", 1: "size"),
        0xfe(),
        0xff(0: "address"),
    }
}
