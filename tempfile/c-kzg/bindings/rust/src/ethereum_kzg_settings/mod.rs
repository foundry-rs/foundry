use crate::{
    bindings::{BYTES_PER_G1_POINT, BYTES_PER_G2_POINT, NUM_G1_POINTS, NUM_G2_POINTS},
    KzgSettings,
};
use alloc::{boxed::Box, sync::Arc};
use once_cell::race::OnceBox;

/// Returns default Ethereum mainnet KZG settings.
///
/// If you need a cloneable settings use `ethereum_kzg_settings_arc` instead.
pub fn ethereum_kzg_settings() -> &'static KzgSettings {
    ethereum_kzg_settings_inner().as_ref()
}

/// Returns default Ethereum mainnet KZG settings as an `Arc`.
///
/// It is useful for sharing the settings in multiple places.
pub fn ethereum_kzg_settings_arc() -> Arc<KzgSettings> {
    ethereum_kzg_settings_inner().clone()
}

fn ethereum_kzg_settings_inner() -> &'static Arc<KzgSettings> {
    static DEFAULT: OnceBox<Arc<KzgSettings>> = OnceBox::new();
    DEFAULT.get_or_init(|| {
        let settings =
            KzgSettings::load_trusted_setup(ETH_G1_POINTS.as_ref(), ETH_G2_POINTS.as_ref())
                .expect("failed to load default trusted setup");
        Box::new(Arc::new(settings))
    })
}

type G1Points = [[u8; BYTES_PER_G1_POINT]; NUM_G1_POINTS];
type G2Points = [[u8; BYTES_PER_G2_POINT]; NUM_G2_POINTS];

/// Default G1 points.
const ETH_G1_POINTS: &G1Points = {
    const BYTES: &[u8] = include_bytes!("./g1_points.bin");
    assert!(BYTES.len() == core::mem::size_of::<G1Points>());
    unsafe { &*BYTES.as_ptr().cast::<G1Points>() }
};

/// Default G2 points.
const ETH_G2_POINTS: &G2Points = {
    const BYTES: &[u8] = include_bytes!("./g2_points.bin");
    assert!(BYTES.len() == core::mem::size_of::<G2Points>());
    unsafe { &*BYTES.as_ptr().cast::<G2Points>() }
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bindings::BYTES_PER_BLOB, Blob, KzgCommitment, KzgProof, KzgSettings};
    use std::path::Path;

    #[test]
    pub fn compare_default_with_file() {
        let ts_settings =
            KzgSettings::load_trusted_setup_file(Path::new("src/trusted_setup.txt")).unwrap();
        let eth_settings = ethereum_kzg_settings();
        let blob = Blob::new([1u8; BYTES_PER_BLOB]);

        // generate commitment
        let ts_commitment = KzgCommitment::blob_to_kzg_commitment(&blob, &ts_settings)
            .unwrap()
            .to_bytes();
        let eth_commitment = KzgCommitment::blob_to_kzg_commitment(&blob, &eth_settings)
            .unwrap()
            .to_bytes();
        assert_eq!(ts_commitment, eth_commitment);

        // generate proof
        let ts_proof = KzgProof::compute_blob_kzg_proof(&blob, &ts_commitment, &ts_settings)
            .unwrap()
            .to_bytes();
        let eth_proof = KzgProof::compute_blob_kzg_proof(&blob, &eth_commitment, &eth_settings)
            .unwrap()
            .to_bytes();
        assert_eq!(ts_proof, eth_proof);
    }
}
