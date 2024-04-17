//! Window functions
//!
//! All functions are periodic, not symmetric.(?)
//!
//! https://en.wikipedia.org/wiki/Window_function
//! https://en.wikipedia.org/wiki/Spectral_leakage
//!
//! Notable missing window functions:
//! * Hann
//! * Rectangular
//! * Triangular
//! * Flat top
//! * Gaussian
//! * Taylor
use crate::Float;

const PI: Float = std::f64::consts::PI as Float;

/// Create Hamming window.
///
/// https://en.wikipedia.org/wiki/Window_function#Hann_and_Hamming_windows
pub fn hamming(ntaps: usize) -> Vec<Float> {
    // a0 notes:
    //
    // 0.54 is commonly used, but Hamming's paper sets it as 25/46.
    //
    // "In the equiripple sense, the optimal values for the
    // coefficients are a0 = 0.53836 and a1 = 0.46164".
    //
    // See wikipedia.
    let a0 = 25.0 / 46.0;
    let a1 = 1.0 - a0;
    let m = (ntaps - 1) as Float;
    (0..ntaps)
        .map(|n| a0 - a1 * (2.0 * PI * (n as Float) / m).cos())
        .collect()
}

/// Create Blackman window.
///
/// https://en.wikipedia.org/wiki/Window_function#Blackman_window
pub fn blackman(m: usize) -> Vec<Float> {
    let mut b = Vec::with_capacity(m);
    for n in 0..m {
        let n = n as Float;
        let m = m as Float;

        // Blackman's "not very serious proposal" magic value: 0.16.
        let a = 0.16;

        // Parameters.
        //
        // "exact Blackman" is:
        //   a0 = 7938/18608 ≈ 0.42659
        //   a1 = 9240/18608 ≈ 0.49656
        //   a2 = 1430/18608 ≈ 0.076849
        //
        // The truncated coefficients do not null the sidelobes as
        // well, but have an improved 18 dB/oct fall-off (compared do
        // 6dB for exact).

        let a0 = (1.0 - a) / 2.0;
        let a1 = 0.5;
        let a2 = a / 2.0;

        // Formula.
        let t1 = 2.0 * PI * n / m;
        let t2 = 4.0 * PI * n / m;
        b.push(a0 - a1 * t1.cos() + a2 * t2.cos());
    }
    b
}

/// Create Blackman-Harris window.
///
/// https://en.wikipedia.org/wiki/Window_function#Blackman%E2%80%93Harris_window
pub fn blackman_harris(m: usize) -> Vec<Float> {
    let mut b = Vec::with_capacity(m);
    for n in 0..m {
        let n = n as Float;
        let m = m as Float;

        // Parameters.
        const A0: Float = 0.35875;
        const A1: Float = 0.48829;
        const A2: Float = 0.14128;
        const A3: Float = 0.01168;

        // Formula.
        let t1 = 2.0 * PI * n / m;
        let t2 = 4.0 * PI * n / m;
        let t3 = 6.0 * PI * n / m;
        b.push(A0 - A1 * t1.cos() + A2 * t2.cos() - A3 * t3.cos());
    }
    b
}
