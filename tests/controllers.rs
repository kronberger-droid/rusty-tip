//! The controller jobs against the mock: what a read reports, what an
//! apply writes and reads back, and that switching is its own job.
use std::sync::{Arc, Mutex};

use rusty_tip::ShutdownFlag;
use rusty_tip::controllers::{
    ApplyProfile, ControllerAppliedEvent, ControllerId, ControllerParams, ControllerProfile,
    ControllerReading, ProfileEntry, ReadControllers, SetControllerEnabled, SettingsLoadedEvent,
    ZControllerParams,
};
use rusty_tip::event::{Event, Observer};
use rusty_tip::experiment_log::LogEvent;
use rusty_tip::mock_controller::{MockController, models};
use rusty_tip::session::{Job, PresetFiles, Session};
use rusty_tip::signal_registry::{SignalIndex, SignalRegistry};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Event>>>);

impl Observer for Recorder {
    fn on_event(&self, event: &Event) {
        self.0.lock().unwrap().push(event.clone());
    }
}

impl Recorder {
    fn custom<E: LogEvent + serde::de::DeserializeOwned>(&self) -> Vec<E> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Custom { kind, data, .. } if kind == E::KIND => {
                    serde_json::from_value(data.clone()).ok()
                }
                _ => None,
            })
            .collect()
    }
}

fn session() -> (
    Session,
    Arc<parking_lot::Mutex<rusty_tip::mock_controller::MockObservations>>,
) {
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

fn run(session: &mut Session, job: &mut dyn Job) -> Recorder {
    let recorder = Recorder::default();
    session
        .run(job, &ShutdownFlag::new(), vec![Box::new(recorder.clone())])
        .unwrap();
    recorder
}

#[test]
fn a_read_reports_every_controller_with_its_definitions() {
    let (mut session, _) = session();
    let events = run(&mut session, &mut ReadControllers);
    let reads = events.custom::<ControllerReading>();
    let ids: Vec<ControllerId> = reads.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![
            ControllerId::Z,
            ControllerId::PllAmplitude { modulator: 1 },
            ControllerId::PllPhase { modulator: 1 },
        ]
    );
    let z = &reads[0];
    assert!(z.enabled);
    assert_eq!(z.status, "on");
    assert_eq!(z.available.len(), 9);
    assert_eq!(z.available[0], "log Current");
    assert!(z.available.contains(&"Frequency (neg)".to_string()));
    match &z.params {
        ControllerParams::Z(p) => {
            assert_eq!(p.active, "log Current");
            assert_eq!(p.input().unit(), Some("A"));
        }
        other => panic!("not Z parameters: {other:?}"),
    }
}

#[test]
fn apply_loads_the_settings_file_then_writes_and_reads_back() {
    let (mut session, obs) = session();
    let wanted = ZControllerParams {
        active: "Frequency (neg)".into(),
        setpoint: -2.0,
        p_gain_m: 5e-12,
        ..Default::default()
    };
    let mut job = ApplyProfile {
        profile: ControllerProfile {
            settings_file: Some("afm.ini".into()),
            controllers: vec![ProfileEntry {
                id: ControllerId::Z,
                params: ControllerParams::Z(wanted.clone()),
            }],
        },
    };
    let events = run(&mut session, &mut job);

    let loaded = events.custom::<SettingsLoadedEvent>();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].path.ends_with("afm.ini"));
    assert_eq!(obs.lock().settings_loaded.len(), 1, "the mock saw the load");

    let applied = events.custom::<ControllerAppliedEvent>();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].id, ControllerId::Z);
    assert_eq!(applied[0].after, ControllerParams::Z(wanted.clone()));
    assert_ne!(applied[0].before, applied[0].after);

    // The closing read shows the new values on the controller and leaves
    // the PLL loops as they were.
    let reads = events.custom::<ControllerReading>();
    assert_eq!(reads.len(), 3, "everything is read back at the end");
    assert_eq!(reads[0].params, ControllerParams::Z(wanted));
    assert!(reads[0].enabled, "applying never switches a loop");
}

#[test]
fn a_wrong_kind_or_an_unknown_controller_is_refused_before_anything_is_written() {
    let (mut session, obs) = session();
    let mut job = ApplyProfile {
        profile: ControllerProfile {
            settings_file: None,
            controllers: vec![ProfileEntry {
                id: ControllerId::Z,
                params: ControllerParams::default_for(ControllerId::PllPhase { modulator: 1 }),
            }],
        },
    };
    let err = session
        .run(&mut job, &ShutdownFlag::new(), Vec::new())
        .unwrap_err();
    assert!(err.to_string().contains("another kind"), "{err}");
    assert!(
        !obs.lock().calls.contains(&"write_controller"),
        "nothing was written"
    );

    let mut job = ApplyProfile {
        profile: ControllerProfile {
            settings_file: None,
            controllers: vec![ProfileEntry {
                id: ControllerId::Z,
                params: ControllerParams::Z(ZControllerParams {
                    active: "No such loop".into(),
                    ..Default::default()
                }),
            }],
        },
    };
    let err = session
        .run(&mut job, &ShutdownFlag::new(), Vec::new())
        .unwrap_err();
    assert!(err.to_string().contains("No such loop"), "{err}");
}

#[test]
fn switching_a_loop_is_its_own_job_and_reads_the_loop_back() {
    let (mut session, obs) = session();
    let mut job = SetControllerEnabled {
        id: ControllerId::Z,
        on: false,
    };
    let events = run(&mut session, &mut job);
    let reads = events.custom::<ControllerReading>();
    assert_eq!(reads.len(), 1);
    assert!(!reads[0].enabled);
    assert_eq!(reads[0].status, "off");
    assert!(!obs.lock().z_controller_on);
}
