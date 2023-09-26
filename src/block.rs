use anyhow::Result;

use crate::{Error, InputStreams, OutputStreams, StreamType, Streamp};

macro_rules! get_output {
    ($w:expr, $index:expr) => {
        Self::get_output($w, $index).borrow_mut()
    };
}

macro_rules! get_input {
    ($r:expr, $index:expr) => {
        Self::get_input($r, $index).borrow()
    };
}

pub enum BlockRet {
    Ok,
}

pub trait Block {
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error>;
    fn get_input<T>(r: &InputStreams, n: usize) -> Streamp<T>
    where
        T: Copy,
        Streamp<T>: From<StreamType>,
    {
        let ret: Streamp<T> = r.get(n).into();
        ret
    }

    fn get_output<T>(w: &mut OutputStreams, n: usize) -> Streamp<T>
    where
        T: Copy,
        Streamp<T>: From<StreamType>,
    {
        let output: Streamp<T> = w.get(n).into();
        output
    }
}

pub trait MapBlock<T>: Block
where
    T: Copy,
    Streamp<T>: From<StreamType>,
{
    fn work_map_block(
        &mut self,
        r: &mut InputStreams,
        w: &mut OutputStreams,
    ) -> Result<BlockRet, Error> {
        // get_output!(w, 0).write(get_input!(r, 0).iter().map(|x| *x + self.val));
        get_output!(w, 0).write(get_input!(r, 0).iter().map(|x| self.process_one(*x)));
        Ok(BlockRet::Ok)
    }
    fn process_one(&self, a: T) -> T;
}

#[macro_export]
macro_rules! map_block_macro {
    ($blockname:ident) => {
        impl<T> Block for $blockname<T>
        where
            T: Copy + std::ops::Add<Output = T>,
            Streamp<T>: From<StreamType>,
        {
            fn work(
                &mut self,
                r: &mut InputStreams,
                w: &mut OutputStreams,
            ) -> Result<BlockRet, Error> {
                self.work_map_block(r, w)
            }
        }
    };
}
