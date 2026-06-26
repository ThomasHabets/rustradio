use log::error;
use serde::{Deserialize, Serialize};

use rustradio::stream::Tag;

use crate::TaggedVec;

const RANDOM_SENTINEL: usize = 0x3074_2D19;

/// This is the struct sent over the serialization layer for shared buffer data.
///
/// When received, a SharedVecPtr MUST reassembled into a `SharedVec` after
/// reception, or the memory will leak.
///
/// We can't trigger the drop from `SharedVecPtr`, because that would make it
/// trigger in the sending worker, and we don't want that (it'd be sent to the
/// wrong worker, which has no ability to drop it from the registry).
///
/// There is no `Ref` version of `SharedVecPtr`, because it's expected that the
/// non-sample part of the message has minimal overhead from an extra copy. This
/// assumption should be verified.
#[derive(Debug, Serialize, Deserialize)]
pub struct SharedVecPtr<T> {
    // This is the data's location in memory, and its size in number of
    // elements.
    ptr: usize,
    len: usize,
    cap: usize,

    // Tags are expected to be much smaller than the samples, so we send them
    // unshared for now.
    tags: Vec<Tag>,

    dummy: std::marker::PhantomData<T>,
}

impl<T: Send> SharedVecPtr<T> {
    /// Create a new shared vec that
    #[must_use]
    pub fn new(v: impl Into<Vec<T>>, tags: impl Into<Vec<Tag>>) -> Self {
        let v = v.into();
        let ptr = v.as_ptr() as usize;
        let cap = v.capacity();
        let len = v.len();
        std::mem::forget(v);
        Self {
            ptr,
            len,
            cap,
            tags: tags.into(),
            dummy: std::marker::PhantomData,
        }
    }
    #[must_use]
    pub fn into_vec(mut self) -> TaggedVec<T> {
        assert_ne!(self.ptr, 0);
        assert_ne!(self.ptr, RANDOM_SENTINEL);
        let data = unsafe { Vec::from_raw_parts(self.ptr as *mut T, self.len, self.cap) };
        let ret = TaggedVec {
            data,
            tags: std::mem::take(&mut self.tags),
        };
        self.forget();
        ret
    }
    /// Forget this pointer.
    pub fn forget(mut self) {
        if self.ptr == RANDOM_SENTINEL {
            error!("Called SharedVecPtr::forget() on an already forgotten pointer");
            return;
        }
        assert_ne!(self.ptr, 0);
        self.ptr = RANDOM_SENTINEL;
    }
}

impl<T> Drop for SharedVecPtr<T> {
    fn drop(&mut self) {
        error!("Dropped SharedVecPtr without converting it to an owned vec");
        debug_assert!(
            self.ptr == RANDOM_SENTINEL,
            "Dropped SharedVecPtr without converting it to an owned vec"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustradio::stream::TagValue;

    #[test]
    fn roundtrip() {
        let shared = SharedVecPtr::new(
            vec![1u8, 2, 3],
            vec![Tag::new(
                123,
                "hello",
                TagValue::String("world".to_string()),
            )],
        );
        let owned = shared.into_vec();
        assert_eq!(
            owned,
            TaggedVec {
                data: vec![1u8, 2, 3],
                tags: vec![Tag::new(
                    123,
                    "hello",
                    TagValue::String("world".to_string())
                )],
            }
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn roundtrip_panic() {
        let _shared = SharedVecPtr::new(
            vec![1u8, 2, 3],
            vec![Tag::new(
                123,
                "hello",
                TagValue::String("world".to_string()),
            )],
        );
    }
}
