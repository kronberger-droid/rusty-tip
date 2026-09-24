# Constant-distance scanning

A constant-current scan holds the tunnelling current fixed and lets the feedback
loop move Z. A constant-distance scan does the opposite: the feedback is
switched off and Z is driven along a path worked out in advance. This document
covers how that path is computed, what the numbers mean, and how to look at the
result.

The work splits into three pieces. Only the first exists today.

| Piece | What it does | Status |
| --- | --- | --- |
| 1. Plan | constant-current Z map to tip trajectory | implemented |
| 2. Acquire | TCP sample stream to 2D Z map | not designed |
| 3. Trace | drive the tip along the trajectory | blocked on hardware measurements |

Piece 1 is pure geometry: no controller, no hardware, no timing. That is
deliberate, and it is why it could be finished and tested first.

## Why the trajectory is not just the topograph

The obvious plan is "hold Z at whatever the constant-current scan measured".
That fails, and it fails in a way that costs tips.

A tip is not a point. It has a body, and near a step edge the flank of that body
touches the upper terrace long before the apex is above it. Drive the apex along
the measured Z and the flank hits the step.

The fix is the rolling-sphere construction, the same idea as the rolling sphere
used to place lightning rods and the rolling-ball baseline used in AFM image
processing: roll a rigid body over the surface and record where its apex goes.
Here the body is an **ellipsoid** rather than a sphere, so lateral and vertical
clearance can be set independently.

```
              ellipsoid rolled over an up-step
                                     _________________
              apex path             /
         ..... . . .   .   .      ./
       .'                    `.  /
      (  tip body             ) /
       `.                    .'/
         `.................'  /
    _______________________ /   <- surface (measured Z)
```

The apex path leaves the lower terrace roughly one lateral semi-axis before the
edge and arrives at full step height exactly at the edge.

## The construction

Take a height map `Z[i, j]` in metres with sample spacing `dx`, `dy`, also in
metres. Nanonis has no notion of a pixel size, so that spacing comes out of the
scan range divided by the sample count, and it is the only thing that ties the
map to physical space.

Give the ellipsoid lateral semi-axes `a_x`, `a_y` and vertical semi-axis `c`.
Sitting with its centre at height `Zc`, its lower surface at lateral offset
`(v*dx, u*dy)` is at `Zc - c*sqrt(1 - t)` where `t = (u*dy/a_y)^2 +
(v*dx/a_x)^2`. The body is just clear of the surface when that lower surface
touches `Z` somewhere and is above it everywhere else:

```
Zc[i, j] = max over (u, v) of ( Z[i+u, j+v] + B[u, v] )

B[u, v] = c * sqrt(1 - t)   for t <= 1
        = -infinity         otherwise
```

That is a grayscale morphological **dilation** of `Z` by the structuring element
`B`. The apex hangs `c` below the centre, so the trajectory is

```
Z_tip = Zc - c
```

`B` is computed once as a small table. The `-infinity` entries need no special
case in the inner loop: they can never win a maximum against a finite candidate,
so "inside the footprint" and "outside" collapse into one branch-free max.

Cost is `O(ny * nx * (2*ry + 1) * (2*rx + 1))` with `ry = ceil(a_y/dy)`,
`rx = ceil(a_x/dx)`. Rows are planned in parallel. That is fast enough for
anything at scan sizes; if the footprint ever gets large enough to hurt, the van
Herk / Gil-Werman running-max trick removes the radius dependence.

## Reading the parameters

Two things about `a` and `c` are easy to get backwards.

**Neither buys a constant lift.** On a flat terrace the ellipsoid rests on the
surface and the trajectory *is* the surface, whatever the semi-axes are. They
set the clearance only where the surface is stepped or curved. If what you want
is a constant standoff, that is `Z + h`, a different and much simpler thing.

**Bluntness is the aspect ratio `a/c`, and it runs the counter-intuitive way.**
Approaching an up-step of height `H` from lateral distance `d`, the apex is
allowed to sag below `H` by `c * (1 - sqrt(1 - d^2/a^2))`, and that sag *grows*
with `c`. So at fixed `a`:

- **large `c`** is a slender, sharp body that tucks in close to the edge
- **small `c`** is a flat, blunt body that stands well off

Wider than tall is what gives edges lateral berth. In the limit `c -> 0` the
dome flattens into a disc and the construction degenerates into a plain
sliding-window maximum, which is the most conservative plan available.

`a` sets the **reach**: no feature further than `a` away laterally can influence
the trajectory at all.

### The footprint has to span several samples

If the ellipsoid is narrower than one sample on an axis, the only offset that
survives the ellipse test is zero, the dilation collapses to `Zc = Z + c`, and
the trajectory comes back as `Z_tip = Z`. Not an error, which is worse than an
error: it looks like a result. `RollingEllipsoid::is_resolved` checks for this
and the `const-distance` binary warns when the footprint is under five samples.

## What happens at the frame edge

The footprint overhangs the edge of the map for any sample within `a` of the
border, and what lies out there was never measured. Two answers, and they differ
only inside that border strip:

- `--border truncate` clips the window to the array, which is the same as
  assuming the surface outside is infinitely far below and never constrains the
  tip. Honest about what was measured. The trajectory near the border is
  computed from less evidence than the interior and can sit lower than it should.
- `--border replicate` (the default) clamps out-of-frame offsets to the nearest
  edge sample, assuming the surface continues outward at its border height.
  Never lets the tip drop for lack of data.

Replicate is the default because the trajectory is eventually going to be driven
on hardware, where a border dive is a crash rather than an artefact.

## Caveats that no amount of geometry fixes

- **Feedback off holds Z, not distance.** This computes the Z path that *would*
  hold distance for a rigid tip of the assumed shape. Drift, creep and a wrong
  tip shape all break that, and none of them are modelled here.
- **A constant-current Z map is an iso-current surface, not topography.**
  Electronic contrast shows up as apparent height. It is a reasonable
  first-order proxy and nothing more.
- **The tip is one smooth ellipsoid.** Double tips, asymmetric tips and
  adsorbate-terminated tips are out of scope.

## Using the planner

```console
$ cargo run --bin const-distance -- plan --synthetic combo -a 1.2 -c 0.4 -o out/
```

Every length on the command line is in **nanometres**; everything written to a
file is in **metres**. The conversion happens once, at argument parsing.

Useful flags:

| Flag | Meaning |
| --- | --- |
| `--synthetic <KIND>` | generate a test surface: `flat`, `step`, `terraces`, `pit`, `bumps`, `combo` |
| `--input <FILE>` | read a `.gsf` file, or a whitespace-separated ASCII grid |
| `--input-unit <m\|nm>` | unit of the values in an ASCII input file |
| `--dx`, `--dy` | sample spacing, metres; taken from the file for `.gsf` |
| `-a`, `--lateral` | lateral semi-axis, metres (`--lateral-y` to differ on the slow axis) |
| `-c`, `--vertical` | vertical semi-axis, metres; smaller is blunter |
| `--border` | `truncate` or `replicate` |
| `--profile <ROW>` | which scan line to draw as a terminal cross-section |
| `--no-xyz` | skip the point clouds, which are large and slow to write |

The terminal cross-section is not the deliverable; the files are. It is there
because a wrong plan is usually obvious in a single scan line, and noticing that
before opening Gwyddion saves a round trip.

## Output files

| File | Contents |
| --- | --- |
| `surface.gsf` | input topograph, as a Gwyddion field |
| `tip.gsf` | planned trajectory |
| `clearance.gsf` | `tip - surface`: where the plan lifts, and how far |
| `compare.dat` | all three co-registered, one line per sample |
| `surface.xyz`, `tip.xyz` | ASCII point clouds, `x y z`, metres |
| `plan.json` | the parameters this plan was computed with |
| `README.txt` | the same summary, next to the data |

Two formats because they answer different questions. **GSF** carries physical
dimensions and units, so Gwyddion opens the file already calibrated in metres
and nothing needs re-entering. It holds one channel, so it is the format for
*looking* at a map. **`compare.dat`** puts the surface, the trajectory and their
difference on one line per sample, so it is the format for *comparing* them: the
difference is already a column rather than something to recompute by joining
files.

Sample `(i, j)` is written at the centre of the area it covers, `x = (j + 0.5) *
dx`, so a point cloud lands exactly on top of the matching `.gsf` map.

### Viewing

```console
$ gwyddion out/surface.gsf out/tip.gsf out/clearance.gsf
```

The `.xyz` files import through Gwyddion's XYZ module (File > Open). For a
single scan line, gnuplot is quicker:

```console
$ gnuplot -p -e "plot 'out/compare.dat' every ::0::255 u 1:3 w l title 'surface', \
                      '' every ::0::255 u 1:4 w l title 'tip'"
```

(`255` being the last sample of the first scan line for a 256-wide map.)

In Python:

```python
x, y, z_surface, z_tip, clearance = np.loadtxt("out/compare.dat", unpack=True)
```

### What to look for

- clearance is **never negative** — the tip is never below the surface
- clearance is **exactly zero on flat terraces** — this is a constant distance,
  not a constant lift
- at a step edge the trajectory **starts rising up to one lateral semi-axis
  before** the edge and **stays high the same distance past** it
- inside a feature narrower than the tip, the trajectory **does not reach the
  floor** — a finite tip physically cannot, and a plan that claimed otherwise
  would be wrong

## Where the code lives

| Path | What |
| --- | --- |
| `src/analyzer/rolling_ellipsoid.rs` | the geometry, and the bulk of the reasoning in doc comments |
| `src/export/gsf.rs` | Gwyddion Simple Field reader and writer |
| `src/export/xyz.rs` | ASCII point clouds and column tables |
| `bin/const-distance/main.rs` | CLI, and the stubs for pieces 2 and 3 |
| `bin/const-distance/surface.rs` | synthetic test surfaces |
| `tests/const_distance_plan.rs` | end-to-end planning and export |

## Drift between scans

`const-distance drift` exposes the Z drift tools on the command line, so they
can be exercised on hardware before a routine depends on them:

```console
$ const-distance drift --host 192.168.1.10 status
$ const-distance drift --host 192.168.1.10 --signal-name "Z (m)" measure
$ const-distance drift --host 192.168.1.10 --signal-name "Z (m)" compensate
$ const-distance drift --host 192.168.1.10 off
```

`measure` fits a line to Z over one window (5 s and 16 samples by default) and
prints the rate in pm/s; it needs the feedback closed, the scan stopped, and a
flat spot under the tip. `compensate` runs three windows: one to measure, one
with a trial velocity applied to learn which way the controller's velocity
sign runs, and one to report what is left. It refuses a compensation channel
that does not respond rather than writing a number derived from noise. An
axis that has hit the saturation limit is reported as such, since the
controller stops compensating it silently and only an off/on cycle restarts
it.

## Still open

- **Piece 2**, mapping the nanonis-rs TCP sample stream into scan lines, has not
  been designed. `const-distance acquire` is a stub that says so.
- **Piece 3** is gated on hardware measurements that have not been made: TCP
  latency, feedback-off sequencing, TipLift, Z step response, whether FolMe
  blocks, and staircase visibility. `examples/folme_probe.rs` measures all
  but the last against the bare client, retracting before every open-loop
  move and measuring the retract direction from TipLift rather than assuming
  it.
- Whether the tip model should come from something measured rather than typed in
  by hand. A tip characteriser on a known sharp feature would give `a` and `c`
  from data instead of from a guess.
