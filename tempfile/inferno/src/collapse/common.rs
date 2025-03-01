use std::borrow::Cow;
use std::io;
#[cfg(feature = "multithreaded")]
use std::mem;
#[cfg(feature = "multithreaded")]
use std::sync::Arc;

use ahash::AHashMap;
#[cfg(feature = "multithreaded")]
use dashmap::DashMap;
use once_cell::sync::Lazy;

macro_rules! invalid_data_error {
    ($($arg:tt)*) => {{
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!($($arg)*),
        ))
    }};
}

const CAPACITY_HASHMAP: usize = 512;

pub(crate) const CAPACITY_READER: usize = 128 * 1024;

/// Internal parameter (not exposed to users) that determines how many stacks of
/// input data make up a "chunk" (unit that is sent to the threadpool for
/// processing). Chosen by benchmarking various values using the following tests:
/// * cargo test bench_nstacks_dtrace --release -- --ignored --nocapture
/// * cargo test bench_nstacks_perf --release -- --ignored --nocapture
pub(crate) const DEFAULT_NSTACKS_PER_JOB: usize = 100;

/// A guess at the number of bytes contained in any given stack of any given format.
/// Used to calculate the initial capacity of the vector used for sending input
/// data across threads.
#[cfg(feature = "multithreaded")]
const NBYTES_PER_STACK_GUESS: usize = 1024;

const RUST_HASH_LENGTH: usize = 17;

#[cfg(feature = "multithreaded")]
#[doc(hidden)]
pub static DEFAULT_NTHREADS: Lazy<usize> =
    Lazy::new(|| std::thread::available_parallelism().unwrap().into());
#[cfg(not(feature = "multithreaded"))]
#[doc(hidden)]
pub static DEFAULT_NTHREADS: Lazy<usize> = Lazy::new(|| 1);

/// Sealed trait for internal library authors.
///
/// If you implement this trait, your type will implement the public-facing
/// `Collapse` trait as well. Implementing this trait gives you parallelism
/// for free as long as you adhere to the requirements described in the
/// comments below.
pub trait CollapsePrivate: Send + Sized {
    // *********************************************************** //
    // ********************* REQUIRED METHODS ******************** //
    // *********************************************************** //

    /// Process any header lines that precede the main body of samples.
    ///
    /// Some formats, such as `dtrace`, contain a header or other non-stack
    /// information at the beginning of their input files. If header information
    /// is present, this method **must** consume it (i.e. advance the provided
    /// reader past it).
    ///
    /// This method also provides an opportunity to do processing of actual
    /// stack data on the main thread before worker threads are spun up. For
    /// example, `perf` requires reading the first stack in order to know how to
    /// process the rest; so this method is used for that "upfront" processing.
    ///
    /// If the format you are working with does not contain header information
    /// or does not need any special, up-front processing, just have this method
    /// return `Ok(())` immediately.
    fn pre_process<R>(&mut self, reader: &mut R, occurrences: &mut Occurrences) -> io::Result<()>
    where
        R: io::BufRead;

    /// Process all samples in a chunk of input (the primary method).
    ///
    /// This method receives a reader whose header has already been consumed (see above),
    /// as well as a mutable reference to an `Occurences` instance (just a hashamp that
    /// works across multiple threads). Implementers should parse the stack data
    /// contained in the reader and write output to the provided `Occurrences` map.
    ///
    /// This method may be called multiple times to process batches of incoming samples.
    /// Therefore, make sure that when end-of-file is reached, the collapser now considers
    /// itself back at the top-level context (e.g., not in the middle of a stack). This
    /// means that some internal state, e.g. stack buffers, must be reset by the time this
    /// method returns. Other internal state, e.g. caches, however, may be kept.
    fn collapse_single_threaded<R>(
        &mut self,
        reader: R,
        occurrences: &mut Occurrences,
    ) -> io::Result<()>
    where
        R: io::BufRead;

    /// Determine the end of a stack.
    ///
    /// Worker threads **must** receive full stacks (as opposed to partial stacks); so this method
    /// determines, for your specific format, when the end of a stack has been reached.
    ///
    /// This method should return `true` if the provided line represents the end of a stack;
    /// `false` otherwise.
    ///
    /// If your format requires more information than merely a line of the input data in order
    /// to determine whether or not you are at the end of a stack, you can retrieve/store
    /// information on the `self` instance, which is also available to you in this method. This
    /// method will be called for every line of input data (excluding those consumed by the
    /// `pre_process` method).
    fn would_end_stack(&mut self, line: &[u8]) -> bool;

    /// Creates a copy and prepares it to be sent to a different thread.
    ///
    /// This method creates a copy of `self` in order to send it to a different thread.
    /// As such, it should clone all the internal fields of `self` **except** those that
    /// should be reset because the collapser will now operate in a different stack context.
    /// For example, any options should be cloned, but any stack buffers or similar "stack state"
    /// should be reset to, for example, an empty vector before this method returns.
    fn clone_and_reset_stack_context(&self) -> Self;

    /// Determine if this format corresponds to the input data.
    ///
    /// This method, used by the `guess` collapser, should return whether or not the
    /// implementation corresponds with the given input string, i.e. if the input data
    /// matches the collapser.
    ///
    /// - `None` means "not sure -- need more input"
    /// - `Some(true)` means "yes, this implementation should work with this string"
    /// - `Some(false)` means "no, this implementation definitely won't work"
    #[allow(clippy::wrong_self_convention)]
    fn is_applicable(&mut self, input: &str) -> Option<bool>;

    /// Returns the number of stacks per job to send to the threadpool.
    fn nstacks_per_job(&self) -> usize;

    /// Sets the number of stacks per job to send to the threadpool.
    fn set_nstacks_per_job(&mut self, n: usize);

    /// Returns the number of threads to use.
    fn nthreads(&self) -> usize;

    /// Sets the number of threads to use.
    fn set_nthreads(&mut self, n: usize);

    // *********************************************************** //
    // ******************** PROVIDED METHODS ********************* //
    // *********************************************************** //

    fn collapse<R, W>(&mut self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: io::BufRead,
        W: io::Write,
    {
        let mut occurrences = Occurrences::new(self.nthreads());

        // Consume the header, if any, and do any other pre-processing
        // that needs to occur.
        self.pre_process(&mut reader, &mut occurrences)?;

        // Do collapsing.
        if occurrences.is_concurrent() {
            self.collapse_multi_threaded(reader, &mut occurrences)?;
        } else {
            self.collapse_single_threaded(reader, &mut occurrences)?;
        }

        // Write results.
        occurrences.write_and_clear(writer)
    }

    #[cfg(not(feature = "multithreaded"))]
    fn collapse_multi_threaded<R>(&mut self, _: R, _: &mut Occurrences) -> io::Result<()>
    where
        R: io::BufRead,
    {
        unimplemented!();
    }

    #[cfg(feature = "multithreaded")]
    fn collapse_multi_threaded<R>(
        &mut self,
        mut reader: R,
        occurrences: &mut Occurrences,
    ) -> io::Result<()>
    where
        R: io::BufRead,
    {
        let nstacks_per_job = self.nstacks_per_job();
        let nthreads = self.nthreads();

        assert_ne!(nstacks_per_job, 0);
        assert!(nthreads > 1);
        assert!(occurrences.is_concurrent());

        crossbeam_utils::thread::scope(|scope| {
            // Channel for sending an error from the worker threads to the main thread
            // in the event a worker has failed.
            let (tx_error, rx_error) = crossbeam_channel::bounded::<io::Error>(1);

            // Channel for sending input data from the main thread to the worker threads.
            // We choose `2 * nthreads` as the channel size here in order to limit memory
            // usage in the case of particularly large input files.
            let (tx_input, rx_input) = crossbeam_channel::bounded::<Vec<u8>>(2 * nthreads);

            // Channel for worker threads that have errored to signal to all the other
            // worker threads that they should stop work immediately and return.
            let (tx_stop, rx_stop) = crossbeam_channel::bounded::<()>(nthreads - 1);

            let mut handles = Vec::with_capacity(nthreads);
            for _ in 0..nthreads {
                let tx_error = tx_error.clone();
                let rx_input = rx_input.clone();
                let (tx_stop, rx_stop) = (tx_stop.clone(), rx_stop.clone());

                let mut folder = self.clone_and_reset_stack_context();
                let mut occurrences = occurrences.clone();

                // Launch the worker thread...
                let handle = scope.spawn(move |_| loop {
                    crossbeam_channel::select! {
                        recv(rx_input) -> input => {
                            // Receive input from the main thread.
                            let data = match input {
                                Ok(data) => data,
                                // The main threads drops it's handle to the input sender once it's
                                // finished sending data; so if we get an error here, it means
                                // there is no more data to be sent and we should exit.
                                Err(_) => return,
                            };
                            // If there is input data, process it.
                            if let Err(e) = folder.collapse_single_threaded(&data[..], &mut occurrences) {
                                // In the event of an error...
                                //
                                // We notify all the threads about it here, rather than wait for the main input
                                // loop to see the error, so that we can also stop the input loop from iterating
                                // through the rest of the file.
                                //
                                // If the channel is full, it means another thread has also errored
                                // and already sent a stop signal to the other threads; so there is
                                // no need to wait or to check for a `SendError` here.
                                for _ in 0..(nthreads - 1) {
                                    let _ = tx_stop.try_send(());
                                }

                                // Then, send the error produced to the main thread for
                                // propagation. If the channel is full, it means another thread
                                // has also errored and already sent its error back to the
                                // main thread; so there is no need to wait or to check for a
                                // `SendError` here.
                                let _ = tx_error.try_send(e);

                                // Finally, return.
                                return;
                            }
                            // If successful, return to the top of the loop and continue to poll
                            // the input and stop channels.
                        },
                        recv(rx_stop) -> _ => {
                            // Received a signal from another worker thread that it has errored;
                            // so should cease work immediately and return.
                            return;
                        },
                    }
                });
                handles.push(handle);
            }

            // On the main thread, we're about to start sending data to the worker threads,
            // but we only want to send data to the worker threads **if** they're still alive!
            // (if one of them produces an error, all of them will exit early). To ensure we don't try
            // to send data to dead worker threads, drop the main thread's handle to the input receiver
            // here. This way, if all the workers die, every handle to the input receiver will have
            // been dropped and we'll get an error when trying to send data on the input sender,
            // which will tell us (the main thread) to stop trying to send data and, instead,
            // skip to trying to pull an error off the error channel.
            drop(rx_input);

            // Now that we've dropped the main thread's handle to the input sender, start
            // trying to send data to the worker threads...

            let buf_capacity = usize::next_power_of_two(NBYTES_PER_STACK_GUESS * nstacks_per_job);
            let mut buf = Vec::with_capacity(buf_capacity);
            let (mut index, mut nstacks) = (0, 0);

            loop {
                let n = reader.read_until(b'\n', &mut buf)?;
                if n == 0 {
                    // If we've reached the end of the data, send the final chunk to the worker
                    // threads and break from the loop, The worker threads may or may not still
                    // be alive (depending on if one errored in between the sending of the last
                    // chunk and the sending of this one), but either way we should break the loop;
                    // so there's no need to check for a `SendError` here.
                    let _ = tx_input.send(buf);
                    break;
                }
                let line = &buf[index..index + n];
                index += n;
                if self.would_end_stack(line) {
                    // If we've reached the end of a stack, count it.
                    nstacks += 1;
                    if nstacks == nstacks_per_job {
                        // If we've accumulated enough stacks to make up a chunk to send to the
                        // worker threads, try to send it.
                        let buf_capacity = usize::next_power_of_two(buf.capacity());
                        let chunk = mem::replace(&mut buf, Vec::with_capacity(buf_capacity));
                        if tx_input.send(chunk).is_err() {
                            // If sending the chunk produces a `SendError`, this means that one
                            // of the worker threads has errored, sent a signal to all the other
                            // worker threads to shut down, and they have all shutdown, in which
                            // case we know there will be an error waiting for us on the error
                            // channel; so we should stop parsing input data (i.e. break).
                            break;
                        }
                        index = 0;
                        nstacks = 0;
                    }
                    continue
                }
            }

            // The main thread needs to drop its handle to the input sender here because
            // that's how we signal to the worker threads that there is no more data coming
            // on the input channel, in which case they should exit.
            drop(tx_input);

            // The main thread needs to drop its handle to the error sender here because we
            // are about to poll the error receiver for errors, which will block until all
            // the error senders have been dropped (including ours).
            drop(tx_error);

            // Now we poll the error channel, which will block until either:
            // * all work has been completely successfully,
            //   in which case the expression below will evaluate to `None`, or
            // * an error has occurred on one of the worker theads,
            //   in which case the expression below will evaluate to `Some(<io::Error>)`.
            if let Some(e) = rx_error.iter().next() {
                return Err(e);
            }

            for handle in handles {
                handle.join().unwrap();
            }

            Ok(())
        })
        .unwrap()
    }
}

/// Occurrences is a HashMap, which uses:
/// * AHashMap if single-threaded
/// * DashMap if multi-threaded
///
/// This is public because it is part of the sealed `CollapsePrivate` trait's API, but it
/// is in a crate-private module so is not nameable by downstream library users.
#[derive(Clone, Debug)]
pub enum Occurrences {
    SingleThreaded(AHashMap<String, usize>),
    #[cfg(feature = "multithreaded")]
    MultiThreaded(Arc<DashMap<String, usize, ahash::RandomState>>),
}

impl Occurrences {
    #[cfg(feature = "multithreaded")]
    pub(crate) fn new(nthreads: usize) -> Self {
        assert_ne!(nthreads, 0);
        if nthreads == 1 {
            Self::new_single_threaded()
        } else {
            Self::new_multi_threaded()
        }
    }

    #[cfg(not(feature = "multithreaded"))]
    pub(crate) fn new(nthreads: usize) -> Self {
        assert_ne!(nthreads, 0);
        Self::new_single_threaded()
    }

    fn new_single_threaded() -> Self {
        let map =
            AHashMap::with_capacity_and_hasher(CAPACITY_HASHMAP, ahash::RandomState::default());
        Occurrences::SingleThreaded(map)
    }

    #[cfg(feature = "multithreaded")]
    fn new_multi_threaded() -> Self {
        let map =
            DashMap::with_capacity_and_hasher(CAPACITY_HASHMAP, ahash::RandomState::default());
        Occurrences::MultiThreaded(Arc::new(map))
    }

    /// Inserts a key-count pair into the map. If the map did not have this key
    /// present, `None` is returned. If the map did have this key present, the
    /// value is updated, and the old value is returned.
    pub(crate) fn insert(&mut self, key: String, count: usize) -> Option<usize> {
        use self::Occurrences::*;
        match self {
            SingleThreaded(map) => map.insert(key, count),
            #[cfg(feature = "multithreaded")]
            MultiThreaded(arc) => arc.insert(key, count),
        }
    }

    /// Inserts a key-count pair into the map if the key does not already exist.
    /// If the key does already exist, adds count to the current value of the
    /// existing key.
    pub(crate) fn insert_or_add(&mut self, key: String, count: usize) {
        use self::Occurrences::*;
        match self {
            SingleThreaded(map) => *map.entry(key).or_insert(0) += count,
            #[cfg(feature = "multithreaded")]
            MultiThreaded(arc) => *arc.entry(key).or_insert(0) += count,
        }
    }

    pub(crate) fn is_concurrent(&self) -> bool {
        use self::Occurrences::*;
        match self {
            SingleThreaded(_) => false,
            #[cfg(feature = "multithreaded")]
            MultiThreaded(_) => true,
        }
    }

    pub(crate) fn write_and_clear<W>(&mut self, mut writer: W) -> io::Result<()>
    where
        W: io::Write,
    {
        use self::Occurrences::*;
        match self {
            SingleThreaded(ref mut map) => {
                let mut contents: Vec<_> = map.drain().collect();
                contents.sort();
                for (key, value) in contents {
                    writeln!(writer, "{} {}", key, value)?;
                }
            }
            #[cfg(feature = "multithreaded")]
            MultiThreaded(ref mut arc) => {
                let map = match Arc::get_mut(arc) {
                    Some(map) => map,
                    None => panic!(
                        "Attempting to drain the contents of a concurrent HashMap \
                         when more than one thread has access to it, which is \
                         not allowed."
                    ),
                };
                let map = mem::replace(
                    map,
                    DashMap::with_capacity_and_hasher(
                        CAPACITY_HASHMAP,
                        ahash::RandomState::default(),
                    ),
                );
                let contents = map.iter().collect::<Vec<_>>();
                let mut pairs = contents.iter().map(|pair| pair.pair()).collect::<Vec<_>>();
                pairs.sort();
                for (key, value) in pairs {
                    writeln!(writer, "{} {}", key, value)?;
                }
            }
        }
        writer.flush()?;
        Ok(())
    }
}

/// Demangles partially demangled Rust symbols that were demangled incorrectly by profilers like
/// `sample` and `DTrace`.
///
/// For example:
///     `_$LT$grep_searcher..searcher..glue..ReadByLine$LT$$u27$s$C$$u20$M$C$$u20$R$C$$u20$S$GT$$GT$::run::h30ecedc997ad7e32`
/// becomes
///     `<grep_searcher::searcher::glue::ReadByLine<'s, M, R, S>>::run`
///
/// Non-Rust symobols, or Rust symbols that are already demangled, will be returned unchanged.
///
/// Based on code in https://github.com/alexcrichton/rustc-demangle/blob/master/src/legacy.rs
#[allow(clippy::cognitive_complexity)]
pub(crate) fn fix_partially_demangled_rust_symbol(symbol: &str) -> Cow<str> {
    // Rust hashes are hex digits with an `h` prepended.
    let is_rust_hash =
        |s: &str| s.starts_with('h') && s[1..].chars().all(|c| c.is_ascii_hexdigit());

    // If there's no trailing Rust hash just return the symbol as is.
    if symbol.len() < RUST_HASH_LENGTH || !is_rust_hash(&symbol[symbol.len() - RUST_HASH_LENGTH..])
    {
        return Cow::Borrowed(symbol);
    }

    // Strip off trailing hash.
    let mut rest = &symbol[..symbol.len() - RUST_HASH_LENGTH];

    if rest.ends_with("::") {
        rest = &rest[..rest.len() - 2];
    }

    if rest.starts_with("_$") {
        rest = &rest[1..];
    }

    let mut demangled = String::new();

    while !rest.is_empty() {
        if rest.starts_with('.') {
            if let Some('.') = rest[1..].chars().next() {
                demangled.push_str("::");
                rest = &rest[2..];
            } else {
                demangled.push('.');
                rest = &rest[1..];
            }
        } else if rest.starts_with('$') {
            macro_rules! demangle {
                ($($pat:expr => $demangled:expr,)*) => ({
                    $(if rest.starts_with($pat) {
                        demangled.push_str($demangled);
                        rest = &rest[$pat.len()..];
                        } else)*
                    {
                        demangled.push_str(rest);
                        break;
                    }

                })
            }

            demangle! {
                "$SP$" => "@",
                "$BP$" => "*",
                "$RF$" => "&",
                "$LT$" => "<",
                "$GT$" => ">",
                "$LP$" => "(",
                "$RP$" => ")",
                "$C$" => ",",
                "$u7e$" => "~",
                "$u20$" => " ",
                "$u27$" => "'",
                "$u3d$" => "=",
                "$u5b$" => "[",
                "$u5d$" => "]",
                "$u7b$" => "{",
                "$u7d$" => "}",
                "$u3b$" => ";",
                "$u2b$" => "+",
                "$u21$" => "!",
                "$u22$" => "\"",
            }
        } else {
            let idx = match rest.char_indices().find(|&(_, c)| c == '$' || c == '.') {
                None => rest.len(),
                Some((i, _)) => i,
            };
            demangled.push_str(&rest[..idx]);
            rest = &rest[idx..];
        }
    }

    Cow::Owned(demangled)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;
    use std::fmt;
    use std::fs::File;
    use std::io::Write;
    use std::io::{self, BufRead, Read};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use libflate::gzip::Decoder;

    use super::*;
    use crate::collapse::Collapse;

    pub(crate) fn read_inputs<P>(inputs: &[P]) -> io::Result<HashMap<PathBuf, Vec<u8>>>
    where
        P: AsRef<Path>,
    {
        let mut map = HashMap::default();
        for path in inputs.iter() {
            let path = path.as_ref();
            let bytes = {
                let mut buf = Vec::new();
                let mut file = File::open(path)?;
                if path.to_str().unwrap().ends_with(".gz") {
                    let mut reader = Decoder::new(file)?;
                    reader.read_to_end(&mut buf)?;
                } else {
                    file.read_to_end(&mut buf)?;
                }
                buf
            };
            map.insert(path.to_path_buf(), bytes);
        }
        Ok(map)
    }

    pub(crate) fn test_collapse_multi<C, P>(folder: &mut C, inputs: &[P]) -> io::Result<()>
    where
        C: Collapse + CollapsePrivate,
        P: AsRef<Path>,
    {
        const MAX_THREADS: usize = 16;
        for (path, bytes) in read_inputs(inputs)? {
            folder.set_nthreads(1);
            let mut writer = Vec::new();
            <C as Collapse>::collapse(folder, &bytes[..], &mut writer)?;
            let expected = std::str::from_utf8(&writer[..]).unwrap();

            for n in 2..=MAX_THREADS {
                folder.set_nthreads(n);
                let mut writer = Vec::new();
                <C as Collapse>::collapse(folder, &bytes[..], &mut writer)?;
                let actual = std::str::from_utf8(&writer[..]).unwrap();

                assert_eq!(
                    actual,
                    expected,
                    "Collapsing with {} threads does not produce the same output as collapsing with 1 thread for {}",
                    n,
                    path.display()
                );
            }
        }

        Ok(())
    }

    pub(crate) fn bench_nstacks<C, P>(folder: &mut C, inputs: &[P]) -> io::Result<()>
    where
        C: CollapsePrivate,
        P: AsRef<Path>,
    {
        const MIN_LINES: usize = 2000;
        const NSAMPLES: usize = 100;
        const WARMUP_SECS: usize = 3;

        let _stdout = io::stdout();
        let _stderr = io::stdout();

        let mut stdout = _stdout.lock();
        let _stderr = _stderr.lock();

        struct Foo<'a> {
            default: usize,
            nlines: usize,
            nstacks: usize,
            path: &'a Path,
            results: HashMap<usize, u64>,
        }

        impl<'a> Foo<'a> {
            fn new<C>(
                folder: &mut C,
                path: &'a Path,
                bytes: &[u8],
                stdout: &mut io::StdoutLock,
            ) -> io::Result<Option<Self>>
            where
                C: CollapsePrivate,
            {
                let default = folder.nstacks_per_job();

                let (nlines, nstacks) = count_lines_and_stacks(bytes);
                if nlines < MIN_LINES {
                    return Ok(None);
                }

                let mut results = HashMap::default();
                let iter = vec![default]
                    .into_iter()
                    .chain(1..=10)
                    .chain((20..=nstacks).step_by(10));
                for nstacks_per_job in iter {
                    folder.set_nstacks_per_job(nstacks_per_job);
                    let mut durations = Vec::new();
                    for _ in 0..NSAMPLES {
                        let now = Instant::now();
                        folder.collapse(bytes, io::sink())?;
                        durations.push(now.elapsed().as_nanos());
                    }
                    let avg_duration =
                        (durations.iter().sum::<u128>() as f64 / durations.len() as f64) as u64;
                    results.insert(nstacks_per_job, avg_duration);
                    stdout.write_all(&[b'.'])?;
                    stdout.flush()?;
                }
                Ok(Some(Self {
                    default,
                    nlines,
                    nstacks,
                    path,
                    results,
                }))
            }
        }

        impl<'a> fmt::Display for Foo<'a> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                writeln!(
                    f,
                    "{} (nstacks: {}, lines: {})",
                    self.path.display(),
                    self.nstacks,
                    self.nlines
                )?;
                let default_duration = self.results[&self.default];
                let mut results = self.results.iter().collect::<Vec<_>>();
                results.sort_by(|(_, d1), (_, d2)| (**d1).cmp(*d2));
                for (nstacks_per_job, duration) in results.iter().take(10) {
                    writeln!(
                        f,
                        "    nstacks_per_job: {:>4} (% of total: {:>3.0}%) | time: {:.0}% of default",
                        nstacks_per_job,
                        (**nstacks_per_job as f32 / self.nstacks as f32) * 100.0,
                        **duration as f64 / default_duration as f64 * 100.0,
                    )?;
                }
                writeln!(f)?;
                Ok(())
            }
        }

        fn count_lines_and_stacks(bytes: &[u8]) -> (usize, usize) {
            let mut reader = io::BufReader::new(bytes);
            let mut line = Vec::new();

            let (mut nlines, mut nstacks) = (0, 0);
            loop {
                line.clear();
                let n = reader.read_until(0x0A, &mut line).unwrap();
                if n == 0 {
                    nstacks += 1;
                    break;
                }
                let l = String::from_utf8_lossy(&line);
                nlines += 1;
                if l.trim().is_empty() {
                    nstacks += 1;
                }
            }
            (nlines, nstacks)
        }

        let inputs = read_inputs(inputs)?;

        // Warmup
        let now = Instant::now();
        stdout.write_fmt(format_args!(
            "# Warming up for approximately {} seconds.\n",
            WARMUP_SECS
        ))?;
        stdout.flush()?;
        while now.elapsed() < std::time::Duration::from_secs(WARMUP_SECS as u64) {
            for (_, bytes) in inputs.iter() {
                folder.collapse(&bytes[..], io::sink())?;
            }
        }

        // Time
        let mut foos = Vec::new();
        for (path, bytes) in &inputs {
            stdout.write_fmt(format_args!("# {} ", path.display()))?;
            stdout.flush()?;
            if let Some(foo) = Foo::new(folder, path, bytes, &mut stdout)? {
                foos.push(foo);
            }
            stdout.write_all(&[b'\n'])?;
            stdout.flush()?;
        }
        stdout.write_all(&[b'\n'])?;
        stdout.flush()?;
        foos.sort_by(|a, b| b.nstacks.cmp(&a.nstacks));
        for foo in foos {
            stdout.write_fmt(format_args!("{}", foo))?;
            stdout.flush()?;
        }

        Ok(())
    }

    pub(crate) fn check_flamegraph_git_submodule_initialised() {
        if !Path::new("./flamegraph/.git").exists() {
            panic!(
                "Some tests require the flamegraph git submodule to be initialised, but it is not.
Initialise it with `git submodule update --init flamegraph`."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    macro_rules! t {
        ($a:expr, $b:expr) => {
            assert!(ok($a, $b))
        };
    }

    macro_rules! t_unchanged {
        ($a:expr) => {
            assert!(ok_unchanged($a))
        };
    }

    fn ok(sym: &str, expected: &str) -> bool {
        let result = super::fix_partially_demangled_rust_symbol(sym);
        if result == expected {
            true
        } else {
            println!("\n{}\n!=\n{}\n", result, expected);
            false
        }
    }

    fn ok_unchanged(sym: &str) -> bool {
        let result = super::fix_partially_demangled_rust_symbol(sym);
        if result == sym {
            true
        } else {
            println!("{} should have been unchanged, but got {}", sym, result);
            false
        }
    }

    #[test]
    fn fix_partially_demangled_rust_symbols() {
        t!(
            "std::sys::unix::fs::File::open::hb90e1c1c787080f0",
            "std::sys::unix::fs::File::open"
        );
        t!("_$LT$std..fs..ReadDir$u20$as$u20$core..iter..traits..iterator..Iterator$GT$::next::hc14f1750ca79129b", "<std::fs::ReadDir as core::iter::traits::iterator::Iterator>::next");
        t!("rg::search_parallel::_$u7b$$u7b$closure$u7d$$u7d$::_$u7b$$u7b$closure$u7d$$u7d$::h6e849b55a66fcd85", "rg::search_parallel::_{{closure}}::_{{closure}}");
        t!(
            "_$LT$F$u20$as$u20$alloc..boxed..FnBox$LT$A$GT$$GT$::call_box::h8612a2a83552fc2d",
            "<F as alloc::boxed::FnBox<A>>::call_box"
        );
        t!(
            "_$LT$$RF$std..fs..File$u20$as$u20$std..io..Read$GT$::read::h5d84059cf335c8e6",
            "<&std::fs::File as std::io::Read>::read"
        );
        t!(
            "_$LT$std..thread..JoinHandle$LT$T$GT$$GT$::join::hca6aa63e512626da",
            "<std::thread::JoinHandle<T>>::join"
        );
        t!(
            "std::sync::mpsc::shared::Packet$LT$T$GT$::recv::hfde2d9e28d13fd56",
            "std::sync::mpsc::shared::Packet<T>::recv"
        );
        t!("crossbeam_utils::thread::ScopedThreadBuilder::spawn::_$u7b$$u7b$closure$u7d$$u7d$::h8fdc7d4f74c0da05", "crossbeam_utils::thread::ScopedThreadBuilder::spawn::_{{closure}}");
    }

    #[test]
    fn fix_partially_demangled_rust_symbol_on_fully_mangled_symbols() {
        t_unchanged!("_ZN4testE");
        t_unchanged!("_ZN4test1a2bcE");
        t_unchanged!("_ZN7inferno10flamegraph5merge6frames17hacfe2d67301633c2E");
        t_unchanged!("_ZN3std2rt19lang_start_internal17h540c897fe52ba9c5E");
        t_unchanged!("_ZN116_$LT$core..str..pattern..CharSearcher$LT$$u27$a$GT$$u20$as$u20$core..str..pattern..ReverseSearcher$LT$$u27$a$GT$$GT$15next_match_back17h09d544049dd719bbE");
        t_unchanged!("_ZN3std5panic12catch_unwind17h0562757d03ff60b3E");
        t_unchanged!("_ZN3std9panicking3try17h9c1cbc5599e1efbfE");
    }

    #[test]
    fn fix_partially_demangled_rust_symbol_on_fully_demangled_symbols() {
        t_unchanged!("std::sys::unix::fs::File::open");
        t_unchanged!("<F as alloc::boxed::FnBox<A>>::call_box");
        t_unchanged!("<std::fs::ReadDir as core::iter::traits::iterator::Iterator>::next");
        t_unchanged!("<rg::search::SearchWorker<W>>::search_impl");
        t_unchanged!("<grep_searcher::searcher::glue::ReadByLine<'s, M, R, S>>::run");
        t_unchanged!("<alloc::raw_vec::RawVec<T, A>>::reserve_internal");
    }
}
