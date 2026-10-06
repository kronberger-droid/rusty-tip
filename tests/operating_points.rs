//! Operating points against the mock: a capture reads everything a point
//! sets, an apply writes it back with the frame's centre kept, and an apply
//! is refused, writing nothing, while a scan runs or when a Z loop names an
//! input the module does not have.
use std::sync::Arc;

use nanonis_rs::Position;
use nanonis_rs::scan::ScanFrame;
use rusty_tip::ShutdownFlag;
use rusty_tip::controllers::{ControllerAppliedEvent, ControllerParams};
use rusty_tip::event::EventAccumulator;
use rusty_tip::mock_controller::{MockController, MockObservations, models};
use rusty_tip::operating_point::{
    ApplyOperatingPoint, CaptureOperatingPoint, OperatingPoint, OperatingPointApplied,
    OperatingPointSaved, OperatingPointStore, OperatingState, SaveAs,
};
use rusty_tip::session::{Job, PresetFiles, Session};
use rusty_tip::signal_registry::{SignalIndex, SignalRegistry};

fn session() -> (Session, Arc<parking_lot::Mutex<MockObservations>>) {
    let mut mock = MockController::builder()
        .freq_shift_index(SignalIndex(2))
        .freq_shift(models::always(-1.0))
        .build();
    let obs = mock.observations();
    let reg = SignalRegistry::from_controller(&mut mock).unwrap();
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    (session, obs)
}

fn run(session: &mut Session, job: &mut dyn Job) -> Arc<EventAccumulator> {
    let recorder = Arc::new(EventAccumulator::new(usize::MAX));
    session
        .run(job, &ShutdownFlag::new(), vec![Box::new(recorder.clone())])
        .unwrap();
    recorder
}

fn capture(session: &mut Session) -> OperatingState {
    let events = run(session, &mut CaptureOperatingPoint::default());
    let states = events.custom::<OperatingState>();
    assert_eq!(states.len(), 1);
    states[0].clone()
}

/// A point that differs from what the mock holds in every part: bias,
/// frame, resolution, speed and the Z loop's setpoint.
fn changed(state: OperatingState) -> OperatingPoint {
    let mut point = OperatingPoint::from_state("imaging", "", None, state);
    point.bias_v = -0.75;
    point.scan.width_m = 30e-9;
    point.scan.height_m = 15e-9;
    point.scan.angle_deg = 12.5;
    point.scan.pixels = 512;
    point.scan.lines = 128;
    point.scan.speed.forward_m_s = 20e-9;
    for entry in &mut point.controllers {
        if let ControllerParams::Z(z) = &mut entry.params {
            z.setpoint = 250e-12;
        }
    }
    point
}

fn writes(obs: &parking_lot::Mutex<MockObservations>) -> usize {
    let obs = obs.lock();
    [
        "set_bias",
        "write_controller",
        "scan_frame_set",
        "scan_buffer_set",
        "scan_speed_set",
    ]
    .iter()
    .map(|call| obs.call_counts.get(call).copied().unwrap_or(0))
    .sum()
}

#[test]
fn a_point_applied_is_what_a_capture_reads_back() {
    let (mut session, _) = session();
    let point = changed(capture(&mut session));

    let events = run(
        &mut session,
        &mut ApplyOperatingPoint {
            point: point.clone(),
        },
    );
    let applied = events.custom::<OperatingPointApplied>();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].after.bias_v, -0.75);
    assert_ne!(applied[0].before.bias_v, applied[0].after.bias_v);
    assert_eq!(
        events.custom::<ControllerAppliedEvent>().len(),
        point.controllers.len(),
        "every loop is written, setpoint included"
    );

    let after = capture(&mut session);
    assert_eq!(after.bias_v, point.bias_v);
    assert_eq!(after.scan, point.scan);
    assert_eq!(after.controllers, point.controllers);
}

/// A capture with a name saves what it read there, replacing a point of
/// that name, and says so.
#[test]
fn a_capture_saves_what_it_read_under_the_name() {
    let (mut session, _) = session();
    let path = std::env::temp_dir().join(format!(
        "rusty-tip-capture-save-{}.toml",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let save = |note: &str| CaptureOperatingPoint {
        save_as: Some(SaveAs {
            path: path.clone(),
            name: "overview".into(),
            note: note.into(),
        }),
    };

    run(&mut session, &mut save("first"));
    let events = run(&mut session, &mut save("second"));

    let read = events.custom::<OperatingState>();
    let saved = events.custom::<OperatingPointSaved>();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "overview");
    let points = OperatingPointStore::new(&path).list().unwrap();
    assert_eq!(points.len(), 1, "the second save replaces the first");
    assert_eq!(points[0].note, "second");
    assert_eq!(points[0].bias_v, read[0].bias_v);
    assert_eq!(points[0].controllers, read[0].controllers);
    let _ = std::fs::remove_file(&path);
}

/// Applying a point resizes and turns the frame but leaves it where it is
/// on the sample.
#[test]
fn an_apply_keeps_the_frame_where_it_is() {
    let (mut session, _) = session();
    session
        .query(|c, _| {
            c.scan_frame_set(ScanFrame::new(
                Position::new(1e-6, -2e-6),
                50e-9,
                50e-9,
                0.0,
            ))
        })
        .unwrap();
    let point = changed(capture(&mut session));
    run(&mut session, &mut ApplyOperatingPoint { point });
    let frame = session.query(|c, _| c.scan_frame_get()).unwrap();
    assert_eq!((frame.center.x, frame.center.y), (1e-6, -2e-6));
    assert_eq!(frame.width_m, 30e-9);
}

#[test]
fn an_apply_while_a_scan_runs_is_refused_and_writes_nothing() {
    let (mut session, obs) = session();
    let point = changed(capture(&mut session));
    obs.lock().scan_running = true;
    let before = writes(&obs);
    let err = session
        .run(
            &mut ApplyOperatingPoint { point },
            &ShutdownFlag::new(),
            Vec::new(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("scan is running"), "{err}");
    assert_eq!(writes(&obs), before, "nothing was written");
}

#[test]
fn a_z_loop_the_module_does_not_have_is_refused_before_any_write() {
    let (mut session, obs) = session();
    let mut point = changed(capture(&mut session));
    for entry in &mut point.controllers {
        if let ControllerParams::Z(z) = &mut entry.params {
            z.active = "No such loop".into();
        }
    }
    let before = writes(&obs);
    let err = session
        .run(
            &mut ApplyOperatingPoint { point },
            &ShutdownFlag::new(),
            Vec::new(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("No such loop"), "{err}");
    assert_eq!(writes(&obs), before, "nothing was written");
}
