//! Shared-device application admission and queue retirement.
//!
//! Planned capacity is not VRAM residency. Vulkan HAL allocation bytes are a
//! separate, optional native measurement; the telemetry wire remains cumulative.
use super::{
    ObservedQueue, WgpuBackend, film_chain::device_image::BufferPayload, scan::PreparedScan,
};
use parking_lot::{Condvar, Mutex};
use spektrafilm_backend::telemetry::{ObservationContext, WorkCounters};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

/// Application policy, independent of the device's maximum binding size.
pub(super) const WORKSPACE_BINDING_BYTES: u64 = 64 << 20;
/// Soft overlap scheduling target, never a supported-workload size ceiling.
const OVERLAP_TARGET_BYTES: u64 = 8 << 30;
const MAX_PENDING_SUBMISSIONS: usize = 2;

#[derive(Clone)]
pub(super) struct Retirement {
    pub(super) index: wgpu::SubmissionIndex,
    complete: Arc<AtomicBool>,
}

impl Retirement {
    pub(super) fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    pub(super) fn same_submission(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.complete, &other.complete)
    }
}

#[derive(Default)]
struct AdmissionState {
    bytes: u64,
    owners: usize,
    exclusive: bool,
    waiters: usize,
    exclusive_waiters: usize,
}

struct AdmissionWaiter<'a> {
    resources: &'a DeviceResources,
    registered: bool,
    exclusive: bool,
}

impl AdmissionWaiter<'_> {
    fn register(&mut self, state: &mut AdmissionState, exclusive: bool) {
        if !self.registered {
            state.waiters += 1;
            self.registered = true;
        }
        if exclusive && !self.exclusive {
            state.exclusive_waiters += 1;
            self.exclusive = true;
        }
    }

    fn unregister(&mut self, state: &mut AdmissionState) {
        if self.registered {
            state.waiters -= 1;
            if self.exclusive {
                state.exclusive_waiters -= 1;
            }
            self.registered = false;
        }
    }
}

impl Drop for AdmissionWaiter<'_> {
    fn drop(&mut self) {
        if self.registered {
            let mut state = self.resources.reserved.lock();
            state.waiters -= 1;
            if self.exclusive {
                state.exclusive_waiters -= 1;
            }
            self.registered = false;
            drop(state);
            self.resources.released.notify_all();
        }
    }
}

#[derive(Default)]
struct Submissions {
    pending: VecDeque<Retirement>,
    submitting: usize,
}

struct SubmissionSlot<'a> {
    resources: &'a DeviceResources,
    armed: bool,
    raw_submit_started: bool,
    context: &'a ObservationContext,
}

impl Drop for SubmissionSlot<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.resources.submissions.lock().submitting -= 1;
            self.resources.submitted.notify_all();
            if self.raw_submit_started {
                // A validation/error callback can unwind raw submit. Retire any
                // work it accepted before command-origin owners unwind. An
                // unused slot needs no device wait when cancellation wins.
                self.resources.device.poll(wgpu::Maintain::Wait);
                self.context.record_work(&WorkCounters {
                    host_wait_count: Some(1),
                    ..Default::default()
                });
            }
        }
    }
}

#[derive(Default)]
struct RetainedOwners {
    images: Vec<Weak<BufferPayload>>,
    scans: Vec<Weak<PreparedScan>>,
}

pub(super) struct DeviceResources {
    device: wgpu::Device,
    native_buffer_counter: bool,
    overlap_target: u64,
    reserved: Mutex<AdmissionState>,
    released: Condvar,
    retained: Mutex<RetainedOwners>,
    submissions: Mutex<Submissions>,
    submitted: Condvar,
}

impl DeviceResources {
    pub(super) fn new(device: wgpu::Device, native_buffer_counter: bool) -> Self {
        Self {
            device,
            native_buffer_counter,
            overlap_target: OVERLAP_TARGET_BYTES,
            reserved: Mutex::new(AdmissionState::default()),
            released: Condvar::new(),
            retained: Mutex::new(RetainedOwners::default()),
            submissions: Mutex::new(Submissions::default()),
            submitted: Condvar::new(),
        }
    }

    pub(super) fn register_image(&self, image: &Arc<BufferPayload>) {
        let mut retained = self.retained.lock();
        retained.images.retain(|owner| owner.strong_count() > 0);
        let owner = Arc::downgrade(image);
        if !retained
            .images
            .iter()
            .any(|existing| existing.ptr_eq(&owner))
        {
            retained.images.push(owner);
        }
    }

    pub(super) fn register_scan(&self, scan: &Arc<PreparedScan>) {
        let mut retained = self.retained.lock();
        retained.scans.retain(|owner| owner.strong_count() > 0);
        let owner = Arc::downgrade(scan);
        if !retained
            .scans
            .iter()
            .any(|existing| existing.ptr_eq(&owner))
        {
            retained.scans.push(owner);
        }
    }

    /// Physical buffer capacity of known retained image carriers and prepared
    /// Scan caches. Weak registrations never prolong their lifetime. This is an
    /// application owner inventory, not all driver-owned/live GPU memory.
    pub(super) fn retained_owner_bytes(&self) -> Result<u64, String> {
        let (images, scans) = {
            let mut retained = self.retained.lock();
            let images: Vec<_> = retained.images.iter().filter_map(Weak::upgrade).collect();
            let scans: Vec<_> = retained.scans.iter().filter_map(Weak::upgrade).collect();
            retained.images.retain(|owner| owner.strong_count() > 0);
            retained.scans.retain(|owner| owner.strong_count() > 0);
            (images, scans)
        };
        // The upgraded owners remain alive until all borrowed buffer identities
        // have been counted. Aliased pages/carriers/cache entries count once.
        unique_buffer_bytes(
            images
                .iter()
                .flat_map(|owner| owner.pages.iter().map(|page| &page.buffer))
                .chain(scans.iter().flat_map(|owner| {
                    owner
                        .exposure
                        .iter()
                        .map(|page| &page.buffer)
                        .chain([&owner.setup, &owner.status])
                })),
        )
    }

    /// Unique native buffer allocation bytes, including driver-owned submitted
    /// resources. Only Vulkan's create/destroy accounting is verified in wgpu 24.
    /// Unsupported counters never masquerade as a zero live set.
    pub(super) fn native_buffer_bytes(&self) -> Option<u64> {
        self.native_buffer_counter
            .then(|| {
                u64::try_from(self.device.get_internal_counters().hal.buffer_memory.read()).ok()
            })
            .flatten()
    }

    pub(super) fn admit(
        self: &Arc<Self>,
        planned_peak_bytes: u64,
        cache_bytes: u64,
        cancellation: Option<&Arc<AtomicBool>>,
        context: &ObservationContext,
    ) -> Result<OperationGuard, String> {
        let mut waiter = AdmissionWaiter {
            resources: self,
            registered: false,
            exclusive: false,
        };
        let mut retired_prior = false;
        loop {
            if cancellation.is_some_and(|token| token.load(Ordering::Relaxed)) {
                return Err("operation cancelled".into());
            }
            let mut reserved = self.reserved.lock();
            let retained = match self.native_buffer_bytes() {
                Some(bytes) => bytes,
                None => self
                    .retained_owner_bytes()?
                    .checked_add(cache_bytes)
                    .ok_or("retained GPU owner capacity overflows")?,
            };
            let footprint = retained
                .checked_add(planned_peak_bytes)
                .ok_or("GPU operation planned footprint overflows")?;
            // A finite checked peak above the scheduling target is supported:
            // serialize it instead of imposing an invented device-memory limit.
            let exclusive = footprint > self.overlap_target || waiter.exclusive;
            let fits_overlap = !reserved.exclusive
                && reserved.exclusive_waiters == 0
                && footprint
                    .checked_add(reserved.bytes)
                    .is_some_and(|bytes| bytes <= self.overlap_target);
            if exclusive {
                waiter.register(&mut reserved, true);
                if reserved.owners == 0 && !retired_prior {
                    drop(reserved);
                    // Retire unowned/prior queue work outside all ledger locks.
                    self.retire_all(context);
                    retired_prior = true;
                    continue;
                }
            }
            if (exclusive && reserved.owners == 0 && retired_prior) || (!exclusive && fits_overlap)
            {
                let bytes = reserved
                    .bytes
                    .checked_add(planned_peak_bytes)
                    .ok_or("GPU operation reservations overflow")?;
                let owners = reserved
                    .owners
                    .checked_add(1)
                    .ok_or("GPU operation owner count overflows")?;
                waiter.unregister(&mut reserved);
                reserved.bytes = bytes;
                reserved.owners = owners;
                reserved.exclusive = exclusive;
                return Ok(OperationGuard {
                    state: Arc::new(OperationState {
                        resources: self.clone(),
                        bytes: planned_peak_bytes,
                        exclusive,
                        context: context.clone(),
                        pending: Mutex::new(OperationSubmissions::default()),
                    }),
                });
            }
            waiter.register(&mut reserved, exclusive);
            // Releasing submissions alone cannot free a live owner's credit.
            // Timed condition waits also observe cancellation without a helper
            // thread or requiring the cancellation producer to know our ledger.
            self.released
                .wait_for(&mut reserved, std::time::Duration::from_millis(20));
        }
    }

    fn retire(&self, retirement: &Retirement, context: &ObservationContext) {
        if retirement.complete.load(Ordering::Acquire) {
            return;
        }
        // No application lock is held while wgpu runs map/completion callbacks.
        // Cancellation never bypasses this ownership boundary. This raw poll
        // intentionally does not publish observation batch/image readiness.
        self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(
            retirement.index.clone(),
        ));
        context.record_work(&WorkCounters {
            host_wait_count: Some(1),
            ..Default::default()
        });
        retirement.complete.store(true, Ordering::Release);
        self.submissions
            .lock()
            .pending
            .retain(|pending| !pending.complete.load(Ordering::Acquire));
        self.submitted.notify_all();
    }

    pub(super) fn submit(
        &self,
        queue: &wgpu::Queue,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
        operation: Option<&Arc<OperationState>>,
        cancellation: Option<&Arc<AtomicBool>>,
        context: &ObservationContext,
    ) -> Result<Retirement, String> {
        loop {
            let mut submissions = self.submissions.lock();
            submissions
                .pending
                .retain(|p| !p.complete.load(Ordering::Acquire));
            if submissions.pending.len() + submissions.submitting < MAX_PENDING_SUBMISSIONS {
                submissions.submitting += 1;
                break;
            }
            if let Some(oldest) = submissions.pending.front().cloned() {
                drop(submissions);
                self.retire(&oldest, context);
            } else {
                // Other threads are inside raw submit. Do not hold their slot
                // lock while waiting for their submission indices to arrive.
                self.submitted.wait(&mut submissions);
            }
        }
        let mut slot = SubmissionSlot {
            resources: self,
            armed: true,
            raw_submit_started: false,
            context,
        };
        // Acquiring capacity may poll callbacks that cancel the request. Check
        // only after owning a slot, before consuming or submitting commands.
        if cancellation.is_some_and(|token| token.load(Ordering::Acquire)) {
            return Err("operation cancelled".into());
        }
        // Neither submit nor poll may run with our locks held: wgpu can execute
        // user map/completion callbacks in either call.
        slot.raw_submit_started = true;
        let index = queue.submit(commands);
        let retirement = Retirement {
            index,
            complete: Arc::new(AtomicBool::new(false)),
        };
        {
            let mut submissions = self.submissions.lock();
            submissions.submitting -= 1;
            submissions.pending.push_back(retirement.clone());
        }
        slot.armed = false;
        self.submitted.notify_all();
        if let Some(operation) = operation {
            operation.retain_submission(retirement.clone());
        }
        Ok(retirement)
    }

    pub(super) fn retire_all(&self, context: &ObservationContext) {
        loop {
            let mut submissions = self.submissions.lock();
            if let Some(pending) = submissions.pending.front().cloned() {
                drop(submissions);
                self.retire(&pending, context);
            } else if submissions.submitting > 0 {
                self.submitted.wait(&mut submissions);
            } else {
                return;
            }
        }
    }
}

#[derive(Default)]
struct OperationSubmissions {
    pending: Vec<Retirement>,
    latest: Option<Retirement>,
}

pub(super) struct OperationState {
    resources: Arc<DeviceResources>,
    bytes: u64,
    exclusive: bool,
    context: ObservationContext,
    pending: Mutex<OperationSubmissions>,
}

impl OperationState {
    pub(super) fn retain_submission(&self, retirement: Retirement) {
        let mut submissions = self.pending.lock();
        submissions.pending.retain(|p| !p.is_complete());
        submissions.latest = Some(retirement.clone());
        submissions.pending.push(retirement);
    }
}

impl Drop for OperationState {
    fn drop(&mut self) {
        // This final owner may be the guard or a backend derivative that outlived
        // it. Retire only this operation's submitted work before returning credit.
        for pending in &self.pending.get_mut().pending {
            self.resources.retire(pending, &self.context);
        }
        let mut reserved = self.resources.reserved.lock();
        reserved.bytes -= self.bytes;
        reserved.owners -= 1;
        if self.exclusive {
            reserved.exclusive = false;
        }
        drop(reserved);
        self.resources.released.notify_all();
    }
}

pub(super) struct OperationGuard {
    state: Arc<OperationState>,
}

impl OperationGuard {
    pub(super) fn reserved_bytes(&self) -> u64 {
        self.state.bytes
    }

    pub(super) fn last_submission(&self) -> Option<Retirement> {
        self.state.pending.lock().latest.clone()
    }

    /// Spill only this operation's submitted resources when the known chain's
    /// next working set cannot coexist with its outstanding stage workspace.
    pub(super) fn retire_pending(&self) {
        let pending = std::mem::take(&mut self.state.pending.lock().pending);
        for retirement in pending {
            self.state
                .resources
                .retire(&retirement, &self.state.context);
        }
    }

    pub(super) fn queue(&self, queue: &ObservedQueue) -> ObservedQueue {
        let mut queue = queue.clone();
        queue.operation = Some(self.state.clone());
        queue
    }

    /// Bind every nested input/viewer/render/export derivative to this owner.
    /// Cloned derivatives retain the reservation until their last GPU use retires.
    pub(super) fn backend(&self, backend: &WgpuBackend) -> WgpuBackend {
        let mut device = backend.device.clone();
        device.operation = Some(self.state.clone());
        let mut queue = backend.queue.clone();
        queue.operation = Some(self.state.clone());
        WgpuBackend {
            pipe: backend.pipe.with_observed(device.clone(), queue.clone()),
            device,
            queue,
            pipeline_cache: backend.pipeline_cache.clone(),
            scan_cache: backend.scan_cache.clone(),
            name: backend.name.clone(),
            adapter: backend.adapter.clone(),
            explicit: backend.explicit,
            backend_id: backend.backend_id,
            cancellation: backend.cancellation.clone(),
        }
    }
}

impl WgpuBackend {
    /// Reserve the checked peak of new allocations before constructing a stage.
    /// Already retained input/cache/carrier allocations belong to the native
    /// sample or weak owner inventory, not this additional working set.
    pub(super) fn admit_operation(
        &self,
        planned_peak_bytes: u64,
    ) -> Result<OperationGuard, String> {
        self.check_cancelled()?;
        if let Some(operation) = &self.queue.operation {
            if planned_peak_bytes > operation.bytes {
                return Err("nested GPU operation exceeds its declared parent peak".into());
            }
            return Ok(OperationGuard {
                state: operation.clone(),
            });
        }
        let cache_bytes = if self.device.resources.native_buffer_bytes().is_none() {
            self.pipe.retained_cache_bytes()?
        } else {
            0
        };
        self.device.resources.admit(
            planned_peak_bytes,
            cache_bytes,
            self.cancellation.as_ref(),
            &self.device.context,
        )
    }
}

/// Count physical buffers once, not once per page view or cloned handle. This is
/// an owner inventory, not a device-wide liveness metric: opaque bindgroups and
/// driver-owned command buffers can retain buffers beyond the supplied owners.
pub(super) fn unique_buffer_bytes<'a>(
    buffers: impl IntoIterator<Item = &'a wgpu::Buffer>,
) -> Result<u64, String> {
    // wgpu Buffer equality/hash delegates to the immutable dispatch identity.
    #[allow(clippy::mutable_key_type)]
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for buffer in buffers {
        if seen.insert(buffer) {
            bytes = bytes
                .checked_add(buffer.size())
                .ok_or("GPU physical allocation inventory overflows")?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
