use textplots::{Chart, Plot};

/// Determine the best scale and unit for a given maximum value
fn determine_scale(max_value: f64) -> (f64, &'static str) {
    if max_value >= 1.0 {
        (1.0, "")
    } else if max_value >= 1e-3 {
        (1e3, "m")
    } else if max_value >= 1e-6 {
        (1e6, "μ")
    } else if max_value >= 1e-9 {
        (1e9, "n")
    } else {
        (1e12, "p")
    }
}

/// Plot any slice of f64 values with automatic dynamic scaling
///
/// # Arguments
/// * `values` - The data values to plot
/// * `title` - Optional title for the plot
/// * `width` - Optional plot width (default: 140)
/// * `height` - Optional plot height (default: 60)
///
/// # Examples
/// ```
/// use rusty_tip::plotting::plot_values;
///
/// let data = vec![1e-12, 2e-12, 1.5e-12, 3e-12];
/// plot_values(&data, Some("Current Signal"), None, None).unwrap();
/// ```
pub fn plot_values(
    values: &[f64],
    title: Option<&str>,
    width: Option<usize>,
    height: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    if values.is_empty() {
        return Err("Cannot plot empty data".into());
    }

    let width = width.unwrap_or(140);
    let height = height.unwrap_or(60);

    // Find min/max values for scaling
    let min_value = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_value = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let max_abs = max_value.abs().max(min_value.abs());

    // Determine scaling
    let (value_scale, value_unit) = determine_scale(max_abs);

    // Create frame data for plotting (index, scaled_value)
    let frame: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(i, &value)| (i as f32, (value * value_scale) as f32))
        .collect();

    let max_index = (values.len() - 1) as f32;
    let scaled_min = min_value * value_scale;
    let scaled_max = max_value * value_scale;

    // Print header and info
    if let Some(title) = title {
        println!("{}", title);
    } else {
        println!("Data Plot");
    }
    println!("X-axis: Sample Index | Y-axis: {}units", value_unit);
    println!(
        "Range: {} samples | Values: {:.3} to {:.3} {}units",
        values.len(),
        scaled_min,
        scaled_max,
        value_unit
    );
    println!("{}", "─".repeat(width));

    // Create and display the plot
    Chart::new(width as u32, height as u32, 0.0, max_index)
        .lineplot(&textplots::Shape::Lines(&frame))
        .nice();

    println!("Sample Index →");

    Ok(())
}

/// Plot with custom Y-axis range (useful when you want to set your own bounds)
/// Note: textplots doesn't support custom Y ranges, so this clips the data to the range
pub fn plot_values_with_range(
    values: &[f64],
    y_min: f64,
    y_max: f64,
    title: Option<&str>,
    width: Option<usize>,
    height: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    if values.is_empty() {
        return Err("Cannot plot empty data".into());
    }

    let width = width.unwrap_or(140);
    let height = height.unwrap_or(60);
    let max_abs = y_max.abs().max(y_min.abs());

    // Determine scaling based on provided range
    let (value_scale, value_unit) = determine_scale(max_abs);

    // Create frame data, clipping values to the specified range
    let frame: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let clipped_value = value.max(y_min).min(y_max);
            (i as f32, (clipped_value * value_scale) as f32)
        })
        .collect();

    let max_index = (values.len() - 1) as f32;
    let scaled_y_min = y_min * value_scale;
    let scaled_y_max = y_max * value_scale;

    // Print header
    if let Some(title) = title {
        println!("{}", title);
    } else {
        println!("Data Plot (Clipped to Range)");
    }
    println!("X-axis: Sample Index | Y-axis: {}units", value_unit);
    println!(
        "Range: {} samples | Y-Range: {:.3} to {:.3} {}units (clipped)",
        values.len(),
        scaled_y_min,
        scaled_y_max,
        value_unit
    );
    println!("{}", "─".repeat(width));

    // Create plot (textplots will auto-scale to the data)
    Chart::new(width as u32, height as u32, 0.0, max_index)
        .lineplot(&textplots::Shape::Lines(&frame))
        .nice();

    println!("Sample Index →");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_scale() {
        // Test different scales
        assert_eq!(determine_scale(5.0), (1.0, ""));
        assert_eq!(determine_scale(0.005), (1e3, "m"));
        assert_eq!(determine_scale(5e-6), (1e6, "μ"));
        assert_eq!(determine_scale(5e-9), (1e9, "n"));
        assert_eq!(determine_scale(5e-12), (1e12, "p"));
    }

    #[test]
    fn test_plot_values_basic() {
        let data = vec![1.0, 2.0, 3.0, 2.0, 1.0];
        // Should not panic
        assert!(plot_values(&data, Some("Test Plot"), None, None).is_ok());
    }

    #[test]
    fn test_plot_empty_data() {
        let data: Vec<f64> = vec![];
        assert!(plot_values(&data, None, None, None).is_err());
    }

    #[test]
    fn test_plot_small_values() {
        let data = vec![1e-12, 2e-12, 1.5e-12, 3e-12];
        // Should not panic and should use pico scaling
        assert!(plot_values(&data, Some("Picoamp Current"), None, None).is_ok());
    }
}
