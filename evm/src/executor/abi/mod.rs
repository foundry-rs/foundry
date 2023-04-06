use ethers::types::{Address, Selector, H160};
use once_cell::sync::Lazy;
use std::collections::HashMap;

/// The cheatcode handler address (0x7109709ECfa91a80626fF3989D68f67F5b1DD12D).
///
/// This is the same address as the one used in DappTools's HEVM.
/// `address(bytes20(uint160(uint256(keccak256('hevm cheat code')))))`
pub const CHEATCODE_ADDRESS: Address = H160([
    0x71, 0x09, 0x70, 0x9E, 0xcf, 0xa9, 0x1a, 0x80, 0x62, 0x6f, 0xf3, 0x98, 0x9d, 0x68, 0xf6, 0x7f,
    0x5b, 0x1d, 0xd1, 0x2d,
]);

// Bindings for cheatcodes
ethers::contract::abigen!(
    HEVM,
    r#"[
            struct Log {bytes32[] topics; bytes data;}
            struct Rpc {string name; string url;}
            struct FsMetadata {bool isDir; bool isSymlink; uint256 length; bool readOnly; uint256 modified; uint256 accessed; uint256 created;}
            roll(uint256)
            warp(uint256)
            difficulty(uint256)
            fee(uint256)
            coinbase(address)
            store(address,bytes32,bytes32)
            load(address,bytes32)(bytes32)
            ffi(string[])(bytes)
            setEnv(string,string)
            envBool(string)(bool)
            envUint(string)(uint256)
            envInt(string)(int256)
            envAddress(string)(address)
            envBytes32(string)(bytes32)
            envString(string)(string)
            envBytes(string)(bytes)
            envBool(string,string)(bool[])
            envUint(string,string)(uint256[])
            envInt(string,string)(int256[])
            envAddress(string,string)(address[])
            envBytes32(string,string)(bytes32[])
            envString(string,string)(string[])
            envBytes(string,string)(bytes[])
            envOr(string,bool)(bool)
            envOr(string,uint256)(uint256)
            envOr(string,int256)(int256)
            envOr(string,address)(address)
            envOr(string,bytes32)(bytes32)
            envOr(string,string)(string)
            envOr(string,bytes)(bytes)
            envOr(string,string,bool[])(bool[])
            envOr(string,string,uint256[])(uint256[])
            envOr(string,string,int256[])(int256[])
            envOr(string,string,address[])(address[])
            envOr(string,string,bytes32[])(bytes32[])
            envOr(string,string,string[])(string[])
            envOr(string,string,bytes[])(bytes[])
            addr(uint256)(address)
            sign(uint256,bytes32)(uint8,bytes32,bytes32)
            deriveKey(string,uint32)(uint256)
            deriveKey(string,string,uint32)(uint256)
            rememberKey(uint256)(address)
            prank(address)
            startPrank(address)
            prank(address,address)
            startPrank(address,address)
            stopPrank()
            deal(address,uint256)
            etch(address,bytes)
            expectRevert()
            expectRevert(bytes)
            expectRevert(bytes4)
            record()
            accesses(address)(bytes32[],bytes32[])
            recordLogs()
            getRecordedLogs()(Log[])
            expectEmit()
            expectEmit(address)
            expectEmit(bool,bool,bool,bool)
            expectEmit(bool,bool,bool,bool,address)
            mockCall(address,bytes,bytes)
            mockCall(address,uint256,bytes,bytes)
            mockCallRevert(address,bytes,bytes)
            mockCallRevert(address,uint256,bytes,bytes)
            clearMockedCalls()
            expectCall(address,bytes)
            expectCall(address,uint256,bytes)
            expectCall(address,uint256,uint64,bytes)
            expectCallMinGas(address,uint256,uint64,bytes)
            expectSafeMemory(uint64,uint64)
            expectSafeMemoryCall(uint64,uint64)
            getCode(string)
            getDeployedCode(string)
            label(address,string)
            assume(bool)
            setNonce(address,uint64)
            getNonce(address)
            chainId(uint256)
            txGasPrice(uint256)
            broadcast()
            broadcast(address)
            broadcast(uint256)
            startBroadcast()
            startBroadcast(address)
            startBroadcast(uint256)
            stopBroadcast()
            projectRoot()(string)
            readFile(string)(string)
            readFileBinary(string)(bytes)
            writeFile(string,string)
            writeFileBinary(string,bytes)
            openFile(string)
            readLine(string)(string)
            writeLine(string,string)
            closeFile(string)
            removeFile(string)
            fsMetadata(string)(FsMetadata)
            toString(bytes)
            toString(address)
            toString(uint256)
            toString(int256)
            toString(bytes32)
            toString(bool)
            parseBytes(string)(bytes)
            parseAddress(string)(address)
            parseUint(string)(uint256)
            parseInt(string)(int256)
            parseBytes32(string)(bytes32)
            parseBool(string)(bool)
            snapshot()(uint256)
            revertTo(uint256)(bool)
            createFork(string,uint256)(uint256)
            createFork(string,bytes32)(uint256)
            createFork(string)(uint256)
            createSelectFork(string,uint256)(uint256)
            createSelectFork(string,bytes32)(uint256)
            createSelectFork(string)(uint256)
            selectFork(uint256)
            activeFork()(uint256)
            transact(bytes32)
            transact(uint256,bytes32)
            makePersistent(address)
            makePersistent(address,address)
            makePersistent(address,address,address)
            makePersistent(address[])
            revokePersistent(address)
            revokePersistent(address[])
            isPersistent(address)(bool)
            rollFork(uint256)
            rollFork(bytes32)
            rollFork(uint256,uint256)
            rollFork(uint256,bytes32)
            rpcUrl(string)(string)
            rpcUrls()(string[2][])
            rpcUrlStructs()(Rpc[])
            parseJson(string)(bytes)
            parseJson(string, string)(bytes)
            parseJsonUint(string, string)(uint256)
            parseJsonUintArray(string, string)(uint256[])
            parseJsonInt(string, string)(int256)
            parseJsonIntArray(string, string)(int256[])
            parseJsonString(string, string)(string)
            parseJsonStringArray(string, string)(string[])
            parseJsonAddress(string, string)(address)
            parseJsonAddressArray(string, string)(address[])
            parseJsonBool(string, string)(bool)
            parseJsonBoolArray(string, string)(bool[])
            parseJsonBytes(string, string)(bytes)
            parseJsonBytesArray(string, string)(bytes[])
            parseJsonBytes32(string, string)(bytes32)
            parseJsonBytes32Array(string, string)(bytes32[])
            allowCheatcodes(address)
            serializeBool(string,string,bool)(string)
            serializeBool(string,string,bool[])(string)
            serializeUint(string,string,uint256)(string)
            serializeUint(string,string,uint256[])(string)
            serializeInt(string,string,int256)(string)
            serializeInt(string,string,int256[])(string)
            serializeAddress(string,string,address)(string)
            serializeAddress(string,string,address[])(string)
            serializeBytes32(string,string,bytes32)(string)
            serializeBytes32(string,string,bytes32[])(string)
            serializeString(string,string,string)(string)
            serializeString(string,string,string[])(string)
            serializeBytes(string,string,bytes)(string)
            serializeBytes(string,string,bytes[])(string)
            writeJson(string, string)
            writeJson(string, string, string)
            pauseGasMetering()
            resumeGasMetering()
            startMappingRecording()
            getMappingLength(address,bytes32)
            getMappingSlotAt(address,bytes32,uint256)
            getMappingKeyOf(address,bytes32)
            getMappingParentOf(address,bytes32)
    ]"#,
);
pub use hevm::{HEVMCalls, HEVM_ABI};

/// The Hardhat console address (0x000000000000000000636F6e736F6c652e6c6f67).
///
/// See: https://github.com/nomiclabs/hardhat/blob/master/packages/hardhat-core/console.sol
pub static HARDHAT_CONSOLE_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x63, 0x6f, 0x6e, 0x73, 0x6f, 0x6c, 0x65,
    0x2e, 0x6c, 0x6f, 0x67,
]);

// Bindings for DS-style event logs. Note that the array logs below are not actually part of DSTest,
// but are part of forge-std, so are included here to ensure they are decoded in output logs.
ethers::contract::abigen!(
    Console,
    r#"[
            event log(string)
            event logs                   (bytes)
            event log_address            (address)
            event log_bytes32            (bytes32)
            event log_int                (int)
            event log_uint               (uint)
            event log_bytes              (bytes)
            event log_string             (string)
            event log_array              (uint256[] val)
            event log_array              (int256[] val)
            event log_array              (address[] val)
            event log_named_address      (string key, address val)
            event log_named_bytes32      (string key, bytes32 val)
            event log_named_decimal_int  (string key, int val, uint decimals)
            event log_named_decimal_uint (string key, uint val, uint decimals)
            event log_named_int          (string key, int val)
            event log_named_uint         (string key, uint val)
            event log_named_bytes        (string key, bytes val)
            event log_named_string       (string key, string val)
            event log_named_array        (string key, uint256[] val)
            event log_named_array        (string key, int256[] val)
            event log_named_array        (string key, address[] val)
    ]"#,
);
pub use console::{ConsoleEvents, CONSOLE_ABI};

// Bindings for Hardhat console
ethers::contract::abigen!(HardhatConsole, "./abi/console.json", event_derives (foundry_macros::ConsoleFmt););
pub use hardhat_console::HARDHATCONSOLE_ABI as HARDHAT_CONSOLE_ABI;

/// If the input starts with a known `hardhat/console.log` `uint` selector, then this will replace
/// it with the selector `abigen!` bindings expect.
pub fn patch_hardhat_console_selector(mut input: Vec<u8>) -> Vec<u8> {
    if input.len() < 4 {
        return input
    }

    let selector = Selector::try_from(&input[..4]).unwrap();
    if let Some(abigen_selector) = HARDHAT_CONSOLE_SELECTOR_PATCHES.get(&selector) {
        input.splice(..4, *abigen_selector);
    }
    input
}

/// This contains a map with all the  `hardhat/console.log` log selectors that use `uint` or `int`
/// as key and the selector of the call with `uint256`,
///
/// This is a bit terrible but a workaround for the differing selectors used by hardhat and the call
/// bindings which `abigen!` creates. `hardhat/console.log` logs its events in functions that accept
/// `uint` manually as `abi.encodeWithSignature("log(int)", p0)`, but `abigen!` uses `uint256` for
/// its call bindings (`HardhatConsoleCalls`) as generated by solc.
pub static HARDHAT_CONSOLE_SELECTOR_PATCHES: Lazy<HashMap<Selector, Selector>> = Lazy::new(|| {
    HashMap::from([
        // log(bool,uint256,uint256,address)
        ([241, 97, 178, 33], [0, 221, 135, 185]),
        // log(uint256,address,address,string)
        ([121, 67, 220, 102], [3, 28, 111, 115]),
        // log(uint256,bool,address,uint256)
        ([65, 181, 239, 59], [7, 130, 135, 245]),
        // log(bool,address,bool,uint256)
        ([76, 182, 15, 209], [7, 131, 21, 2]),
        // log(bool,uint256,address)
        ([196, 210, 53, 7], [8, 142, 249, 210]),
        // log(uint256,address,address,bool)
        ([1, 85, 11, 4], [9, 31, 250, 245]),
        // log(address,bool,uint256,string)
        ([155, 88, 142, 204], [10, 166, 207, 173]),
        // log(bool,bool,uint256,uint256)
        ([70, 103, 222, 142], [11, 176, 14, 171]),
        // log(bool,address,address,uint256)
        ([82, 132, 189, 108], [12, 102, 209, 190]),
        // log(uint256,address,uint256,uint256)
        ([202, 154, 62, 180], [12, 156, 217, 193]),
        // log(string,address,uint256)
        ([7, 200, 18, 23], [13, 38, 185, 37]),
        // log(address,string,uint256,bool)
        ([126, 37, 13, 91], [14, 247, 224, 80]),
        // log(address,uint256,address,uint256)
        ([165, 217, 135, 104], [16, 15, 101, 14]),
        // log(string,string,uint256,address)
        ([93, 79, 70, 128], [16, 35, 247, 178]),
        // log(bool,string,uint256)
        ([192, 56, 42, 172], [16, 147, 238, 17]),
        // log(bool,bool,uint256)
        ([176, 19, 101, 187], [18, 242, 22, 2]),
        // log(bool,address,uint256,address)
        ([104, 241, 88, 181], [19, 107, 5, 221]),
        // log(bool,uint256,address,uint256)
        ([202, 165, 35, 106], [21, 55, 220, 135]),
        // log(bool,string,uint256,address)
        ([91, 34, 185, 56], [21, 150, 161, 206]),
        // log(address,string,string,uint256)
        ([161, 79, 208, 57], [21, 159, 137, 39]),
        // log(uint256,address,uint256,address)
        ([253, 178, 236, 212], [21, 193, 39, 181]),
        // log(uint256,uint256,address,bool)
        ([168, 232, 32, 174], [21, 202, 196, 118]),
        // log(bool,string,bool,uint256)
        ([141, 111, 156, 165], [22, 6, 163, 147]),
        // log(address,address,uint256)
        ([108, 54, 109, 114], [23, 254, 97, 133]),
        // log(uint256,uint256,uint256,uint256)
        ([92, 160, 173, 62], [25, 63, 184, 0]),
        // log(bool,string,uint256,string)
        ([119, 161, 171, 237], [26, 217, 109, 230]),
        // log(bool,uint256,address,string)
        ([24, 9, 19, 65], [27, 179, 176, 154]),
        // log(string,uint256,address)
        ([227, 132, 159, 121], [28, 126, 196, 72]),
        // log(uint256,bool)
        ([30, 109, 212, 236], [28, 157, 126, 179]),
        // log(address,uint256,address,string)
        ([93, 113, 243, 158], [29, 169, 134, 234]),
        // log(address,string,uint256,uint256)
        ([164, 201, 42, 96], [29, 200, 225, 184]),
        // log(uint256,bool,uint256)
        ([90, 77, 153, 34], [32, 9, 128, 20]),
        // log(uint256,bool,bool)
        ([213, 206, 172, 224], [32, 113, 134, 80]),
        // log(address,uint256,uint256,address)
        ([30, 246, 52, 52], [32, 227, 152, 77]),
        // log(uint256,string,string,string)
        ([87, 221, 10, 17], [33, 173, 6, 131]),
        // log(address,uint256,bool,uint256)
        ([105, 143, 67, 146], [34, 246, 185, 153]),
        // log(uint256,address,address,address)
        ([85, 71, 69, 249], [36, 136, 180, 20]),
        // log(string,bool,string,uint256)
        ([52, 203, 48, 141], [36, 249, 20, 101]),
        // log(bool,uint256,address,address)
        ([138, 47, 144, 170], [38, 245, 96, 168]),
        // log(uint256,uint256,string,string)
        ([124, 3, 42, 50], [39, 216, 175, 210]),
        // log(bool,string,uint256,uint256)
        ([142, 74, 232, 110], [40, 134, 63, 203]),
        // log(uint256,bool,string,uint256)
        ([145, 95, 219, 40], [44, 29, 7, 70]),
        // log(address,uint256,uint256,uint256)
        ([61, 14, 157, 228], [52, 240, 230, 54]),
        // log(uint256,bool,address)
        ([66, 78, 255, 191], [53, 8, 95, 123]),
        // log(string,uint256,bool,bool)
        ([227, 127, 243, 208], [53, 76, 54, 214]),
        // log(bool,uint256,uint256)
        ([59, 92, 3, 224], [55, 16, 51, 103]),
        // log(bool,uint256,uint256,uint256)
        ([50, 223, 165, 36], [55, 75, 180, 178]),
        // log(uint256,string,uint256)
        ([91, 109, 232, 63], [55, 170, 125, 76]),
        // log(address,bool,uint256,uint256)
        ([194, 16, 160, 30], [56, 111, 245, 244]),
        // log(address,address,bool,uint256)
        ([149, 214, 95, 17], [57, 113, 231, 140]),
        // log(bool,uint256)
        ([54, 75, 106, 146], [57, 145, 116, 211]),
        // log(uint256,string,uint256,address)
        ([171, 123, 217, 253], [59, 34, 121, 180]),
        // log(address,uint256,bool,bool)
        ([254, 161, 213, 90], [59, 245, 229, 55]),
        // log(uint256,address,string,string)
        ([141, 119, 134, 36], [62, 18, 140, 163]),
        // log(string,address,bool,uint256)
        ([197, 209, 187, 139], [62, 159, 134, 106]),
        // log(uint256,uint256,string,address)
        ([67, 50, 133, 162], [66, 210, 29, 183]),
        // log(address,string,uint256,string)
        ([93, 19, 101, 201], [68, 136, 48, 168]),
        // log(uint256,bool,address,bool)
        ([145, 251, 18, 66], [69, 77, 84, 165]),
        // log(address,string,address,uint256)
        ([140, 25, 51, 169], [69, 127, 227, 207]),
        // log(uint256,address,string,uint256)
        ([160, 196, 20, 232], [70, 130, 107, 93]),
        // log(uint256,uint256,bool)
        ([103, 87, 15, 247], [71, 102, 218, 114]),
        // log(address,uint256,address,address)
        ([236, 36, 132, 111], [71, 141, 28, 98]),
        // log(address,uint256,uint256,string)
        ([137, 52, 13, 171], [74, 40, 192, 23]),
        // log(bool,bool,address,uint256)
        ([96, 147, 134, 231], [76, 18, 61, 87]),
        // log(uint256,string,bool)
        ([70, 167, 208, 206], [76, 237, 167, 90]),
        // log(string,uint256,address,uint256)
        ([88, 73, 122, 254], [79, 4, 253, 198]),
        // log(address,string,bool,uint256)
        ([231, 32, 82, 28], [81, 94, 56, 182]),
        // log(bool,address,uint256,string)
        ([160, 104, 88, 51], [81, 240, 159, 248]),
        // log(bool,bool,uint256,address)
        ([11, 255, 149, 13], [84, 167, 169, 160]),
        // log(uint256,uint256,address,address)
        ([202, 147, 155, 32], [86, 165, 209, 177]),
        // log(string,string,uint256)
        ([243, 98, 202, 89], [88, 33, 239, 161]),
        // log(string,uint256,string)
        ([163, 245, 199, 57], [89, 112, 224, 137]),
        // log(uint256,uint256,uint256,string)
        ([120, 173, 122, 12], [89, 207, 203, 227]),
        // log(string,address,uint256,string)
        ([76, 85, 242, 52], [90, 71, 118, 50]),
        // log(uint256,address,uint256)
        ([136, 67, 67, 170], [90, 155, 94, 213]),
        // log(string,uint256,string,string)
        ([108, 152, 218, 226], [90, 184, 78, 31]),
        // log(uint256,address,bool,uint256)
        ([123, 8, 232, 235], [90, 189, 153, 42]),
        // log(address,uint256,string,address)
        ([220, 121, 38, 4], [92, 67, 13, 71]),
        // log(uint256,uint256,address)
        ([190, 51, 73, 27], [92, 150, 179, 49]),
        // log(string,bool,address,uint256)
        ([40, 223, 78, 150], [93, 8, 187, 5]),
        // log(string,string,uint256,string)
        ([141, 20, 44, 221], [93, 26, 151, 26]),
        // log(uint256,uint256,string,uint256)
        ([56, 148, 22, 61], [93, 162, 151, 235]),
        // log(string,uint256,address,address)
        ([234, 200, 146, 129], [94, 162, 183, 174]),
        // log(uint256,address,uint256,bool)
        ([25, 246, 115, 105], [95, 116, 58, 124]),
        // log(bool,address,uint256)
        ([235, 112, 75, 175], [95, 123, 154, 251]),
        // log(uint256,string,address,address)
        ([127, 165, 69, 139], [97, 104, 237, 97]),
        // log(bool,bool,uint256,bool)
        ([171, 92, 193, 196], [97, 158, 77, 14]),
        // log(address,string,uint256,address)
        ([223, 215, 216, 11], [99, 24, 54, 120]),
        // log(uint256,address,string)
        ([206, 131, 4, 123], [99, 203, 65, 249]),
        // log(string,address,uint256,address)
        ([163, 102, 236, 128], [99, 251, 139, 197]),
        // log(uint256,string)
        ([15, 163, 243, 69], [100, 63, 208, 223]),
        // log(string,bool,uint256,uint256)
        ([93, 191, 240, 56], [100, 181, 187, 103]),
        // log(address,uint256,uint256,bool)
        ([236, 75, 168, 162], [102, 241, 188, 103]),
        // log(address,uint256,bool)
        ([229, 74, 225, 68], [103, 130, 9, 168]),
        // log(address,string,uint256)
        ([28, 218, 242, 138], [103, 221, 111, 241]),
        // log(uint256,bool,string,string)
        ([164, 51, 252, 253], [104, 200, 184, 189]),
        // log(uint256,string,uint256,bool)
        ([135, 90, 110, 46], [105, 26, 143, 116]),
        // log(uint256,address)
        ([88, 235, 134, 12], [105, 39, 108, 134]),
        // log(uint256,bool,bool,address)
        ([83, 6, 34, 93], [105, 100, 11, 89]),
        // log(bool,uint256,string,uint256)
        ([65, 128, 1, 27], [106, 17, 153, 226]),
        // log(bool,string,uint256,bool)
        ([32, 187, 201, 175], [107, 14, 93, 83]),
        // log(uint256,uint256,address,string)
        ([214, 162, 209, 222], [108, 222, 64, 184]),
        // log(bool,bool,bool,uint256)
        ([194, 72, 131, 77], [109, 112, 69, 193]),
        // log(uint256,uint256,string)
        ([125, 105, 14, 230], [113, 208, 74, 242]),
        // log(uint256,address,address,uint256)
        ([154, 60, 191, 150], [115, 110, 251, 182]),
        // log(string,bool,uint256,string)
        ([66, 185, 162, 39], [116, 45, 110, 231]),
        // log(uint256,bool,bool,uint256)
        ([189, 37, 173, 89], [116, 100, 206, 35]),
        // log(string,uint256,uint256,bool)
        ([247, 60, 126, 61], [118, 38, 219, 146]),
        // log(uint256,uint256,string,bool)
        ([178, 46, 175, 6], [122, 246, 171, 37]),
        // log(uint256,string,address)
        ([31, 144, 242, 74], [122, 250, 201, 89]),
        // log(address,uint256,address)
        ([151, 236, 163, 148], [123, 192, 216, 72]),
        // log(bool,string,string,uint256)
        ([93, 219, 37, 146], [123, 224, 195, 235]),
        // log(bool,address,uint256,uint256)
        ([155, 254, 114, 188], [123, 241, 129, 161]),
        // log(string,uint256,string,address)
        ([187, 114, 53, 233], [124, 70, 50, 164]),
        // log(string,string,address,uint256)
        ([74, 129, 165, 106], [124, 195, 198, 7]),
        // log(string,uint256,string,bool)
        ([233, 159, 130, 207], [125, 36, 73, 29]),
        // log(bool,bool,uint256,string)
        ([80, 97, 137, 55], [125, 212, 208, 224]),
        // log(bool,uint256,bool,uint256)
        ([211, 222, 85, 147], [127, 155, 188, 162]),
        // log(address,bool,string,uint256)
        ([158, 18, 123, 110], [128, 230, 162, 11]),
        // log(string,uint256,address,bool)
        ([17, 6, 168, 247], [130, 17, 42, 66]),
        // log(uint256,string,uint256,uint256)
        ([192, 4, 56, 7], [130, 194, 91, 116]),
        // log(address,uint256)
        ([34, 67, 207, 163], [131, 9, 232, 168]),
        // log(string,uint256,uint256,string)
        ([165, 78, 212, 189], [133, 75, 52, 150]),
        // log(uint256,bool,string)
        ([139, 14, 20, 254], [133, 119, 80, 33]),
        // log(address,uint256,string,string)
        ([126, 86, 198, 147], [136, 168, 196, 6]),
        // log(uint256,bool,uint256,address)
        ([79, 64, 5, 142], [136, 203, 96, 65]),
        // log(uint256,uint256,address,uint256)
        ([97, 11, 168, 192], [136, 246, 228, 178]),
        // log(string,bool,uint256,bool)
        ([60, 197, 181, 211], [138, 247, 207, 138]),
        // log(address,bool,bool,uint256)
        ([207, 181, 135, 86], [140, 78, 93, 230]),
        // log(address,address,uint256,address)
        ([214, 198, 82, 118], [141, 166, 222, 245]),
        // log(string,bool,bool,uint256)
        ([128, 117, 49, 232], [142, 63, 120, 169]),
        // log(bool,uint256,uint256,string)
        ([218, 6, 102, 200], [142, 105, 251, 93]),
        // log(string,string,string,uint256)
        ([159, 208, 9, 245], [142, 175, 176, 43]),
        // log(string,address,address,uint256)
        ([110, 183, 148, 61], [142, 243, 243, 153]),
        // log(uint256,string,address,bool)
        ([249, 63, 255, 55], [144, 195, 10, 86]),
        // log(uint256,address,bool,string)
        ([99, 240, 226, 66], [144, 251, 6, 170]),
        // log(bool,uint256,bool,string)
        ([182, 213, 105, 212], [145, 67, 219, 177]),
        // log(uint256,bool,uint256,bool)
        ([210, 171, 196, 253], [145, 160, 46, 42]),
        // log(string,address,string,uint256)
        ([143, 98, 75, 233], [145, 209, 17, 46]),
        // log(string,bool,uint256,address)
        ([113, 211, 133, 13], [147, 94, 9, 191]),
        // log(address,address,address,uint256)
        ([237, 94, 172, 135], [148, 37, 13, 119]),
        // log(uint256,uint256,bool,address)
        ([225, 23, 116, 79], [154, 129, 106, 131]),
        // log(bool,uint256,bool,address)
        ([66, 103, 199, 248], [154, 205, 54, 22]),
        // log(address,address,uint256,bool)
        ([194, 246, 136, 236], [155, 66, 84, 226]),
        // log(uint256,address,bool)
        ([122, 208, 18, 142], [155, 110, 192, 66]),
        // log(uint256,string,address,string)
        ([248, 152, 87, 127], [156, 58, 223, 161]),
        // log(address,bool,uint256)
        ([44, 70, 141, 21], [156, 79, 153, 251]),
        // log(uint256,address,string,address)
        ([203, 229, 142, 253], [156, 186, 143, 255]),
        // log(string,uint256,address,string)
        ([50, 84, 194, 232], [159, 251, 47, 147]),
        // log(address,uint256,address,bool)
        ([241, 129, 161, 233], [161, 188, 201, 179]),
        // log(uint256,bool,address,address)
        ([134, 237, 193, 12], [161, 239, 76, 187]),
        // log(address,uint256,string)
        ([186, 249, 104, 73], [161, 242, 232, 170]),
        // log(address,uint256,bool,address)
        ([35, 229, 73, 114], [163, 27, 253, 204]),
        // log(uint256,uint256,bool,string)
        ([239, 217, 203, 238], [165, 180, 252, 153]),
        // log(bool,string,address,uint256)
        ([27, 11, 149, 91], [165, 202, 218, 148]),
        // log(address,bool,address,uint256)
        ([220, 113, 22, 210], [167, 92, 89, 222]),
        // log(string,uint256,uint256,uint256)
        ([8, 238, 86, 102], [167, 168, 120, 83]),
        // log(uint256,uint256,bool,bool)
        ([148, 190, 59, 177], [171, 8, 90, 230]),
        // log(string,uint256,bool,string)
        ([118, 204, 96, 100], [171, 247, 58, 152]),
        // log(uint256,bool,address,string)
        ([162, 48, 118, 30], [173, 224, 82, 199]),
        // log(uint256,string,bool,address)
        ([121, 111, 40, 160], [174, 46, 197, 129]),
        // log(uint256,string,string,uint256)
        ([118, 236, 99, 94], [176, 40, 201, 189]),
        // log(uint256,string,string)
        ([63, 87, 194, 149], [177, 21, 97, 31]),
        // log(uint256,string,string,bool)
        ([18, 134, 43, 152], [179, 166, 182, 189]),
        // log(bool,uint256,address,bool)
        ([101, 173, 244, 8], [180, 195, 20, 255]),
        // log(string,uint256)
        ([151, 16, 169, 208], [182, 14, 114, 204]),
        // log(address,uint256,uint256)
        ([135, 134, 19, 94], [182, 155, 202, 246]),
        // log(uint256,bool,bool,bool)
        ([78, 108, 83, 21], [182, 245, 119, 161]),
        // log(uint256,string,uint256,string)
        ([162, 188, 12, 153], [183, 185, 20, 202]),
        // log(uint256,string,bool,bool)
        ([81, 188, 43, 193], [186, 83, 93, 156]),
        // log(uint256,address,address)
        ([125, 119, 166, 27], [188, 253, 155, 224]),
        // log(address,address,uint256,uint256)
        ([84, 253, 243, 228], [190, 85, 52, 129]),
        // log(bool,uint256,uint256,bool)
        ([164, 29, 129, 222], [190, 152, 67, 83]),
        // log(address,uint256,string,uint256)
        ([245, 18, 207, 155], [191, 1, 248, 145]),
        // log(bool,address,string,uint256)
        ([11, 153, 252, 34], [194, 31, 100, 199]),
        // log(string,string,uint256,bool)
        ([230, 86, 88, 202], [195, 168, 166, 84]),
        // log(bool,uint256,string)
        ([200, 57, 126, 176], [195, 252, 57, 112]),
        // log(address,bool,uint256,bool)
        ([133, 205, 197, 175], [196, 100, 62, 32]),
        // log(uint256,uint256,uint256,bool)
        ([100, 82, 185, 203], [197, 152, 209, 133]),
        // log(address,uint256,bool,string)
        ([142, 142, 78, 117], [197, 173, 133, 249]),
        // log(string,uint256,string,uint256)
        ([160, 196, 178, 37], [198, 126, 169, 209]),
        // log(uint256,bool,uint256,uint256)
        ([86, 130, 141, 164], [198, 172, 199, 168]),
        // log(string,bool,uint256)
        ([41, 27, 185, 208], [201, 89, 88, 214]),
        // log(string,uint256,uint256)
        ([150, 156, 221, 3], [202, 71, 196, 235]),
        // log(string,uint256,bool)
        ([241, 2, 238, 5], [202, 119, 51, 177]),
        // log(uint256,address,string,bool)
        ([34, 164, 121, 166], [204, 50, 171, 7]),
        // log(address,bool,uint256,address)
        ([13, 140, 230, 30], [204, 247, 144, 161]),
        // log(bool,uint256,bool,bool)
        ([158, 1, 247, 65], [206, 181, 244, 215]),
        // log(uint256,string,bool,uint256)
        ([164, 180, 138, 127], [207, 0, 152, 128]),
        // log(address,uint256,string,bool)
        ([164, 2, 79, 17], [207, 24, 16, 92]),
        // log(uint256,uint256,uint256)
        ([231, 130, 10, 116], [209, 237, 122, 60]),
        // log(uint256,string,bool,string)
        ([141, 72, 156, 160], [210, 212, 35, 205]),
        // log(uint256,string,string,address)
        ([204, 152, 138, 160], [213, 131, 198, 2]),
        // log(bool,address,uint256,bool)
        ([238, 141, 134, 114], [214, 1, 159, 28]),
        // log(string,string,bool,uint256)
        ([134, 129, 138, 122], [214, 174, 250, 210]),
        // log(uint256,address,uint256,string)
        ([62, 211, 189, 40], [221, 176, 101, 33]),
        // log(uint256,bool,bool,string)
        ([49, 138, 229, 155], [221, 219, 149, 97]),
        // log(uint256,bool,uint256,string)
        ([232, 221, 188, 86], [222, 3, 231, 116]),
        // log(string,uint256,bool,address)
        ([229, 84, 157, 145], [224, 233, 91, 152]),
        // log(string,uint256,uint256,address)
        ([190, 215, 40, 191], [226, 29, 226, 120]),
        // log(uint256,address,bool,bool)
        ([126, 39, 65, 13], [227, 81, 20, 15]),
        // log(bool,bool,string,uint256)
        ([23, 139, 70, 133], [227, 169, 202, 47]),
        // log(string,uint256,bool,uint256)
        ([85, 14, 110, 245], [228, 27, 111, 111]),
        // log(bool,uint256,string,bool)
        ([145, 210, 248, 19], [229, 231, 11, 43]),
        // log(uint256,string,address,uint256)
        ([152, 231, 243, 243], [232, 211, 1, 141]),
        // log(bool,uint256,bool)
        ([27, 173, 201, 235], [232, 222, 251, 169]),
        // log(uint256,uint256,bool,uint256)
        ([108, 100, 124, 140], [235, 127, 111, 210]),
        // log(uint256,bool,string,bool)
        ([52, 110, 184, 199], [235, 146, 141, 127]),
        // log(address,address,string,uint256)
        ([4, 40, 147, 0], [239, 28, 239, 231]),
        // log(uint256,bool,string,address)
        ([73, 110, 43, 180], [239, 82, 144, 24]),
        // log(uint256,address,bool,address)
        ([182, 49, 48, 148], [239, 114, 197, 19]),
        // log(string,string,uint256,uint256)
        ([213, 207, 23, 208], [244, 93, 125, 44]),
        // log(bool,uint256,string,string)
        ([211, 42, 101, 72], [245, 188, 34, 73]),
        // log(uint256,uint256)
        ([108, 15, 105, 128], [246, 102, 113, 90]),
        // log(uint256) and logUint(uint256)
        ([245, 177, 187, 169], [248, 44, 80, 241]),
        // log(string,address,uint256,uint256)
        ([218, 163, 148, 189], [248, 245, 27, 30]),
        // log(uint256,uint256,uint256,address)
        ([224, 133, 63, 105], [250, 129, 133, 175]),
        // log(string,address,uint256,bool)
        ([90, 193, 193, 60], [252, 72, 69, 240]),
        // log(address,address,uint256,string)
        ([157, 209, 46, 173], [253, 180, 249, 144]),
        // log(bool,uint256,string,address)
        ([165, 199, 13, 41], [254, 221, 31, 255]),
        // logInt(int256)
        ([78, 12, 29, 29], [101, 37, 181, 245]),
        // logBytes(bytes)
        ([11, 231, 127, 86], [225, 123, 249, 86]),
        // logBytes1(bytes1)
        ([110, 24, 161, 40], [111, 65, 113, 201]),
        // logBytes2(bytes2)
        ([233, 182, 34, 150], [155, 94, 148, 62]),
        // logBytes3(bytes3)
        ([45, 131, 73, 38], [119, 130, 250, 45]),
        // logBytes4(bytes4)
        ([224, 95, 72, 209], [251, 163, 173, 57]),
        // logBytes5(bytes5)
        ([166, 132, 128, 141], [85, 131, 190, 46]),
        // logBytes6(bytes6)
        ([174, 132, 165, 145], [73, 66, 173, 198]),
        // logBytes7(bytes7)
        ([78, 213, 126, 40], [69, 116, 175, 171]),
        // logBytes8(bytes8)
        ([79, 132, 37, 46], [153, 2, 228, 127]),
        // logBytes9(bytes9)
        ([144, 189, 140, 208], [80, 161, 56, 223]),
        // logBytes10(bytes10)
        ([1, 61, 23, 139], [157, 194, 168, 151]),
        // logBytes11(bytes11)
        ([4, 0, 74, 46], [220, 8, 182, 167]),
        // logBytes12(bytes12)
        ([134, 160, 106, 189], [118, 86, 214, 199]),
        // logBytes13(bytes13)
        ([148, 82, 158, 52], [52, 193, 216, 27]),
        // logBytes14(bytes14)
        ([146, 102, 240, 127], [60, 234, 186, 101]),
        // logBytes15(bytes15)
        ([218, 149, 116, 224], [89, 26, 61, 162]),
        // logBytes16(bytes16)
        ([102, 92, 97, 4], [31, 141, 115, 18]),
        // logBytes17(bytes17)
        ([51, 159, 103, 58], [248, 154, 83, 47]),
        // logBytes18(bytes18)
        ([196, 210, 61, 154], [216, 101, 38, 66]),
        // logBytes19(bytes19)
        ([94, 107, 90, 51], [0, 245, 107, 201]),
        // logBytes20(bytes20)
        ([81, 136, 227, 233], [236, 184, 86, 126]),
        // logBytes21(bytes21)
        ([233, 218, 53, 96], [48, 82, 192, 143]),
        // logBytes22(bytes22)
        ([213, 250, 232, 156], [128, 122, 180, 52]),
        // logBytes23(bytes23)
        ([171, 161, 207, 13], [73, 121, 176, 55]),
        // logBytes24(bytes24)
        ([241, 179, 91, 52], [9, 119, 174, 252]),
        // logBytes25(bytes25)
        ([11, 132, 188, 88], [174, 169, 150, 63]),
        // logBytes26(bytes26)
        ([248, 177, 73, 241], [211, 99, 86, 40]),
        // logBytes27(bytes27)
        ([58, 55, 87, 221], [252, 55, 47, 159]),
        // logBytes28(bytes28)
        ([200, 42, 234, 238], [56, 47, 154, 52]),
        // logBytes29(bytes29)
        ([75, 105, 195, 213], [122, 24, 118, 65]),
        // logBytes30(bytes30)
        ([238, 18, 196, 237], [196, 52, 14, 246]),
        // logBytes31(bytes31)
        ([194, 133, 77, 146], [129, 252, 134, 72]),
        // logBytes32(bytes32)
        ([39, 183, 207, 133], [45, 33, 214, 247]),
    ])
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardhat_console_path_works() {
        for (hh, abigen) in HARDHAT_CONSOLE_SELECTOR_PATCHES.iter() {
            let patched = patch_hardhat_console_selector(hh.to_vec());
            assert_eq!(abigen.to_vec(), patched);
        }
    }
}
