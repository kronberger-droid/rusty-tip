use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use rusty_tip::{NanonisClient, NanonisError};

struct ScanReaderApp {
    client: Arc<Mutex<Option<NanonisClient>>>,
    connected: bool,
    address: String,
    status_message: String,
    scan_data: Vec<Vec<f32>>,
    channel_name: String,
    rows: i32,
    columns: i32,
    auto_refresh: bool,
    channel_index: u32,
}

impl Default for ScanReaderApp {
    fn default() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
            connected: false,
            address: "127.0.0.1:6501".to_string(),
            status_message: "Not connected".to_string(),
            scan_data: Vec::new(),
            channel_name: String::new(),
            rows: 0,
            columns: 0,
            auto_refresh: false,
            channel_index: 0,
        }
    }
}

impl ScanReaderApp {
    fn connect(&mut self) {
        match NanonisClient::new(&self.address) {
            Ok(client) => {
                *self.client.lock().unwrap() = Some(client);
                self.connected = true;
                self.status_message = format!("Connected to {}", self.address);
            }
            Err(e) => {
                self.connected = false;
                self.status_message = format!("Connection failed: {}", e);
            }
        }
    }

    fn disconnect(&mut self) {
        *self.client.lock().unwrap() = None;
        self.connected = false;
        self.status_message = "Disconnected".to_string();
    }

    fn fetch_scan_data(&mut self) {
        if !self.connected {
            return;
        }

        let client_arc = Arc::clone(&self.client);
        let channel_index = self.channel_index;
        
        if let Ok(mut client_guard) = client_arc.lock() {
            if let Some(ref mut client) = *client_guard {
                match client.scan_frame_data_grab(channel_index, 1) {
                    Ok((name, rows, cols, data, _direction)) => {
                        self.channel_name = name;
                        self.rows = rows;
                        self.columns = cols;
                        self.scan_data = data;
                        self.status_message = format!(
                            "Loaded scan data: {} ({}x{} pixels)",
                            self.channel_name, self.rows, self.columns
                        );
                    }
                    Err(e) => {
                        self.status_message = format!("Failed to fetch scan data: {}", e);
                    }
                }
            }
        }
    }

    fn get_heatmap_data(&self) -> Vec<[f64; 3]> {
        let mut heatmap_data = Vec::new();
        
        if self.scan_data.is_empty() {
            return heatmap_data;
        }

        for (row_idx, row) in self.scan_data.iter().enumerate() {
            for (col_idx, &value) in row.iter().enumerate() {
                heatmap_data.push([
                    col_idx as f64,
                    row_idx as f64,
                    value as f64,
                ]);
            }
        }
        
        heatmap_data
    }

    fn get_line_profile(&self, row: usize) -> Vec<[f64; 2]> {
        if row >= self.scan_data.len() {
            return Vec::new();
        }
        
        self.scan_data[row]
            .iter()
            .enumerate()
            .map(|(x, &y)| [x as f64, y as f64])
            .collect()
    }
}

impl eframe::App for ScanReaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.auto_refresh && self.connected {
            ctx.request_repaint_after(Duration::from_millis(1000));
            self.fetch_scan_data();
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Nanonis Address:");
                ui.text_edit_singleline(&mut self.address);
                
                if self.connected {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                } else {
                    if ui.button("Connect").clicked() {
                        self.connect();
                    }
                }
                
                ui.separator();
                ui.label("Channel Index:");
                ui.add(egui::DragValue::new(&mut self.channel_index).range(0..=127));
                
                if ui.button("Fetch Data").clicked() && self.connected {
                    self.fetch_scan_data();
                }
                
                ui.checkbox(&mut self.auto_refresh, "Auto Refresh");
            });
            
            ui.label(&self.status_message);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.scan_data.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("No scan data loaded. Connect and fetch data to begin.");
                });
                return;
            }

            ui.horizontal(|ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.heading("Scan Information");
                        ui.label(format!("Channel: {}", self.channel_name));
                        ui.label(format!("Dimensions: {} x {} pixels", self.columns, self.rows));
                        ui.label(format!("Total pixels: {}", self.scan_data.len() * self.scan_data.get(0).map_or(0, |row| row.len())));
                    });
                });
            });

            ui.separator();

            // Simple 2D visualization using a plot
            ui.heading("Scan Data Visualization");
            
            let plot = Plot::new("scan_plot")
                .view_aspect(1.0)
                .auto_bounds_x()
                .auto_bounds_y()
                .show_axes([true, true]);

            plot.show(ui, |plot_ui| {
                // Show scan data as scattered points with color based on value
                let heatmap_points = self.get_heatmap_data();
                
                if !heatmap_points.is_empty() {
                    // Find min/max values for color scaling
                    let (min_val, max_val) = heatmap_points.iter()
                        .map(|point| point[2])
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), val| {
                            (min.min(val), max.max(val))
                        });
                    
                    // Create colored points
                    let points: PlotPoints = heatmap_points.iter()
                        .map(|point| [point[0], point[1]])
                        .collect();
                    
                    let line = Line::new(points)
                        .color(egui::Color32::from_rgb(100, 150, 255))
                        .name("Scan Data");
                    
                    plot_ui.line(line);
                }
            });

            // Show a line profile for the middle row
            if !self.scan_data.is_empty() {
                ui.separator();
                ui.heading("Line Profile (Middle Row)");
                
                let middle_row = self.scan_data.len() / 2;
                let profile_data = self.get_line_profile(middle_row);
                
                if !profile_data.is_empty() {
                    let plot = Plot::new("line_profile")
                        .height(200.0)
                        .auto_bounds_x()
                        .auto_bounds_y();
                    
                    plot.show(ui, |plot_ui| {
                        let line = Line::new(PlotPoints::new(profile_data))
                            .color(egui::Color32::from_rgb(255, 100, 100))
                            .name(format!("Row {}", middle_row));
                        
                        plot_ui.line(line);
                    });
                }
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    env_logger::init();
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Nanonis Scan Data Reader"),
        ..Default::default()
    };
    
    eframe::run_native(
        "Nanonis Scan Reader",
        options,
        Box::new(|_cc| Ok(Box::new(ScanReaderApp::default()))),
    )
}