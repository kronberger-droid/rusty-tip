# Coding standards

Read at review time. Formatting, clippy (`-D warnings`, CI's flags) and
spelling are enforced by CI and `.githooks/pre-push`; this file covers the
judgement calls tooling cannot make.

## Library

- **One resolver per lookup.** When more than one place needs the same signal,
  preset or channel by name, give it a named resolver and call that everywhere
  (`TipPrepSignals::resolve`). Copies drift, and positional tuples of indexes
  get swapped.
- **Typed log events.** A custom experiment-log event is a struct with a
  `KIND` const (`CycleEvent`). Readers deserialize into it rather than picking
  fields out of the JSON by name.

## Workbench GUI (`bin/rusty-tip-gui`)

- **Headings and label/value rows, not prose.** Structure a panel with
  `widgets::section` and short label/value rows or a small table. Explanations
  go in hover text (`section`'s `help`, `on_hover_text`).
- **Colours come from `widgets::SCHEME`.** New plot series take the unused
  accents (`base09`, `base0C`, `base0E`, `base0F`); plots stay muted next to
  the toned buttons. `Tone` button colours keep their own palette.

## Tests

- **Test names read as sentences** about the behaviour:
  `a_planned_trajectory_survives_a_gsf_round_trip`.
