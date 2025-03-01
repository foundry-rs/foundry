#[cfg(target_arch = "x86")]
use std::arch::x86;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64 as x86;

use std::time::{Duration, Instant};

use crate::time::{fence, TscUnavailable};

#[inline(always)]
pub(crate) fn start_timestamp() -> u64 {
    // Serialize previous operations before `rdtsc` to ensure they are not
    // inside the timed section.
    util::lfence();

    let tsc = util::rdtsc();

    // Serialize `rdtsc` before any measured code.
    util::lfence();

    tsc
}

#[inline(always)]
pub(crate) fn end_timestamp() -> u64 {
    // `rdtscp` is serialized after all previous operations.
    let tsc = util::rdtscp();

    // Serialize `rdtscp` before any subsequent code.
    util::lfence();

    tsc
}

pub(crate) fn frequency() -> Result<u64, TscUnavailable> {
    if !util::tsc_is_available() {
        return Err(TscUnavailable::MissingInstructions);
    }

    if !util::tsc_is_invariant() {
        return Err(TscUnavailable::VariableFrequency);
    }

    let nominal = nominal_frequency();
    let measured = measure::measure_frequency();

    // Use the nominal frequency if within 0.1% of the measured frequency.
    //
    // The nominal frequency is used for getting an exact value if the measured
    // frequency is slightly off. It is not blindly trusted because it may not
    // match the TSC frequency.
    if let Some(nominal) = nominal {
        if measured * 0.999 < nominal && nominal < measured * 1.001 {
            return Ok(nominal.round() as u64);
        }
    }

    Ok(measured.round() as u64)
}

/// Parses the CPU frequency in the brand name, e.g. "2.50GHz".
fn nominal_frequency() -> Option<f64> {
    let name = util::cpu_name()?;
    let name = {
        let len = name.iter().position(|&ch| ch == 0).unwrap_or(name.len());
        std::str::from_utf8(&name[..len]).ok()?
    };

    #[rustfmt::skip]
    let frequencies = [
        ("MHz", 1e6),
        ("GHz", 1e9),
        ("THz", 1e12),
    ];

    for (unit, scale) in frequencies {
        let Some(unit_start) = name.find(unit) else {
            continue;
        };

        let pre_unit = &name[..unit_start];
        let num = match pre_unit.rsplit_once(' ') {
            Some((_, num)) => num,
            None => pre_unit,
        };

        if let Ok(num) = num.parse::<f64>() {
            return Some(num * scale);
        };
    }

    None
}

mod util {
    use super::*;

    #[inline(always)]
    pub fn rdtsc() -> u64 {
        fence::compiler_fence();

        // SAFETY: Reading the TSC is memory safe.
        let tsc = unsafe { x86::_rdtsc() };

        fence::compiler_fence();
        tsc
    }

    #[inline(always)]
    pub fn rdtscp() -> u64 {
        fence::compiler_fence();

        // SAFETY: Reading the TSC is memory safe.
        let tsc = unsafe { x86::__rdtscp(&mut 0) };

        fence::compiler_fence();
        tsc
    }

    #[inline(always)]
    pub fn lfence() {
        // SAFETY: A load fence is memory safe.
        unsafe { x86::_mm_lfence() }
    }

    #[inline]
    fn cpuid(leaf: u32) -> x86::CpuidResult {
        // SAFETY: `cpuid` is never unsafe to call.
        unsafe { x86::__cpuid(leaf) }
    }

    /// Invokes CPUID and converts its output registers to an ordered array.
    #[inline]
    fn cpuid_array(leaf: u32) -> [u32; 4] {
        let cpuid = cpuid(leaf);
        [cpuid.eax, cpuid.ebx, cpuid.ecx, cpuid.edx]
    }

    /// Returns `true` if the given CPUID leaf is available.
    #[inline]
    fn cpuid_has_leaf(leaf: u32) -> bool {
        cpuid(0x8000_0000).eax >= leaf
    }

    /// Returns `true` if CPUID indicates that the `rdtsc` and `rdtscp`
    /// instructions are available.
    #[inline]
    pub fn tsc_is_available() -> bool {
        let bits = cpuid(0x8000_0001).edx;

        let rdtsc = 1 << 4;
        let rdtscp = 1 << 27;

        bits & (rdtsc | rdtscp) != 0
    }

    /// Returns `true` if CPUID indicates that the timestamp counter has a
    /// constant frequency.
    #[inline]
    pub fn tsc_is_invariant() -> bool {
        let leaf = 0x8000_0007;

        if !cpuid_has_leaf(leaf) {
            return false;
        }

        cpuid(leaf).edx & (1 << 8) != 0
    }

    /// Returns the processor model name as a null-terminated ASCII string.
    pub fn cpu_name() -> Option<[u8; 48]> {
        if !cpuid_has_leaf(0x8000_0004) {
            return None;
        }

        #[rustfmt::skip]
        let result = [
            cpuid_array(0x8000_0002),
            cpuid_array(0x8000_0003),
            cpuid_array(0x8000_0004),
        ];

        // SAFETY: Converting from `u32` to bytes.
        Some(unsafe { std::mem::transmute(result) })
    }
}

mod measure {
    use super::*;

    /// Returns the TSC frequency by measuring it.
    pub fn measure_frequency() -> f64 {
        const TRIES: usize = 8;

        // Start with delay of 1ms up to 256ms (2^TRIES).
        let mut delay_ms = 1;

        let mut prev_measure = f64::NEG_INFINITY;
        let mut measures = [0.0; TRIES];

        for slot in &mut measures {
            let measure = measure_frequency_once(Duration::from_millis(delay_ms));

            // This measurement is sufficiently accurate if within 0.1% of the
            // previous.
            if measure * 0.999 < prev_measure && prev_measure < measure * 1.001 {
                return measure;
            }

            *slot = measure;
            prev_measure = measure;

            delay_ms *= 2;
        }

        // If no frequencies were within 0.1% of each other, find the frequency
        // with the smallest delta.
        let mut min_delta = f64::INFINITY;
        let mut result_index = 0;

        for i in 0..TRIES {
            for j in (i + 1)..TRIES {
                let delta = (measures[i] - measures[j]).abs();

                if delta < min_delta {
                    min_delta = delta;
                    result_index = i;
                }
            }
        }

        measures[result_index]
    }

    fn measure_frequency_once(delay: Duration) -> f64 {
        let (start_tsc, start_instant) = tsc_instant_pair();
        std::thread::sleep(delay);
        let (end_tsc, end_instant) = tsc_instant_pair();

        let elapsed_tsc = end_tsc.saturating_sub(start_tsc);
        let elapsed_duration = end_instant.duration_since(start_instant);

        (elapsed_tsc as f64 / elapsed_duration.as_nanos() as f64) * 1e9
    }

    /// Returns a timestamp/instant pair that has a small latency between
    /// getting the two values.
    fn tsc_instant_pair() -> (u64, Instant) {
        let mut best_latency = Duration::MAX;
        let mut best_pair = (0, Instant::now());

        // Make up to 100 attempts to get a low latency pair.
        for _ in 0..100 {
            let instant = Instant::now();
            let tsc = util::rdtsc();
            let latency = instant.elapsed();

            let pair = (tsc, instant);

            if latency.is_zero() {
                return pair;
            }

            if latency < best_latency {
                best_latency = latency;
                best_pair = pair;
            }
        }

        best_pair
    }
}
