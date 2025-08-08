#import "@preview/lilaq:0.4.0" as lq

#set page(
  paper: "a4",
  flipped: true,
  margin: 1cm,
)

#let read_jsonl(filename) = {
  let content = read(filename)
  let lines = content.split("\n")

  lines.filter(line => line.trim() != "").map(line => json(bytes(line)))
}

#let data = read_jsonl("./history/2025-08-06T13-17-01.jsonl")

#let values = data.map(sample => sample.primary_signal)
#let time = data.map(sample => sample.timestamp)

#let time = time.map(sample => sample - time.first())

#lq.diagram(
  width: 26cm,
  height: 18cm,
  lq.plot(time, values),
)
