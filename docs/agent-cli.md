# `rusty-tip`: the command line for scripts and agents

`rusty-tip` drives a session the way the workbench does, but for a program on
the other end: every command prints one JSON reply and exits with a code that
says how it went. An agent can learn the interface from `rusty-tip describe`
and plan inside the limits it reports, without reading this page.

This first version only reads. Commands that act on the instrument come next,
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
```

Signal names are the registry's, case-insensitive. Each reading carries the
`unit` the controller names the signal with, `"A"` for `"Current (A)"`, or
`null` when the name has none. `status` answers from the session's last
report, so it answers while a job runs; a `read` needs the controller and
waits up to 10 s for a running job before answering `busy`, and a request that
timed out is dropped, never run late.

Over the socket the same commands are JSON, one request per line and one reply
per line, as many as you like on one connection:

```json
{"cmd":"read","signals":["current"],"samples":100}
```

A misspelt parameter is an error, not ignored.

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
| 6 | `busy` | A job held the session past the timeout. |
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
