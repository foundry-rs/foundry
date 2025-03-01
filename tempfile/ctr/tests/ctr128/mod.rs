use aes::{Aes128, Aes256};
use ctr::{flavors, Ctr128BE, CtrCore};

cipher::stream_cipher_test!(aes128_ctr_core, "aes128-ctr", Ctr128BE<Aes128>);
cipher::stream_cipher_test!(aes256_ctr_core, "aes256-ctr", Ctr128BE<Aes256>);
cipher::stream_cipher_seek_test!(aes128_ctr_seek, Ctr128BE<Aes128>);
cipher::stream_cipher_seek_test!(aes256_ctr_seek, Ctr128BE<Aes256>);
cipher::iv_state_test!(
    aes128_ctr_iv_state,
    CtrCore<Aes128, flavors::Ctr128BE>,
    apply_ks,
);
