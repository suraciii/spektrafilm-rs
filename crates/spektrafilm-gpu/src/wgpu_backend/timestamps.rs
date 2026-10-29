//! Bounded timestamp resources, leased until the existing completion poll maps them.
//!
//! Query resolution travels in the image's command buffer. Mapping callbacks
//! never submit or poll; they return a block only after unmapping it.
use super::observation::ObservedDevice;
use parking_lot::Mutex;
use spektrafilm_backend::telemetry::{AvailabilityReason, ObservationContext, WorkCounters};
use std::sync::Arc;

pub(super) const QUERY_CAPACITY: u32 = 2048;
const MAX_BLOCKS: usize = 8;

#[derive(Default)]
pub(super) struct TimestampPool {
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    allocated: usize,
    idle: Vec<QueryBlock>,
}

struct QueryBlock {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
}

pub(super) struct QueryLease {
    pool: Arc<TimestampPool>,
    block: Option<QueryBlock>,
}

impl TimestampPool {
    pub(super) fn acquire(
        self: &Arc<Self>,
        device: &ObservedDevice,
        context: &ObservationContext,
    ) -> Option<QueryLease> {
        let mut state = self.state.lock();
        if let Some(block) = state.idle.pop() {
            return Some(QueryLease {
                pool: self.clone(),
                block: Some(block),
            });
        }
        if state.allocated == MAX_BLOCKS {
            return None;
        }
        state.allocated += 1;
        drop(state);
        let bytes = u64::from(QUERY_CAPACITY) * 8;
        let block = QueryBlock {
            set: device.raw.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("telemetry_timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERY_CAPACITY,
            }),
            resolve: device.raw.create_buffer(&wgpu::BufferDescriptor {
                label: Some("telemetry_resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.raw.create_buffer(&wgpu::BufferDescriptor {
                label: Some("telemetry_readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        context.record_work(&WorkCounters {
            telemetry_buffer_create_count: Some(2),
            telemetry_buffer_create_bytes: Some(bytes * 2),
            ..Default::default()
        });
        Some(QueryLease {
            pool: self.clone(),
            block: Some(block),
        })
    }
}

impl QueryLease {
    pub(super) fn set(&self) -> &wgpu::QuerySet {
        &self.block.as_ref().expect("live query lease").set
    }

    #[cfg(test)]
    pub(super) fn readback(&self) -> &wgpu::Buffer {
        &self.block.as_ref().expect("live query lease").readback
    }

    pub(super) fn resolve(&self, encoder: &mut wgpu::CommandEncoder, count: u32) {
        let block = self.block.as_ref().expect("live query lease");
        encoder.resolve_query_set(&block.set, 0..count, &block.resolve, 0);
        encoder.copy_buffer_to_buffer(&block.resolve, 0, &block.readback, 0, u64::from(count) * 8);
    }

    /// Register only after queue submission. No new completion boundary is added.
    pub(super) fn map(
        self,
        count: u32,
        complete: impl FnOnce(Result<&[u64], AvailabilityReason>) + Send + 'static,
    ) {
        let buffer = self
            .block
            .as_ref()
            .expect("live query lease")
            .readback
            .clone();
        buffer
            .slice(..u64::from(count) * 8)
            .map_async(wgpu::MapMode::Read, move |result| {
                let block = self.block.as_ref().expect("live query lease");
                if result.is_err() {
                    complete(Err(AvailabilityReason::QueryFailed));
                    return;
                }
                let mapped = block
                    .readback
                    .slice(..u64::from(count) * 8)
                    .get_mapped_range();
                complete(
                    bytemuck::try_cast_slice(&mapped)
                        .map_err(|_| AvailabilityReason::InvalidTimestamp),
                );
                drop(mapped);
                block.readback.unmap();
                // `self` returns the unmapped block to the pool at the end of the callback.
            });
    }
}

impl Drop for QueryLease {
    fn drop(&mut self) {
        if let Some(block) = self.block.take() {
            self.pool.state.lock().idle.push(block);
        }
    }
}
