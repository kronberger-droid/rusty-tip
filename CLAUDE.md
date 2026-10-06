# rusty-tip

Tip preparation and SPM automation for Nanonis controllers. The protocol
client is the sibling repo `../nanonis-rs`; its TCP command reference is
`../nanonis-rs/docs/tcp-protocol.md`.

## Build and check

- "The GUI" or "the workbench" is `rusty-tip-gui`. It and `tip-prep-gui` need
  `--features gui`; `cuox-finder` needs `--features cuox`.
- Work inside `nix develop`, which carries CI's stable toolchain and `typos`.
  `.githooks/pre-push` runs fmt, clippy with CI's flags, and typos.
- CI also runs `cargo test --all-targets` and `cargo test --doc` (the first
  skips doctests). Run both when a change touches tests or doc examples.
- After an intended log-format change, refresh snapshots with
  `UPDATE_SNAPSHOTS=1 cargo test --test experiment_log`.
- A user-visible change gets an entry under `## [Unreleased]` in
  `CHANGELOG.md`. PRs open as drafts.

## The instrument

- A Nanonis V5 demo runs locally under wine, from `~/Projects/nix/nanonis-wine`
  (Martin starts it with `nix run ~/Projects/nix/nanonis-wine`; launch trouble
  is covered in its README). Control ports 6501-6504, TCP logger data port
  6590, one client per port. `configs/tip_prep_with_stability.toml` targets it.
- Run a protocol claim against the demo before building a fix on it. A claim
  you have not run is reported as untested.
- Read live state through the agent CLI (`docs/agent-cli.md`, then
  `rusty-tip describe`) before asking Martin for values or screenshots.
- Data Logger `.dat` headers carry neither the sample rate nor the bias. Both
  are whatever Martin set, so get them from him before any spectral or
  bias-dependent analysis.
- Log formats are in `docs/experiment-log.md`. Query large logs with a script
  that prints aggregates or windows.

## Lab sessions

When Martin is at the instrument he reads between hands-on steps. Lead with the
next action in at most five lines, give tables and derivations when asked, and
answer a procedural question before running analysis or opening source files.
