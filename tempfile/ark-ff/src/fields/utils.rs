/// Calculates the k-adicity of n, i.e., the number of trailing 0s in a base-k
/// representation.
pub fn k_adicity(k: u64, mut n: u64) -> u32 {
    let mut r = 0;
    while n > 1 {
        if n % k == 0 {
            r += 1;
            n /= k;
        } else {
            return r;
        }
    }
    r
}
