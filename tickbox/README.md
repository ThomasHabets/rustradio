# Checks and releases

The pre-commit checks test HEAD plus the staged diff. Unstaged changes and
ignored files do not affect the result. `SLOW=true` enables the full suite;
CI runs these tasks in addition to the normal checks. Native hardware,
nightly, and WASM feature groups live in `common.sh`. WASM backend tests run
on the host; the wasm-check task also checks the actual wasm32 target.

## Release

Run the GitHub Release workflow from `main`, supplying a stable version
(`major.minor.patch`). Locally, start with a clean `main` checkout and run:

```sh
MANUAL_VERSION=0.19.0 tickbox --dir tickbox/bump_version --max-concurrency=1
```

Prerequisites are Rust, tickbox, cargo-semver-checks,
and the native system libraries listed in the release workflow. The version
must increase, all three crates must start at the same version, and the
release tag must not exist. The bump updates the workspace lockfile; checks
and publication then use `--locked`.

All three bumped packages are dry-run together before committing. The release
commit is pushed to `main` before publication so partial uploads retain their
source revision. Cargo publishes the macro implementation, macros, and library
in dependency order. Only after publication succeeds is the exact release tag
pushed. Other local tags are never pushed by this workflow.

If publication fails, do not bump again: crates.io uploads cannot be undone.
Check out the existing release commit from `main` and inspect crates.io to
identify which versions were uploaded. Publish only the remaining packages,
in dependency order, using `cargo publish --locked -p PACKAGE`. Run the normal
verification checks before retrying. Once all three versions are available,
create `vVERSION` at that release commit if needed, and push just that tag:

```sh
git push origin refs/tags/vVERSION:refs/tags/vVERSION
```

If the commit push fails, publication has not started. If the final tag push
fails, the packages have already been published; retry only that tag push.
