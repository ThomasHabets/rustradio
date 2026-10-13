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
