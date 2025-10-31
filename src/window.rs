/*! Window functions

All functions are periodic, not symmetric.(?)

<https://en.wikipedia.org/wiki/Window_function>
<https://en.wikipedia.org/wiki/Spectral_leakage>

Notable missing window functions:
* Hann
* Rectangular
* Triangular
* Flat top
* Gaussian
* Taylor

# Example

```
use rustradio::window::WindowType;
let window_type = WindowType::Hamming;
let ntaps = 3;
let taps = window_type.make_window(ntaps).0;
assert_eq!(taps.len(), ntaps);

let correct = [0.0869565, 1.0, 0.0869565];
assert_eq!(correct.len(), taps.len());
for (x,y) in taps.iter().zip(correct) {
  assert!((x-y).abs() < 0.1);
}
```
*/
use crate::Float;

const PI: Float = std::f64::consts::PI as Float;

// 0.54 is commonly used, but Hamming's paper sets it as 25/46.
const DEFAULT_HAMMING_PARM: Float = 25.0 / 46.0;

/// Window type.
///
/// See <https://en.wikipedia.org/wiki/Window_function>
pub enum WindowType {
    /// Blackman window.
    Blackman,

    /// Blackman-Harris window.
    BlackmanHarris,

    /// Hamming window.
    Hamming,

    /// Hamming window with a specific a0.
    /// 0.54 is commonly used, but Hamming's paper sets it as 25/46.
    ///
    /// "In the equiripple sense, the optimal values for the
    /// coefficients are a0 = 0.53836 and a1 = 0.46164".
    ///
    /// See wikipedia.
    HammingParm(Float),
}

impl WindowType {
    /// Return max attenuation.
    ///
    /// TODO: More description.
    #[must_use]
    pub fn max_attenuation(&self) -> Float {
        match self {
            // TODO: what are these magic numbers?
            WindowType::Blackman => 74.0,
            WindowType::BlackmanHarris => 92.0,
            WindowType::Hamming => 53.0,
            WindowType::HammingParm(_) => 53.0,
        }
    }

    /// Make a window of a dynamic type.
    #[must_use]
    pub fn make_window(&self, ntaps: usize) -> Window {
        match self {
            WindowType::Blackman => blackman(ntaps),
            WindowType::BlackmanHarris => blackman_harris(ntaps),
            WindowType::Hamming => hamming(ntaps, DEFAULT_HAMMING_PARM),
            WindowType::HammingParm(parm) => hamming(ntaps, *parm),
        }
    }
}

/// Window functions are "weights" used for applying filters and other
/// operations.
///
/// <https://en.wikipedia.org/wiki/Window_function>
pub struct Window(pub Vec<Float>);

/// Create Hamming window.
///
/// <https://en.wikipedia.org/wiki/Window_function#Hann_and_Hamming_windows>
fn hamming(ntaps: usize, a0: Float) -> Window {
    let a1 = 1.0 - a0;
    let m = (ntaps - 1) as Float;
    Window(
        (0..ntaps)
            .map(|n| a0 - a1 * (2.0 * PI * (n as Float) / m).cos())
            .collect(),
    )
}

/// Create Blackman window.
///
/// <https://en.wikipedia.org/wiki/Window_function#Blackman_window>
fn blackman(m: usize) -> Window {
    // Blackman's "not very serious proposal" magic value: 0.16.
    const A: Float = 0.16;

    let mut b = Vec::with_capacity(m);
    for n in 0..m {
        let n = n as Float;
        let m = m as Float;

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

        let a0 = (1.0 - A) / 2.0;
        let a1 = 0.5;
        let a2 = A / 2.0;

        // Formula.
        let t1 = 2.0 * PI * n / m;
        let t2 = 4.0 * PI * n / m;
        b.push(a0 - a1 * t1.cos() + a2 * t2.cos());
    }
    Window(b)
}

/// Create Blackman-Harris window.
///
/// <https://en.wikipedia.org/wiki/Window_function#Blackman%E2%80%93Harris_window>
fn blackman_harris(m: usize) -> Window {
    // Parameters.
    const A0: Float = 0.35875;
    const A1: Float = 0.48829;
    const A2: Float = 0.14128;
    const A3: Float = 0.01168;

    let mut b = Vec::with_capacity(m);
    for n in 0..m {
        let n = n as Float;
        let m = m as Float;

        // Formula.
        let t1 = 2.0 * PI * n / m;
        let t2 = 4.0 * PI * n / m;
        let t3 = 6.0 * PI * n / m;
        b.push(A0 - A1 * t1.cos() + A2 * t2.cos() - A3 * t3.cos());
    }
    Window(b)
}
