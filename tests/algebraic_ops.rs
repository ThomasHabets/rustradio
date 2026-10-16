//! Public arithmetic capabilities, mixed operands, and minimal FIR bounds.
use rustradio::fir::Fir;
use rustradio::{AlgebraicAdd, AlgebraicDiv, AlgebraicMul, Result, SampleEncoding};

fn add<L: AlgebraicAdd<R>, R>(lhs: L, rhs: R) -> L::Output {
    lhs.algebraic_add(rhs)
}
fn mul<L: AlgebraicMul<R>, R>(lhs: L, rhs: R) -> L::Output {
    lhs.algebraic_mul(rhs)
}
fn div<L: AlgebraicDiv<R>, R>(lhs: L, rhs: R) -> L::Output {
    lhs.algebraic_div(rhs)
}

macro_rules! float_operations {
    ($name:ident, $float:ty, $tolerance:expr) => {
        #[test]
        fn $name() {
            type Complex = num_complex::Complex<$float>;
            let a = Complex::new(3.0, -4.0);
            let b = Complex::new(1.0, 2.0);
            let scalar: $float = 2.0;
            assert_eq!(add(scalar, scalar), 4.0);
            assert_eq!(mul(scalar, scalar), 4.0);
            assert_eq!(div(scalar, scalar), 1.0);
            assert_eq!(add(a, b), Complex::new(4.0, -2.0));
            assert_eq!(mul(a, b), Complex::new(11.0, 2.0));
            assert!((div(a, b) - Complex::new(-1.0, -2.0)).norm() < $tolerance);
            assert_eq!(add(a, scalar), Complex::new(5.0, -4.0));
            assert_eq!(add(scalar, a), Complex::new(5.0, -4.0));
            assert_eq!(mul(a, scalar), Complex::new(6.0, -8.0));
            assert_eq!(mul(scalar, a), Complex::new(6.0, -8.0));
            assert_eq!(div(a, scalar), Complex::new(1.5, -2.0));
            assert!((div(scalar, a) - Complex::new(0.24, 0.32)).norm() < $tolerance);
            assert!((div(a, a) - Complex::new(1.0, 0.0)).norm() < $tolerance);
            // Complex method syntax selects the appropriate RHS implementation.
            assert_eq!(a.algebraic_mul(scalar), mul(a, scalar));
        }
    };
}
float_operations!(f32_operations, f32, 1e-6);
float_operations!(f64_operations, f64, 1e-12);

#[test]
fn one_capability_can_have_distinct_operand_and_output_types() {
    struct Count(u8);
    impl AlgebraicAdd<u16> for Count {
        type Output = u32;
        fn algebraic_add(self, rhs: u16) -> u32 {
            u32::from(self.0) + u32::from(rhs)
        }
    }
    // Count implements neither multiplication nor division.
    let result: u32 = add(Count(255), 65_535u16);
    assert_eq!(result, 65_790);
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
struct FilterSample(f32);

impl SampleEncoding for FilterSample {
    const ENCODED_SIZE: std::num::NonZeroUsize = <f32 as SampleEncoding>::ENCODED_SIZE;
    fn parse(bytes: &[u8]) -> Result<Self> {
        <f32 as SampleEncoding>::parse(bytes).map(Self)
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        self.0.serialize_into(out);
    }
}
impl AlgebraicAdd for FilterSample {
    type Output = Self;
    fn algebraic_add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}
impl AlgebraicMul for FilterSample {
    type Output = Self;
    fn algebraic_mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}

#[test]
fn fir_needs_only_algebraic_add_and_mul() -> Result<()> {
    // No AlgebraicDiv, std::ops::Add, or std::ops::Mul implementation.
    let filter = Fir::new(vec![FilterSample(2.0), FilterSample(3.0)])?;
    assert_eq!(
        filter.filter(&[FilterSample(5.0), FilterSample(7.0)]),
        FilterSample(29.0)
    );
    Ok(())
}
