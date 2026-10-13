# API changes for the next release

## Filter design

`fir::low_pass`, `low_pass_complex`, `hilbert`, and `multiband` now return
`rustradio::Result<Box<[...]>>`. Use `?` to propagate design errors. For
`multiband`, replace handling of `None` with handling of `Err`.

Low pass frequencies must be finite and positive, with cutoff at most
Nyquist. Tap counts must fit the allocation layout. Multiband endpoints are
finite fractions of Nyquist satisfying `0 <= low <= high <= 1`, and the tap
count must match the nonempty window. Hilbert windows must have an odd length
greater than one. Nonfinite window weights and invalid normalization gains
are rejected.

## FFT engines

`fft_filter::Engine::run` now returns `Result<()>`. Custom engines should
return `Ok(())` after successful processing and propagate backend errors.
Callers must handle the result; `FftFilter` propagates failures through
`Block::work` so graph execution can return an error.

Both supplied engines reject input lengths other than twice the next power
of two of the tap count. After a processing error, discard the engine and
its input rather than retrying with partially transformed data.
