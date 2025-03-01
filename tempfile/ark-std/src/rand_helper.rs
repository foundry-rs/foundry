#[cfg(feature = "std")]
use rand::RngCore;
use rand::{
    distributions::{Distribution, Standard},
    prelude::StdRng,
    Rng,
};

pub use rand;

pub trait UniformRand: Sized {
    fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self;
}

impl<T> UniformRand for T
where
    Standard: Distribution<T>,
{
    #[inline]
    fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self {
        rng.sample(Standard)
    }
}

fn test_rng_helper() -> StdRng {
    use rand::SeedableRng;
    // arbitrary seed
    let seed = [
        1, 0, 0, 0, 23, 0, 0, 0, 200, 1, 0, 0, 210, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0,
    ];
    rand::rngs::StdRng::from_seed(seed)
}

/// Should be used only for tests, not for any real world usage.
#[cfg(not(feature = "std"))]
pub fn test_rng() -> impl rand::Rng {
    test_rng_helper()
}

/// Should be used only for tests, not for any real world usage.
#[cfg(feature = "std")]
pub fn test_rng() -> impl rand::Rng {
    #[cfg(any(feature = "getrandom", test))]
    {
        let is_deterministic =
            std::env::vars().any(|(key, val)| key == "DETERMINISTIC_TEST_RNG" && val == "1");
        if is_deterministic {
            RngWrapper::Deterministic(test_rng_helper())
        } else {
            RngWrapper::Randomized(rand::thread_rng())
        }
    }
    #[cfg(not(any(feature = "getrandom", test)))]
    {
        RngWrapper::Deterministic(test_rng_helper())
    }
}

/// Helper wrapper to enable `test_rng` to return `impl::Rng`.
#[cfg(feature = "std")]
enum RngWrapper {
    Deterministic(StdRng),
    #[cfg(any(feature = "getrandom", test))]
    Randomized(rand::rngs::ThreadRng),
}

#[cfg(feature = "std")]
impl RngCore for RngWrapper {
    #[inline(always)]
    fn next_u32(&mut self) -> u32 {
        match self {
            Self::Deterministic(rng) => rng.next_u32(),
            #[cfg(any(feature = "getrandom", test))]
            Self::Randomized(rng) => rng.next_u32(),
        }
    }

    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        match self {
            Self::Deterministic(rng) => rng.next_u64(),
            #[cfg(any(feature = "getrandom", test))]
            Self::Randomized(rng) => rng.next_u64(),
        }
    }

    #[inline(always)]
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        match self {
            Self::Deterministic(rng) => rng.fill_bytes(dest),
            #[cfg(any(feature = "getrandom", test))]
            Self::Randomized(rng) => rng.fill_bytes(dest),
        }
    }

    #[inline(always)]
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        match self {
            Self::Deterministic(rng) => rng.try_fill_bytes(dest),
            #[cfg(any(feature = "getrandom", test))]
            Self::Randomized(rng) => rng.try_fill_bytes(dest),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod test {
    #[test]
    fn test_deterministic_rng() {
        use super::*;

        let mut rng = super::test_rng();
        let a = u128::rand(&mut rng);

        // Reset the rng by sampling a new one.
        let mut rng = super::test_rng();
        let b = u128::rand(&mut rng);
        assert_ne!(a, b); // should be unequal with high probability.

        // Let's make the rng deterministic.
        std::env::set_var("DETERMINISTIC_TEST_RNG", "1");
        let mut rng = super::test_rng();
        let a = u128::rand(&mut rng);

        // Reset the rng by sampling a new one.
        let mut rng = super::test_rng();
        let b = u128::rand(&mut rng);
        assert_eq!(a, b); // should be equal with high probability.
    }
}
