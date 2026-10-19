//! Typed producer-side callbacks, serialized under the stream's existing lock.
use std::any::TypeId;
use std::sync::{Arc, Weak};

use crate::stream::Tag;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Float,
    Complex,
    U8,
    String,
    Unsupported,
}
impl Kind {
    pub(crate) fn of<T: 'static>(messages: bool) -> Self {
        let t = TypeId::of::<T>();
        if messages {
            if t == TypeId::of::<String>() {
                Self::String
            } else {
                Self::Unsupported
            }
        } else if t == TypeId::of::<crate::Float>() {
            Self::Float
        } else if t == TypeId::of::<crate::Complex>() {
            Self::Complex
        } else if t == TypeId::of::<u8>() {
            Self::U8
        } else {
            Self::Unsupported
        }
    }
}

pub(crate) trait Control<T>: Send + Sync {
    // Lock order: stream state, then subscriber state. Never the reverse.
    #[cfg_attr(not(test), allow(dead_code))]
    fn with_state(&self, action: &mut dyn FnMut(&mut Observers<T>, u64, bool));
}

/// A typed handle erased only at the registry boundary. Retaining this handle
/// must not retain primary sample storage.
pub(crate) struct TypedControl<T>(pub Arc<dyn Control<T>>);
impl<T> std::fmt::Debug for TypedControl<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedControl").finish_non_exhaustive()
    }
}
impl<T> std::ops::Deref for TypedControl<T> {
    type Target = dyn Control<T>;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

pub(crate) trait Observer<T>: Send + Sync {
    /// Samples are borrowed only for this synchronous call under the stream lock.
    fn samples(&self, samples: &[T], tags: &[Tag], absolute: u64);
    fn end(&self);
}

pub(crate) struct Observers<T>(pub Vec<Weak<dyn Observer<T>>>);
impl<T> Default for Observers<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<T> std::fmt::Debug for Observers<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Observers").field(&self.0.len()).finish()
    }
}
impl<T> Observers<T> {
    pub fn samples(&mut self, samples: &[T], tags: &[Tag], absolute: u64) {
        self.0.retain(|observer| {
            if let Some(observer) = observer.upgrade() {
                observer.samples(samples, tags, absolute);
                true
            } else {
                false
            }
        });
    }
    pub fn end(&mut self) {
        for observer in self.0.drain(..).filter_map(|o| o.upgrade()) {
            observer.end();
        }
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn remove(&mut self, target: &Arc<dyn Observer<T>>) {
        self.0
            .retain(|o| !o.ptr_eq(&Arc::downgrade(target)) && o.strong_count() > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::stream::{StreamWait, TagValue, new_nocopy_stream, new_stream};
    use crate::stream_registry::{lookup, streams};

    #[derive(Debug, PartialEq)]
    enum Event<T> {
        Samples(u64, Vec<T>, Vec<Tag>),
        End,
    }
    struct Recording<T> {
        events: Mutex<Vec<Event<T>>>,
    }
    impl<T: Clone + Send + Sync> Observer<T> for Recording<T> {
        fn samples(&self, samples: &[T], tags: &[Tag], absolute: u64) {
            self.events.lock().unwrap().push(Event::Samples(
                absolute,
                samples.to_vec(),
                tags.to_vec(),
            ));
        }
        fn end(&self) {
            self.events.lock().unwrap().push(Event::End);
        }
    }
    fn attach<T: Clone + Send + Sync + 'static>(
        id: usize,
        messages: bool,
        position: u64,
    ) -> (Arc<Recording<T>>, Arc<TypedControl<T>>) {
        let (metadata, control) = lookup::<T>(id).unwrap();
        assert_eq!(metadata.kind, Kind::of::<T>(messages));
        assert_ne!(metadata.kind, Kind::Unsupported);
        let recording = Arc::new(Recording {
            events: Mutex::new(vec![]),
        });
        let observer: Arc<dyn Observer<T>> = recording.clone();
        control.with_state(&mut |observers, actual_position, ended| {
            assert_eq!(actual_position, position);
            assert!(!ended);
            observers.0.push(Arc::downgrade(&observer));
        });
        (recording, control)
    }

    fn numeric_batches<T>(samples: &[T]) -> crate::Result<()>
    where
        T: Copy + Default + Send + Sync + PartialEq + std::fmt::Debug + 'static,
    {
        let (writer, reader) = new_stream::<T>()?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples[..2]);
        window.produce(2, &[]);
        let (recording, control) = attach::<T>(writer.id(), false, 2);
        let (reading, _) = reader.read_buf()?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(samples);
        let tags = vec![Tag::new(1, "batch", TagValue::Bool(true))];
        drop(writer);
        control.with_state(&mut |_, position, ended| {
            assert_eq!(position, 2);
            assert!(!ended);
        });
        window.produce(samples.len(), &tags);
        assert_eq!(
            *recording.events.lock().unwrap(),
            [Event::Samples(2, samples.to_vec(), tags), Event::End]
        );
        control.with_state(&mut |observers, position, ended| {
            assert_eq!(position, 2 + samples.len() as u64);
            assert!(ended);
            assert!(observers.0.is_empty());
        });
        assert_eq!(reading.slice(), &samples[..2]);
        drop(reading);
        assert!(!reader.eof());
        let (reading, _) = reader.read_buf()?;
        assert_eq!(reading.slice(), [&samples[..2], samples].concat());
        reading.consume(2 + samples.len());
        assert!(reader.eof());
        Ok(())
    }

    #[test]
    fn numeric_callbacks_follow_producer_batches_and_preserve_primary_data() -> crate::Result<()> {
        numeric_batches(&[1f32, 2., 3.])?;
        numeric_batches(&[1u8, 2, 3])?;
        numeric_batches(&[
            crate::Complex::new(1., 2.),
            crate::Complex::new(3., 4.),
            crate::Complex::new(5., 6.),
        ])
    }

    #[test]
    fn removal_and_expired_subscriptions_do_not_receive_more_callbacks() -> crate::Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        let (recording, control) = attach::<u8>(writer.id(), false, 0);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1]);
        window.produce(1, &[]);
        let observer: Arc<dyn Observer<u8>> = recording.clone();
        control.with_state(&mut |observers, _, _| observers.remove(&observer));
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[2]);
        window.produce(1, &[]);
        assert_eq!(
            *recording.events.lock().unwrap(),
            [Event::Samples(0, vec![1], vec![])]
        );
        let (expired, _) = attach::<u8>(writer.id(), false, 2);
        let weak = Arc::downgrade(&expired);
        drop(expired);
        assert!(weak.upgrade().is_none());
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[3]);
        window.produce(1, &[]);
        control.with_state(&mut |observers, position, ended| {
            assert!(observers.0.is_empty());
            assert_eq!(position, 3);
            assert!(!ended);
        });
        drop(writer);
        assert_eq!(
            *recording.events.lock().unwrap(),
            [Event::Samples(0, vec![1], vec![])]
        );
        reader.read_buf()?.0.consume(3);
        assert!(reader.eof());
        Ok(())
    }

    #[test]
    fn string_callbacks_preserve_empty_messages_tags_and_primary_queue() {
        let (writer, reader) = new_nocopy_stream::<String>();
        writer.push("before".into(), []);
        let (recording, _control) = attach::<String>(writer.id(), true, 1);
        let tags = vec![Tag::new(0, "message", TagValue::Bool(true))];
        for value in ["", "hello", ""] {
            writer.push(value.into(), tags.as_slice());
        }
        drop(writer);
        assert_eq!(
            *recording.events.lock().unwrap(),
            [
                Event::Samples(1, vec![String::new()], tags.clone()),
                Event::Samples(2, vec![String::from("hello")], tags.clone()),
                Event::Samples(3, vec![String::new()], tags.clone()),
                Event::End,
            ]
        );
        assert!(!reader.eof());
        assert_eq!(reader.pop().unwrap().0, "before");
        for value in ["", "hello", ""] {
            assert_eq!(reader.pop().unwrap(), (value.to_owned(), tags.clone()));
        }
        assert!(reader.eof());
    }

    #[test]
    fn dropped_and_zero_length_windows_only_emit_completion() -> crate::Result<()> {
        for produce_zero in [false, true] {
            let (writer, reader) = new_stream::<u8>()?;
            let (recording, control) = attach::<u8>(writer.id(), false, 0);
            let window = writer.write_buf()?;
            drop(writer);
            control.with_state(&mut |_, _, ended| assert!(!ended));
            if produce_zero {
                window.produce(0, &[]);
            } else {
                drop(window);
            }
            assert_eq!(*recording.events.lock().unwrap(), [Event::End]);
            assert!(reader.eof());
        }
        Ok(())
    }

    #[test]
    fn invalid_commit_unwinds_with_completion_and_no_sample_callback() -> crate::Result<()> {
        let (writer, _reader) = new_stream::<u8>()?;
        let (recording, _control) = attach::<u8>(writer.id(), false, 0);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1]);
        drop(writer);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            window.produce(1, [Tag::new(1, "invalid", TagValue::Bool(true))].as_slice());
        }));
        assert!(result.is_err());
        assert_eq!(*recording.events.lock().unwrap(), [Event::End]);
        Ok(())
    }

    #[test]
    fn lookup_rejects_wrong_sample_type() -> crate::Result<()> {
        let (writer, _reader) = new_stream::<u8>()?;
        assert!(lookup::<u8>(writer.id()).is_some());
        assert!(lookup::<crate::Float>(writer.id()).is_none());
        assert!(lookup::<String>(writer.id()).is_none());
        let (writer, _reader) = new_nocopy_stream::<String>();
        assert!(lookup::<String>(writer.id()).is_some());
        assert!(lookup::<u8>(writer.id()).is_none());
        Ok(())
    }

    #[test]
    fn unsupported_types_remain_discoverable_without_subscription_controls() -> crate::Result<()> {
        let (_writer, reader) = new_stream::<u64>()?;
        let info = streams()
            .into_iter()
            .find(|s| s.name() == reader.name())
            .unwrap();
        assert!(lookup::<u64>(info.id).is_none());
        let (_writer, reader) = new_nocopy_stream::<std::sync::Arc<String>>();
        let info = streams()
            .into_iter()
            .find(|s| s.name() == reader.name())
            .unwrap();
        assert!(lookup::<std::sync::Arc<String>>(info.id).is_none());
        Ok(())
    }
}
