pub mod circular_buffer;

pub mod export {
    use super::*;
    pub type Buffer<T> = circular_buffer::Buffer<T>;
    pub type BufferReader<T> = circular_buffer::BufferReader<T>;
    pub type BufferWriter<T> = circular_buffer::BufferWriter<T>;
}
