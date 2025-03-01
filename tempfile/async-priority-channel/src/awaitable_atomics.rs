use event_listener::{Event, EventListener};
use std::{
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

const U64_TOP_BIT_MASK: u64 = 0x1000000000000000;

#[derive(Debug)]
pub struct AwaitableAtomicCounterAndBit {
    incr_event: Event,
    decr_event: Event,
    value: AtomicU64,
}

impl AwaitableAtomicCounterAndBit {
    pub fn new(value: u64) -> Self {
        if value & U64_TOP_BIT_MASK > 0 {
            panic!("Initial value cannot be larger than 2**63");
        }
        Self {
            incr_event: Event::new(),
            decr_event: Event::new(),
            value: AtomicU64::new(value),
        }
    }

    pub fn set_bit(&self) -> bool {
        let prior = self.value.fetch_or(U64_TOP_BIT_MASK, Ordering::SeqCst);
        self.incr_event.notify(usize::MAX);
        self.decr_event.notify(usize::MAX);
        prior & U64_TOP_BIT_MASK > 0
    }

    pub fn incr(&self, n: u64) -> (bool, u64) {
        let prior = self.value.fetch_add(n, Ordering::SeqCst);
        if prior & !U64_TOP_BIT_MASK >= (1 << 63) - 1 {
            panic!("Cannot increase size past 2**63-1");
        }
        self.incr_event.notify(usize::MAX);
        (prior & U64_TOP_BIT_MASK > 0, prior & !U64_TOP_BIT_MASK)
    }

    pub fn decr(&self) -> (bool, u64) {
        let prior = self.value.fetch_sub(1, Ordering::SeqCst);
        self.decr_event.notify(usize::MAX);
        (prior & U64_TOP_BIT_MASK > 0, prior & !U64_TOP_BIT_MASK)
    }

    pub fn load(&self) -> (bool, u64) {
        let value = self.value.load(Ordering::SeqCst);
        (value & U64_TOP_BIT_MASK > 0, value & !U64_TOP_BIT_MASK)
    }

    pub fn listen_incr(&self) -> Pin<Box<EventListener>> {
        self.incr_event.listen()
    }
    pub fn listen_decr(&self) -> Pin<Box<EventListener>> {
        self.decr_event.listen()
    }
}
