//! Bounded coarse progress transport; never used by GPU copies or passes.
use super::{ObservationKind, Outcome};
use parking_lot::{Condvar, Mutex};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
    mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
};
use std::time::{Duration, Instant};

pub const PROGRESS_QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct ProgressEvent {
    pub id: u64,
    pub name: &'static str,
    pub kind: ObservationKind,
    pub begin: bool,
}

#[derive(Debug)]
struct Shared {
    start: Instant,
    dropped: AtomicU64,
    terminal: AtomicU8,
    terminal_elapsed_bits: AtomicU64,
    readiness: Mutex<Option<bool>>,
    ready: Condvar,
}

#[derive(Clone, Debug)]
pub(super) struct ProgressSender {
    sender: SyncSender<ProgressEvent>,
    shared: Arc<Shared>,
}

#[derive(Debug)]
pub struct CoarseProgressReceiver {
    receiver: Receiver<ProgressEvent>,
    shared: Arc<Shared>,
}

pub(super) fn channel(start: Instant) -> (ProgressSender, CoarseProgressReceiver) {
    let (sender, receiver) = sync_channel(PROGRESS_QUEUE_CAPACITY);
    let shared = Arc::new(Shared {
        start,
        dropped: AtomicU64::new(0),
        terminal: AtomicU8::new(0),
        terminal_elapsed_bits: AtomicU64::new(0),
        readiness: Mutex::new(None),
        ready: Condvar::new(),
    });
    (
        ProgressSender {
            sender,
            shared: Arc::clone(&shared),
        },
        CoarseProgressReceiver { receiver, shared },
    )
}

impl ProgressSender {
    pub(super) fn wait_ready(&self) -> bool {
        let mut ready = self.shared.readiness.lock();
        while ready.is_none() {
            self.shared.ready.wait(&mut ready);
        }
        ready.unwrap_or(false)
    }

    pub(super) fn wait_ready_or_cancel(&self, mut cancelled: impl FnMut() -> bool) -> Option<bool> {
        let mut ready = self.shared.readiness.lock();
        loop {
            if cancelled() {
                return None;
            }
            if let Some(ready) = *ready {
                return Some(ready);
            }
            self.shared
                .ready
                .wait_for(&mut ready, Duration::from_millis(10));
        }
    }

    pub(super) fn is_ready(&self) -> bool {
        self.shared.readiness.lock().is_some()
    }

    pub(super) fn send(&self, event: ProgressEvent) {
        if self.sender.try_send(event).is_err() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn finish(&self, outcome: Outcome) {
        let code = match outcome {
            Outcome::Succeeded => 1,
            Outcome::Failed => 2,
            Outcome::Cancelled => 3,
            Outcome::Superseded => 4,
        };
        self.shared.terminal_elapsed_bits.store(
            self.shared.start.elapsed().as_secs_f64().to_bits(),
            Ordering::Relaxed,
        );
        // Terminal delivery does not compete with the bounded event queue.
        self.shared.terminal.store(code, Ordering::Release);
    }
}

impl CoarseProgressReceiver {
    /// Release image workers only after the begin record has been synced.
    pub fn mark_ready(&self) {
        *self.shared.readiness.lock() = Some(true);
        self.shared.ready.notify_all();
    }

    pub fn try_recv(&self) -> Result<ProgressEvent, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn elapsed_seconds(&self) -> f64 {
        if self.shared.terminal.load(Ordering::Acquire) != 0 {
            f64::from_bits(self.shared.terminal_elapsed_bits.load(Ordering::Relaxed))
        } else {
            self.shared.start.elapsed().as_secs_f64()
        }
    }

    pub fn dropped_events(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    pub fn outcome(&self) -> Option<Outcome> {
        match self.shared.terminal.load(Ordering::Acquire) {
            1 => Some(Outcome::Succeeded),
            2 => Some(Outcome::Failed),
            3 => Some(Outcome::Cancelled),
            4 => Some(Outcome::Superseded),
            _ => None,
        }
    }
}

impl Drop for CoarseProgressReceiver {
    fn drop(&mut self) {
        let mut ready = self.shared.readiness.lock();
        if ready.is_none() {
            *ready = Some(false);
            self.shared.ready.notify_all();
        }
    }
}
