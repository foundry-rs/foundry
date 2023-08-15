// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract DeriveTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testDerive() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = vm.deriveKey(mnemonic, 0);
        assertEq(privateKey, 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

        uint256 privateKeyDerivationPathChanged = vm.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0);
        assertEq(privateKeyDerivationPathChanged, 0x6abb89895f93b02c1b9470db0fa675297f6cca832a5fc66d5dfd7661a42b37be);

        uint256 privateKeyFile = vm.deriveKey("fixtures/Derive/mnemonic_english.txt", 2);
        assertEq(privateKeyFile, 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a);
    }

    uint256 constant numLanguages = 10;

    function testDeriveLang() public {
        string[numLanguages] memory mnemonics = [
            unicode"谐 谐 谐 谐 谐 谐 谐 谐 谐 谐 谐 宗",
            unicode"諧 諧 諧 諧 諧 諧 諧 諧 諧 諧 諧 宗",
            "uzenina uzenina uzenina uzenina uzenina uzenina uzenina uzenina uzenina uzenina uzenina nevina",
            "test test test test test test test test test test test junk",
            unicode"sonde sonde sonde sonde sonde sonde sonde sonde sonde sonde sonde hématome",
            "surgelato surgelato surgelato surgelato surgelato surgelato surgelato surgelato surgelato surgelato surgelato mansarda",
            unicode"ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ほんけ ぜんご",
            unicode"큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 큰어머니 시스템",
            "sobra sobra sobra sobra sobra sobra sobra sobra sobra sobra sobra guarani",
            "tacto tacto tacto tacto tacto tacto tacto tacto tacto tacto tacto lacra"
        ];
        string[numLanguages] memory languages = [
            "chinese_simplified",
            "chinese_traditional",
            "czech",
            "english",
            "french",
            "italian",
            "japanese",
            "korean",
            "portuguese",
            "spanish"
        ];
        uint256[numLanguages] memory privateKeys = [
            0x533bbfc4a21d5cc6ca8ac3a4b6b1dc76e15804e078b0d53d72ba698ca0733a5d,
            0x3ed7268b64e326a75fd4e894a979eed93cc1480f1badebc869542d8508168fe8,
            0x56ab29e6a8d77caeb67976faf95980ee5bbd672a6ae98cac507e8a0cb252b47c,
            0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80,
            0xcdb159305d67bfba6096f47090c34895a75b8f9cc96b6d7e01b99f271e039586,
            0x9976dba80160dd3b89735ea2af8e2b474011972fc92883f4100dd3955f8d921d,
            0xff0bda7ec337713c62b948f307d8197f1a7f95db93b739b0c354654395005b7f,
            0x0ca388477381e73413bbc6188fdac45c583d0215cc43eebec49dc90e4903591a,
            0x857c78c7e0866fcd734077d92892ba215a7b5a9afb6ff437be271686fb0ef9bd,
            0x7eb53bee6299530662f3c91a9b1754cf80aa2d9d89b254ae241825f9414a4a0a
        ];
        uint256[numLanguages] memory privateKeysDerivationPathChanged = [
            0xce09fd1ec0fa74f801f85faa6eeb20019c7378180fed673676ddfb48f1360fc8,
            0x504425bc503d1a6842acbda23e76e852d568a947a7f3ee6cae3bebb677baf1ee,
            0x2e5ff4571add07ecb1f3aac0d394a5564582f9c57c01c12229121e5dff2582f3,
            0x6abb89895f93b02c1b9470db0fa675297f6cca832a5fc66d5dfd7661a42b37be,
            0xe8b159aa146238eaab2b44614aaec7e5f1e0cffa2c3526c198cf101a833e222f,
            0xe2099cc4ccacb8cd902213c5056e54460dfde550a0cf036bd3070e5b176c2f42,
            0xef82b00bb18b2efb9ac1af3530afcba8c1b4c2b16041993d898cfa5d04b81e09,
            0xa851aca713f11e2b971c1a34c448fb112974321d13f4ecf1db19554d72a9c6c7,
            0xcaec3e6839b0eeebcbea3861951721846d4d38bac91b78f1a5c8d2e1269f61fe,
            0x41211f5e0f7373cbd8728cbf2431d4ea0732136475e885328f36e5fd3cee2a43
        ];
        uint256[numLanguages] memory privateKeysFile = [
            0xa540f6a3a6df6d39dc8b5b2290c9d08cc1e2a2a240023933b10940ec9320a7d9,
            0xacfa4014ea48cb4849422952ac083f49e95409d4d7ac6131ec1481c6e91ffbb0,
            0x3f498bf39f2c211208edac088674526d2edd9acf02464fb0e559bb9352b90ccd,
            0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a,
            0x18031c4ccc75784e1b9f772a9d158efe3ca83a525ca2b4bf29f09e09d19ce195,
            0x4da226c5aacbba261bc160f9e43e7147c0b9bfa581f7160d5f2bb9d2e34358f9,
            0x59d7d5fb59d74775cae83c162c49da9af6ded75664200f675168c9322403b291,
            0xd515ca4969e31a59a4a8f23a2fcdad0c7702137ae7cb59fdfbe863c617ca4794,
            0x81b81ee315311874aab9a6560e775e74af5e4003832746df4bf044a9c3987c2f,
            0x2ba37b948b89117cde7ebb7e22bb3954249fa0575f05bfbf47cd3ec20c6f7ebd
        ];

        for (uint256 i = 0; i < numLanguages; ++i) {
            string memory language = languages[i];
            string memory mnemonic = mnemonics[i];

            uint256 privateKey = vm.deriveKey(mnemonic, 0, language);
            assertEq(privateKey, privateKeys[i]);

            uint256 privateKeyDerivationPathChanged = vm.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0, language);
            assertEq(privateKeyDerivationPathChanged, privateKeysDerivationPathChanged[i]);

            string memory prefix = "fixtures/Derive/mnemonic_";
            string memory postfix = ".txt";
            string memory mnemonicPath = string(abi.encodePacked(prefix, language, postfix));
            uint256 privateKeyFile = vm.deriveKey(mnemonicPath, 2, language);
            assertEq(privateKeyFile, privateKeysFile[i]);
        }
    }
}
