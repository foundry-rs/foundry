//! An async channel where pending messages are delivered in order of priority.
//!
//! There are two kinds of channels:
//!
//! 1. [Bounded][`bounded()`] channel with limited capacity.
//! 2. [Unbounded][`unbounded()`] channel with unlimited capacity.
//!
//! A channel has the [`Sender`] and [`Receiver`] side. Both sides are cloneable and can be shared
//! among multiple threads. When [sending][`Sender::send()`], you pass in a message and its
//! priority. When [receiving][`Receiver::recv()`], you'll get back the pending message with the
//! highest priotiy.
//!
//! When all [`Sender`]s or all [`Receiver`]s are dropped, the channel becomes closed. When a
//! channel is closed, no more messages can be sent, but remaining messages can still be received.
//!
//! The channel can also be closed manually by calling [`Sender::close()`] or
//! [`Receiver::close()`]. The API and much of the documentation is based on  [async_channel](https://docs.rs/async-channel/1.6.1/async_channel/).
//!
//! # Examples
//!
//! ```
//! # futures_lite::future::block_on(async {
//! let (s, r) = async_priority_channel::unbounded();
//!
//! assert_eq!(s.send("Foo", 0).await, Ok(()));
//! assert_eq!(s.send("Bar", 2).await, Ok(()));
//! assert_eq!(s.send("Baz", 1).await, Ok(()));
//! assert_eq!(r.recv().await, Ok(("Bar", 2)));
//! # });
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

mod awaitable_atomics;

use awaitable_atomics::AwaitableAtomicCounterAndBit;
use std::{
    collections::BinaryHeap,
    convert::TryInto,
    error, fmt,
    iter::Peekable,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

/// Creates a bounded channel.
///
/// The created channel has space to hold at most `cap` messages at a time.
///
/// # Panics
///
/// Capacity must be a positive number. If `cap` is zero, this function will panic.
///
/// # Examples
///
/// ```
/// # futures_lite::future::block_on(async {
/// let (s, r) = async_priority_channel::bounded(1);
///
/// assert_eq!(s.send("Foo", 0).await, Ok(()));
/// assert_eq!(r.recv().await, Ok(("Foo", 0)));
/// # });
/// ```
pub fn bounded<I, P>(cap: u64) -> (Sender<I, P>, Receiver<I, P>)
where
    P: Ord,
{
    if cap == 0 {
        panic!("cap must be positive");
    }

    let channel = Arc::new(PriorityQueueChannel {
        heap: Mutex::new(BinaryHeap::new()),
        len_and_closed: AwaitableAtomicCounterAndBit::new(0),
        cap,
        sender_count: AtomicUsize::new(1),
        receiver_count: AtomicUsize::new(1),
    });
    let s = Sender {
        channel: channel.clone(),
    };
    let r = Receiver { channel };
    (s, r)
}

/// Creates an unbounded channel.
///
/// The created channel can hold an unlimited number of messages.
///
/// # Examples
///
/// ```
/// # futures_lite::future::block_on(async {
/// let (s, r) = async_priority_channel::unbounded();
///
/// assert_eq!(s.send("Foo", 0).await, Ok(()));
/// assert_eq!(s.send("Bar", 2).await, Ok(()));
/// assert_eq!(s.send("Baz", 1).await, Ok(()));
/// assert_eq!(r.recv().await, Ok(("Bar", 2)));
/// # });
/// ```
pub fn unbounded<I, P>() -> (Sender<I, P>, Receiver<I, P>)
where
    P: Ord,
{
    bounded(u64::MAX)
}

#[derive(Debug)]
struct PriorityQueueChannel<I, P>
where
    P: Ord,
{
    // the data that needs to be maintained under a mutex
    heap: Mutex<BinaryHeap<Item<I, P>>>,

    // number of items in the channel, and is the channel closed,
    // all accessible without holding the mutex?
    len_and_closed: AwaitableAtomicCounterAndBit,

    // capacity = 0 means unbounded, otherwise the bound.
    cap: u64,

    sender_count: AtomicUsize,
    receiver_count: AtomicUsize,
}

#[derive(Debug)]
/// Send side of the channel. Can be cloned.
pub struct Sender<I, P>
where
    P: Ord,
{
    channel: Arc<PriorityQueueChannel<I, P>>,
}

#[derive(Debug)]
/// Receive side of the channel. Can be cloned.
pub struct Receiver<I, P>
where
    P: Ord,
{
    channel: Arc<PriorityQueueChannel<I, P>>,
}

impl<I, P> Drop for Sender<I, P>
where
    P: Ord,
{
    fn drop(&mut self) {
        // Decrement the sender count and close the channel if it drops down to zero.
        if self.channel.sender_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.channel.close();
        }
    }
}

impl<I, P> Drop for Receiver<I, P>
where
    P: Ord,
{
    fn drop(&mut self) {
        // Decrement the receiver count and close the channel if it drops down to zero.
        if self.channel.receiver_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.channel.close();
        }
    }
}

impl<I, P> Clone for Sender<I, P>
where
    P: Ord,
{
    fn clone(&self) -> Sender<I, P> {
        let count = self.channel.sender_count.fetch_add(1, Ordering::Relaxed);

        // Make sure the count never overflows, even if lots of sender clones are leaked.
        if count > usize::MAX / 2 {
            panic!("bailing due to possible overflow");
        }

        Sender {
            channel: self.channel.clone(),
        }
    }
}

impl<I, P> Clone for Receiver<I, P>
where
    P: Ord,
{
    fn clone(&self) -> Receiver<I, P> {
        let count = self.channel.receiver_count.fetch_add(1, Ordering::Relaxed);

        // Make sure the count never overflows, even if lots of sender clones are leaked.
        if count > usize::MAX / 2 {
            panic!("bailing due to possible overflow");
        }

        Receiver {
            channel: self.channel.clone(),
        }
    }
}

impl<I, P> PriorityQueueChannel<I, P>
where
    P: Ord,
{
    /// Closes the channel and notifies all blocked operations.
    ///
    /// Returns `true` if this call has closed the channel and it was not closed already.
    ///
    fn close(&self) -> bool {
        let was_closed = self.len_and_closed.set_bit();
        !was_closed
    }

    // Return `true` if the channel is closed
    fn is_closed(&self) -> bool {
        self.len_and_closed.load().0
    }

    /// Return `true` if the channel is empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return `true` if the channel is full
    fn is_full(&self) -> bool {
        self.cap > 0 && self.len() == self.cap
    }

    /// Returns the number of messages in the channel.
    fn len(&self) -> u64 {
        self.len_and_closed.load().1
    }

    fn len_and_closed(&self) -> (bool, u64) {
        self.len_and_closed.load()
    }
}

impl<T, P> Sender<T, P>
where
    P: Ord,
{
    /// Attempts to send a message into the channel.
    ///
    /// If the channel is full or closed, this method returns an error.
    ///
    pub fn try_send(&self, msg: T, priority: P) -> Result<(), TrySendError<(T, P)>> {
        self.try_sendv(std::iter::once((msg, priority)).peekable())
            .map_err(|e| match e {
                TrySendError::Closed(mut value) => TrySendError::Closed(value.next().expect("foo")),
                TrySendError::Full(mut value) => TrySendError::Full(value.next().expect("foo")),
            })
    }

    /// Attempts to send multiple messages into the channel.
    ///
    /// If the channel is closed, this method returns an error.
    ///
    /// If the channel is full or nearly full, this method inserts as many messages
    /// as it can into the channel and then returns an error containing the
    /// remaining unsent messages.
    pub fn try_sendv<I>(&self, msgs: Peekable<I>) -> Result<(), TrySendError<Peekable<I>>>
    where
        I: Iterator<Item = (T, P)>,
    {
        let mut msgs = msgs;
        let (is_closed, len) = self.channel.len_and_closed();
        if is_closed {
            return Err(TrySendError::Closed(msgs));
        }
        if len > self.channel.cap {
            panic!("size of channel is larger than capacity. this must indicate a bug");
        }

        match len == self.channel.cap {
            true => Err(TrySendError::Full(msgs)),
            false => {
                // we're below capacity according to the atomic len() field.
                // but it's possible that two threads will get here at the same time
                // because we haven't acquired the lock yet, so lets acquire the lock
                // and only let one through
                let mut heap = self
                    .channel
                    .heap
                    .lock()
                    .expect("task panicked while holding lock");
                let mut n = 0;
                loop {
                    if heap.len().try_into().unwrap_or(u64::MAX) < self.channel.cap {
                        if let Some((msg, priority)) = msgs.next() {
                            heap.push(Item { msg, priority });
                            n += 1;
                        } else {
                            break;
                        }
                    } else {
                        self.channel.len_and_closed.incr(n);
                        return match msgs.peek() {
                            Some(_) => Err(TrySendError::Full(msgs)),
                            None => Ok(()),
                        };
                    }
                }
                self.channel.len_and_closed.incr(n);
                Ok(())
            }
        }
    }

    /// Sends a message into the channel.
    ///
    /// If the channel is full, this method waits until there is space for a message.
    ///
    /// If the channel is closed, this method returns an error.
    ///
    pub async fn send(&self, msg: T, priority: P) -> Result<(), SendError<(T, P)>> {
        let mut msg2 = msg;
        let mut priority2 = priority;
        loop {
            let decr_listener = self.channel.len_and_closed.listen_decr();
            match self.try_send(msg2, priority2) {
                Ok(_) => {
                    return Ok(());
                }
                Err(TrySendError::Full((msg, priority))) => {
                    msg2 = msg;
                    priority2 = priority;
                    decr_listener.await;
                }
                Err(TrySendError::Closed((msg, priority))) => {
                    return Err(SendError((msg, priority)));
                }
            }
        }
    }

    /// Send multiple messages into the channel
    ///
    /// If the channel is full, this method waits until there is space.
    ///
    /// If the channel is closed, this method returns an error.
    pub async fn sendv<I>(&self, msgs: Peekable<I>) -> Result<(), SendError<Peekable<I>>>
    where
        I: Iterator<Item = (T, P)>,
    {
        let mut msgs2 = msgs;
        loop {
            let decr_listener = self.channel.len_and_closed.listen_decr();
            match self.try_sendv(msgs2) {
                Ok(_) => {
                    return Ok(());
                }
                Err(TrySendError::Full(msgs)) => {
                    msgs2 = msgs;
                    decr_listener.await;
                }
                Err(TrySendError::Closed(msgs)) => {
                    return Err(SendError(msgs));
                }
            }
        }
    }

    /// Closes the channel and notifies all blocked operations.
    ///
    /// Returns `true` if this call has closed the channel and it was not closed already.
    ///
    pub fn close(&self) -> bool {
        self.channel.close()
    }

    /// Returns `true` if the channel is closed
    pub fn is_closed(&self) -> bool {
        self.channel.is_closed()
    }

    /// Return `true` if the channel is empty
    pub fn is_empty(&self) -> bool {
        self.channel.is_empty()
    }

    /// Return `true` if the channel is full
    pub fn is_full(&self) -> bool {
        self.channel.is_full()
    }

    /// Returns the number of messages in the channel.
    pub fn len(&self) -> u64 {
        self.channel.len()
    }

    /// Returns the channel capacity if it's bounded.
    pub fn capacity(&self) -> Option<u64> {
        match self.channel.cap {
            u64::MAX => None,
            c => Some(c),
        }
    }

    /// Returns the number of receivers for the channel.
    pub fn receiver_count(&self) -> usize {
        self.channel.receiver_count.load(Ordering::SeqCst)
    }

    /// Returns the number of senders for the channel.
    pub fn sender_count(&self) -> usize {
        self.channel.sender_count.load(Ordering::SeqCst)
    }
}

impl<I, P> Receiver<I, P>
where
    P: Ord,
{
    /// Attempts to receive a message from the channel.
    ///
    /// If the channel is empty or closed, this method returns an error.
    ///
    pub fn try_recv(&self) -> Result<(I, P), TryRecvError> {
        match (self.channel.is_empty(), self.channel.is_closed()) {
            (true, true) => Err(TryRecvError::Closed),
            (true, false) => Err(TryRecvError::Empty),
            (false, _) => {
                // channel contains items and is either open or closed
                let mut heap = self
                    .channel
                    .heap
                    .lock()
                    .expect("task panicked while holding lock");
                let item = heap.pop();
                match item {
                    Some(item) => {
                        self.channel.len_and_closed.decr();
                        Ok((item.msg, item.priority))
                    }
                    None => Err(TryRecvError::Empty),
                }
            }
        }
    }

    /// Receives a message from the channel.
    ///
    /// If the channel is empty, this method waits until there is a message.
    ///
    /// If the channel is closed, this method receives a message or returns an error if there are
    /// no more messages.
    pub async fn recv(&self) -> Result<(I, P), RecvError> {
        loop {
            let incr_listener = self.channel.len_and_closed.listen_incr();
            match self.try_recv() {
                Ok(item) => {
                    return Ok(item);
                }
                Err(TryRecvError::Closed) => {
                    return Err(RecvError);
                }
                Err(TryRecvError::Empty) => {
                    incr_listener.await;
                }
            }
        }
    }

    /// Closes the channel and notifies all blocked operations.
    ///
    /// Returns `true` if this call has closed the channel and it was not closed already.
    ///
    pub fn close(&self) -> bool {
        self.channel.close()
    }

    /// Returns whether the channel is closed
    pub fn is_closed(&self) -> bool {
        self.channel.is_closed()
    }

    /// Return `true` if the channel is empty
    pub fn is_empty(&self) -> bool {
        self.channel.is_empty()
    }

    /// Return `true` if the channel is full
    pub fn is_full(&self) -> bool {
        self.channel.is_full()
    }

    /// Returns the number of messages in the channel.
    pub fn len(&self) -> u64 {
        self.channel.len()
    }

    /// Returns the channel capacity if it's bounded.
    pub fn capacity(&self) -> Option<u64> {
        match self.channel.cap {
            u64::MAX => None,
            c => Some(c),
        }
    }

    /// Returns the number of receivers for the channel.
    pub fn receiver_count(&self) -> usize {
        self.channel.receiver_count.load(Ordering::SeqCst)
    }

    /// Returns the number of senders for the channel.
    pub fn sender_count(&self) -> usize {
        self.channel.sender_count.load(Ordering::SeqCst)
    }
}

/// Private 2-tuple that sorts only by the `[priority]`
#[derive(Debug)]
struct Item<I, P>
where
    P: Eq + Ord,
{
    msg: I,
    priority: P,
}

impl<I, P> Ord for Item<I, P>
where
    P: Eq + Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl<I, P> PartialOrd for Item<I, P>
where
    P: Eq + Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<I, P: std::cmp::Eq> PartialEq for Item<I, P>
where
    P: Eq + Ord,
{
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<I, P> Eq for Item<I, P> where P: Eq + Ord {}

/// An error returned from [`Sender::send()`].
///
/// Received because the channel is closed.
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

impl<T> SendError<T> {
    /// Unwraps the message that couldn't be sent.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> error::Error for SendError<T> {}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sending into a closed channel")
    }
}

/// An error returned from [`Receiver::recv()`].
///
/// Received because the channel is empty and closed.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct RecvError;

impl error::Error for RecvError {}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "receiving from an empty and closed channel")
    }
}

/// An error returned from [`Sender::try_send()`].
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TrySendError<T> {
    /// The channel is full but not closed.
    Full(T),

    /// The channel is closed.
    Closed(T),
}

impl<T> TrySendError<T> {
    /// Unwraps the message that couldn't be sent.
    pub fn into_inner(self) -> T {
        match self {
            TrySendError::Full(t) => t,
            TrySendError::Closed(t) => t,
        }
    }

    /// Returns `true` if the channel is full but not closed.
    pub fn is_full(&self) -> bool {
        match self {
            TrySendError::Full(_) => true,
            TrySendError::Closed(_) => false,
        }
    }

    /// Returns `true` if the channel is closed.
    pub fn is_closed(&self) -> bool {
        match self {
            TrySendError::Full(_) => false,
            TrySendError::Closed(_) => true,
        }
    }
}

impl<T> error::Error for TrySendError<T> {}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TrySendError::Full(..) => write!(f, "Full(..)"),
            TrySendError::Closed(..) => write!(f, "Closed(..)"),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TrySendError::Full(..) => write!(f, "sending into a full channel"),
            TrySendError::Closed(..) => write!(f, "sending into a closed channel"),
        }
    }
}

/// An error returned from [`Receiver::try_recv()`].
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryRecvError {
    /// The channel is empty but not closed.
    Empty,

    /// The channel is empty and closed.
    Closed,
}

impl TryRecvError {
    /// Returns `true` if the channel is empty but not closed.
    pub fn is_empty(&self) -> bool {
        match self {
            TryRecvError::Empty => true,
            TryRecvError::Closed => false,
        }
    }

    /// Returns `true` if the channel is empty and closed.
    pub fn is_closed(&self) -> bool {
        match self {
            TryRecvError::Empty => false,
            TryRecvError::Closed => true,
        }
    }
}

impl error::Error for TryRecvError {}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TryRecvError::Empty => write!(f, "receiving from an empty channel"),
            TryRecvError::Closed => write!(f, "receiving from an empty and closed channel"),
        }
    }
}
