# Agent Notes

## Context Handoffs

When context is getting tight, leave a compact handoff that lets the
next agent continue without rediscovering everything. Include:

- Current task and any explicit user constraints.
- Repository path, branch state, and relevant untracked files.
- Files read or edited, with the reason each one matters.
- Commands already run and whether they passed or failed.
- Commits created during the task, including their hashes.
- Bugs found, fixes applied, and remaining leads that are not proven.
- Exact next step if work is unfinished.

Keep handoffs factual and short. Do not paste full command logs unless
the exact output is necessary to diagnose the next step.

## Dependencies

Avoid adding dependencies. Ask the user if it truly is the best choice. Never
vendor dependencies.

## Commit Messages

Use the existing project style:

```text
Area: imperative summary
```

Examples:

```text
Fir: reject zero filter_n decimation
Delay: wait for pending delay before EOF
workflows: Add --bench
```

Commit message rules:

- Keep the subject line at or below 72 characters.
- Wrap body text at 72 characters.
- Use a body when the fixed control flow or failure mode is not obvious
  from the subject line.
- Make one logical fix per commit.
- Do not use `--no-verify`.
- Preserve unrelated user changes and untracked files.
