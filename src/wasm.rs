pub mod export {
    use crate::Result;
    use crate::stream::Tag;
    use std::sync::Arc;
    use std::sync::Mutex;
    //use std::collections::VecDeque;

    #[derive(Debug)]
    struct BufferState<T> {
        rpos: usize,
        wpos: usize,
        used: usize,
        stream: Vec<T>,
    }
    impl<T: Default + Clone> BufferState<T> {
        fn new(size: usize) -> Self {
            Self {
                rpos: 0,
                wpos: 0,
                used: 0,
                stream: vec![T::default(); size],
            }
        }
    }
    impl<T> BufferState<T> {
        #[must_use]
        fn capacity(&self) -> usize {
            self.size()
        }
        #[must_use]
        fn free(&self) -> usize {
            self.size() - self.used
        }
        #[must_use]
        fn size(&self) -> usize {
            self.stream.len()
        }
    }
    #[derive(Debug)]
    pub struct Buffer<T> {
        id: usize,
        state: Mutex<BufferState<T>>,
    }
    impl<T: Clone + Default> Buffer<T> {
        pub fn new(size: usize) -> Result<Self> {
            Ok(Self {
                id: crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                state: Mutex::new(BufferState::new(size)),
            })
        }
    }
    impl<T> Buffer<T> {
        pub fn id(&self) -> usize {
            self.id
        }
        pub(crate) fn is_empty(&self) -> bool {
            self.state.lock().unwrap().used == 0
        }
        pub(crate) fn len(&self) -> usize {
            self.state.lock().unwrap().used
        }
        /// Available space to write, in bytes(?).
        pub(crate) fn free(&self) -> usize {
            self.state.lock().unwrap().free()
        }
        pub(crate) fn slice(&self, start: usize, end: usize) -> &[T] {
            self.slice_mut(start, end)
        }
        pub(crate) fn slice_mut(&self, start: usize, end: usize) -> &mut [T] {
            unsafe {
                let l = self.state.lock().unwrap();
                let ptr = l.stream.as_ptr() as *mut T;
                std::slice::from_raw_parts_mut(ptr.add(start), end - start)
            }
        }
        pub fn consume(&self, n: usize) {
            let mut l = self.state.lock().unwrap();
            l.rpos = (l.rpos + n) % l.size();
            l.used -= n;
        }
        pub fn produce(&self, n: usize, _tags: &[Tag]) {
            // TODO: tags.
            let mut l = self.state.lock().unwrap();
            l.wpos = (l.wpos + n) % l.size();
            l.used += n + 1;
        }
        pub fn total_size(&self) -> usize {
            self.len()
        }
        pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
            let l = self.state.lock().unwrap();
            let start = l.rpos;
            let end = if l.wpos < l.rpos {
                l.stream.len()
            } else {
                l.wpos
            };
            drop(l);
            // TODO: tag support.
            Ok((BufferReader::new(self, start, end), vec![]))
        }
        pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
            let l = self.state.lock().unwrap();
            let start = l.wpos;
            let end = if l.rpos <= l.rpos {
                l.stream.len()
            } else {
                l.rpos
            };
            drop(l);
            Ok(BufferWriter::new(self, start, end))
        }
        pub fn wait_for_write(&self, _need: usize) -> usize {
            // TODO
            1
        }
        pub fn wait_for_read(&self, _need: usize) -> usize {
            // TODO
            1
        }
    }

    pub struct BufferReader<T> {
        parent: Arc<Buffer<T>>,
        start: usize,
        end: usize,
    }
    impl<T> BufferReader<T> {
        #[must_use]
        fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> Self {
            Self { parent, start, end }
        }

        /// Return slice to read from.
        #[must_use]
        pub fn slice(&self) -> &[T] {
            self.parent.slice(self.start, self.end)
        }

        /// Helper function to iterate over input instead.
        pub fn iter(&self) -> std::slice::Iter<'_, T> {
            self.slice().iter()
        }

        /// We're done with the buffer. Consume `n` samples.
        pub fn consume(self, n: usize) {
            self.parent.consume(n);
        }

        /// len convenience function.
        #[must_use]
        pub fn len(&self) -> usize {
            self.slice().len()
        }

        /// is_empty convenience function.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }
    }
    pub struct BufferWriter<T> {
        parent: Arc<Buffer<T>>,
        start: usize,
        end: usize,
    }
    impl<T> BufferWriter<T> {
        #[must_use]
        fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> BufferWriter<T> {
            Self { parent, start, end }
        }

        /// Return the slice to write to.
        #[must_use]
        pub fn slice(&mut self) -> &mut [T] {
            self.parent.slice_mut(self.start, self.end)
        }
    }
    impl<T: Copy> BufferWriter<T> {
        /// Shortcut to save typing for the common operation of copying
        /// from an iterator.
        pub fn fill_from_slice(&mut self, src: &[T]) {
            self.slice()[..src.len()].copy_from_slice(src);
        }

        /// Shortcut to save typing for the common operation of copying
        /// from an iterator.
        pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
            for (place, item) in self.slice().iter_mut().zip(src) {
                *place = item;
            }
        }

        /// Having written into the write buffer, now tell the buffer
        /// we're done. Also here are the tags, with positions relative to
        /// start of buffer.
        ///
        // Tags inherently need to be copied in, because they need to be added to
        // the underlying stream.
        pub fn produce(self, n: usize, tags: &[Tag]) {
            self.parent.produce(n, tags);
        }

        /// len convenience function.
        #[must_use]
        pub fn len(&self) -> usize {
            self.parent.slice(self.start, self.end).len()
        }

        /// is_empty convenience function.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }
    }
}
