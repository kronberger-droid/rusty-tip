//! Writing scan-shaped data out to formats other programs can read.
//!
//! Everything in here is pure file I/O over `ndarray` maps in SI units; no
//! hardware, no controller. Two formats are supported, for two different jobs:
//!
//! - [`gsf`] — Gwyddion Simple Field. A binary 2D image format that carries its
//!   own physical dimensions and units, so Gwyddion (and anything else that
//!   reads GSF) opens the file already calibrated in metres. This is the one to
//!   use for looking at a map.
//! - [`xyz`] — plain ASCII point clouds and column tables. Slower and much
//!   larger, but readable by essentially everything: Gwyddion's XYZ import,
//!   gnuplot, numpy, ParaView, a text editor. This is the one to use for
//!   comparing two maps sample by sample.
//!
//! Both take row-major `[row, column]` = `[y, x]` maps, matching the layout a
//! scan frame arrives in.

pub mod gsf;
pub mod xyz;

pub use gsf::{GsfField, read_gsf, write_gsf};
pub use xyz::{write_table, write_xyz};
