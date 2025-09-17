#set page(paper: "a4", flipped: true, margin: 1cm)
#import "@preview/lilaq:0.4.0" as lq
#import calc: abs


#{
  // let file_name = sys.inputs.path
  let file_name = "../history"
  let read_jsonl(file_name) = {
    let content = read(file_name)
    let lines = content.split("\n")
    lines.filter(line => line.trim() != "").map(line => json(bytes(line)))
  }

  let data = read_jsonl(file_name).map(entry => entry.data)

  let dot_size = 8pt

  show: lq.cond-set(
    lq.grid.with(kind: "x"),
    stroke: none,
    stroke-sub: gray,
  )

  show: lq.cond-set(
    lq.grid.with(kind: "y"),
    stroke: none,
  )
  align(
    horizon + center,
    lq.diagram(
      width: 20cm,
      height: 10cm,
      xaxis: (
        ticks: data.map(v => v.cycle),
        subticks: 1,
      ),
      lq.bar(
        data.map(entry => entry.cycle),
        data.map(entry => {
          entry.pulse_voltage
        }),
        fill: none,
        stroke: (top: 2pt, bottom: none, left: none, right: none),
        width: 1.0,
        label: $V_"pulse"$,
      ),
      lq.bar(
        data.map(entry => entry.cycle),
        data.map(entry => {
          entry.freq_shift
        }),
        fill: none,
        stroke: (top: 2pt, bottom: none, left: none, right: none),
        width: 1.0,
        label: $Delta f$,
      ),
      lq.bar(
        data.map(entry => entry.cycle),
        data.map(entry => {
          if entry.freq_shift_change == none { 0 } else {
            abs(entry.freq_shift_change)
          }
        }),
        fill: none,
        stroke: (top: 2pt, bottom: none, left: none, right: none),
        width: 1.0,
        label: $Delta Delta f$,
      ),
      lq.bar(
        data.map(entry => entry.z),
        data.map(entry => {
          if entry.freq_shift_change == none { 0 } else {
            abs(entry.freq_shift_change)
          }
        }),
        fill: none,
        stroke: (top: 2pt, bottom: none, left: none, right: none),
        width: 1.0,
        label: $z_pos$,
      ),
    ),
  )
}

