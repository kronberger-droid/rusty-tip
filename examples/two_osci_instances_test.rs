use rusty_tip::NanonisClient;
use std::error::Error;
use textplots::{Chart, Plot, Shape};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut client_1 = NanonisClient::new("127.0.0.1", 6501)?;
    let mut client_2 = NanonisClient::new("127.0.0.1", 6502)?;

    client_1.osci1t_run()?;
    client_1.osci1t_ch_set(0)?;

    client_2.osci1t_run()?;
    client_2.osci1t_ch_set(0)?;

    println!("{:?}", client_1.osci1t_trig_get());

    let mut samples: Vec<Vec<f64>> = Vec::new();

    let slice_len = 20;

    samples.push(
        client_1
            .osci1t_data_get(0)?
            .3
            .into_iter()
            .take(slice_len)
            .collect(),
    );

    samples.push(
        client_2
            .osci1t_data_get(0)?
            .3
            .into_iter()
            .skip(255 - slice_len)
            .collect(),
    );

    println!("First Sample:");
    plot_osci_data(samples[0].clone())?;

    println!("Second Sample:");
    plot_osci_data(samples[1].clone())?;

    Ok(())
}

fn plot_osci_data(osci_data: Vec<f64>) -> Result<(), Box<dyn Error>> {
    let plot_data: Vec<(f32, f32)> = osci_data
        .iter()
        .enumerate()
        .map(|(i, &value)| (i as f32, value as f32))
        .collect();

    Chart::new(120, 30, 0.0, 20.0)
        .lineplot(&Shape::Lines(&plot_data))
        .display();

    Ok(())
}
