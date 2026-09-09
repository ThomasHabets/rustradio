---
name: new-block
description: Add or modify a RustRadio processing block in this repository, including synchronous transforms, tag-aware blocks, stream or PDU sources and sinks, buffered or rate-changing blocks, builders, feature-gated blocks, public exports, documentation, and tests. Use when asked to create, port, scaffold, integrate, or review a RustRadio block.
---

# Add a RustRadio Block

## Establish context

Work from the repository root. Read `AGENTS.md`, inspect `git status`, and
preserve unrelated changes and untracked files.

Read these canonical sources before editing; do not rely on a copied block
template:

- `doc/writing-a-block.md` for the intended public API and scheduler model.
- `src/block.rs` for the current `Block`, `BlockName`, `BlockEOF`, and
  `BlockRet` contracts.
- `rustradio_macros/src/lib.rs` for supported derive modes and attributes.
- `rustradio_macros_code/src/lib.rs` only when expansion details or an unusual
  generic bound matter.
- `src/stream.rs` for the current copying and no-copy stream APIs and tag
  semantics.
- `src/lib.rs` and `src/blocks.rs` for module declaration, feature gating, and
  public re-export structure.

Re-run `rg -l 'derive\(rustradio_macros::Block\)|impl.*Block for'` when the
repository may have changed. Select the closest existing block by data model
and scheduler behavior, not merely by name.

## Decide whether to add a block

Use `Map` from `src/convert.rs` for a one-off one-input/one-output mapping when
a reusable named type is unnecessary. Add a block when behavior, state,
validation, tags, rate changes, multiple ports, or a reusable API justify it.

Choose the smallest applicable implementation shape:

- Use `sync` for exactly one output item per output per set of input items and
  automatic first-input tag propagation. Refer to `src/add.rs`,
  `src/add_const.rs`, and `src/tee.rs`.
- Use `sync_tag` when the same one-for-one schedule must inspect, replace, or
  route tags. Refer to `src/burst_tagger.rs` and
  `src/correlate_access_code.rs`.
- Use `sync_nocopy_tag` only for one-for-one no-copy messages. Refer to
  `src/morse_encode.rs`.
- Implement `Block::work` for sources, sinks, variable rates, lookahead,
  buffering, partial frames, or external readiness. Refer to
  `src/vector_source.rs`, `src/vector_sink.rs`, `src/delay.rs`,
  `src/rational_resampler.rs`, `src/fft_stream.rs`, and `src/reader_source.rs`.
- Use no-copy streams for owned messages or PDUs. Refer to `src/fft.rs` and
  `src/pdu_average.rs`; use copying streams for `Sample` sequences.
- Add a builder when construction has optional settings, staged type-state, or
  fallible validation. Refer to `src/vector_source.rs`, `src/fir.rs`, and
  `src/file_source.rs`.
- Mirror a hardware block's gating only when the new block actually introduces
  an optional platform or dependency. Refer to `src/audio_sink.rs` together
  with its entries in `Cargo.toml`, `src/lib.rs`, and `src/blocks.rs`.

For an example-local or `rustradio-ui` block, follow that crate's local module
and import structure. The main crate's `#[rustradio(crate, ...)]` convention
does not automatically apply outside the main crate; compare
`examples/capture.rs` and `rustradio-ui/src/worker/float_sink.rs`.

## Satisfy hard requirements

Treat this section as required for correctness or complete standard-library
integration. The exact syntax is flexible unless stated otherwise.

1. Make the block type satisfy `Block`, including the `BlockName`, `BlockEOF`,
   and `Send` supertrait requirements in `src/block.rs`. Use
   `#[derive(rustradio_macros::Block)]` as every existing repository block does.
   For a main-crate block, include the derive's `crate` option so generated
   paths resolve locally.

2. Mark every input and output stream field correctly for the derive. Own read
   sides as inputs and write sides as outputs. Ensure the public constructor
   accepts input streams and returns the block plus the read side of every
   output, in field order. Prefer the generated `new` only when its generated
   signature and initialization are the intended API.

3. Implement exactly one scheduler path: select a sync derive mode and its
   required processing method, or implement `Block::work`. Confirm the current
   signatures in `rustradio_macros/src/lib.rs` rather than reproducing an old
   signature from memory.

4. Preserve scheduler progress:

   - Return `WaitForStream` with the actual blocking stream and meaningful
     minimum count when input is empty or output lacks space.
   - Return `Pending` only for readiness that stream activity cannot wake, such
     as a background device or channel.
   - Return `Again` only after consuming, producing, or changing state so the
     next call can behave differently. Never poll with it.
   - Return `EOF` only when the block can never produce again.

5. Preserve data under backpressure. Check output capacity before irreversibly
   popping no-copy input. For copying streams, consume only the samples whose
   corresponding outputs or retained state are accounted for. Produce exactly
   the initialized output count.

6. Preserve tag correctness. Treat positions as relative to the current
   buffer/message. Forward only tags covered by consumed data, translate
   positions when changing rates or buffering, and attach only tags valid for
   the produced range. State deliberate tag dropping in the public docs. The
   sync macro's default first-input policy is documented in
   `rustradio_macros/src/lib.rs`.

7. Override generated EOF behavior when input EOF does not imply immediate
   output EOF. Use the derive's `noeof` option and implement `BlockEOF` when
   pending delay, buffered output, a tail, or another state must drain. Refer
   to `src/delay.rs`, `src/pdu_to_stream.rs`, and
   `src/rational_resampler.rs`.

8. Reject invalid public configuration before processing when practical. Use
   the crate's `Result` and `Error` conventions; do not leave reachable panics
   for caller-controlled values. Make a constructor or builder fallible when
   validation can fail.

9. Integrate a new public main-crate block in both places:

   - Declare its module in `src/lib.rs`, maintaining the existing alphabetical
     layout and matching any feature gate.
   - Re-export its public block types from `src/blocks.rs` under the identical
     feature gate.

   There is no separate runtime registry. Do not add these exports for a
   private example-local or UI-only block.

10. Document the module, public block, constructor or builder, configuration,
    stream types, rate relationship, tag behavior, and failure conditions that
    callers need. Make any doctest construct the current API and compile under
    the block's feature set.

11. Add focused tests for the new behavior even though older block modules do
    not all have tests. Cover the happy path and the applicable boundaries:
    empty input, invalid configuration, input EOF and buffered tail, output
    backpressure, partial input, multiple `work` calls, tags, and rate or size
    rounding. Assert `BlockRet` where scheduler behavior is part of the bug
    surface. Use `src/add.rs` for a small sync test, `src/fft.rs` for no-copy
    validation, `src/delay.rs` for EOF state, `src/rational_resampler.rs` for
    backpressure, and `src/fft_filter.rs` for tag propagation.

## Recognize common patterns, not invariants

Apply these when they improve consistency; do not force them when the block's
contract calls for something else:

- Put one related block family in a snake-case module and use CamelCase public
  type names.
- Name a single input/output `src` and `dst`; use semantic names for multiple
  ports.
- Bound copying-stream sample types with `Sample`; use `Send + Sync + 'static`
  bounds appropriate to owned no-copy payloads.
- Use derive `default` for internal initial state and `into` for ergonomic
  string-like constructor arguments.
- Batch work in a loop until blocked rather than processing only one item.
- Keep unit tests in a `#[cfg(test)] mod tests` beside the block and use
  `VectorSource`/`VectorSink` or direct test streams as appropriate.
- Add a runnable example or public doctest when it materially teaches graph
  composition. Many small or specialized existing blocks omit one.
- Add a Cargo feature only for an optional dependency or platform boundary.

Do not infer a hard rule merely from majority usage. In particular, builders,
generated constructors, sync mode, doctests, and in-file tests are not present
in every existing block. Conversely, the repository's universal use of the
derive is a strong convention that still exists to generate trait plumbing,
not a Rust language requirement.

## Verify the change

Run the narrow checks first, then broaden them. Replace `new_block` with the
module's test path and add the relevant feature flag for a gated block.

```text
cargo fmt -- --check
cargo test new_block::tests
cargo test --doc
cargo clippy --no-deps --workspace -- -D warnings
cargo test --workspace
git diff --check
```

For an ordinary ungated main-crate block, also run:

```text
cargo test --workspace --no-default-features
cargo test -F wasm --lib
```

For a feature-gated or SIMD-sensitive block, test the relevant feature and
toolchain explicitly. Use `doc/testing.md` for target-feature guidance.

Run the repository's canonical precommit suite before handoff when its external
tools and system libraries are available and the intended block diff is already
staged:

```text
tickbox --dir tickbox/precommit
```

`tickbox/precommit/10-setup.sh` tests archived `HEAD` plus `git diff --cached`;
it ignores unstaged and untracked changes. Do not stage unrelated work merely
to run it. Prefer the direct commands above when the intended diff is not
staged. Use `SLOW=true` for Tickbox's individual-feature,
normal/no-default-feature, and nightly coverage. If a check cannot run because
a toolchain, device library, or service is unavailable, report the exact
skipped command and reason; do not claim it passed.

Finally, inspect `git diff -- src/lib.rs src/blocks.rs src/<module>.rs` and
confirm that the constructor shape, gates, public docs, tags, progress, EOF,
and tests agree with the chosen canonical analog. If asked to commit, follow
the `Area: imperative summary` rules in `AGENTS.md`.
