pub trait Sample {
    type Type;
    fn size() -> usize;
    fn parse(data: &[u8]) -> Result<Self::Type>;
}

impl Sample for Complex {
    type Type = Complex;
    fn size() -> usize {
        8
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}

impl Sample for Float {
    type Type = Float;
    fn size() -> usize {
        4
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}
impl Sample for u32 {
    type Type = u32;
    fn size() -> usize {
        4
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}
