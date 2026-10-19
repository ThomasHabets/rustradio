//! Nonblocking, typed IQ subscriptions on producer commits.
#![forbid(unsafe_code)]
use std::sync::Arc;

use super::sink::{Resource, Session, queue_numeric};
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
    fn observe<T: IqSample>(&self, input: &[T], tags: &[Tag], absolute: u64) -> crate::Result<()> {
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
            if let Err(error) = numeric(&self.resource, session, input, tags, absolute) {
                session.failure = Some(error.to_string());
            }
        }
        state.position = super::advance(absolute, input.len() as u64)?;
        drop(state);
        self.resource.notify.notify_one();
        Ok(())
    }
}
impl<T: IqSample> Observer<T> for Tap {
    fn samples(&self, samples: &[T], tags: &[Tag], absolute: u64) {
        if let Err(error) = self.observe(samples, tags, absolute) {
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
    fn end(&self) {
        self.resource.lock().eof = true;
        self.resource.notify.notify_one();
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
        Kind::String | Kind::Unsupported => return None,
    })
}
