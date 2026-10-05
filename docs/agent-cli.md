# `rusty-tip`: the command line for scripts and agents

`rusty-tip` drives a session the way the workbench does, but for a program on
the other end: every command prints one JSON reply and exits with a code that
says how it went. An agent can learn the interface from `rusty-tip describe`
and plan inside the limits it reports, without reading this page.

This first version only reads: signals, the feedback loops, the scan and its
frames. Commands that act on the instrument come next,
each checked against the read-only gate and the limits described below, and
each run as a job with its own experiment log; plain reads are not logged.

## Three ways to reach a controller

| Mode | How | Connection |
| --- | --- | --- |
| Workbench (default) | `rusty-tip status` | The window's agent socket, Connection page. Read-only unless switched to full. |
| Headless | `rusty-tip serve --config lab.toml [--read-only]`, then `rusty-tip status` | Held by `serve` until Ctrl+C. |
| One-shot | `rusty-tip --one-shot --config lab.toml status` | Opened for the one command, closed after. |

The first two keep the connection between commands, so a command costs a round
trip, not a layout and settings load. `serve` loads the config's layout and
settings files on connect, as the workbench does; one-shot does not, since a
command that only reads should not change the instrument to answer.

The workbench and `serve` listen on `127.0.0.1:47474` unless `--addr` or the
Connection page says otherwise, and only ever on loopback: the socket has no
authentication of its own. `--mock` stands in for `--config` and
connects to the simulator, to rehearse a plan without hardware.

`serve` prints one line once it listens, `{"ok":true,"result":{"listening":
"127.0.0.1:47474","read_only":true,"limits":{}}}`, and one more when it stops.

## Commands

```text
rusty-tip describe                          # commands, request schema, exit codes
rusty-tip status                            # state, controller, capabilities, readouts
rusty-tip read current "freq shift"         # one value each
rusty-tip read "Z (m)" --samples 500        # mean and std_dev of 500 stream samples
rusty-tip controllers                       # Z-controller and PLL loops: parameters, on/off, status
rusty-tip scan                              # frame, buffer, speed, running
rusty-tip frame "Z (m)"                     # one recorded signal's frame, both directions
rusty-tip psd "Z (m)"                       # spectrum of a streamed signal, with its peaks
```

Signal names are the registry's, case-insensitive. Each reading carries the
`unit` the controller names the signal with, `"A"` for `"Current (A)"`, or
`null` when the name has none. `status` answers from the session's last
report, so it answers while a job runs; a `read` needs the controller and
waits up to 10 s for whatever holds the session, a running job or another
client's request, to let it start before answering `busy`, and the message
says which of the two it was. A request answered `busy` is dropped, never run
late. One that has started is waited for, however long past those 10 s it
runs, up to 60 s; past that the reply is `failed`, since the controller has
likely stopped answering.

With `--samples`, signals on the data stream are read from the same frames:
their readings are of one moment, and the read lasts as long as one signal's,
20 s for 20000 samples at the usual 1 kHz. A signal off the stream is polled,
one after another. `--samples` times the number of signals is at most 20000.

Over the socket the same commands are JSON, one request per line and one reply
per line, as many as you like on one connection:

```json
{"cmd":"read","signals":["current"],"samples":100}
```

A misspelt parameter is an error, not ignored.

## Reading for tuning

`controllers` reads every feedback loop the controller exposes, the
Z-controller and the PLL's amplitude and phase loops, each with its
parameters, whether it is on, and its status word. `scan` gives the frame's
centre, size and angle, the signals the scan buffer records (by name where the
registry has one), pixels and lines, the speeds, and whether a scan runs.

`frame <signal>` grabs the signal's current frame in both directions. The
signal has to be one the buffer records; any other is a `bad_request` that
lists the ones it does. The pixels do not come back in the reply, where a
512 × 512 frame would be megabytes; they go to a JSON file in a `frames`
directory beside the job logs, or under the system's temporary directory when
the session keeps none, and the reply names it. The directory is the server's
choice, never the request's, so a read-only server cannot be made to write
where a client says.

The reply carries per-line statistics instead, as columns where entry `i` is
line `i`, and their means over the scanned lines:

| Field | What it is |
| --- | --- |
| `rms_forward`, `rms_backward` | RMS about the line's own mean and slope, so tilt is not roughness. Ringing shows here. |
| `retrace_offset` | Mean of backward minus forward. Mostly hysteresis and creep. |
| `trace_retrace_rms` | RMS of backward minus forward about that offset. A loop that lags shifts features between the directions, which shows here. |

Lines not scanned yet read `null` and stay out of the means, so a partial frame
is fine. The two directions are compared pixel for pixel as the controller
sends them; Nanonis sends backward rows the same way round as forward ones,
checked on the lab's controller. The file keeps the rows as sent.

`psd <signal> [--samples N] [--segment N]` takes `N` evenly spaced samples of a
signal on the data stream (16384 by default, at most 20000) and averages their
spectrum by Welch's method: segments of `--segment` samples, a power of two,
overlapping by half, each with its mean removed and a Hann window applied. The
default segment is 1024, shorter when that would average fewer than seven.
Longer segments resolve finer and average fewer, so the trace is noisier.
The default takes 16 s on a 1 kHz stream, and the session is held for that long.

| Field | What it is |
| --- | --- |
| `rate_hz` | The stream's rate, the one the logger's divisor gives on Nanonis; the frequency axis comes from it. |
| `rms` | The RMS the spectrum holds, its integral, in the signal's unit. |
| `rms_total` | RMS of all the samples about their one mean. Larger than `rms` by drift slower than a segment, which each segment's mean removal leaves out of the spectrum. |
| `peaks` | The eight highest local maxima above DC, highest first: `freq_hz` and `asd`, the amplitude density in unit/√Hz. |
| `floor_asd` | The noise floor in unit/√Hz, the root of the median PSD above DC. On a flat spectrum most peaks are bumps in the noise; only those well above the floor are lines. |
| `spectrum` | `freq_hz` and `psd` (unit²/Hz) from 0 to Nyquist, with `segment`, `averages` and `resolution_hz`. |

`rms²` is the PSD summed times `resolution_hz`, so it always agrees with the
spectrum; where `rms_total` is well above it, the signal drifts. A pure tone's `asd` depends on the resolution, its frequency
does not: a line that stays put while the gains change is the room, one that
moves or grows with the gain is the loop. A signal not on the stream is a
`bad_request` naming the ones that are, since polled samples have no time base
and their spectrum would show lines that are not there.

## Replies and exit codes

```json
{"ok":true,"result":{"readings":[{"asked":"current","name":"Current (A)","index":0,"value":5.0e-11,"std_dev":1.2e-12,"samples":100}]}}
{"ok":false,"error":{"kind":"bad_request","message":"no signal called warp core; ..."}}
```

| Code | Kind | Meaning |
| --- | --- | --- |
| 0 | | Done. |
| 1 | `failed` | Anything not below. |
| 2 | `bad_request` | Did not parse, or named something that does not exist. |
| 3 | `not_connected` | No server, or the session is not connected. |
| 4 | `read_only` | The command acts and the server is read-only. |
| 5 | `refused` | Outside the limits, or beyond the controller. |
| 6 | `busy` | A job or another request held the session past the timeout. |
| 7 | `controller` | The controller or the link to it failed. |

Logs go to stderr (`--log info` for more), so stdout is only ever the reply. A
malformed command line is a `bad_request` reply too; only `--help` and
`--version` answer in plain text, since they are asked for as text.

## Read-only and limits

Every command declares whether it acts. A server started read-only, which the
workbench is by default, refuses those with `read_only` before anything
reaches the controller, so an agent can be handed a connection that watches
but cannot move.

`serve --limits limits.toml` loads the bounds acting commands will have to stay
inside. `describe` and `status` report them, so an agent knows them up front:

```toml
max_bias_v = 2.0          # largest bias magnitude a command may set, V
max_pulse_v = 6.0         # largest pulse magnitude, V
max_coarse_steps = 10     # most coarse steps per command, any axis
z_setpoint = [10e-12, 1e-9]  # Z setpoint range, in the loop input's unit
```

A field left out sets no limit; a misspelt one is an error.

For now only `serve` takes a limits file. The workbench's socket and one-shot
commands will need one before the first acting command lands; until then
there is nothing for limits to bound.
