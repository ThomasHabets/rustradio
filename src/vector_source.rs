//! Generate values from a fixed vector.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, new_streamp2, Streamp, Streamp2, Tag, TagValue};
use crate::Error;

/// Repeat or counts.
pub enum Repeat {
    /// Repeat finite number of times. 0 Means no output at all. 1 is default.
    Finite(u64),

    /// Repeat forever.
    Infinite,
}

/// VectorSource builder.
pub struct VectorSourceBuilder<T: Copy> {
    block: VectorSource<T>,
}

impl<T: Copy> VectorSourceBuilder<T> {
    /// New VectorSource builder.
    pub fn new(data: Vec<T>) -> Self {
        Self {
            block: VectorSource::new(data),
        }
    }
    /// Set a finite repeat count.
    pub fn repeat(mut self, r: u64) -> VectorSourceBuilder<T> {
        self.block.set_repeat(Repeat::Finite(r));
        self
    }
    /// Repeat the block forever.
    pub fn repeat_forever(mut self) -> VectorSourceBuilder<T> {
        self.block.set_repeat(Repeat::Infinite);
        self
    }
    /// Build the VectorSource.
    pub fn build(self) -> VectorSource<T> {
        self.block
    }
}

/// Generate values from a fixed vector.
pub struct VectorSource<T>
where
    T: Copy,
{
    dst: Streamp2<T>,
    data: Vec<T>,
    repeat: Repeat,
    repeat_count: u64,
    pos: usize,
}

impl<T: Copy> VectorSource<T> {
    /// Create new Vector Source block.
    ///
    /// Optionally the data can repeat.
    pub fn new(data: Vec<T>) -> Self {
        Self {
            dst: new_streamp2(),
            data,
            repeat: Repeat::Finite(1),
            pos: 0,
            repeat_count: 0,
        }
    }

    /// Set repeat status.
    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp2<T> {
        self.dst.clone()
    }
}

impl<T> Block for VectorSource<T>
where
    T: Copy,
{
    fn block_name(&self) -> &'static str {
        "VectorSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        if let Repeat::Finite(repeat) = self.repeat {
            if self.repeat_count == repeat {
                return Ok(BlockRet::EOF);
            }
        }
        let mut out = &self.dst;
        let n = std::cmp::min(out.capacity(), self.data.len() - self.pos);
        let mut tags = if self.pos == 0 {
            vec![
                Tag::new(0, "VectorSource::start".to_string(), TagValue::Bool(true)),
                Tag::new(
                    0,
                    "VectorSource::repeat".to_string(),
                    TagValue::U64(self.repeat_count),
                ),
            ]
        } else {
            vec![]
        };
        if self.repeat_count == 0 {
            tags.push(Tag::new(
                0,
                "VectorSource::first".to_string(),
                TagValue::Bool(true),
            ));
        }
        // TODO: write tags.

        let mut os = out.write_buf().unwrap();
        os.slice()[..n].clone_from_slice(&self.data[self.pos..(self.pos + n)]);
        os.produce(n);

        self.pos += n;
        if self.pos == self.data.len() {
            self.repeat_count += 1;
            self.pos = 0;
        }
        Ok(BlockRet::Ok)
    }
}
