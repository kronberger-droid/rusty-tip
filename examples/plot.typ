#import "@preview/lilaq:0.4.0" as lq

#let read_jsonl(filename) = {
  let content = read(filename)
  let lines = content.split("\n")

  lines.filter(line => line.trim() != "").map(line => json(bytes(line)))
}

#let data = read_jsonl("./history/2025-08-06T13-17-01.jsonl")

#let ys = data.map(sample => sample.primary_signal)
#let xs = data.map(sample => sample.timestamp)

#lq.diagram(
  width: 12cm,
  height: 8cm,
  lq.plot(xs, ys),
)
