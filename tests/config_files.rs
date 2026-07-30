//! Guards the configs shipped in `configs/`.
//!
//! These files are the documented entry point for running the tools, so a typo
//! or a schema drift in one of them should fail here rather than in front of a
//! microscope.

/// Every config in `configs/` must both parse and pass `validate()`.
#[test]
fn shipped_configs_parse_and_validate() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("configs");
    let mut checked = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("configs/ should exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }

        let cfg = rusty_tip::config::load_config(&path).unwrap_or_else(|e| {
            panic!("{} failed to parse: {e}", path.display())
        });
        cfg.validate().unwrap_or_else(|e| {
            panic!("{} failed to validate: {e}", path.display())
        });
        checked.push(path);
    }

    assert!(!checked.is_empty(), "no configs found in {}", dir.display());
}

/// A `[tip_prep.stability]` table may set only the fields it cares about; the
/// rest fall back to `StabilityConfig::default()`. Regression test for shipped
/// configs failing to load with a bare "missing field bias_range".
#[test]
fn partial_stability_table_falls_back_to_defaults() {
    let dir = std::env::temp_dir().join("rusty_tip_partial_stability_cfg");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("partial.toml");

    std::fs::write(
        &path,
        r#"
[nanonis]
host_ip = "127.0.0.1"
control_ports = [6501]

[data_acquisition]
data_port = 6590
sample_rate = 2000

[experiment_logging]
enabled = false
output_path = "./experiments"

[console]
verbosity = "info"

[tip_prep]
sharp_tip_bounds = [-2.0, 0.0]

[tip_prep.stability]
check_stability = false

[pulse_method]
type = "fixed"
voltage = 5.0
"#,
    )
    .expect("write temp config");

    let cfg = rusty_tip::config::load_config(&path)
        .expect("a partial stability table should load");

    assert!(
        !cfg.tip_prep.stability.check_stability,
        "explicit value wins"
    );

    let defaults = rusty_tip::controller_types::StabilityConfig::default();
    assert_eq!(
        cfg.tip_prep.stability.bias_range, defaults.bias_range,
        "unspecified fields should come from Default"
    );
    assert_eq!(cfg.tip_prep.stability.bias_steps, defaults.bias_steps);

    let _ = std::fs::remove_dir_all(&dir);
}
