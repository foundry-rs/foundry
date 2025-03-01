//! Support for ECDSA signatures with low-S normalization.

use crate::Signature;
use elliptic_curve::PrimeCurve;

/// ECDSA signature with low-S normalization applied.
#[derive(Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct NormalizedSignature<C: PrimeCurve> {
    inner: Signature<C>,
}
