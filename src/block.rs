use anyhow::Result;

use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

#[macro_export]
macro_rules! get_output {
    ($w:expr, $index:expr) => {
        Self::get_output($w, $index).borrow_mut()
    };
}

#[macro_export]
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

/*
* TODO: why is this macro needed? Why can't MapBlock<T> override work()?
*/
#[macro_export]
macro_rules! map_block_macro {
    ($blockname:ident, $($tr:path), *) => {
        impl<T> Block for $blockname<T>
        where
            T: Copy $(+$tr)*,
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

#[macro_export]
macro_rules! map_block_macro_v2 {
    ($name:path, $($tr:path), *) => {
        impl<T> Block for $name
        where
            T: Copy $(+$tr)*,
            Streamp<T>: From<StreamType>,
        {
            fn work(
                &mut self,
                r: &mut InputStreams,
                w: &mut OutputStreams,
            ) -> Result<BlockRet, Error> {
                let i = Self::get_input(r, 0);
                Self::get_output(w, 0)
                    .borrow_mut()
                    .write(i
                           .borrow()
                           .iter()
                           .map(|x| self.process_one(x)));
                i.borrow_mut().clear();
                Ok(BlockRet::Ok)
            }
        }
    };
}
