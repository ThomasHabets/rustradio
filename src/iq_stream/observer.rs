//! Nonblocking, typed IQ subscriptions on producer commits.
#![forbid(unsafe_code)]
use std::sync::Arc;

use super::sink::{Resource, Session, queue_numeric, queue_string};
use super::{IqSample, proto};
use crate::stream::Tag;
use crate::stream_observer::{Kind, Observer, TypedControl};

// Connections erase only the subscription's item type. Detachment remains typed
// and releases the resource lock before acquiring the producer lock.
pub(super) trait Subscription: Send + Sync {
    fn detach(&self);
}
pub(super) struct TypedSubscription<T> {
    pub control: Arc<TypedControl<T>>,
    pub observer: Arc<dyn Observer<T>>,
}
impl<T> Subscription for TypedSubscription<T> {
    fn detach(&self) {
        self.control.with_state(&mut |observers, _, _| {
            observers.remove(&self.observer);
        });
    }
}

pub(super) struct Tap {
    pub resource: Arc<Resource>,
    pub generation: u64,
    pub _metadata: Arc<crate::stream_metadata::Metadata>,
}
impl Tap {
    fn observe(
        &self,
        count: usize,
        tags: &[Tag],
        absolute: u64,
        queue: impl FnOnce(&Resource, &mut Session, &[Tag]) -> crate::Result<()>,
    ) -> crate::Result<()> {
        let mut state = self.resource.lock();
        if state.stopped {
            return Ok(());
        }
        if let Some(session) = state
            .active
            .as_mut()
            .filter(|s| s.generation == self.generation)
        {
            if session.failure.is_some() {
                return Ok(());
            }
            super::sink::flush_gap(&self.resource, session);
            // Producers accept unsorted tags. Sort only for an active observer
            // retaining samples; a full queue drops the batch without encoding.
            let mut sorted;
            let tags = if session.queue.len() >= session.limits.max_in_flight_frames as usize
                || tags.windows(2).all(|pair| pair[0].pos() <= pair[1].pos())
            {
                tags
            } else {
                sorted = tags.to_vec();
                sorted.sort_by_key(|tag| tag.pos());
                &sorted
            };
            if let Err(error) = queue(&self.resource, session, tags) {
                session.failure = Some(error.to_string());
            }
        }
        state.position = super::advance(absolute, count as u64)?;
        drop(state);
        self.resource.notify.notify_one();
        Ok(())
    }
    fn record_failure(&self, result: crate::Result<()>) {
        if let Err(error) = result {
            if let Some(session) = self
                .resource
                .lock()
                .active
                .as_mut()
                .filter(|s| s.generation == self.generation)
            {
                session.failure = Some(error.to_string());
            }
            self.resource.notify.notify_one();
        }
    }
    fn finish(&self) {
        self.resource.lock().eof = true;
        self.resource.notify.notify_one();
    }
}
impl<T: IqSample> Observer<T> for Tap {
    fn samples(&self, samples: &[T], tags: &[Tag], absolute: u64) {
        self.record_failure(self.observe(
            samples.len(),
            tags,
            absolute,
            |resource, session, tags| numeric(resource, session, samples, tags, absolute),
        ));
    }
    fn end(&self) {
        self.finish();
    }
}
impl Observer<String> for Tap {
    fn samples(&self, messages: &[String], tags: &[Tag], absolute: u64) {
        self.record_failure(self.observe(
            messages.len(),
            tags,
            absolute,
            |resource, session, tags| {
                let [message] = messages else {
                    return Err(super::err(
                        "String observations require one message per push",
                    ));
                };
                if tags.iter().any(|tag| tag.pos() != 0) {
                    return Err(super::err("IQ string tags must have position zero"));
                }
                // Reject an oversized borrowed payload before allocating a copy.
                // Full queues discard messages without copying or encoding them.
                if session.queue.len() < session.limits.max_in_flight_frames as usize
                    && message.len() > session.limits.max_frame_bytes as usize
                {
                    return Err(super::err("IQ string and tags exceed frame limit"));
                }
                queue_string(session, tags, absolute, || {
                    let mut bytes = resource.pool.take(message.len());
                    bytes.extend_from_slice(message.as_bytes());
                    bytes
                })
            },
        ));
    }
    fn end(&self) {
        self.finish();
    }
}
fn numeric<T: IqSample>(
    resource: &Resource,
    session: &mut Session,
    input: &[T],
    tags: &[Tag],
    absolute: u64,
) -> crate::Result<()> {
    let mut offset = 0;
    while offset < input.len() {
        offset += queue_numeric(
            resource,
            session,
            &input[offset..],
            tags,
            super::advance(absolute, offset as u64)?,
            offset,
        )?;
    }
    Ok(())
}
pub(super) fn encoding(kind: Kind) -> Option<proto::Encoding> {
    Some(match kind {
        Kind::Float => crate::Float::encoding(),
        Kind::Complex => crate::Complex::encoding(),
        Kind::U8 => u8::encoding(),
        Kind::String => super::string_encoding(),
        Kind::Unsupported => return None,
    })
}
