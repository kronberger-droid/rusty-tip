# Configuration reference

`tip-prep` and `tip-prep-gui` read a TOML file; see `configs/` in the
repository for working examples. Every field below that shows a value has that
value as its default, so a config only needs the fields it wants to change.
Fields without a default are required.

## `[nanonis]` — connection

```toml
[nanonis]
host_ip = "127.0.0.1"                 # required
control_ports = [6501, 6502, 6503, 6504]  # required; the first port is used
layout_file = "./layout.lyt"          # optional, loaded on connect, before the stream starts
settings_file = "./settings.ini"      # optional, loaded on connect, before the stream starts
motor_group = 1                       # coarse motor group, 1 to 6 as the Motor module numbers them
motor_z_approach = "plus"             # which coarse Z direction moves the tip toward the sample:
                                      # "plus" (Z+ approaches, Z- retracts) or "minus"
```

Every Z step count in the routine is signed against `motor_z_approach`:
positive approaches, negative retracts. The workbench edits these two on
its Connection page, next to the TCP channel mapping.

## `[data_acquisition]` — TCP data stream

How the signal stream is acquired. The thresholds a reading is *judged*
against live in `[tip_prep.signal_stability]`, not here.

```toml
[data_acquisition]
data_port = 6590            # required; Nanonis TCP logger port
sample_rate = 1000          # required; stream rate to ask for (Hz)
stable_signal_samples = 100 # samples averaged per stable signal read
```

The TCP logger delivers its base rate divided by an integer, so the nearest
such rate to `sample_rate` is what arrives: on a 2 kHz base, 1000 or 667 Hz
but not 800. The controller works out the divisor from the RT frequency,
measures what the stream delivers, corrects the divisor once if the guess
was off, and logs the result. The rate it records, and the drift gate
uses, is the nominal base over divisor once the measurement has confirmed
it, since that is exact where the measurement carries packet-timing
jitter. A request that had to be rounded costs nothing but a warning at
startup.

## `[controllers]` — presets

```toml
[controllers]
presets_file = "./controllers.toml"  # the preset file, relative to the working directory
```

A preset is one controller's parameters under a name, with the operating
point they were tuned at. The file is `[[presets]]` tables; the repo root's `controllers.toml`,
where the default path finds it when the tools run from the checkout,
ships two, a log-current loop for tip prep and a frequency loop for imaging:

```toml
[[presets]]
name = "tip-prep"
id = { kind = "z" }

[presets.params]
kind = "z"
active = "log Current"   # which of the Z-controllers Nanonis has defined
setpoint = 100e-12
p_gain_m = 1.5e-12
time_constant_s = 50e-6  # I = P / T

[presets.tuned_at]       # informative; a preset never writes these
setpoint = 100e-12
bias_v = 1.0
note = "CuOx, landings without a peak"
```

Applying a preset writes its parameters and keeps the setpoint the loop
holds, so `setpoint` under `params` is what the gains were tuned at, not
what gets written. That matters by loop law: a log loop on the current has
gains that do not depend on the setpoint or the bias, so its preset
transfers; a linear loop's gain scales with the setpoint and a frequency
loop's slope changes with distance, bias and amplitude, so those presets
are only good near their `tuned_at`. The workbench's Controllers page
lists the file's presets for the selected controller, loads one into the
form, applies it, deletes it, and saves the form as one with the live
setpoint, bias and amplitude recorded. The path is on its Connection page
next to the log directory.

## `[tip_prep]` — the routine

```toml
[tip_prep]
sharp_tip_bounds = [-2.0, 0.0]  # required; freq-shift window that counts as sharp (Hz)
max_cycles = 10000              # optional; omit for unlimited
max_duration_secs = 12000       # optional; omit for unlimited
initial_bias_v = -0.5           # bias set before the first approach (V)
initial_z_setpoint_a = 100e-12  # z-controller setpoint before the first approach (A)
safe_tip_threshold = 1e-9       # safe-tip current threshold (A)
z_controller_preset = "tip-prep"  # optional; a Z preset from [controllers].presets_file,
                                  # written before the first approach with initial_z_setpoint_a
```

## `[tip_prep.timing]` — settle times and repositioning

```toml
[tip_prep.timing]
pulse_width_ms = 50
post_approach_settle_ms = 0       # extra wait after a landing; the landing gate
post_reposition_settle_ms = 0     # already waits for the loop, so none by default
landing_tolerance = 0.5           # a landing counts once the current reads within
                                  # this fraction of the setpoint, and holds still
landing_timeout_ms = 30000        # stop waiting for that and carry on, with a warning
post_pulse_settle_ms = 1000
buffer_clear_wait_ms = 500
reposition_steps = [1, 1]         # coarse motor steps (x, y) per reposition
status_interval = 10              # log a status line every N cycles
approach_timeout_ms = 600000      # approaches from a full withdraw (first
                                  # approach, around each stability sweep)
reposition_approach_timeout_ms = 300000  # the short approach inside a reposition
exit_retract_steps = 2            # coarse Z steps back after the final withdraw
```

An approach that overruns its budget is stopped, the run ends in an error,
and the tip is withdrawn. The two budgets differ because a reposition only
retracts three coarse steps before re-approaching, while the first approach
of a run starts wherever the tip was left.

There is no fixed settle between the coarse steps and the re-approach any
more. A stick-slip step leaves the stage creeping for a while, and a
landing on that creep, or any landing the loop is still ringing from,
shows as a current well off the setpoint for a moment. The approach waits
on that instead of on a clock: after each landing it reads batches of
`stable_signal_samples` of the current until the mean sits within
`landing_tolerance` of the setpoint and the batch's standard deviation and
drift per second are within the same band. `landing_timeout_ms` bounds the
wait; when it runs out the landing is taken as done with a warning, which
is what a fixed wait would have done. Every batch is in the run log as a
`landing` measurement. One coarse step in x and y is enough to leave the
last pulse's debris behind; every further step is more creep to wait out.

## `[tip_prep.signal_stability]` — when is a reading trusted

Gates a single frequency-shift measurement must pass before the routine
believes it. Loosen for noisier tips, tighten for cleaner ones.

```toml
[tip_prep.signal_stability]
max_std_dev_hz = 1.5      # max standard deviation of the sample batch (Hz)
max_slope_hz_per_s = 0.5  # max drift rate of the batch (Hz/s)
read_retry_count = 3      # retries with exponential backoff before giving up
```

`data_collection_duration_ms` and `read_timeout_secs` are still accepted so
v1-era configs parse, but the v2 read path ignores them: batch size comes from
`data_acquisition.stable_signal_samples` and a read is bounded by
`read_retry_count`.

## `[tip_prep.stability]` — is the tip *stable*, not just sharp

Optional verification that a sharp tip survives bias sweeps while scanning.
When the check fails, the routine fires a maximum-voltage pulse and starts
over.

```toml
[tip_prep.stability]
check_stability = true
stable_tip_allowed_change = 0.2  # max freq-shift drift across the sweep (Hz)
bias_range = [0.01, 2.0]         # sweep magnitude range (V), strictly positive
bias_steps = 1000
step_period_ms = 200
polarity_mode = "both"           # "positive", "negative", or "both"
scan_speed_m_s = 5e-9            # scan speed during the check; omit to keep current
```

`bias_range` is magnitude-only; `polarity_mode` decides the sign. `"both"`
runs a positive sweep followed by a negative one.

## `[pulse_method]` — how pulse voltages are chosen

Exactly one of the three variants.

**Fixed** — the same voltage every cycle:

```toml
[pulse_method]
type = "fixed"
voltage = 4.0
polarity = "positive"  # or "negative"
```

**Stepping** — walk the voltage up when progress stalls:

```toml
[pulse_method]
type = "stepping"
voltage_bounds = [2.0, 6.0]  # start at 2.0 V, cap at 6.0 V
voltage_steps = 4            # number of steps across the range
cycles_before_step = 2
threshold_value = 0.1        # freq-shift change that counts as progress (Hz)
polarity = "positive"
```

**Linear** — map the measured frequency shift onto a voltage range:

```toml
[pulse_method]
type = "linear"
voltage_bounds = [2.0, 7.0]
linear_clamp = [-20.0, 0.0]  # freq-shift range mapped onto voltage_bounds
polarity = "positive"
```

Outside `linear_clamp` the maximum voltage is used; inside, the voltage
interpolates linearly.

All three variants accept optional random polarity switching:

```toml
[pulse_method.random_polarity_switch]
enabled = true
switch_every_n_pulses = 5
```

## `[experiment_logging]` and `[console]`

```toml
[experiment_logging]
enabled = true               # required section
output_path = "./experiments"  # one timestamped JSONL event log per run

[console]
verbosity = "info"  # required; trace | debug | info | warn | error
```

## `[[tcp_channel_mapping]]` — more signals to stream

The library streams a standard set of signals. Add one by its Nanonis
signal index:

```toml
[[tcp_channel_mapping]]
nanonis_index = 76  # signal index (0-127)
tcp_channel = 18    # takes the standard signal on this number off the stream
```

The TCP logger is asked for signals by index and announces which column
each lands in, so `tcp_channel` no longer says where a signal goes. Pick a
number the standard set does not use to add a signal without losing
another.

Signals without any TCP channel mapping still work; reads for them fall back
to polling instead of the high-rate stream.
