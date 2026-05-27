# Tip Prep

## General

- Reset Safe-tip on drop

- Implement script for multiple different tip prep cycle testing

  - Create Action Destroy Tip to be able to run

- Check if bias voltage bounds are positive To not mess up polarity

- Reset the list of signals to make TCP channels work

## Testing

- Implement scripts to interpret experiment logs

## Logger Injection

- Make it possible to inject Action Results without having to run an action

## Machine testing (v2 rewrite)

- Validate `tip-prep-v2` end-to-end on the real machine.
- Dry-run the routine first against the mock — no hardware needed:
  `cargo run --example tip-prep-mock` (scenarios: `sharpen` | `cyclelimit` |
  `unstable` | `fault` | `shutdown`).
- Sanity-check the hard-coded settle times in `src/tip_prep/runner.rs` against
  real timing: the `post_move_settle_ms: 500` literals, `CalibratedApproach`'s
  200/500 ms waits, and the 100 ms `scan_status` poll loop.

## Cleanup follow-ups (after v1 removal)

- Dead types stranded by removing the v1 stack — no longer referenced by v2,
  remove when convenient:
  - `types.rs`: `SignalStats`, `StableOsciData`, `DataToGet`,
    `AutoApproachResult`, `AutoApproachStatus`
  - `controller_types.rs`: `ControllerAction`, `ControllerState`,
    `TipControllerConfig`, `TipStateConfig`
  - `error.rs`: `RunOutcome`
- Drop the deprecated `TCPLoggerData` re-export (nanonis-rs deprecated it; v2
  no longer constructs it).

## Mock controller

- Consider a `models::realistic(...)` tip model that mirrors how the tip
  actually responds on the machine (stochastic sharpening, over-pulsing past
  the window, slow drift) for richer edge-case rehearsals.
