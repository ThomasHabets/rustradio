# Contributing

Welcome! Pull Requests (PR) to https://github.com/ThomasHabets/rustradio will be
warmly received.

## Commit messages

Follow this pattern:

```
BlockName: Add support for foo and bar

More information here, if needed.
```

In other words: If applicable, prefix with block name, component name, example
name, and then what was changed phrased in imperative form ("Add", not "Adding"
or "Added").

Check existing history for inspiration, though nothing is perfect.

## AI

While AI can certainly be involved, you are expected to understand and have
tested the code you send. Don't outsource this work to the reviewer.

IANAL, but a this section probably needs a note on copyright: You are assuring
that the code you supply is not in violation of anybody else's rights and claims
on the code, and can be incorporated into this MIT licensed project with the
attributions (or lack thereof) that the PR provides.

## Tests

A github workflow will run on all PR, running linters and tests. To be merged, a
PR must pass all of these.

If your PR is not intended to be squashed, then please make sure they all pass
on all commits. It makes bisections easier in the future to always have a clean
history. This may not be a hard rule, but attempt to do this.

A precommit script is provided in `extra/pre-commit`, and can be installed by:

```
cd .git/hooks
ln -s ../../extra/pre-commit
```

### Running the tests locally

Tests run using [tickbox](https://github.com/ThomasHabets/tickbox), which can be
installed using `cargo install tickbox`.

After that, the same tests ran be run locally with:

```
tickbox --dir tickbox/precommit
```

They run must faster after the first initial run, but it does take a lot of
space for caching those builds.

Setting environment `FAST=true` will skip the heaviest of the tests, and is
recommended. To only run some tests, one can run something like:

```
tickbox \
    --wait \
    --dir tickbox/precommit \
    --matching '(10|.*fmt|.*deny|.*clippy).*'
```

### Lint objections

If you think the linter is wrong, then either first send a PR to disable that
lint, where that can be discussed separately, or disable the lint in a limited
scope where it triggered.
