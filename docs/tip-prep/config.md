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
layout_file = "./layout.lyt"          # optional, loaded during prepare()
settings_file = "./settings.ini"      # optional, loaded during prepare()
```

## `[data_acquisition]` — TCP data stream

How the signal stream is acquired. The thresholds a reading is *judged*
against live in `[tip_prep.signal_stability]`, not here.

```toml
[data_acquisition]
data_port = 6590            # required; Nanonis TCP logger port
sample_rate = 2000          # required; oversampling passed to the TCP logger
stable_signal_samples = 100 # samples averaged per stable signal read
```

## `[tip_prep]` — the routine

```toml
[tip_prep]
sharp_tip_bounds = [-2.0, 0.0]  # required; freq-shift window that counts as sharp (Hz)
max_cycles = 10000              # optional; omit for unlimited
max_duration_secs = 12000       # optional; omit for unlimited
initial_bias_v = -0.5           # bias set before the first approach (V)
initial_z_setpoint_a = 100e-12  # z-controller setpoint before the first approach (A)
safe_tip_threshold = 1e-9       # safe-tip current threshold (A)
```

## `[tip_prep.timing]` — settle times and repositioning

```toml
[tip_prep.timing]
pulse_width_ms = 50
post_approach_settle_ms = 2000
post_reposition_settle_ms = 1000  # ends every reposition
post_move_settle_ms = 500         # between motor move and re-approach
post_pulse_settle_ms = 1000
buffer_clear_wait_ms = 500
reposition_steps = [3, 3]         # coarse motor steps (x, y) per reposition
status_interval = 10              # log a status line every N cycles
```

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
max_duration_secs = 100
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

## `[[tcp_channel_mapping]]` — custom signal-to-channel mapping

The library ships a standard mapping from Nanonis signal indices to TCP
logger channels. If your instrument's TCP logger is configured differently,
override entries per signal:

```toml
[[tcp_channel_mapping]]
nanonis_index = 76  # signal index (0-127)
tcp_channel = 18    # TCP logger channel (0-23)
```

Signals without any TCP channel mapping still work; reads for them fall back
to polling instead of the high-rate stream.
