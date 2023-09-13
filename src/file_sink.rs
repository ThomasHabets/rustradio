use anyhow::Result;
use log::debug;
use std::io::Write;

use crate::{Sample, Sink, StreamReader};

pub enum Mode {
    Create,
    Overwrite,
    Append,
}

pub struct FileSink<T> {
    _t: T, // TODO: remove this dummy.
    f: std::fs::File,
}

impl<T> FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    pub fn new(filename: String, mode: Mode) -> Result<Self> {
        let f = match mode {
            Mode::Create => {
                todo!()
            }
            Mode::Overwrite => std::fs::File::create(&filename)?,
            Mode::Append => {
                todo!()
            }
        };
        debug!("Opening sink {filename}");
        Ok(Self {
            f,
            _t: T::default(),
        })
    }
}

impl<T> Sink<T> for FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()> {
        for s in r.buffer() {
            self.f.write_all(&s.serialize())?;
        }
        Ok(())
    }
}
