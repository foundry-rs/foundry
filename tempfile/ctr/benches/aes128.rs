#![feature(test)]
extern crate test;

cipher::stream_cipher_bench!(
    ctr::Ctr32LE<aes::Aes128>;
    ctr_32le_aes128_stream_bench1_16b 16;
    ctr_32le_aes128_stream_bench2_256b 256;
    ctr_32le_aes128_stream_bench3_1kib 1024;
    ctr_32le_aes128_stream_bench4_16kib 16384;
);

cipher::stream_cipher_bench!(
    ctr::Ctr64LE<aes::Aes128>;
    ctr_64le_aes128_stream_bench1_16b 16;
    ctr_64le_aes128_stream_bench2_256b 256;
    ctr_64le_aes128_stream_bench3_1kib 1024;
    ctr_64le_aes128_stream_bench4_16kib 16384;
);

cipher::stream_cipher_bench!(
    ctr::Ctr128BE<aes::Aes128>;
    ctr_128be_aes128_stream_bench1_16b 16;
    ctr_128be_aes128_stream_bench2_256b 256;
    ctr_128be_aes128_stream_bench3_1kib 1024;
    ctr_128be_aes128_stream_bench4_16kib 16384;
);
