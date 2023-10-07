use anyhow::Result;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub type Float = f32;
pub type Complex = num_complex::Complex<Float>;

struct ConstantSource<T> {
    val: T,
}

impl<T> ConstantSource<T>
where
    T: Copy,
{
    fn new(val: T) -> Self {
        Self { val }
    }
}

impl<T> Iterator for ConstantSource<T>
where
    T: Copy,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        Some(self.val)
    }
}

struct AddConst<'a, T> {
    src: &'a mut dyn Iterator<Item = T>,
    val: T,
}

impl<'a, T> AddConst<'a, T>
where
    T: Copy,
{
    fn new(src: &'a mut dyn Iterator<Item = T>, val: T) -> Self {
        Self { src, val }
    }
}

impl<'a, T> Iterator for AddConst<'a, T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.src.next().map(|v| v + self.val)
    }
}

struct FloatToComplex<'a> {
    src1: &'a mut dyn Iterator<Item = Float>,
    src2: &'a mut dyn Iterator<Item = Float>,
}

impl<'a> FloatToComplex<'a> {
    fn new(
        src1: &'a mut dyn Iterator<Item = Float>,
        src2: &'a mut dyn Iterator<Item = Float>,
    ) -> Self {
        Self { src1, src2 }
    }
}

impl<'a> Iterator for FloatToComplex<'a> {
    type Item = Complex;
    fn next(&mut self) -> Option<Complex> {
        let a = self.src1.next()?;
        let b = self.src2.next()?;
        Some(Complex::new(a, b))
    }
}

struct Tee<'a, T> {
    pub src: &'a mut dyn Iterator<Item = T>,
    for_left: VecDeque<T>,
    for_right: VecDeque<T>,
}

struct TeePipe<'a, T> {
    left: bool,
    parent: Rc<RefCell<Tee<'a, T>>>,
}

impl<'a, T> Tee<'a, T> {
    fn new(src: &'a mut dyn Iterator<Item = T>) -> (TeePipe<'a, T>, TeePipe<'a, T>) {
        let t = Rc::new(RefCell::new(Self {
            src,
            for_left: VecDeque::new(),
            for_right: VecDeque::new(),
        }));
        (
            TeePipe::<T> {
                parent: t.clone(),
                left: true,
            },
            TeePipe::<T> {
                parent: t.clone(),
                left: false,
            },
        )
    }
}

impl<'a, T> Iterator for TeePipe<'a, T>
where
    T: Copy,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.left {
            let mut m = self.parent.borrow_mut();
            if !m.for_left.is_empty() {
                return m.for_left.pop_front();
            }
            let ret = m.src.next()?;
            m.for_right.push_back(ret);
            Some(ret)
        } else {
            let mut m = self.parent.borrow_mut();
            if !m.for_right.is_empty() {
                return m.for_right.pop_front();
            }
            let ret = m.src.next()?;
            m.for_left.push_back(ret);
            Some(ret)
        }
    }
}

struct DebugSink<'a, T> {
    src: &'a mut dyn Iterator<Item = T>,
    dummy: std::marker::PhantomData<T>,
}

impl<'a, T> DebugSink<'a, T> {
    fn new(src: &'a mut dyn Iterator<Item = T>) -> Self {
        Self {
            src,
            dummy: std::marker::PhantomData,
        }
    }
}

impl<'a, T> Iterator for DebugSink<'a, T>
where
    T: std::fmt::Debug,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        for v in &mut *self.src {
            println!("debug> {:?}", v)
        }
        None
    }
}

fn main() -> Result<()> {
    let mut src = ConstantSource::new(1.0);
    let (mut tee1, mut tee2) = Tee::new(&mut src);
    let mut add = AddConst::new(&mut tee1, 0.5);
    let mut convert = FloatToComplex::new(&mut add, &mut tee2);
    let sink = DebugSink::new(&mut convert);
    for _ in sink {
        panic!("debugsink should never produc");
    }
    Ok(())
}
