//! Window functions
//!
//! https://en.wikipedia.org/wiki/Window_function
use crate::Float;

/// Create Hamming window.
///
/// https://en.wikipedia.org/wiki/Window_function#Hann_and_Hamming_windows
pub fn hamming(ntaps: usize) -> Vec<Float> {
    let pi = std::f64::consts::PI as Float;

    // a0 notes:
    //
    // 0.54 is commonly used, but Hamming's paper sets it as 25/46.
    //
    // "In the equiripple sense, the optimal values for the
    // coefficients are a0 = 0.53836 and a1 = 0.46164".
    //
    // See wikipedia.
    let a0 = 25.0 / 46.0; // 0.54 is commonly used.
    let a1 = 1.0 - a0;
    let m = (ntaps - 1) as Float;
    (0..ntaps)
        .map(|n| a0 - a1 * (2.0 * pi * (n as Float) / m).cos())
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
        let pi = std::f64::consts::PI as Float;

        // Blackman's "not very serious proposal" magic value: 0.16.
        let a = 0.16;

        // Parameters.
        let a0 = (1.0 - a) / 2.0;
        let a1 = 0.5;
        let a2 = a / 2.0;

        // Formula.
        let t1 = 2.0 * pi * n / m;
        let t2 = 4.0 * pi * n / m;
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
        let pi = std::f64::consts::PI as Float;

        // Parameters.
        let a0 = 0.35875;
        let a1 = 0.48829;
        let a2 = 0.14128;
        let a3 = 0.01168;

        // Formula.
        let t1 = 2.0 * pi * n / m;
        let t2 = 4.0 * pi * n / m;
        let t3 = 6.0 * pi * n / m;
        b.push(a0 - a1 * t1.cos() + a2 * t2.cos() - a3 * t3.cos());
    }
    b
}
