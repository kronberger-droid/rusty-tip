use clap::Parser;
use image::ImageReader;
use rusty_tip::analyzer::cuox_rows::CuoxRowDetector;
use rusty_tip::analyzer::{Analyzer, AnalyzerInput};
use std::path::PathBuf;

/// Detect CuOx reconstruction rows in STM images of Cu(110).
///
/// Reads a grayscale PNG, detects band orientation and edges,
/// and prints results as JSON with image-space coordinates for
/// each band's center and edge lines.
#[derive(Parser)]
#[command(name = "cuox-finder", version)]
struct Cli {
    /// Input image path (PNG, grayscale or RGB)
    #[arg(short = 'i', long)]
    input: PathBuf,

    /// Calibration in nm/pixel (enables nm readouts)
    #[arg(short = 'c', long)]
    calibration: Option<f64>,

    /// Fix angle (degrees) instead of auto-detecting
    #[arg(short = 'a', long)]
    angle: Option<f32>,

    /// Local variance window radius (default: 5)
    #[arg(long, default_value_t = 5)]
    var_radius: usize,

    /// Detection threshold as fraction of max (default: 0.3)
    #[arg(long, default_value_t = 0.3)]
    threshold: f32,

    /// Minimum band width in pixels (default: 5)
    #[arg(long, default_value_t = 5)]
    min_band_width: usize,
}

fn main() {
    let cli = Cli::parse();

    // Load image
    let img = ImageReader::open(&cli.input)
        .unwrap_or_else(|e| {
            eprintln!("Failed to open {}: {}", cli.input.display(), e);
            std::process::exit(1);
        })
        .decode()
        .unwrap_or_else(|e| {
            eprintln!("Failed to decode {}: {}", cli.input.display(), e);
            std::process::exit(1);
        });

    let gray = img.to_luma32f();
    let (width, height) = gray.dimensions();

    // Convert to Vec<Vec<f32>>
    let data: Vec<Vec<f32>> = (0..height)
        .map(|y| (0..width).map(|x| gray.get_pixel(x, y).0[0]).collect())
        .collect();

    // Build detector
    let detector = CuoxRowDetector {
        var_radius: cli.var_radius,
        threshold: cli.threshold,
        min_band_width: cli.min_band_width,
        fixed_angle: cli.angle,
    };

    // Build input
    let input = AnalyzerInput {
        channel_name: "grayscale".into(),
        data,
        calibration_m_per_px: cli.calibration.map(|nm| nm * 1e-9),
    };

    // Run analysis
    let output = detector.analyze(&input).unwrap_or_else(|e| {
        eprintln!("Analysis failed: {}", e);
        std::process::exit(1);
    });

    // Print results
    println!("{}", serde_json::to_string_pretty(&output.data).unwrap());
}
