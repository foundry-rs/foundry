#![allow(unused_imports)]

use alloc::{string::String, vec::Vec};
use alloy_primitives::{Address, Bytes, B256, B512, U256};

// re-export account type for `eth_getAccount`
pub use alloy_consensus::Account;

/// Account information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AccountInfo {
    /// Account name
    pub name: String,
}

/// Data structure with proof for one single storage-entry
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg(feature = "serde")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct EIP1186StorageProof {
    /// Storage key.
    pub key: alloy_serde::storage::JsonStorageKey,
    /// Value that the key holds
    pub value: U256,
    /// proof for the pair
    pub proof: Vec<Bytes>,
}

#[cfg(feature = "serde")]
impl EIP1186StorageProof {
    /// Create a new `EIP1186StorageProof` instance.
    pub const fn new(
        key: alloy_serde::storage::JsonStorageKey,
        value: U256,
        proof: Vec<Bytes>,
    ) -> Self {
        Self { key, value, proof }
    }
}

/// Response for EIP-1186 account proof `eth_getProof`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg(feature = "serde")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct EIP1186AccountProofResponse {
    /// The account address.
    pub address: Address,
    /// The account balance.
    pub balance: U256,
    /// The hash of the code of the account.
    pub code_hash: B256,
    /// The account nonce.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub nonce: u64,
    /// The hash of the storage of the account.
    pub storage_hash: B256,
    /// The account proof.
    pub account_proof: Vec<Bytes>,
    /// The storage proof.
    pub storage_proof: Vec<EIP1186StorageProof>,
}

/// Extended account information (used by `parity_allAccountInfo`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtAccountInfo {
    /// Account name
    pub name: String,
    /// Account meta JSON
    pub meta: String,
    /// Account UUID (`None` for address book entries)
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub uuid: Option<String>,
}

/// account derived from a signature
/// as well as information that tells if it is valid for
/// the current chain
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct RecoveredAccount {
    /// address of the recovered account
    pub address: Address,
    /// public key of the recovered account
    pub public_key: B512,
    /// If the signature contains chain replay protection,
    /// And the chain_id encoded within the signature
    /// matches the current chain this would be true, otherwise false.
    pub is_valid_for_current_chain: bool,
}

#[test]
#[cfg(feature = "serde")]
fn test_eip_1186_account_without_storage_proof() {
    let response = r#"{
       "address":"0xc36442b4a4522e871399cd717abdd847ab11fe88",
       "accountProof":["0xf90211a0a3deb2d4417de23e3c64a80ab58fa1cf4b62d7f193e36e507c8cf3794477b5fba0fc7ce8769dcfa9ae8d9d9537098c5cc5477b5920ed494e856049f5783c843c50a0f7d083f1e79a4c0ba1686b97a0e27c79c3a49432d333dc3574d5879cad1ca897a0cd36cf391201df64a786187d99013bdbaf5f0da6bfb8f5f2d6f0f60504f76ad9a03a9f09c92c3cefe87840938dc15fe68a3586d3b28b0f47c7037b6413c95a9feda0decb7e1969758d401af2d1cab14c0951814c094a3da108dd9f606a96840bae2ba060bf0c44ccc3ccbb5ab674841858cc5ea16495529442061295f1cecefd436659a039f8b307e0a295d6d03df089ee8211b52c5ae510d071f17ae5734a7055858002a0508040aef23dfe9c8ab16813258d95c4e765b4a557c2987fb7f3751693f34f4fa0c07e58aa6cd257695cdf147acd800c6197c235e2b5242c22e9da5d86b169d56aa00f2e89ddd874d28e62326ba365fd4f26a86cbd9f867ec0b3de69441ef8870f4ea06c1eb5455e43a36ec41a0372bde915f889cee070b8c8b8a78173d4d7df3ccebaa0cee4848c4119ed28e165e963c5b46ffa6dbeb0b14c8c51726124e7d26ff3f27aa0fc5b82dce2ee5a1691aa92b91dbeec7b2ba94df8116ea985dd7d3f4d5b8292c0a03675e148c987494e22a9767b931611fb1b7c7c287af128ea23aa70b88a1c458ba04f269f556f0f8d9cb2a9a6de52d35cf5a9098f7bb8badb1dc1d496096236aed880",
       "0xf90211a0715ed9b0b002d050084eaecb878f457a348ccd47c7a597134766a7d705303de9a0c49f0fe23b0ca61892d75aebaf7277f00fdfd2022e746bab94de5d049a96edfca0b01f9c91f2bc1373862d7936198a5d11efaf370e2b9bb1dac2134b8e256ecdafa0888395aa7e0f699bb632215f08cdf92840b01e5d8e9a61d18355098cdfd50283a0ba748d609b0018667d311527a2302267209a38b08378f7d833fdead048de0defa098878e5d1461ceddeddf62bd8277586b120b5097202aa243607bc3fc8f30fc0ba0ad4111ee1952b6db0939a384986ee3fb34e0a5fc522955588fc22e159949196fa00fc948964dff427566bad468d62b0498c59df7ca7ae799ab29555d5d829d3742a0766922a88ebc6db7dfb06b03a5b17d0773094e46e42e7f2ba6a0b8567d9f1000a0db25676c4a36591f37c5e16f7199ab16559d82a2bed8c0c6a35f528a3c166bfda0149a5d50d238722e7d44c555169ed32a7f182fcb487ea378b4410a46a63a4e66a06b2298bbfe4972113e7e18cac0a8a39792c1a940ea128218343b8f88057d90aea096b2adb84105ae2aca8a7edf937e91e40872070a8641a74891e64db94d059df0a0ddbb162125ecfbd42edad8d8ef5d5e97ca7c72f54ddc404a61ae318bad0d2108a00e9a68f3e2b0c793d5fcd607edc5c55226d53fdfacd713077d6e01cb38d00d5ba05dc099f1685b2a4b7308e063e8e7905994f5c36969b1c6bfe3780c9878a4d85c80",
       "0xf90211a05fc921be4d63ee07fe47a509e1abf2d69b00b6ea582a755467bf4371c2d2bd1fa0d552faa477e95f4631e2f7247aeb58693d90b03b2eee57e3fe8a9ddbd19ee42da028682c15041aa6ced1a5306aff311f5dbb8bbf7e77615994305ab3132e7842b5a0e5e0316b5046bde22d09676210885c5bea6a71703bf3b4dbac2a7199910f54faa0527fccccef17df926ccfb608f76d3c259848ed43cd24857a59c2a9352b6f1fa4a02b3863355b927b78c80ca379a4f7165bbe1644aaefed8a0bfa2001ae6284b392a09964c73eccc3d12e44dba112e31d8bd3eacbc6a42b4f17985d5b99dff968f24ea0cc426479c7ff0573629dcb2872e57f7438a28bd112a5c3fb2241bdda8031432ba04987fe755f260c2f7218640078af5f6ac4d98c2d0c001e398debc30221b14668a0e811d046c21c6cbaee464bf55553cbf88e70c2bda6951800c75c3896fdeb8e13a04aa8d0ab4946ac86e784e29000a0842cd6eebddaf8a82ece8aa69b72c98cfff5a0dfc010051ddceeec55e4146027c0eb4c72d7c242a103bf1977033ebe00a57b5da039e4da79576281284bf46ce6ca90d47832e4aefea4846615d7a61a7b976c8e3ea0dad1dfff731f7dcf37c499f4afbd5618247289c2e8c14525534b826a13b0a5a6a025f356cbc0469cb4dc326d98479e3b756e4418a67cbbb8ffb2d1abab6b1910e9a03f4082bf1da27b2a76f6bdc930eaaaf1e3f0e4d3135c2a9fb85e301f47f5174d80",
       "0xf90211a0df6448f21c4e19da33f9c64c90bbcc02a499866d344c73576f63e3b4cbd4c000a010efb3b0f1d6365e2e4a389965e114e2a508ef8901f7d6c7564ba88793ff974aa0295bef2313a4f603614a5d5af3c659f63edfaa5b59a6ea2ac1da05f69ff4657ba0d8f16d5ddf4ba09616008148d2993dc50658accc2edf9111b6f464112db5d369a084604d9e06ddb53aeb7b13bb70fbe91f60df6bdc30f59bc7dc57ff37b6fe3325a04c64bd1dbeaecc54f18b23ab1ade2200970757f437e75e285f79a8c405315a14a0868075fc7f73b13863fc653c806f9a20f8e52dce44c15d2c4f94d6711021b985a01e85c49da7a8c91068468779e79b267d93d4fad01f44183353a381207304723ea05fcf186d55c53413f6988b16aa34721f0539f1cf0917f02e9d1a6ec8d3e191ffa00ad581842eab665351913e0afb3bfc070b9e4fad4d354c073f44c4f2a0c425c9a0000cb2066d81bf07f80703a40a5c5012e2c4b387bc53d381d37ee1d0f0a6643ba061f221d01c98721e79c525af5fc2eb9cc648c2ca54bb70520b868e2bdc037967a0e580f297c477df46362eb8e20371d8f0528091454bb5ad00d40368ca3ffdbd1fa079a13d35f79699f9e51d4fa07d03cd9b9dec4de9906559c0470629a663181652a0dbb402183633dbaa73e6e6a6b66bfffc4570763b264d3a702de165032298b858a065d5321015531309bb3abe0235f825d5be4270d2e511dca3b984d1e70ef308d880",
       "0xf90211a06d0adafe89896724704275a42a8a63f0910dce83188add0073f621b8ca1167aaa00de7d4efad36d08f5a0320cdfd964484eba803d9933efae12c292d3ff2d06a20a083341fc12fffccf4b11df314b14f7bcead154525a097493fdf15dde4ec0c0d2aa088b7759fe3aef617828e7abd9e554add2e84ef3e2e024b1a0e2f537fce7d37f9a01e73c28722d825063304c6b51be3a8c7b6312ba8be4c6e99602e623993c014c0a0e50fbe12ddbaf184f3ba0cda971675a55abbf44c73f771bc5824b393262e5255a0b1a937d4c50528cb6aeb80aa5fe83bcfa8c294124a086302caf42cead1f99f96a04c4376b13859af218b5b09ffb33e3465288837c37fa254a46f8d0e75afecae10a0f158c0171bdb454eab6bb6dc5e276e749b6aa550f53b497492c0a392425035c3a0ac496050db1fbb1d34180ee7fd7bed18efa4cf43299390a72dcf530cc3422630a02cacb30ac3b4bab293d31833be4865cd1d1de8db8630edac4af056979cc903aea090cbb538f0f4601289db4cf49485ab3a178044daeae325c525bc3978714a7219a0542021427adbe890896fcc888418a747a555b2a7121fe3c683e07dcf5012e96ca006569c5e3715f52f62dd856dec2136e60c49bbadc1cf9fb625930da3e8f1c16ea0a2539ebb66a2c10c3809626181a2389f043e0b54867cd356eb5f20daaeb521b4a0ab49972dced10010275f2604e6182722dbc426ca1b0ae128defe80c0baefd3c080",
       "0xf90211a006c1d8a7c5deeb435ea0b080aea8b7acb58d2d898e12e3560d399594a77863a1a088105243bc96e1f10baa73d670929a834c51eb7f695cf43f4fab94e73c9a5b8da0fce3a21f09b62d65607bbdabb8d675d58a5f3bfb19ae46510a4ea2205070aa03a0039ae7a999ed83bfdb49b6df7074589059ba6c2eed22bfc6dac8ff5241c71bd7a09feca6f7331b6c147f4fd7bd94de496144b85543d868f47be6345330b3f8ccd3a00e55c30d16438567979c92d387a2b99e51a4026192ccfda2ac87a190c3aee511a0a86c5bb52651e490203c63670b569b2337e838e4d80d455cc83e64571e2552f1a0cfb31ae59b691c15ffd97658bab646ff4b90dbc72a81ec52731b3fbd38d0dd5ba0d83936fc4143cc885be5fa420ef22fb97f6a8dd24e9ece9af965792565a7b2c8a0abb179481f4b29578adb8768aa4f6ba6ed6bd43c7572d7c3405c879a362f1ab1a0506651daa07d44901dfd76c12d302b2242e5ceac385f95ea928f20a0336eccf6a010e8a7f461231438987fb26adc4c5004721dc401dc2b77e9b79d26b1308d0079a09174afa82e6d27dfdde74f556d0e782ae6222dc66104d84ea0f1e21e093578c4a0391e24ed0033cc58f149af753b485de3c8b9e4b3c8e145c308db60e51cabbefca03b0991359019197dd53e3798e55a14c8795d655b0693efd37404cf8f8d979cfba0594d95bbfe8e2ea5040b571010549a233bc33bf959792e1e41c515c65abac14480",
       "0xf90151a0e8ed81735d358657020dd6bc4bc58cf751cc037fa57e1d0c668bf24049e720d280a03e8bf7abdd8a4190a0ee5f92a78bf1dba529312ed66dd7ead7c9be55c81a2db480a006312425a007cda585740355f52db74d0ae43c21d562c599112546e3ffe22f01a023bbbb0ffb33c7a5477ab514c0f4f3c94ba1748a5ea1dc3edc7c4b5330cd70fe80a03ed45ab6045a10fa00b2fba662914f4dedbf3f3a5f2ce1e6e53a12ee3ea21235a01e02c98684cea92a7c0b04a01658530a09d268b395840a66263923e44b93d2b5a0a585db4a911fe6452a4540bf7dc143981ca31035ccb2c51d02eccd021a6163a480a06032919dcb44e22852b6367473bbc3f43311226ac28991a90b9c9da669f9e08a80a0146aee58a46c30bc84f6e99cd76bf29b3bd238053102679498a3ea15d4ff6d53a04cf57cfdc046c135004b9579059c84b2d902a51fb6feaed51ea272f0ca1cdc648080",
       "0xf871a059ce2e1f470580853d88511bf8672f9ffaefadd80bc07b2e3d5a18c3d7812007a0867e978faf3461d2238ccf8d6a138406cb6d8bd36dfa60caddb62af14447a6f880808080a0fc6209fdaa57d224ee35f73e96469a7f95760a54d5de3da07953430b001aee6980808080808080808080",
       "0xf8669d20852b2b985cd8c252fddae2acb4f798d0fecdcb1e2da53726332eb559b846f8440180a079fe22fe88fc4b45db10ce94d975e02e8a42b57dc190f8ae15e321f72bbc08eaa0692e658b31cbe3407682854806658d315d61a58c7e4933a2f91d383dc00736c6"],
       "balance":"0x0",
       "codeHash":"0x692e658b31cbe3407682854806658d315d61a58c7e4933a2f91d383dc00736c6",
       "nonce":"0x1",
       "storageHash":"0x79fe22fe88fc4b45db10ce94d975e02e8a42b57dc190f8ae15e321f72bbc08ea",
       "storageProof":[]
    }"#;
    let val = serde_json::from_str::<EIP1186AccountProofResponse>(response).unwrap();
    serde_json::to_value(val).unwrap();
}
