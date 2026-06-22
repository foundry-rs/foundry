//! Contains various tests for `forge test` with precompiles.

use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::{str, util::OutputExt};

forgetest_init!(precompile_trace_decoding, |prj, cmd| {
    prj.add_test(
        "PrecompileTrace.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

contract PrecompileCaller {
    constructor() {
        // 0x01 - ECRECOVER
        {
            bytes32 hash = keccak256("test message");
            uint8 v = 27;
            bytes32 r = bytes32(uint256(1));
            bytes32 s = bytes32(uint256(2));
            address(0x01).staticcall(abi.encode(hash, v, r, s));
        }

        // 0x02 - SHA256
        address(0x02).staticcall(abi.encodePacked("hello"));

        // 0x03 - RIPEMD160
        address(0x03).staticcall(abi.encodePacked("hello"));

        // 0x04 - IDENTITY (datacopy)
        address(0x04).staticcall(abi.encodePacked("hello"));

        // 0x05 - MODEXP: compute 2^3 mod 5 = 3
        {
            bytes memory modexpInput = abi.encodePacked(
                uint256(1),  // base length
                uint256(1),  // exponent length
                uint256(1),  // modulus length
                uint8(2),    // base = 2
                uint8(3),    // exponent = 3
                uint8(5)     // modulus = 5
            );
            address(0x05).staticcall(modexpInput);
        }

        // 0x06 - BN254 ADD (ecadd): P + O = P
        {
            uint256 g1x = 1;
            uint256 g1y = 2;
            uint256 zerox = 0;
            uint256 zeroy = 0;
            address(0x06).staticcall(abi.encode(g1x, g1y, zerox, zeroy));
        }

        // 0x07 - BN254 MUL (ecmul): 1 * G = G
        {
            uint256 g1x = 1;
            uint256 g1y = 2;
            uint256 scalar = 1;
            address(0x07).staticcall(abi.encode(g1x, g1y, scalar));
        }

        // 0x08 - BN254 PAIRING: empty input returns success (1)
        address(0x08).staticcall("");

        // 0x09 - BLAKE2F
        {
            bytes memory blake2fInput = new bytes(213);
            blake2fInput[3] = 0x0c; // 12 rounds
            bytes8[8] memory iv = [
                bytes8(0x6a09e667f3bcc908),
                bytes8(0xbb67ae8584caa73b),
                bytes8(0x3c6ef372fe94f82b),
                bytes8(0xa54ff53a5f1d36f1),
                bytes8(0x510e527fade682d1),
                bytes8(0x9b05688c2b3e6c1f),
                bytes8(0x1f83d9abfb41bd6b),
                bytes8(0x5be0cd19137e2179)
            ];
            for (uint256 i = 0; i < 8; i++) {
                for (uint256 j = 0; j < 8; j++) {
                    blake2fInput[4 + i * 8 + j] = iv[i][j];
                }
            }
            blake2fInput[212] = 0x01;
            address(0x09).staticcall(blake2fInput);
        }

        // 0x0B - BLS12-381 G1 ADD (two points at infinity)
        address(0x0B).staticcall(new bytes(256));

        // 0x0C - BLS12-381 G1 MSM
        address(0x0C).staticcall(new bytes(160));

        // 0x0D - BLS12-381 G2 ADD (two points at infinity)
        address(0x0D).staticcall(new bytes(512));

        // 0x0E - BLS12-381 G2 MSM
        address(0x0E).staticcall(new bytes(288));

        // 0x0F - BLS12-381 PAIRING (G1 + G2 infinity points)
        address(0x0F).staticcall(new bytes(384));

        // 0x10 - BLS12-381 MAP FP TO G1
        address(0x10).staticcall(new bytes(64));

        // 0x11 - BLS12-381 MAP FP2 TO G2
        address(0x11).staticcall(new bytes(128));

        // 0x100 - P256VERIFY (secp256r1)
        address(0x100).staticcall(new bytes(160));
    }
}

contract PrecompileTraceTest is Test {
    function test_precompile_traces() public {
        new PrecompileCaller();
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "test_precompile_traces", "-vvvv", "--evm-version", "osaka"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/PrecompileTrace.t.sol:PrecompileTraceTest
[PASS] test_precompile_traces() ([GAS])
Traces:
  [..] PrecompileTraceTest::test_precompile_traces()
    ├─ [..] → new PrecompileCaller@[..]
    │   ├─ [..] PRECOMPILES::ecrecover(0xea83cdcdd06bf61e414054115a551e23133711d0507dcbc07a4bab7dc4581935, 27, 1, 2) [staticcall]
    │   │   └─ ← [Return] 0xBe038042508C42Df7b2A529cd4Cc0a9447c7D2b6
    │   ├─ [..] PRECOMPILES::sha256(0x68656c6c6f) [staticcall]
    │   │   └─ ← [Return] 0x2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    │   ├─ [..] PRECOMPILES::ripemd(0x68656c6c6f) [staticcall]
    │   │   └─ ← [Return] 0x000000000000000000000000108f07b838241261
    │   ├─ [..] PRECOMPILES::identity(0x68656c6c6f) [staticcall]
    │   │   └─ ← [Return] 0x68656c6c6f
    │   ├─ [..] PRECOMPILES::modexp(1, 1, 1, 0x02, 0x03, 0x05) [staticcall]
    │   │   └─ ← [Return] 0x03
    │   ├─ [..] PRECOMPILES::ecadd(1, 2, 0, 0) [staticcall]
    │   │   └─ ← [Return] (1, 2)
    │   ├─ [..] PRECOMPILES::ecmul(1, 2, 1) [staticcall]
    │   │   └─ ← [Return] (1, 2)
    │   ├─ [..] PRECOMPILES::ecpairing() [staticcall]
    │   │   └─ ← [Return] true
    │   ├─ [..] PRECOMPILES::blake2f(12, [633244976228469098, 4298627039875721147, 3168446158426304060, 17381112106731261861, 15096882533739138641, 2264253069420660123, 7763433881832358687, 8728396173323133019], [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], [0, 0], 1) [staticcall]
    │   │   └─ ← [Return] 0x1a48bfec594a1b13bb024be345656b8af895d662ccbc3f39fb5ecf2ef05942b5acace594cb81cdff6044b5bfaabfea105168676ce5753f6bb559ce3f92ad4850
    │   ├─ [..] PRECOMPILES::bls12G1Add(0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    │   ├─ [..] PRECOMPILES::bls12G1Msm(0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    │   ├─ [..] PRECOMPILES::bls12G2Add(0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000, 0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    │   ├─ [..] PRECOMPILES::bls12G2Msm(0x000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    │   ├─ [..] PRECOMPILES::bls12PairingCheck(0x000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] true
    │   ├─ [..] PRECOMPILES::bls12MapFpToG1(0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x0000000000000000000000000000000011a9a0372b8f332d5c30de9ad14e50372a73fa4c45d5f2fa5097f2d6fb93bcac592f2e1711ac43db0519870c7d0ea41500000000000000000000000000000000092c0f994164a0719f51c24ba3788de240ff926b55f58c445116e8bc6a47cd63392fd4e8e22bdf9feaa96ee773222133
    │   ├─ [..] PRECOMPILES::bls12MapFp2ToG2(0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   │   └─ ← [Return] 0x00000000000000000000000000000000018320896ec9eef9d5e619848dc29ce266f413d02dd31d9b9d44ec0c79cd61f18b075ddba6d7bd20b7ff27a4b324bfce000000000000000000000000000000000a67d12118b5a35bb02d2e86b3ebfa7e23410db93de39fb06d7025fa95e96ffa428a7a27c3ae4dd4b40bd251ac658892000000000000000000000000000000000260e03644d1a2c321256b3246bad2b895cad13890cbe6f85df55106a0d334604fb143c7a042d878006271865bc359410000000000000000000000000000000004c69777a43f0bda07679d5805e63f18cf4e0e7c6112ac7f70266d199b4f76ae27c6269a3ceebdae30806e9a76aadf5c
    │   ├─ [..] P256VERIFY::fulfillBasicOrder_efficient_6GL6yc() [staticcall]
    │   │   └─ ← [Return]
    │   └─ ← [Return] 62 bytes of code
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest_init!(precompile_cheatcode_load_is_read_only, |prj, cmd| {
    prj.add_test(
        "PrecompileCheatcodeLoad.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

contract PrecompileCheatcodeLoadTest is Test {
    address constant ECRECOVER = address(0x01);
    bytes32 constant SLOT = bytes32(uint256(1));

    function test_load_allows_precompile_target() public view {
        assertEq(vm.load(ECRECOVER, SLOT), bytes32(0));
    }

    function test_mutation_cheatcodes_reject_precompile_target() public {
        (bool storeSuccess,) = address(this).call(abi.encodeCall(this.storePrecompileSlot, ()));
        assertFalse(storeSuccess);

        (bool etchSuccess,) = address(this).call(abi.encodeCall(this.etchPrecompile, ()));
        assertFalse(etchSuccess);
    }

    function storePrecompileSlot() external {
        vm.store(ECRECOVER, SLOT, bytes32(uint256(1)));
    }

    function etchPrecompile() external {
        vm.etch(ECRECOVER, hex"00");
    }
}
   "#,
    );

    cmd.args(["test", "--match-contract", "PrecompileCheatcodeLoadTest"]).assert_success();
});

forgetest_init!(tempo_t5_hardfork_precompile_smoke, |prj, cmd| {
    prj.update_config(|config| {
        config.networks = NetworkConfigs::with_tempo();
        config.hardfork = Some("tempo:T5".parse::<foundry_config::FoundryHardfork>().unwrap());
    });

    prj.add_test(
        "TempoT5PrecompileSmoke.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

interface IAddressRegistry {
    function isImplicitlyApproved(address precompile) external view returns (bool);
}

interface ITIP20ChannelReserve {
    function domainSeparator() external view returns (bytes32);
}

contract TempoT5PrecompileSmokeTest is Test {
    address constant ADDRESS_REGISTRY = address(bytes20(hex"FDC0000000000000000000000000000000000000"));
    address constant FEE_MANAGER = address(bytes20(hex"feec000000000000000000000000000000000000"));
    address constant STABLECOIN_DEX = address(bytes20(hex"dec0000000000000000000000000000000000000"));
    address constant TIP20_CHANNEL_RESERVE = address(bytes20(hex"4D50500000000000000000000000000000000000"));

    function test_t5_hardfork_precompile_smoke() public {
        assertGt(TIP20_CHANNEL_RESERVE.code.length, 0);

        IAddressRegistry registry = IAddressRegistry(ADDRESS_REGISTRY);
        assertTrue(registry.isImplicitlyApproved(FEE_MANAGER));
        assertTrue(registry.isImplicitlyApproved(STABLECOIN_DEX));
        assertTrue(registry.isImplicitlyApproved(TIP20_CHANNEL_RESERVE));

        bytes32 separator = ITIP20ChannelReserve(TIP20_CHANNEL_RESERVE).domainSeparator();
        assertTrue(separator != bytes32(0));
    }
}
   "#,
    );

    let stdout = cmd
        .args(["test", "--mt", "test_t5_hardfork_precompile_smoke", "-vvvv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("AddressRegistry::isImplicitlyApproved"), "{stdout}");
    assert!(stdout.contains("TIP20ChannelReserve::domainSeparator"), "{stdout}");
});

forgetest_init!(tempo_t6_keychain_helpers_and_decoding, |prj, cmd| {
    prj.update_config(|config| {
        config.networks = NetworkConfigs::with_tempo();
        config.hardfork = Some("tempo:T6".parse::<foundry_config::FoundryHardfork>().unwrap());
    });

    prj.add_test(
        "TempoT6KeychainHelpers.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

interface IAccountKeychain {
    function isAdminKey(address account, address keyId) external view returns (bool);
}

interface ISignatureVerifier {
    function verifyKeychain(address account, bytes32 hash, bytes calldata signature) external view returns (bool);
    function verifyKeychainAdmin(address account, bytes32 hash, bytes calldata signature) external view returns (bool);
}

interface ITIP403Registry {
    function validateReceivePolicy(
        address token,
        address sender,
        address receiver
    ) external view returns (bool authorized, uint8 blockedReason);
}

interface IReceivePolicyGuard {
    function balanceOf(bytes calldata receipt) external view returns (uint256 amount);
}

interface TempoVm {
    function signKeychain(uint256 privateKey, address account, bytes32 digest)
        external
        pure
        returns (bytes memory signature);
    function signKeychainAdmin(uint256 privateKey, address account, bytes32 digest)
        external
        pure
        returns (bytes memory signature);
    function expectKeychainVerified(address account, bytes32 digest, bytes calldata signature) external;
    function expectKeychainAdminVerified(address account, bytes32 digest, bytes calldata signature) external;
}

contract TempoT6KeychainHelpersTest is Test {
    TempoVm constant tempoVm = TempoVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    address constant ACCOUNT_KEYCHAIN = address(bytes20(hex"aaaaaaaa00000000000000000000000000000000"));
    address constant SIGNATURE_VERIFIER = address(bytes20(hex"5165300000000000000000000000000000000000"));
    address constant TIP403_REGISTRY = address(bytes20(hex"403c000000000000000000000000000000000000"));
    address constant RECEIVE_POLICY_GUARD = address(bytes20(hex"b10c000000000000000000000000000000000000"));
    address constant PATH_USD = address(bytes20(hex"20c0000000000000000000000000000000000000"));

    uint256 constant ROOT_PK = 0xA11CE;
    uint256 constant ACCESS_PK = 0xB0B;

    IAccountKeychain constant keychain = IAccountKeychain(ACCOUNT_KEYCHAIN);
    ISignatureVerifier constant verifier = ISignatureVerifier(SIGNATURE_VERIFIER);
    ITIP403Registry constant registry = ITIP403Registry(TIP403_REGISTRY);
    IReceivePolicyGuard constant guard = IReceivePolicyGuard(RECEIVE_POLICY_GUARD);

    address root;
    address accessKey;
    bytes32 digest;

    function setUp() public {
        root = vm.addr(ROOT_PK);
        accessKey = vm.addr(ACCESS_PK);
        digest = keccak256("tempo t6 forge keychain");
    }

    function test_sign_keychain_signature_shape_and_missing_key_fails() public {
        bytes memory signature = tempoVm.signKeychain(ACCESS_PK, root, digest);

        assertEq(signature.length, 86);
        assertEq(uint8(signature[0]), 4);
        assertEq(_embeddedAccount(signature), root);
        assertFalse(verifier.verifyKeychain(root, digest, signature));
        assertFalse(verifier.verifyKeychain(address(0xbeef), digest, signature));
    }

    function test_keychain_admin_signature_verifies_root_key_and_rejects_non_admin() public {
        bytes32 adminDigest = _adminDigest("admin");
        bytes memory rootSignature = tempoVm.signKeychainAdmin(ROOT_PK, root, adminDigest);
        bytes memory nonAdminSignature = tempoVm.signKeychainAdmin(ACCESS_PK, root, adminDigest);

        assertTrue(keychain.isAdminKey(root, root));
        assertFalse(keychain.isAdminKey(root, accessKey));
        assertTrue(verifier.verifyKeychainAdmin(root, adminDigest, rootSignature));
        assertFalse(verifier.verifyKeychainAdmin(root, adminDigest, nonAdminSignature));
        assertFalse(verifier.verifyKeychainAdmin(address(0xbeef), adminDigest, rootSignature));
    }

    function test_expect_helpers_match_signature_verifier_calls() public {
        bytes memory signature = tempoVm.signKeychain(ACCESS_PK, root, digest);
        tempoVm.expectKeychainVerified(root, digest, signature);
        verifier.verifyKeychain(root, digest, signature);

        bytes32 adminDigest = _adminDigest("expect-admin");
        bytes memory rootSignature = tempoVm.signKeychainAdmin(ROOT_PK, root, adminDigest);
        tempoVm.expectKeychainAdminVerified(root, adminDigest, rootSignature);
        verifier.verifyKeychainAdmin(root, adminDigest, rootSignature);
    }

    function test_malformed_keychain_signature_reverts() public {
        vm.expectRevert();
        verifier.verifyKeychain(root, digest, hex"04");
    }

    function test_receive_policy_interfaces_are_callable() public {
        (bool authorized, uint8 blockedReason) = registry.validateReceivePolicy(PATH_USD, root, address(0xbeef));
        assertTrue(authorized);
        assertEq(blockedReason, 0);

        bytes memory receipt = abi.encode(
            uint8(1),
            PATH_USD,
            address(0),
            root,
            address(0xbeef),
            uint64(block.timestamp),
            uint64(1),
            uint8(1),
            uint8(0),
            bytes32("forge")
        );
        assertEq(guard.balanceOf(receipt), 0);
    }

    function _adminDigest(string memory label) internal view returns (bytes32) {
        return keccak256(abi.encode(block.chainid, address(this), root, label));
    }

    function _embeddedAccount(bytes memory signature) internal pure returns (address account) {
        assembly {
            account := shr(96, mload(add(signature, 0x21)))
        }
    }
}
   "#,
    );

    let stdout = cmd
        .args(["test", "--mc", "TempoT6KeychainHelpersTest", "-vvvv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("AccountKeychain::isAdminKey"), "{stdout}");
    assert!(stdout.contains("SignatureVerifier::verifyKeychain"), "{stdout}");
    assert!(stdout.contains("SignatureVerifier::verifyKeychainAdmin"), "{stdout}");
    assert!(stdout.contains("TIP403Registry::validateReceivePolicy"), "{stdout}");
    assert!(stdout.contains("ReceivePolicyGuard::balanceOf"), "{stdout}");
});

// tests transfer using celo precompile.
// <https://github.com/foundry-rs/foundry/issues/11622>
forgetest_init!(celo_transfer, |prj, cmd| {
    prj.update_config(|config| {
        config.networks = NetworkConfigs::with_celo();
    });

    prj.add_test(
        "CeloTransfer.t.sol",
        r#"
import "forge-std/Test.sol";

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}

contract CeloTransferTest is Test {
    IERC20 celo = IERC20(0x471EcE3750Da237f93B8E339c536989b8978a438);
    IERC20 usdc = IERC20(0xcebA9300f2b948710d2653dD7B07f33A8B32118C);
    IERC20 usdt = IERC20(0x48065fbBE25f71C9282ddf5e1cD6D6A887483D5e);

    address binanceAccount = 0xf6436829Cf96EA0f8BC49d300c536FCC4f84C4ED;
    address recipient = makeAddr("recipient");

    function setUp() public {
        vm.createSelectFork("https://forno.celo.org");
    }

    function testCeloBalance() external {
        console2.log("recipient balance before", celo.balanceOf(recipient));
        vm.prank(binanceAccount);
        celo.transfer(recipient, 100);
        console2.log("recipient balance after", celo.balanceOf(recipient));
        assertEq(celo.balanceOf(recipient), 100);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "testCeloBalance", "-vvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CeloTransfer.t.sol:CeloTransferTest
[PASS] testCeloBalance() ([GAS])
Logs:
  recipient balance before 0
  recipient balance after 100

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
