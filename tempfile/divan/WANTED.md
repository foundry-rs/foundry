# Wanted

It would be great to have the following features added to Divan. If you have
ideas to expand this list, please [find](https://github.com/nvzqz/divan/discussions)
or [create](https://github.com/nvzqz/divan/discussions/new?category=ideas) a
discussion first.

- Async benchmarks

- Baseline benchmark
    - Should match baselines across equal generic types and constants
    - Idea:
    ```rs
    #[divan::bench]
    fn old() { ... }

    #[divan::bench(baseline = old)]
    fn new() { ... }
    ```

- Cross-device: run benchmarks on other devices and report the data on the local
device

- HTML output

- CSV output

- Custom counters

- Time complexity of counters
    - Also space complexity when measuring heap allocation

- Measure heap allocations
    - Custom [`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html)
    that wraps another `GlobalAlloc`, defaulting to [`System`](https://doc.rust-lang.org/std/alloc/struct.System.html)

- Custom timers

- Timer for kernel/user mode
    - Unix:
        - [`getrusage(2)`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/getrusage.html)
        - Per-thread:
            - Linux/FreeBSD/OpenBSD: [`RUSAGE_THREAD`](https://man7.org/linux/man-pages/man2/getrusage.2.html)
            - macOS/iOS: [`thread_info(mach_thread_self(), ...)`](https://www.gnu.org/software/hurd/gnumach-doc/Thread-Information.html)
    - Windows:
        - [`GetProcessTimes`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)
        - [`GetThreadTimes`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getthreadtimes)
