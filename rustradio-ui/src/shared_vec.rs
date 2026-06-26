use serde::{Deserialize, Serialize};

use rustradio::stream::Tag;

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
    pub fn into_vec(self) -> SharedVec<T> {
        SharedVec {
            data: unsafe { Vec::from_raw_parts(self.ptr as *mut T, self.len, self.cap) },
            tags: self.tags,
        }
    }
}

/// A received shared memory vector, plus its tags.
pub struct SharedVec<T> {
    pub data: Vec<T>,
    pub tags: Vec<Tag>,
}
