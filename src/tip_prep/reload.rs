//! Handing a running tip prep a new config.
//!
//! The run takes it at the top of its next pulse cycle, never inside a
//! stability check, and writes nothing to the controller for it. What the
//! controller was set up with at the start stays as it was, so the fields
//! that describe that setup ([`FROZEN`]) are kept from the running config
//! and a change to them waits for the next run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::config::AppConfig;

/// A mailbox for a config the run should switch to, shared between the
/// run and whoever edits the config. Cloning it shares the mailbox.
#[derive(Clone, Default)]
pub struct ConfigReload {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    pending: Mutex<Option<AppConfig>>,
    live: AtomicBool,
}

impl ConfigReload {
    pub fn new() -> Self {
        Self::default()
    }

    /// Leave a config for the run. A second send before the run took the
    /// first replaces it.
    pub fn send(&self, config: AppConfig) {
        *self.lock() = Some(config);
    }

    /// The config waiting, if any, emptying the mailbox.
    pub fn take(&self) -> Option<AppConfig> {
        self.lock().take()
    }

    /// Whether a config is waiting for the run to take it.
    pub fn is_pending(&self) -> bool {
        self.lock().is_some()
    }

    /// Whether a run is reading the mailbox now.
    pub fn is_live(&self) -> bool {
        self.inner.live.load(Ordering::Acquire)
    }

    /// Mark a run as reading the mailbox, or done with it. Either way a
    /// config left untaken is dropped: it was meant for a run that ended
    /// first.
    pub fn set_live(&self, live: bool) {
        self.take();
        self.inner.live.store(live, Ordering::Release);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<AppConfig>> {
        // A panic while holding the lock leaves at worst a stale config,
        // which is still a valid one.
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The fields a run keeps from its start, by their path in the config.
///
/// The setpoint and initial bias are what the controller was set to before
/// the first approach; the landing gate judges every approach against the
/// setpoint, and the final read of a stability check returns to the bias,
/// so a new value would only describe a controller that is not there. The
/// Z preset and safe-tip threshold were applied once at the start. The
/// connection tables describe the connection the run is on.
pub const FROZEN: &[&str] = &[
    "tip_prep.initial_bias_v",
    "tip_prep.initial_z_setpoint_a",
    "tip_prep.z_controller_preset",
    "tip_prep.safe_tip_threshold",
    "nanonis",
    "data_acquisition",
    "experiment_logging",
    "console",
    "controllers",
    "tcp_channel_mapping",
];

/// `incoming` with the [`FROZEN`] fields put back as `running` has them,
/// and the paths of those that differed.
pub fn keep_frozen(running: &AppConfig, mut incoming: AppConfig) -> (AppConfig, Vec<String>) {
    let mut kept = Vec::new();
    let mut keep = |path: &str, differs: bool| {
        if differs {
            kept.push(path.to_string());
        }
    };
    let (old, new) = (&running.tip_prep, &mut incoming.tip_prep);
    keep(FROZEN[0], old.initial_bias_v != new.initial_bias_v);
    new.initial_bias_v = old.initial_bias_v;
    keep(
        FROZEN[1],
        old.initial_z_setpoint_a != new.initial_z_setpoint_a,
    );
    new.initial_z_setpoint_a = old.initial_z_setpoint_a;
    keep(
        FROZEN[2],
        old.z_controller_preset != new.z_controller_preset,
    );
    new.z_controller_preset = old.z_controller_preset.clone();
    keep(FROZEN[3], old.safe_tip_threshold != new.safe_tip_threshold);
    new.safe_tip_threshold = old.safe_tip_threshold;

    keep(FROZEN[4], differs(&running.nanonis, &incoming.nanonis));
    incoming.nanonis = running.nanonis.clone();
    keep(
        FROZEN[5],
        differs(&running.data_acquisition, &incoming.data_acquisition),
    );
    incoming.data_acquisition = running.data_acquisition.clone();
    keep(
        FROZEN[6],
        differs(&running.experiment_logging, &incoming.experiment_logging),
    );
    incoming.experiment_logging = running.experiment_logging.clone();
    keep(FROZEN[7], differs(&running.console, &incoming.console));
    incoming.console = running.console.clone();
    keep(
        FROZEN[8],
        differs(&running.controllers, &incoming.controllers),
    );
    incoming.controllers = running.controllers.clone();
    keep(
        FROZEN[9],
        differs(&running.tcp_channel_mapping, &incoming.tcp_channel_mapping),
    );
    incoming.tcp_channel_mapping = running.tcp_channel_mapping.clone();
    (incoming, kept)
}

/// Compared as JSON, since the config tables do not all derive
/// `PartialEq`.
pub(crate) fn differs<T: Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_setup_is_kept_and_the_rest_comes_through() {
        let running = AppConfig::default();
        let mut incoming = AppConfig::default();
        incoming.tip_prep.sharp_tip_bounds = [-9.0, -1.0];
        incoming.tip_prep.max_cycles = Some(7);
        incoming.tip_prep.initial_z_setpoint_a = 1e-9;
        incoming.nanonis.host_ip = "10.0.0.1".into();

        let (applied, kept) = keep_frozen(&running, incoming);
        assert_eq!(applied.tip_prep.sharp_tip_bounds, [-9.0, -1.0]);
        assert_eq!(applied.tip_prep.max_cycles, Some(7));
        assert_eq!(
            applied.tip_prep.initial_z_setpoint_a,
            running.tip_prep.initial_z_setpoint_a
        );
        assert_eq!(applied.nanonis.host_ip, running.nanonis.host_ip);
        assert_eq!(kept, vec!["tip_prep.initial_z_setpoint_a", "nanonis"]);
    }

    #[test]
    fn a_new_run_drops_what_the_last_left() {
        let reload = ConfigReload::new();
        reload.send(AppConfig::default());
        reload.set_live(true);
        assert!(reload.is_live());
        assert!(reload.take().is_none());
        reload.send(AppConfig::default());
        reload.set_live(false);
        assert!(!reload.is_live());
        assert!(
            !reload.is_pending(),
            "a run that ended leaves nothing waiting"
        );
    }
}
