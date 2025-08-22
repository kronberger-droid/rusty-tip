#import "@preview/lilaq:0.4.0" as lq

// Set up a very large page for zooming into individual plots
#set page(
  width: 200cm, // Very wide page
  height: 150cm, // Very tall page
  margin: 2cm,
)

#let read_jsonl(filename) = {
  let content = read(filename)
  let lines = content.split("\n")
  lines.filter(line => line.trim() != "").map(line => json(bytes(line)))
}

#let read_metadata(filename) = {
  let content = read(filename)
  json(bytes(content))
}

#let file_name = "./history/2025-08-09T20-05-02"

// Read the session metadata to get signal names and active indices
#let metadata_file = file_name + ".metadata.json"
#let data_file = file_name + ".jsonl" // Old format with real data

#let metadata = read_metadata(metadata_file)
#let raw_data = read_jsonl(data_file)

// Convert old format data to work with new metadata structure
#let data = raw_data.map(sample => {
  // Extract only the signals that correspond to our active indices
  let active_signals = metadata.active_indices.map(idx => {
    if sample.all_signals != none and idx < sample.all_signals.len() {
      sample.all_signals.at(idx)
    } else {
      0.0
    }
  })

  (
    primary_signal: active_signals.first(),
    all_signals: active_signals, // Only active signals, matching metadata
    timestamp: sample.timestamp,
    classification: sample.classification,
    approach_count: if "approach_count" in sample {
      sample.approach_count
    } else { 0 },
    last_action: if "last_action" in sample { sample.last_action } else {
      none
    },
    system_parameters: if "system_parameters" in sample {
      sample.system_parameters
    } else { () },
    position: if "position" in sample { sample.position } else { none },
    z_position: if "z_position" in sample { sample.z_position } else { none },
  )
})

// Extract time series (convert to relative time in seconds)
#let time = data.map(sample => sample.timestamp)
#let time_relative = time.map(t => t - time.first())

// Extract signal names for active signals only
#let signal_names = metadata.active_indices.map(idx => metadata.signal_names.at(
  idx,
))
#let num_signals = signal_names.len()

// Create a grid layout - calculate optimal rows/cols
#let cols = calc.ceil(calc.sqrt(num_signals))
#let rows = calc.ceil(num_signals / cols)

#align(center)[
  = Nanonis Signal Monitor - All Active Signals
  #text(size: 12pt)[
    Session: #metadata.session_id \
    Total signals: #metadata.signal_names.len(), Active: #num_signals \
    Samples: #data.len(), Duration: #{ calc.round((time_relative.last()), digits: 1) }s
  ]
]

// Create grid of plots
#grid(
  columns: cols,
  rows: rows,
  gutter: 1cm,

  // Generate a plot for each active signal
  ..range(num_signals).map(signal_idx => {
    let signal_name = signal_names.at(signal_idx)
    let values = data.map(sample => {
      if sample.all_signals != none and signal_idx < sample.all_signals.len() {
        sample.all_signals.at(signal_idx)
      } else {
        0.0
      }
    })

    // Create individual plot
    [
      #align(center)[
        #text(size: 14pt, weight: "bold")[#signal_name]
        #text(size: 10pt)[Index: #metadata.active_indices.at(signal_idx)]
      ]

      #lq.diagram(
        width: 25cm,
        height: 18cm,
        lq.plot(
          time_relative,
          values,
        ),
      )
    ]
  })
)

// Add footer with instructions
#v(2cm)
#align(center)[
  #text(size: 11pt, style: "italic")[
    Zoom into individual plots to see details. Each plot shows the full time series for one active signal. \
    Signal indices correspond to the original Nanonis channel numbers.
  ]
]
