# The experiment log

Every tool writes one JSONL file per run into the experiments directory
(`experiment_logging.output_path` for tip prep, `--log-dir` for
const-distance), named `<tool>_<UTC timestamp>.jsonl`. The file is the
record of what the software did to the instrument and what it measured,
in order, with enough in the first line for a reader to interpret the rest
without knowing which tool wrote it.

## Lines

One JSON object per line. Every line has:

| field | meaning |
| --- | --- |
| `seq` | 0-based line number, so order never depends on timestamp resolution |
| `type` | one of the event types below |
| `timestamp` | seconds since the Unix epoch, fractional |

The first line is `run_started`, the last is `run_finished`. A file that
ends without `run_finished` was cut off: the process died or was killed
before the harness could write it.

## Event types

**`run_started`** carries a `header`:

| field | meaning |
| --- | --- |
| `tool` | `tip_prep`, `const_distance`, … |
| `version`, `git_commit` | what built the binary; `git_commit` is null outside a checkout |
| `envelope_version` | version of this line format, currently 1 |
| `config` | the tool's configuration as loaded, after defaults; for a command-line tool, its arguments |
| `controller` | what was learned at startup: the resolved signals with their indices and TCP channels, and the measured stream rate |
| `schema` | the tool's declared custom events, see below |

**`run_finished`**: `outcome` is `completed`, `stopped_by_user`,
`cycle_limit`, `timed_out`, `error` or `panicked`; `detail` carries the
limit or the error text; `duration` is milliseconds since the run started.

**`action_started`**, **`action_completed`**, **`action_failed`**: one
pair per action. `params` is the action's own fields (a `bias_pulse` has
its `voltage`, `duration_ms`, `z_hold`, `absolute`), `output` is what it
returned, `duration` is milliseconds, `error` is the failure text.
`depth` is the nesting: 0 for an action the routine ran directly, 1 for a
step run by that action. A `calibrated_approach` at depth 0 is followed by
its `auto_approach`, `wait`, `z_home`, `center_freq_shift` and second
`auto_approach` at depth 1, then its own `action_completed`. Reconstruct
the tree by stacking on depth.

**`data_collected`**: a measurement, with a `label` and a `value` object.
A stable read is `label: "stable_read"` with the mean as `value.value`,
plus `std_dev`, `slope` (Hz/s), `n` and whether the batch passed the gates.

**`custom`**: a tool's own event. `kind` is `tool/name`, and `data` has
the shape the header's schema declares for that kind.

## Declared schemas

A tool cannot write a custom event it has not declared. Each kind is a
Rust struct implementing `LogEvent` with its kind name; the tool lists them
in a `ToolSchema`, and the JSON Schema of each is generated from the struct
and written into the header. The two cannot disagree because they come
from the same type.

`tests/snapshots/<tool>.schema.json` pins each tool's generated schema. A
change to a logged struct fails the snapshot test until the snapshot is
regenerated:

```nu
UPDATE_SNAPSHOTS=1 cargo test --test experiment_log
```

That diff, committed with the change, is the format's changelog.

Tip prep declares `tip_prep/cycle` (one per pulse cycle: cycle number,
elapsed seconds, the frequency shift measured after the reposition, the
pulse voltage as fired with its sign, whether it was sharp),
`tip_prep/phase` (`confirming`, `stability_check` with the baseline,
`stable` or `unstable` with the final read), and `tip_prep/max_pulse`.
The routine harness adds `routine/cleanup_failed` and `routine/panicked`
to every routine's schema. const-distance declares no custom kinds yet.

## Reading a log: `rt-log`

`rt-log` is the terminal reader. It works on any log with a header,
whichever tool wrote it, because the columns it exports and the series it
plots come from the schema in that header.

```nu
rt-log ls experiments/                  # every run: tool, start, duration, outcome
rt-log summary <file>                   # outcome, time per action, measurements, events
rt-log timeline <file>                  # the action tree with durations and params
rt-log timeline <file> --action calibrated_approach --max-depth 1
rt-log timeline <file> --failed         # only what failed or was cut off
rt-log plot <file>                      # list the plottable series
rt-log plot <file> tip_prep/cycle.freq_shift
rt-log export <file> --out <dir>        # flat CSV tables
```

`export` writes `run.json` (the header plus the outcome), `actions.csv`
(every action at every depth: seq, time, depth, name, duration, status,
error, params as JSON), one `<label>.csv` per measurement label with the
value's scalar fields as columns, and one `<tool>_<kind>.csv` per custom
kind with the columns the declared schema lists. Those tables load into
anything: pandas, Julia, R, or Typst's `csv()` for a lilaq plot in a
note.

For a dry run to try it on:

```nu
cargo run --example tip-prep-mock -- realistic --log /tmp/mock.jsonl
rt-log summary /tmp/mock.jsonl
```

Anything that reads JSONL can read a log directly as well:

```nu
open experiments/tip_prep_20260914_101500.jsonl | lines | each { from json } | where type == "custom" and kind == "tip_prep/cycle" | select seq data.cycle data.freq_shift data.pulse_voltage
```

Logs from before the header existed still parse: `seq`, `depth` and the
header default, and `export` derives columns from the data it finds.
