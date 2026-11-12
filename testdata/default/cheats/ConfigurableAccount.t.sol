// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// Expected values are reference vectors from ox 0.14.65, the version viem 2.57.18's Tempo account
// code (`src/tempo/accounts`) pins: `AccountConfig.getAddress`, `AccountConfig.getSignPayload`,
// `SignatureEnvelope.sortApprovals` and `SignatureEnvelope.serialize`, with deterministic
// (RFC 6979) secp256k1 and P-256 signatures, and WebAuthn assertions from
// `Authentication.getSignPayload` for the RP ID `localhost` and origin `http://localhost`. The keychain
// payload is `keccak256(0x04 || INNER || ROOT)`, which `TxEnvelopeTempo.getSignPayload` computes with
// `from: ROOT`. `CAROL_SIGNATURE` is ox's encoding of the approval set the cheatcode is expected to
// choose; the T14 CLI test checks that the verifier accepts trimmed approvals. The quorum case's
// expected signature is ox's encoding of the weight-nine owner's approval alone.
contract ConfigurableAccountTest is Test {
    uint256 constant ALICE_PK = 0xA11CE;
    uint256 constant BOB_PK = 0xB0B;
    uint256 constant CAROL_PK = 0xCA201;
    uint256 constant P256_PK = 0xCA11;
    uint256 constant WEBAUTHN_PK = 0xBEEF;
    uint256 constant ROOT_PK = 0x5007;

    address constant ALICE = 0xe05fcC23807536bEe418f142D19fa0d21BB0cfF7;
    address constant BOB = 0x0376AAc07Ad725E01357B1725B5ceC61aE10473c;
    address constant CAROL = 0xb12C95eC70c92ad3461431c2CA4e8F400045b7bD;
    address constant P256_OWNER = 0x2529d5fD061c1d4E7350Bec775cE38FbB39a434a;
    address constant WEBAUTHN_OWNER = 0x22F8f1c700f14B68f0B4987348685d08A67f8444;
    address constant ROOT = 0xCc2491704a7D4D459bDA6028609bE0437d74c3e9;

    address constant ACCOUNT = 0xA44c73b3e3d1C015bC17a3be952dd701D6c6E876;
    bytes32 constant INNER = keccak256("forge configurable account");
    bytes32 constant DIGEST_V0 = 0x24d3ee1f1a14a51a7ced03762e1a591ff4fd2d2db8cb6e20d3d408d13e79b27e;
    bytes32 constant DIGEST_V7 = 0x556ac76bc9fbded54c254a6f6a5334f150505575adf90ca383694616439f902a;

    /// Alice and Bob's approvals, sorted by owner.
    bytes constant SECP_APPROVAL_BOB =
        hex"27475929da4f9b0e8914d08c13f4534c1fe9ea075753f313b21d2a213f98381f2feec7ecdb5adf37b4a4ec535ba1b54be2a38a7ce99b9b8ff9e49e95ff8a94c81b";
    bytes constant SECP_APPROVAL_ALICE =
        hex"bbe50584ed765b8617e5b5f7f85af078e189f3c59e498fe4484330faa2dc16a44ebf3d2f67f041d87f79d20fe43d1008cfe7089f7c8e071b79cf44f02a9ea1f91c";
    bytes constant SECP_SIGNATURE =
        hex"05f9013794a44c73b3e3d1c015bc17a3be952dd701d6c6e876f898a0000000000000000000000000000000000000000000000000000000000000005a8002f873d6940376aac07ad725e01357b1725b5cec61ae10473c01d69422f8f1c700f14b68f0b4987348685d08a67f844401d6942529d5fd061c1d4e7350bec775ce38fbb39a434a01d694b12c95ec70c92ad3461431c2ca4e8f400045b7bd02d694e05fcc23807536bee418f142d19fa0d21bb0cff701f886b84127475929da4f9b0e8914d08c13f4534c1fe9ea075753f313b21d2a213f98381f2feec7ecdb5adf37b4a4ec535ba1b54be2a38a7ce99b9b8ff9e49e95ff8a94c81bb841bbe50584ed765b8617e5b5f7f85af078e189f3c59e498fe4484330faa2dc16a44ebf3d2f67f041d87f79d20fe43d1008cfe7089f7c8e071b79cf44f02a9ea1f91c";
    /// Carol's weight alone reaches the threshold.
    bytes constant CAROL_SIGNATURE =
        hex"05f8f494a44c73b3e3d1c015bc17a3be952dd701d6c6e876f898a0000000000000000000000000000000000000000000000000000000000000005a8002f873d6940376aac07ad725e01357b1725b5cec61ae10473c01d69422f8f1c700f14b68f0b4987348685d08a67f844401d6942529d5fd061c1d4e7350bec775ce38fbb39a434a01d694b12c95ec70c92ad3461431c2ca4e8f400045b7bd02d694e05fcc23807536bee418f142d19fa0d21bb0cff701f843b84101ef5a77deb7df9fec86409dccacb7869b5a3f80446251c18b2cd4a6c4e3a8d149b9f5856af1b76d1a24479dbd8f91a968e6002be21fbe76720e1647bcedcd891b";
    /// The WebAuthn and P-256 owners' approvals, sorted by owner.
    bytes constant WEBAUTHN_APPROVAL =
        hex"0249960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d976305000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a224a4e507548786f557052703837514e324c68705a485f54394c533234793234673039514930543535736e34222c226f726967696e223a22687474703a2f2f6c6f63616c686f7374222c2263726f73734f726967696e223a66616c73657d959e37539d771e1750f8f112807a7be848521259f33320d38e1f5e30309740f40ca78f5b235eac32ab1d5a7d1649ae131d3b8f7452445c93432656c4e8d3372cf7631cc34b56c24b1758ae6867b23811c129c035fa67cc2a40c617bedb3c4b51db6bdf29e488c62b18e938e5fb42eed65fa74ea4f3d53ebd35cf9b385374d042";
    bytes constant P256_APPROVAL =
        hex"01e88c8c9a24f1a9b0a7911960d3d817f17c94359a792a6b808a0f3ea40907c6b224896fdc98091836827efa3f0485bffb25b3dbfcfc3d7fd0e8fcbbcdac33b4d8c27daba02b93e821c4c47b4d0f906254d166f402f4988b44026d52b5c3dffc6d42e816e512ea4af558941d61193b58d67d5c7a7869e76afcde948d873fa8e4a900";
    bytes constant MIXED_SIGNATURE =
        hex"05f9026094a44c73b3e3d1c015bc17a3be952dd701d6c6e876f898a0000000000000000000000000000000000000000000000000000000000000005a8002f873d6940376aac07ad725e01357b1725b5cec61ae10473c01d69422f8f1c700f14b68f0b4987348685d08a67f844401d6942529d5fd061c1d4e7350bec775ce38fbb39a434a01d694b12c95ec70c92ad3461431c2ca4e8f400045b7bd02d694e05fcc23807536bee418f142d19fa0d21bb0cff701f901aeb901270249960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d976305000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a224a4e507548786f557052703837514e324c68705a485f54394c533234793234673039514930543535736e34222c226f726967696e223a22687474703a2f2f6c6f63616c686f7374222c2263726f73734f726967696e223a66616c73657d959e37539d771e1750f8f112807a7be848521259f33320d38e1f5e30309740f40ca78f5b235eac32ab1d5a7d1649ae131d3b8f7452445c93432656c4e8d3372cf7631cc34b56c24b1758ae6867b23811c129c035fa67cc2a40c617bedb3c4b51db6bdf29e488c62b18e938e5fb42eed65fa74ea4f3d53ebd35cf9b385374d042b88201e88c8c9a24f1a9b0a7911960d3d817f17c94359a792a6b808a0f3ea40907c6b224896fdc98091836827efa3f0485bffb25b3dbfcfc3d7fd0e8fcbbcdac33b4d8c27daba02b93e821c4c47b4d0f906254d166f402f4988b44026d52b5c3dffc6d42e816e512ea4af558941d61193b58d67d5c7a7869e76afcde948d873fa8e4a900";
    /// Keychain V2 signature for `ROOT` with `ACCOUNT` as its access key, approved by Alice and the
    /// WebAuthn owner.
    bytes constant KEYCHAIN_SIGNATURE =
        hex"04cc2491704a7d4d459bda6028609be0437d74c3e905f9021f94a44c73b3e3d1c015bc17a3be952dd701d6c6e876f898a0000000000000000000000000000000000000000000000000000000000000005a8002f873d6940376aac07ad725e01357b1725b5cec61ae10473c01d69422f8f1c700f14b68f0b4987348685d08a67f844401d6942529d5fd061c1d4e7350bec775ce38fbb39a434a01d694b12c95ec70c92ad3461431c2ca4e8f400045b7bd02d694e05fcc23807536bee418f142d19fa0d21bb0cff701f9016db901270249960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d976305000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a224f73537065516568363730744b5870705f446b574c344b4f71746f4e4e6574453155737031736c6a496f73222c226f726967696e223a22687474703a2f2f6c6f63616c686f7374222c2263726f73734f726967696e223a66616c73657d7b64b01d1af48905570f97e1b46e0b850049aa5b240d674265985fcfb4349dc444258d6619267eef536043873d91d5e384c20a26beb034497d56c189a0b60629f7631cc34b56c24b1758ae6867b23811c129c035fa67cc2a40c617bedb3c4b51db6bdf29e488c62b18e938e5fb42eed65fa74ea4f3d53ebd35cf9b385374d042b841eef59431abc0fe23a85c717aefadea16e0ffdeb41a8c89d641e008bc9de9d6b133c3f0ba8ea7d97583c62f1cc03b2ddb744d7f5a5b38a71b1daa4e49ca2348371b";

    function config() internal pure returns (Vm.ConfigurableAccountConfig memory cfg) {
        cfg.salt = bytes32(uint256(0x5a));
        cfg.threshold = 2;
        cfg.owners = new Vm.ConfigurableAccountOwner[](5);
        cfg.owners[0] = Vm.ConfigurableAccountOwner(BOB, 1);
        cfg.owners[1] = Vm.ConfigurableAccountOwner(WEBAUTHN_OWNER, 1);
        cfg.owners[2] = Vm.ConfigurableAccountOwner(P256_OWNER, 1);
        cfg.owners[3] = Vm.ConfigurableAccountOwner(CAROL, 2);
        cfg.owners[4] = Vm.ConfigurableAccountOwner(ALICE, 1);
    }

    function key(Vm.OwnerKeyType keyType, uint256 privateKey) internal pure returns (Vm.OwnerKey memory) {
        return Vm.OwnerKey(keyType, privateKey);
    }

    function p256Owner(uint256 privateKey) internal pure returns (address) {
        (uint256 x, uint256 y) = vm.publicKeyP256(privateKey);
        return address(uint160(uint256(keccak256(abi.encode(x, y)))));
    }

    function testOwnerKeysMatchViemOwners() public {
        assertEq(vm.addr(ALICE_PK), ALICE);
        assertEq(vm.addr(BOB_PK), BOB);
        assertEq(vm.addr(CAROL_PK), CAROL);
        assertEq(vm.addr(ROOT_PK), ROOT);
        assertEq(p256Owner(P256_PK), P256_OWNER);
        assertEq(p256Owner(WEBAUTHN_PK), WEBAUTHN_OWNER);
    }

    function testDeriveConfigurableAccount() public {
        assertEq(vm.deriveConfigurableAccount(config()), ACCOUNT);
    }

    function testDeriveConfigurableAccountRejectsUnsortedOwners() public {
        Vm.ConfigurableAccountConfig memory cfg = config();
        (cfg.owners[0], cfg.owners[1]) = (cfg.owners[1], cfg.owners[0]);
        vm._expectCheatcodeRevert(
            "vm.deriveConfigurableAccount: configurable account owners must be strictly ascending"
        );
        vm.deriveConfigurableAccount(cfg);
    }

    function testConfigurableAccountDigest() public {
        assertEq(vm.configurableAccountDigest(ACCOUNT, 0, INNER), DIGEST_V0);
        assertEq(vm.configurableAccountDigest(ACCOUNT, 7, INNER), DIGEST_V7);
    }

    function testEncodeConfigurableAccountSignature() public {
        bytes[] memory approvals = new bytes[](2);
        approvals[0] = SECP_APPROVAL_BOB;
        approvals[1] = SECP_APPROVAL_ALICE;
        assertEq(vm.encodeConfigurableAccountSignature(ACCOUNT, config(), approvals), SECP_SIGNATURE);

        approvals[0] = WEBAUTHN_APPROVAL;
        approvals[1] = P256_APPROVAL;
        assertEq(vm.encodeConfigurableAccountSignature(ACCOUNT, config(), approvals), MIXED_SIGNATURE);
    }

    function testSignConfigurableAccountSecp256k1() public {
        // Keys in any order are sorted by owner.
        uint256[] memory keys = new uint256[](2);
        keys[0] = ALICE_PK;
        keys[1] = BOB_PK;
        assertEq(vm.signConfigurableAccount(keys, ACCOUNT, config(), INNER), SECP_SIGNATURE);
    }

    function testSignConfigurableAccountStopsAtThreshold() public {
        // Carol sorts before Alice and reaches the threshold alone, so Alice does not sign.
        uint256[] memory keys = new uint256[](2);
        keys[0] = ALICE_PK;
        keys[1] = CAROL_PK;
        assertEq(vm.signConfigurableAccount(keys, ACCOUNT, config(), INNER), CAROL_SIGNATURE);
    }

    function testSignConfigurableAccountMixedOwners() public {
        Vm.OwnerKey[] memory keys = new Vm.OwnerKey[](2);
        keys[0] = key(Vm.OwnerKeyType.P256, P256_PK);
        keys[1] = key(Vm.OwnerKeyType.WebAuthn, WEBAUTHN_PK);
        assertEq(vm.signConfigurableAccount(keys, ACCOUNT, config(), INNER), MIXED_SIGNATURE);
    }

    function testSignKeychainConfigurableAccount() public {
        Vm.OwnerKey[] memory keys = new Vm.OwnerKey[](2);
        keys[0] = key(Vm.OwnerKeyType.Secp256k1, ALICE_PK);
        keys[1] = key(Vm.OwnerKeyType.WebAuthn, WEBAUTHN_PK);
        assertEq(vm.signKeychainConfigurableAccount(keys, ACCOUNT, config(), ROOT, INNER), KEYCHAIN_SIGNATURE);
    }

    function testSignConfigurableAccountFindsQuorumBeyondAddressOrder() public {
        // Eight weight-one owners and a weight-nine owner last in address order, threshold nine:
        // the first eight keys in address order fall short, but the weight-nine owner alone is a
        // quorum. Keys 0x9001..0x9009 own these addresses; 0x9008 owns the last one.
        Vm.ConfigurableAccountConfig memory cfg;
        cfg.salt = bytes32(uint256(0x09));
        cfg.threshold = 9;
        cfg.owners = new Vm.ConfigurableAccountOwner[](9);
        cfg.owners[0] = Vm.ConfigurableAccountOwner(0x05517D576f9fBA85374E374B2007326833C728Dd, 1);
        cfg.owners[1] = Vm.ConfigurableAccountOwner(0x2A3A43a4C76c28493CAC2c4649d547BF3CB2910f, 1);
        cfg.owners[2] = Vm.ConfigurableAccountOwner(0x53C2ddA8317F7acCA3832D7b2e25f9FFE77388A5, 1);
        cfg.owners[3] = Vm.ConfigurableAccountOwner(0xAb13a23AE34426bDC9Bc55EeD6C2A7154989B7d5, 1);
        cfg.owners[4] = Vm.ConfigurableAccountOwner(0xB796FaE05C1a227C7DA6409074f33522759f5034, 1);
        cfg.owners[5] = Vm.ConfigurableAccountOwner(0xBC3483ceF2C7F33E5C6ccb2E3C36c9f2D180C8C9, 1);
        cfg.owners[6] = Vm.ConfigurableAccountOwner(0xD6b0f4213BD46169fEF562df08736Ba54227b163, 1);
        cfg.owners[7] = Vm.ConfigurableAccountOwner(0xefa2548D3E4efa575F7e0f98124ee81799e8285E, 1);
        cfg.owners[8] = Vm.ConfigurableAccountOwner(0xFc916eec50B43C5C4C57422134F27710CF081157, 9);
        address account = 0xECdE7dc2fe7FAfE5e05e7C89A28FCe41b6C10450;
        assertEq(vm.deriveConfigurableAccount(cfg), account);

        uint256[] memory keys = new uint256[](9);
        for (uint256 i = 0; i < 9; i++) {
            keys[i] = 0x9001 + i;
        }
        assertEq(
            vm.signConfigurableAccount(keys, account, cfg, INNER),
            hex"05f9015094ecde7dc2fe7fafe5e05e7c89a28fce41b6c10450f8f4a000000000000000000000000000000000000000000000000000000000000000098009f8cfd69405517d576f9fba85374e374b2007326833c728dd01d6942a3a43a4c76c28493cac2c4649d547bf3cb2910f01d69453c2dda8317f7acca3832d7b2e25f9ffe77388a501d694ab13a23ae34426bdc9bc55eed6c2a7154989b7d501d694b796fae05c1a227c7da6409074f33522759f503401d694bc3483cef2c7f33e5c6ccb2e3c36c9f2d180c8c901d694d6b0f4213bd46169fef562df08736ba54227b16301d694efa2548d3e4efa575f7e0f98124ee81799e8285e01d694fc916eec50b43c5c4c57422134f27710cf08115709f843b84103302873f6294fb820ef28335bf6760b0263a580d40cc86fcb6c9d1502b7c32606460ea0fa5f6d5f8ef127ddc28602bb9f7da4f077b2c6bd22205f80fa7aac4e1b"
        );
    }

    function testSignConfigurableAccountRejectsUnsortedOwners() public {
        Vm.ConfigurableAccountConfig memory cfg = config();
        (cfg.owners[0], cfg.owners[4]) = (cfg.owners[4], cfg.owners[0]);
        uint256[] memory keys = new uint256[](2);
        keys[0] = ALICE_PK;
        keys[1] = BOB_PK;
        vm._expectCheatcodeRevert("vm.signConfigurableAccount: configurable account owners must be strictly ascending");
        vm.signConfigurableAccount(keys, ACCOUNT, cfg, INNER);
    }

    function testSignConfigurableAccountRejectsConfigOfOtherAccount() public {
        // A new salt derives another account, so version zero no longer matches `ACCOUNT`.
        Vm.ConfigurableAccountConfig memory cfg = config();
        cfg.salt = bytes32(uint256(0x5b));
        uint256[] memory keys = new uint256[](2);
        keys[0] = ALICE_PK;
        keys[1] = BOB_PK;
        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "vm.signConfigurableAccount: configurable account config with version 0 derives ",
                    vm.toString(vm.deriveConfigurableAccount(cfg)),
                    ", not 0xA44c73b3e3d1C015bC17a3be952dd701D6c6E876"
                )
            )
        );
        vm.signConfigurableAccount(keys, ACCOUNT, cfg, INNER);
    }

    function testSignKeychainConfigurableAccountRejectsConfigOfOtherAccount() public {
        // New owner weights derive another account, so version zero no longer matches `ACCOUNT`.
        Vm.ConfigurableAccountConfig memory cfg = config();
        cfg.owners[0].weight += 1;
        Vm.OwnerKey[] memory keys = new Vm.OwnerKey[](2);
        keys[0] = key(Vm.OwnerKeyType.Secp256k1, ALICE_PK);
        keys[1] = key(Vm.OwnerKeyType.WebAuthn, WEBAUTHN_PK);
        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "vm.signKeychainConfigurableAccount: configurable account config with version 0 derives ",
                    vm.toString(vm.deriveConfigurableAccount(cfg)),
                    ", not 0xA44c73b3e3d1C015bC17a3be952dd701D6c6E876"
                )
            )
        );
        vm.signKeychainConfigurableAccount(keys, ACCOUNT, cfg, ROOT, INNER);
    }

    function testSignConfigurableAccountRejectsNonOwner() public {
        uint256[] memory keys = new uint256[](2);
        keys[0] = ALICE_PK;
        keys[1] = ROOT_PK;
        vm._expectCheatcodeRevert(
            "vm.signConfigurableAccount: 0xCc2491704a7D4D459bDA6028609bE0437d74c3e9 is not an owner of configurable account 0xA44c73b3e3d1C015bC17a3be952dd701D6c6E876"
        );
        vm.signConfigurableAccount(keys, ACCOUNT, config(), INNER);
    }

    function testSignConfigurableAccountRejectsWeightBelowThreshold() public {
        uint256[] memory keys = new uint256[](1);
        keys[0] = ALICE_PK;
        vm._expectCheatcodeRevert(
            "vm.signConfigurableAccount: owner keys reach weight 1, but configurable account 0xA44c73b3e3d1C015bC17a3be952dd701D6c6E876 requires 2"
        );
        vm.signConfigurableAccount(keys, ACCOUNT, config(), INNER);
    }
}
