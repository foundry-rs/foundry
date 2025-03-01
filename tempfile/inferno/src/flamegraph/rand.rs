use std::cell::RefCell;

pub(super) struct XorShift64 {
    a: u64,
}

impl XorShift64 {
    pub(super) fn new(seed: u64) -> XorShift64 {
        XorShift64 { a: seed }
    }

    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.a;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.a = x;
        x
    }

    pub(super) fn next_f64(&mut self) -> f64 {
        sample(self.next())
    }
}

thread_local! {
    pub(super) static RNG: RefCell<XorShift64> = RefCell::new(XorShift64::new(1234));
}

// Copied from `rand` with minor modifications.
fn sample(value: u64) -> f64 {
    let fraction_bits = 52;

    // Multiply-based method; 24/53 random bits; [0, 1) interval.
    // We use the most significant bits because for simple RNGs
    // those are usually more random.
    let float_size = std::mem::size_of::<f64>() as u32 * 8;
    let precision = fraction_bits + 1;
    let scale = 1.0 / ((1_u64 << precision) as f64);

    let value = value >> (float_size - precision);
    scale * (value as f64)
}

pub(super) fn thread_rng() -> impl Fn() -> f32 {
    || RNG.with(|rng| rng.borrow_mut().next_f64() as f32)
}

#[test]
fn test_rng() {
    const ITERATIONS: usize = 10000;

    let mut rng = XorShift64::new(1234);
    let mut sum = rng.next_f64();
    let mut min = sum;
    let mut max = sum;
    for _ in 0..ITERATIONS - 1 {
        let value = rng.next_f64();
        sum += value;
        if value < min {
            min = value;
        }
        if value > max {
            max = value;
        }
    }

    let avg = sum / ITERATIONS as f64;

    // Make sure the RNG is uniform.
    assert!(min >= 0.000);
    assert!(min <= 0.001);
    assert!(max <= 1.000);
    assert!(max >= 0.999);
    assert!(avg >= 0.490);
    assert!(avg <= 0.510);
}
