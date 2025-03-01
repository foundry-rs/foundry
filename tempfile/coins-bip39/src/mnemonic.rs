use crate::{Wordlist, WordlistError};
use bitvec::prelude::*;
use coins_bip32::{path::DerivationPath, xkeys::XPriv, Bip32Error};
use hmac::Hmac;
use pbkdf2::pbkdf2;
use rand::Rng;
use sha2::{Digest, Sha256, Sha512};
use std::{convert::TryInto, marker::PhantomData};
use thiserror::Error;

const PBKDF2_ROUNDS: u32 = 2048;
const PBKDF2_BYTES: usize = 64;

#[derive(Debug, Error)]
/// The error type returned while interacting with mnemonics.
pub enum MnemonicError {
    /// Describes the error when the mnemonic's entropy length is invalid.
    #[error("the mnemonic's entropy length `{0}` is invalid")]
    InvalidEntropyLength(usize),
    /// Describes the error when the given phrase is invalid.
    #[error("the phrase `{0}` is invalid")]
    InvalidPhrase(String),
    /// Describes the error when the word count provided for mnemonic generation is invalid.
    #[error("invalid word count (expected 12, 15, 18, 21, 24, found `{0}`")]
    InvalidWordCount(usize),
    /// Describes an error propagated from the wordlist errors.
    #[error(transparent)]
    WordlistError(#[from] WordlistError),
    /// Describes an error propagated from the BIP-32 crate.
    #[error(transparent)]
    Bip32Error(#[from] Bip32Error),
}

/// Holds valid entropy lengths for a mnemonic
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Entropy {
    /// Sixteen bytes of entropy
    Sixteen([u8; 16]),
    /// Twenty bytes of entropy
    Twenty([u8; 20]),
    /// TwentyFour bytes of entropy
    TwentyFour([u8; 24]),
    /// TwentyEight bytes of entropy
    TwentyEight([u8; 28]),
    /// ThirtyTwo bytes of entropy
    ThirtyTwo([u8; 32]),
}

impl std::fmt::Debug for Entropy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sixteen(_) => f.debug_tuple("Sixteen bytes").finish(),
            Self::Twenty(_) => f.debug_tuple("Twenty bytes").finish(),
            Self::TwentyFour(_) => f.debug_tuple("Twenty-four bytes").finish(),
            Self::TwentyEight(_) => f.debug_tuple("Twenty-eight bytes").finish(),
            Self::ThirtyTwo(_) => f.debug_tuple("Thirty-two bytes").finish(),
        }
    }
}

impl std::convert::TryFrom<&[u8]> for Entropy {
    type Error = MnemonicError;

    fn try_from(buf: &[u8]) -> Result<Self, Self::Error> {
        match buf.len() {
            16 | 17 => Ok(Entropy::Sixteen(buf[..16].try_into().expect("len checked"))),
            20 | 21 => Ok(Entropy::Twenty(buf[..20].try_into().expect("len checked"))),
            24 | 25 => Ok(Entropy::TwentyFour(
                buf[..24].try_into().expect("len checked"),
            )),
            28 | 29 => Ok(Entropy::TwentyEight(
                buf[..28].try_into().expect("len checked"),
            )),
            32 | 33 => Ok(Entropy::ThirtyTwo(
                buf[..32].try_into().expect("len checked"),
            )),
            _ => Err(MnemonicError::InvalidEntropyLength(buf.len())),
        }
    }
}

impl From<[u8; 16]> for Entropy {
    fn from(val: [u8; 16]) -> Entropy {
        Entropy::Sixteen(val)
    }
}

impl From<[u8; 20]> for Entropy {
    fn from(val: [u8; 20]) -> Entropy {
        Entropy::Twenty(val)
    }
}

impl From<[u8; 24]> for Entropy {
    fn from(val: [u8; 24]) -> Entropy {
        Entropy::TwentyFour(val)
    }
}

impl From<[u8; 28]> for Entropy {
    fn from(val: [u8; 28]) -> Entropy {
        Entropy::TwentyEight(val)
    }
}

impl From<[u8; 32]> for Entropy {
    fn from(val: [u8; 32]) -> Entropy {
        Entropy::ThirtyTwo(val)
    }
}

impl AsRef<[u8]> for Entropy {
    fn as_ref(&self) -> &[u8] {
        match self {
            Entropy::Sixteen(arr) => arr.as_ref(),
            Entropy::Twenty(arr) => arr.as_ref(),
            Entropy::TwentyFour(arr) => arr.as_ref(),
            Entropy::TwentyEight(arr) => arr.as_ref(),
            Entropy::ThirtyTwo(arr) => arr.as_ref(),
        }
    }
}

impl Entropy {
    /// Attempts to instantiate Entropy from a slice. Fails if the slice is not
    /// a valid entropy length
    pub fn from_slice(buf: impl AsRef<[u8]>) -> Result<Self, MnemonicError> {
        buf.as_ref().try_into()
    }

    /// Instantiates new entropy from an RNG. Fails if the specified bytes is
    /// not a valid entropy length
    pub fn from_rng<R: Rng>(bytes: usize, rng: &mut R) -> Result<Self, MnemonicError> {
        match bytes {
            16 => Ok(Entropy::Sixteen(rng.gen())),
            20 => Ok(Entropy::Twenty(rng.gen())),
            24 => Ok(Entropy::TwentyFour(rng.gen())),
            28 => Ok(Entropy::TwentyEight(rng.gen())),
            32 => Ok(Entropy::ThirtyTwo(rng.gen())),
            _ => Err(MnemonicError::InvalidEntropyLength(bytes)),
        }
    }

    /// Computes the number of words in the mnemonic
    pub const fn words(&self) -> usize {
        match self {
            Entropy::Sixteen(_) => 12,
            Entropy::Twenty(_) => 15,
            Entropy::TwentyFour(_) => 18,
            Entropy::TwentyEight(_) => 21,
            Entropy::ThirtyTwo(_) => 24,
        }
    }

    /// Returns the length of the entropy array
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> usize {
        match self {
            Entropy::Sixteen(_) => 16,
            Entropy::Twenty(_) => 20,
            Entropy::TwentyFour(_) => 24,
            Entropy::TwentyEight(_) => 28,
            Entropy::ThirtyTwo(_) => 32,
        }
    }
}

/// Mnemonic represents entropy that can be represented as a phrase. A mnemonic can be used to
/// deterministically generate an extended private key or derive its child keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mnemonic<W>
where
    W: Wordlist,
{
    /// Entropy used to generate mnemonic.
    entropy: Entropy,
    /// Wordlist used to produce phrases from entropy.
    _wordlist: PhantomData<W>,
}

impl<W> std::str::FromStr for Mnemonic<W>
where
    W: Wordlist,
{
    type Err = MnemonicError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new_from_phrase(s)
    }
}

impl<W> Mnemonic<W>
where
    W: Wordlist,
{
    /// Returns a new mnemonic generated using the provided random number generator.
    pub fn new<R: Rng>(rng: &mut R) -> Self {
        let entropy: [u8; 16] = rng.gen();
        Self {
            entropy: entropy.into(),
            _wordlist: PhantomData,
        }
    }

    /// Returns a new mnemonic instantiated from an existing entropy.
    pub const fn new_from_entropy(entropy: Entropy) -> Self {
        Self {
            entropy,
            _wordlist: PhantomData,
        }
    }

    /// Returns a new mnemonic given the word count, generated using the provided random number
    /// generator.
    pub fn new_with_count<R: Rng>(rng: &mut R, count: usize) -> Result<Self, MnemonicError> {
        let bytes: usize = match count {
            12 => 16,
            15 => 20,
            18 => 24,
            21 => 28,
            24 => 32,
            wc => return Err(MnemonicError::InvalidWordCount(wc)),
        };
        Ok(Self {
            entropy: Entropy::from_rng(bytes, rng)?,
            _wordlist: PhantomData,
        })
    }

    /// Returns a new mnemonic for a given phrase. The 12-24 space-separated words are used to
    /// calculate the entropy that must have produced it.
    pub fn new_from_phrase(phrase: &str) -> Result<Self, MnemonicError> {
        let words = phrase.split(' ').collect::<Vec<&str>>();

        let mut entropy: BitVec<u8, Msb0> = BitVec::new();
        for word in words {
            let index = W::get_index(word)?;
            let index_u8: [u8; 2] = (index as u16).to_be_bytes();

            // 11-bits per word as per BIP-39, and max index (2047) can be represented in 11-bits.
            let index_slice = &BitVec::from_slice(&index_u8)[5..];

            entropy.append(&mut BitVec::<u8, Msb0>::from_bitslice(index_slice));
        }

        let mnemonic = Self {
            entropy: Entropy::from_slice(entropy.as_raw_slice())?,
            _wordlist: PhantomData,
        };

        // Ensures the checksum word matches the checksum word in the given phrase.
        match phrase == mnemonic.to_phrase() {
            true => Ok(mnemonic),
            false => Err(MnemonicError::InvalidPhrase(phrase.into())),
        }
    }

    /// Converts the mnemonic into phrase.
    pub fn to_phrase(&self) -> String {
        let length = self.word_count();

        // Compute checksum. Checksum is the most significant (ENTROPY_BYTES/4) bits. That is also
        // equivalent to (WORD_COUNT/3).
        let mut hasher = Sha256::new();
        hasher.update(self.entropy.as_ref());
        let hash = hasher.finalize();
        let hash_0 = BitVec::<u8, Msb0>::from_element(hash[0]);
        let (checksum, _) = hash_0.split_at(length / 3);

        // Convert the entropy bytes into bits and append the checksum.
        let mut encoding = BitVec::<u8, Msb0>::from_slice(self.entropy.as_ref());
        encoding.append(&mut checksum.to_bitvec());

        // Compute the phrase in 11 bit chunks which encode an index into the word list
        let wordlist = W::get_all();
        let phrase = encoding
            .chunks(11)
            .map(|index| {
                let index = index.load_be::<u16>();
                wordlist[index as usize]
            })
            .collect::<Vec<&str>>();

        phrase.join(" ")
    }

    const fn word_count(&self) -> usize {
        self.entropy.words()
    }

    /// Returns the master private key of the corresponding mnemonic.
    pub fn master_key(&self, password: Option<&str>) -> Result<XPriv, MnemonicError> {
        Ok(XPriv::root_from_seed(
            self.to_seed(password)?.as_slice(),
            None,
        )?)
    }

    /// Returns the derived child private key of the corresponding mnemonic at the given index.
    pub fn derive_key<E, P>(&self, path: P, password: Option<&str>) -> Result<XPriv, MnemonicError>
    where
        E: Into<Bip32Error>,
        P: TryInto<DerivationPath, Error = E>,
    {
        Ok(self.master_key(password)?.derive_path(path)?)
    }

    /// Convert to a bip23 seed
    pub fn to_seed(&self, password: Option<&str>) -> Result<[u8; PBKDF2_BYTES], MnemonicError> {
        let mut seed = [0u8; PBKDF2_BYTES];
        let salt = format!("mnemonic{}", password.unwrap_or(""));
        pbkdf2::<Hmac<Sha512>>(
            self.to_phrase().as_bytes(),
            salt.as_bytes(),
            PBKDF2_ROUNDS,
            &mut seed,
        )
        .expect("cannot have invalid length");

        Ok(seed)
    }
}

#[cfg(all(test, feature = "english"))]
mod tests {
    use crate::English;
    use coins_bip32::enc::{MainnetEncoder, XKeyEncoder};

    use super::*;

    type W = English;

    #[test]
    #[should_panic(expected = "InvalidWordCount(11)")]
    fn test_invalid_word_count() {
        let mut rng = rand::thread_rng();
        let _mnemonic = Mnemonic::<W>::new_with_count(&mut rng, 11usize).unwrap();
    }

    #[test]
    #[should_panic(expected = "WordlistError(InvalidWord(\"mnemonic\"))")]
    fn test_invalid_word_in_phrase() {
        let phrase = "mnemonic zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo";
        let _mnemonic: Mnemonic<English> = phrase.parse().unwrap();
    }

    #[test]
    #[should_panic(
        expected = "InvalidPhrase(\"zoo zone zoo zone zoo zone zoo zone zoo zone zoo zone\")"
    )]
    fn test_invalid_phrase() {
        let phrase = "zoo zone zoo zone zoo zone zoo zone zoo zone zoo zone";
        let _mnemonic: Mnemonic<English> = phrase.parse().unwrap();
    }

    // (entropy, phrase, seed, extended_private_key)
    const TESTCASES: [(&str, &str, &str, &str); 26] = [
        (
            "00000000000000000000000000000000",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04",
            "xprv9s21ZrQH143K3h3fDYiay8mocZ3afhfULfb5GX8kCBdno77K4HiA15Tg23wpbeF1pLfs1c5SPmYHrEpTuuRhxMwvKDwqdKiGJS9XFKzUsAF"
        ),
        (
            "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            "2e8905819b8723fe2c1d161860e5ee1830318dbf49a83bd451cfb8440c28bd6fa457fe1296106559a3c80937a1c1069be3a3a5bd381ee6260e8d9739fce1f607",
            "xprv9s21ZrQH143K2gA81bYFHqU68xz1cX2APaSq5tt6MFSLeXnCKV1RVUJt9FWNTbrrryem4ZckN8k4Ls1H6nwdvDTvnV7zEXs2HgPezuVccsq"
        ),
        (
            "80808080808080808080808080808080",
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
            "d71de856f81a8acc65e6fc851a38d4d7ec216fd0796d0a6827a3ad6ed5511a30fa280f12eb2e47ed2ac03b5c462a0358d18d69fe4f985ec81778c1b370b652a8",
            "xprv9s21ZrQH143K2shfP28KM3nr5Ap1SXjz8gc2rAqqMEynmjt6o1qboCDpxckqXavCwdnYds6yBHZGKHv7ef2eTXy461PXUjBFQg6PrwY4Gzq"
        ),
        (
            "ffffffffffffffffffffffffffffffff",
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
            "ac27495480225222079d7be181583751e86f571027b0497b5b5d11218e0a8a13332572917f0f8e5a589620c6f15b11c61dee327651a14c34e18231052e48c069",
            "xprv9s21ZrQH143K2V4oox4M8Zmhi2Fjx5XK4Lf7GKRvPSgydU3mjZuKGCTg7UPiBUD7ydVPvSLtg9hjp7MQTYsW67rZHAXeccqYqrsx8LcXnyd"
        ),
        (
            "000000000000000000000000000000000000000000000000",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon agent",
            "035895f2f481b1b0f01fcf8c289c794660b289981a78f8106447707fdd9666ca06da5a9a565181599b79f53b844d8a71dd9f439c52a3d7b3e8a79c906ac845fa",
            "xprv9s21ZrQH143K3mEDrypcZ2usWqFgzKB6jBBx9B6GfC7fu26X6hPRzVjzkqkPvDqp6g5eypdk6cyhGnBngbjeHTe4LsuLG1cCmKJka5SMkmU"
        ),
        (
            "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal will",
            "f2b94508732bcbacbcc020faefecfc89feafa6649a5491b8c952cede496c214a0c7b3c392d168748f2d4a612bada0753b52a1c7ac53c1e93abd5c6320b9e95dd",
            "xprv9s21ZrQH143K3Lv9MZLj16np5GzLe7tDKQfVusBni7toqJGcnKRtHSxUwbKUyUWiwpK55g1DUSsw76TF1T93VT4gz4wt5RM23pkaQLnvBh7"
        ),
        (
            "808080808080808080808080808080808080808080808080",
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter always",
            "107d7c02a5aa6f38c58083ff74f04c607c2d2c0ecc55501dadd72d025b751bc27fe913ffb796f841c49b1d33b610cf0e91d3aa239027f5e99fe4ce9e5088cd65",
            "xprv9s21ZrQH143K3VPCbxbUtpkh9pRG371UCLDz3BjceqP1jz7XZsQ5EnNkYAEkfeZp62cDNj13ZTEVG1TEro9sZ9grfRmcYWLBhCocViKEJae"
        ),
        (
            "ffffffffffffffffffffffffffffffffffffffffffffffff",
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo when",
            "0cd6e5d827bb62eb8fc1e262254223817fd068a74b5b449cc2f667c3f1f985a76379b43348d952e2265b4cd129090758b3e3c2c49103b5051aac2eaeb890a528",
            "xprv9s21ZrQH143K36Ao5jHRVhFGDbLP6FCx8BEEmpru77ef3bmA928BxsqvVM27WnvvyfWywiFN8K6yToqMaGYfzS6Db1EHAXT5TuyCLBXUfdm"
        ),
        (
            "0000000000000000000000000000000000000000000000000000000000000000",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
            "bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8",
            "xprv9s21ZrQH143K32qBagUJAMU2LsHg3ka7jqMcV98Y7gVeVyNStwYS3U7yVVoDZ4btbRNf4h6ibWpY22iRmXq35qgLs79f312g2kj5539ebPM"
        ),
        (
            "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title",
            "bc09fca1804f7e69da93c2f2028eb238c227f2e9dda30cd63699232578480a4021b146ad717fbb7e451ce9eb835f43620bf5c514db0f8add49f5d121449d3e87",
            "xprv9s21ZrQH143K3Y1sd2XVu9wtqxJRvybCfAetjUrMMco6r3v9qZTBeXiBZkS8JxWbcGJZyio8TrZtm6pkbzG8SYt1sxwNLh3Wx7to5pgiVFU"
        ),
        (
            "8080808080808080808080808080808080808080808080808080808080808080",
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
            "c0c519bd0e91a2ed54357d9d1ebef6f5af218a153624cf4f2da911a0ed8f7a09e2ef61af0aca007096df430022f7a2b6fb91661a9589097069720d015e4e982f",
            "xprv9s21ZrQH143K3CSnQNYC3MqAAqHwxeTLhDbhF43A4ss4ciWNmCY9zQGvAKUSqVUf2vPHBTSE1rB2pg4avopqSiLVzXEU8KziNnVPauTqLRo"
        ),
        (
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
            "dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad",
            "xprv9s21ZrQH143K2WFF16X85T2QCpndrGwx6GueB72Zf3AHwHJaknRXNF37ZmDrtHrrLSHvbuRejXcnYxoZKvRquTPyp2JiNG3XcjQyzSEgqCB"
        ),
        (
            "9e885d952ad362caeb4efe34a8e91bd2",
            "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
            "274ddc525802f7c828d8ef7ddbcdc5304e87ac3535913611fbbfa986d0c9e5476c91689f9c8a54fd55bd38606aa6a8595ad213d4c9c9f9aca3fb217069a41028",
            "xprv9s21ZrQH143K2oZ9stBYpoaZ2ktHj7jLz7iMqpgg1En8kKFTXJHsjxry1JbKH19YrDTicVwKPehFKTbmaxgVEc5TpHdS1aYhB2s9aFJBeJH"
        ),
        (
            "6610b25967cdcca9d59875f5cb50b0ea75433311869e930b",
            "gravity machine north sort system female filter attitude volume fold club stay feature office ecology stable narrow fog",
            "628c3827a8823298ee685db84f55caa34b5cc195a778e52d45f59bcf75aba68e4d7590e101dc414bc1bbd5737666fbbef35d1f1903953b66624f910feef245ac",
            "xprv9s21ZrQH143K3uT8eQowUjsxrmsA9YUuQQK1RLqFufzybxD6DH6gPY7NjJ5G3EPHjsWDrs9iivSbmvjc9DQJbJGatfa9pv4MZ3wjr8qWPAK"
        ),
        (
            "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
            "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
            "64c87cde7e12ecf6704ab95bb1408bef047c22db4cc7491c4271d170a1b213d20b385bc1588d9c7b38f1b39d415665b8a9030c9ec653d75e65f847d8fc1fc440",
            "xprv9s21ZrQH143K2XTAhys3pMNcGn261Fi5Ta2Pw8PwaVPhg3D8DWkzWQwjTJfskj8ofb81i9NP2cUNKxwjueJHHMQAnxtivTA75uUFqPFeWzk"
        ),
        (
            "c0ba5a8e914111210f2bd131f3d5e08d",
            "scheme spot photo card baby mountain device kick cradle pact join borrow",
            "ea725895aaae8d4c1cf682c1bfd2d358d52ed9f0f0591131b559e2724bb234fca05aa9c02c57407e04ee9dc3b454aa63fbff483a8b11de949624b9f1831a9612",
            "xprv9s21ZrQH143K3FperxDp8vFsFycKCRcJGAFmcV7umQmcnMZaLtZRt13QJDsoS5F6oYT6BB4sS6zmTmyQAEkJKxJ7yByDNtRe5asP2jFGhT6"
        ),
        (
            "6d9be1ee6ebd27a258115aad99b7317b9c8d28b6d76431c3",
            "horn tenant knee talent sponsor spell gate clip pulse soap slush warm silver nephew swap uncle crack brave",
            "fd579828af3da1d32544ce4db5c73d53fc8acc4ddb1e3b251a31179cdb71e853c56d2fcb11aed39898ce6c34b10b5382772db8796e52837b54468aeb312cfc3d",
            "xprv9s21ZrQH143K3R1SfVZZLtVbXEB9ryVxmVtVMsMwmEyEvgXN6Q84LKkLRmf4ST6QrLeBm3jQsb9gx1uo23TS7vo3vAkZGZz71uuLCcywUkt"
        ),
        (
            "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
            "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
            "72be8e052fc4919d2adf28d5306b5474b0069df35b02303de8c1729c9538dbb6fc2d731d5f832193cd9fb6aeecbc469594a70e3dd50811b5067f3b88b28c3e8d",
            "xprv9s21ZrQH143K2WNnKmssvZYM96VAr47iHUQUTUyUXH3sAGNjhJANddnhw3i3y3pBbRAVk5M5qUGFr4rHbEWwXgX4qrvrceifCYQJbbFDems"
        ),
        (
            "23db8160a31d3e0dca3688ed941adbf3",
            "cat swing flag economy stadium alone churn speed unique patch report train",
            "deb5f45449e615feff5640f2e49f933ff51895de3b4381832b3139941c57b59205a42480c52175b6efcffaa58a2503887c1e8b363a707256bdd2b587b46541f5",
            "xprv9s21ZrQH143K4G28omGMogEoYgDQuigBo8AFHAGDaJdqQ99QKMQ5J6fYTMfANTJy6xBmhvsNZ1CJzRZ64PWbnTFUn6CDV2FxoMDLXdk95DQ"
        ),
        (
            "8197a4a47f0425faeaa69deebc05ca29c0a5b5cc76ceacc0",
            "light rule cinnamon wrap drastic word pride squirrel upgrade then income fatal apart sustain crack supply proud access",
            "4cbdff1ca2db800fd61cae72a57475fdc6bab03e441fd63f96dabd1f183ef5b782925f00105f318309a7e9c3ea6967c7801e46c8a58082674c860a37b93eda02",
            "xprv9s21ZrQH143K3wtsvY8L2aZyxkiWULZH4vyQE5XkHTXkmx8gHo6RUEfH3Jyr6NwkJhvano7Xb2o6UqFKWHVo5scE31SGDCAUsgVhiUuUDyh"
        ),
        (
            "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
            "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
            "26e975ec644423f4a4c4f4215ef09b4bd7ef924e85d1d17c4cf3f136c2863cf6df0a475045652c57eb5fb41513ca2a2d67722b77e954b4b3fc11f7590449191d",
            "xprv9s21ZrQH143K3rEfqSM4QZRVmiMuSWY9wugscmaCjYja3SbUD3KPEB1a7QXJoajyR2T1SiXU7rFVRXMV9XdYVSZe7JoUXdP4SRHTxsT1nzm"
        ),
        (
            "f30f8c1da665478f49b001d94c5fc452",
            "vessel ladder alter error federal sibling chat ability sun glass valve picture",
            "2aaa9242daafcee6aa9d7269f17d4efe271e1b9a529178d7dc139cd18747090bf9d60295d0ce74309a78852a9caadf0af48aae1c6253839624076224374bc63f",
            "xprv9s21ZrQH143K2QWV9Wn8Vvs6jbqfF1YbTCdURQW9dLFKDovpKaKrqS3SEWsXCu6ZNky9PSAENg6c9AQYHcg4PjopRGGKmdD313ZHszymnps"
        ),
        (
            "c10ec20dc3cd9f652c7fac2f1230f7a3c828389a14392f05",
            "scissors invite lock maple supreme raw rapid void congress muscle digital elegant little brisk hair mango congress clump",
            "7b4a10be9d98e6cba265566db7f136718e1398c71cb581e1b2f464cac1ceedf4f3e274dc270003c670ad8d02c4558b2f8e39edea2775c9e232c7cb798b069e88",
            "xprv9s21ZrQH143K4aERa2bq7559eMCCEs2QmmqVjUuzfy5eAeDX4mqZffkYwpzGQRE2YEEeLVRoH4CSHxianrFaVnMN2RYaPUZJhJx8S5j6puX"
        ),
        (
            "f585c11aec520db57dd353c69554b21a89b20fb0650966fa0a9d6f74fd989d8f",
            "void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold",
            "01f5bced59dec48e362f2c45b5de68b9fd6c92c6634f44d6d40aab69056506f0e35524a518034ddc1192e1dacd32c1ed3eaa3c3b131c88ed8e7e54c49a5d0998",
            "xprv9s21ZrQH143K39rnQJknpH1WEPFJrzmAqqasiDcVrNuk926oizzJDDQkdiTvNPr2FYDYzWgiMiC63YmfPAa2oPyNB23r2g7d1yiK6WpqaQS"
        ),
        (
            "d292b36884b647974ff2167649e8255c8226a942",
            "spoon night surface annual good slight divert drift iron exercise announce ribbon carbon feed answer",
            "1c662e030a65b8e943a7f7fb304a1ecf415dcd1c99bfd587efae245ca9270058e853df0070abe61af152756c63a0b67ed74bf6e916b112289499e6052ccacc19",
            "xprv9s21ZrQH143K3pskpuVw5DMEBZ1hWZnVxwTpPc4QqjCPHbinjx5dyosHqPubQbGRoKdPci6hYRdr2QNDc2GwhCpSEAtKMrsjiBbYJJLfFj9"
        ),
        (
            "608945c274e181d9376c651255db6481ccb525532554eaea611cbbd1",
            "gauge enforce identify truth blossom uncle tank million banner put summer adjust slender naive erode pride turtle fantasy elbow jeans bar",
            "79da8e9aaeea7b28f9045fb0e4763fef5a7aae300b34c9f32aa8bb9a4aacd99896943beb22bbf9b50646658fd72cdf993b16a7cb5b7a77d1b443cf41f5183067",
            "xprv9s21ZrQH143K2Cy1ePyrB2tRcm97F6YFMzDZkhy9QS6PeCDtiDuZLrtt9WBfWhXEz8W5KbSnF7nWBKFzStfs8UPeyzbrCPPbHLC25HB8aFe"
        )
    ];

    #[test]
    fn test_from_phrase() {
        TESTCASES.iter().for_each(|(entropy_str, phrase, _, _)| {
            let expected_entropy: Entropy = hex::decode(entropy_str)
                .unwrap()
                .as_slice()
                .try_into()
                .unwrap();
            dbg!(&phrase);
            let mnemonic: Mnemonic<English> = phrase.parse().unwrap();
            assert_eq!(mnemonic.entropy, expected_entropy);
            assert_eq!(mnemonic.to_phrase(), phrase.to_string());
        })
    }

    #[test]
    fn test_to_phrase() {
        TESTCASES
            .iter()
            .for_each(|(entropy_str, expected_phrase, _, _)| {
                let entropy: Entropy = hex::decode(entropy_str)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let mnemonic = Mnemonic::<W> {
                    entropy,
                    _wordlist: PhantomData,
                };
                assert_eq!(mnemonic.entropy, entropy);
                assert_eq!(mnemonic.to_phrase(), expected_phrase.to_string())
            })
    }

    #[test]
    fn test_to_seed() {
        TESTCASES
            .iter()
            .for_each(|(entropy_str, _, expected_seed, _)| {
                let entropy: Entropy = hex::decode(entropy_str)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let mnemonic = Mnemonic::<W> {
                    entropy,
                    _wordlist: PhantomData,
                };
                assert_eq!(
                    expected_seed,
                    &hex::encode(mnemonic.to_seed(Some("TREZOR")).unwrap()),
                )
            });
    }

    #[test]
    fn test_master_key() {
        TESTCASES
            .iter()
            .for_each(|(_, phrase, _, expected_master_key)| {
                let mnemonic: Mnemonic<English> = phrase.parse().unwrap();
                let master_key = mnemonic.master_key(Some("TREZOR")).unwrap();
                assert_eq!(
                    MainnetEncoder::xpriv_from_base58(expected_master_key).unwrap(),
                    master_key,
                );
            });
    }

    #[test]
    fn test_derive_key_try_into_derivation() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::<W>::new_from_phrase(phrase).unwrap();
        mnemonic.derive_key(0, None).unwrap();
        mnemonic.derive_key("m/44'/61'/0'/0", None).unwrap();
    }
}
