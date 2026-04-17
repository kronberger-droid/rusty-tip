use std::time::Instant;

use crate::action::{
    Action, ActionContext, ActionOutput, ActionRegistry, DataStore,
};
use crate::event::{Event, EventBus, EventEmitter, Observer};
use crate::machine_state::{MachineState, StateField, ValidationPolicy};
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

use super::{
    CompareOp, Condition, ShutdownFlag, Step, StepOutcome, Workflow,
    WorkflowOutcome,
};

/// Executes workflows by walking the Step tree.
///
/// Owns the controller, event bus, data store, and action registry.
/// Single-threaded and recursive -- the Rust call stack mirrors the
/// workflow nesting for predictable execution and clear backtraces.
pub struct WorkflowExecutor {
    registry: ActionRegistry,
    controller: Box<dyn SpmController>,
    events: EventBus,
    store: DataStore,
    shutdown: ShutdownFlag,
    state: MachineState,
    policy: ValidationPolicy,
}

impl WorkflowExecutor {
    pub fn new(
        registry: ActionRegistry,
        controller: Box<dyn SpmController>,
    ) -> Self {
        Self {
            registry,
            controller,
            events: EventBus::new(),
            store: DataStore::new(),
            shutdown: ShutdownFlag::new(),
            state: MachineState::uninitialized(),
            policy: ValidationPolicy::default(),
        }
    }

    pub fn add_observer(&mut self, observer: Box<dyn Observer>) {
        self.events.add_observer(observer);
    }

    pub fn shutdown_flag(&self) -> &ShutdownFlag {
        &self.shutdown
    }

    pub fn set_shutdown_flag(&mut self, flag: ShutdownFlag) {
        self.shutdown = flag;
    }

    pub fn store(&self) -> &DataStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DataStore {
        &mut self.store
    }

    /// Current software model of the machine state.
    pub fn state(&self) -> &MachineState {
        &self.state
    }

    /// Pre-seed the state model (e.g. from a known startup configuration).
    pub fn seed_state(&mut self, state: MachineState) {
        self.state = state;
    }

    /// Current enforcement policy for action preconditions.
    pub fn policy(&self) -> ValidationPolicy {
        self.policy
    }

    /// Change how the executor consults `MachineState`. Defaults to
    /// `Disabled` (no state checking) for backwards compatibility with
    /// workflows that predate the state-tracking feature.
    pub fn set_policy(&mut self, policy: ValidationPolicy) {
        self.policy = policy;
    }

    /// Execute a complete workflow.
    pub fn run(
        &mut self,
        workflow: &Workflow,
    ) -> Result<WorkflowOutcome, SpmError> {
        self.events.emit(Event::custom(
            "workflow_started",
            serde_json::json!({ "name": &workflow.name }),
        ));

        let result = self.execute_step(&Step::Sequence {
            steps: workflow.steps.clone(),
            label: Some(workflow.name.clone()),
        });

        match result {
            Ok(StepOutcome::Completed(_)) => {
                self.events.emit(Event::custom(
                    "workflow_completed",
                    serde_json::json!({ "name": &workflow.name }),
                ));
                Ok(WorkflowOutcome::Completed)
            }
            Ok(StepOutcome::Shutdown) => {
                self.events.emit(Event::custom(
                    "workflow_stopped",
                    serde_json::json!({ "name": &workflow.name, "reason": "shutdown" }),
                ));
                Ok(WorkflowOutcome::StoppedByUser)
            }
            Ok(StepOutcome::CycleLimit { label, max }) => {
                self.events.emit(Event::custom(
                    "workflow_cycle_limit",
                    serde_json::json!({
                        "name": &workflow.name,
                        "loop_label": label,
                        "max_iterations": max,
                    }),
                ));
                Ok(WorkflowOutcome::CycleLimit { label, max })
            }
            Err(e) => {
                self.events.emit(Event::custom(
                    "workflow_failed",
                    serde_json::json!({ "name": &workflow.name, "error": e.to_string() }),
                ));
                Err(e)
            }
        }
    }

    /// Execute a single step (recursive).
    fn execute_step(&mut self, step: &Step) -> Result<StepOutcome, SpmError> {
        // Check shutdown between every step
        if self.shutdown.is_requested() {
            return Ok(StepOutcome::Shutdown);
        }

        match step {
            Step::Do {
                action,
                params,
                store_as,
                ..
            } => self.execute_do(action, params, store_as.as_deref()),

            Step::Sequence { steps, .. } => {
                let mut last_output = ActionOutput::Unit;
                for step in steps {
                    match self.execute_step(step)? {
                        StepOutcome::Completed(output) => last_output = output,
                        StepOutcome::Shutdown => {
                            return Ok(StepOutcome::Shutdown);
                        }
                        outcome @ StepOutcome::CycleLimit { .. } => {
                            return Ok(outcome);
                        }
                    }
                }
                Ok(StepOutcome::Completed(last_output))
            }

            Step::If {
                condition,
                then,
                otherwise,
            } => {
                if self.evaluate_condition(condition)? {
                    self.execute_step(then)
                } else if let Some(alt) = otherwise {
                    self.execute_step(alt)
                } else {
                    Ok(StepOutcome::Completed(ActionOutput::Unit))
                }
            }

            Step::Loop {
                body,
                until,
                max_iterations,
                label,
            } => {
                for _i in 0..*max_iterations {
                    match self.execute_step(body)? {
                        StepOutcome::Completed(_) => {}
                        outcome @ StepOutcome::CycleLimit { .. } => {
                            // Only propagate inner CycleLimit if this loop has
                            // an exit condition. Plain count-loops (until: None)
                            // treat inner CycleLimit as completed — the inner
                            // loop simply exhausted its iterations for this
                            // outer iteration.
                            if until.is_some() {
                                return Ok(outcome);
                            }
                        }
                        StepOutcome::Shutdown => {
                            return Ok(StepOutcome::Shutdown);
                        }
                    }
                    if let Some(condition) = until
                        && self.evaluate_condition(condition)?
                    {
                        return Ok(StepOutcome::Completed(ActionOutput::Unit));
                    }
                }
                // If an exit condition was set and never met, report CycleLimit
                if until.is_some() {
                    return Ok(StepOutcome::CycleLimit {
                        label: label.clone(),
                        max: *max_iterations,
                    });
                }
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }

            Step::SetVar { key, value } => {
                self.store.set(key, value)?;
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }

            Step::Wait { duration_ms } => {
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(*duration_ms);
                let poll = std::time::Duration::from_millis(50);
                while std::time::Instant::now() < deadline {
                    if self.shutdown.is_requested() {
                        return Ok(StepOutcome::Shutdown);
                    }
                    let remaining = deadline
                        .saturating_duration_since(std::time::Instant::now());
                    std::thread::sleep(poll.min(remaining));
                }
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }
        }
    }

    /// Execute a Do step: create the action, emit events, run it, store result.
    fn execute_do(
        &mut self,
        action_name: &str,
        params: &serde_json::Value,
        store_as: Option<&str>,
    ) -> Result<StepOutcome, SpmError> {
        let action = self.registry.create(action_name, params.clone())?;

        // Check required capabilities before running
        let required = action.requires();
        if !required.is_empty() {
            let caps = self.controller.capabilities();
            for cap in &required {
                if !caps.contains(cap) {
                    return Err(SpmError::Unsupported(format!(
                        "Action '{}' requires {:?}, which the controller does not support",
                        action.name(),
                        cap,
                    )));
                }
            }
        }

        self.enforce_preconditions(&*action)?;

        self.events
            .emit(Event::action_started(action.name(), params.clone()));

        let start = Instant::now();
        let mut ctx = self.make_ctx();

        match action.execute(&mut ctx) {
            Ok(output) => {
                let duration = start.elapsed();
                self.events.emit(Event::action_completed(
                    action.name(),
                    &output,
                    duration,
                ));

                if self.policy != ValidationPolicy::Disabled {
                    action.apply_to_state(&output, &mut self.state);
                }

                if let Some(key) = store_as {
                    self.store.set(key, &output)?;
                }

                Ok(StepOutcome::Completed(output))
            }
            Err(e) => {
                let duration = start.elapsed();
                self.events.emit(Event::action_failed(
                    action.name(),
                    &e.to_string(),
                    duration,
                ));

                if self.policy != ValidationPolicy::Disabled {
                    action.effects().degrade(&mut self.state);
                }

                // A partial write may have left the hardware in an
                // indeterminate state; after reconnect, degrade every Known
                // field so downstream actions force a fresh query before
                // trusting the state model.
                if e.is_connection_error() && !self.controller.is_connected() {
                    log::warn!(
                        "Connection lost during '{}', attempting reconnect...",
                        action.name()
                    );
                    match self.controller.reconnect() {
                        Ok(()) => {
                            log::info!(
                                "Reconnected successfully after '{}' failure",
                                action.name()
                            );
                            if self.policy != ValidationPolicy::Disabled {
                                self.state.degrade_all();
                            }
                            self.events.emit(Event::custom(
                                "connection_restored",
                                serde_json::json!({
                                    "after_action": action.name(),
                                }),
                            ));
                        }
                        Err(re) => {
                            log::error!("Reconnect failed: {re}");
                            self.events.emit(Event::custom(
                                "reconnect_failed",
                                serde_json::json!({
                                    "after_action": action.name(),
                                    "error": re.to_string(),
                                }),
                            ));
                        }
                    }
                }

                Err(e)
            }
        }
    }

    fn make_ctx(&mut self) -> ActionContext<'_> {
        ActionContext {
            controller: &mut *self.controller,
            store: &mut self.store,
            events: &self.events,
        }
    }

    /// In Strict mode, auto-insert Query resolvers for any required field
    /// that is Unknown/Uninitialized, then recheck. Advisory logs and
    /// proceeds. Disabled is a no-op.
    fn enforce_preconditions(
        &mut self,
        action: &dyn Action,
    ) -> Result<(), SpmError> {
        if self.policy == ValidationPolicy::Disabled {
            return Ok(());
        }

        let expects = action.expects();
        if expects.is_empty() {
            return Ok(());
        }

        let mut violations = expects.check(&self.state);
        if violations.is_empty() {
            return Ok(());
        }

        if self.policy == ValidationPolicy::Strict {
            let unresolved: Vec<StateField> = expects
                .required_fields()
                .into_iter()
                .filter(|f| self.state.needs_resolution(*f))
                .collect();

            if !unresolved.is_empty() {
                for field in unresolved {
                    self.run_resolver(field);
                }
                violations = expects.check(&self.state);
                if violations.is_empty() {
                    return Ok(());
                }
            }
        }

        let msg = format!(
            "Action '{}' preconditions not satisfied: {}",
            action.name(),
            violations
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );

        if self.policy == ValidationPolicy::Strict {
            Err(SpmError::Workflow(msg))
        } else {
            log::warn!("{msg} (advisory — proceeding)");
            Ok(())
        }
    }

    /// Run the registered Query resolver for a state field, if any.
    /// A failed resolver falls through to the normal check, which will
    /// surface the still-unresolved field as a proper violation.
    fn run_resolver(&mut self, field: StateField) {
        let Some(resolver) = self.registry.resolver_for(field) else {
            log::debug!("No resolver registered for {:?}", field);
            return;
        };

        log::info!("Auto-resolving {:?} via '{}'", field, resolver.name());

        let mut ctx = self.make_ctx();
        match resolver.execute(&mut ctx) {
            Ok(output) => {
                resolver.apply_to_state(&output, &mut self.state);
            }
            Err(e) => {
                log::warn!(
                    "Resolver '{}' for {:?} failed: {e}",
                    resolver.name(),
                    field
                );
            }
        }
    }

    /// Evaluate a condition against the current DataStore.
    fn evaluate_condition(
        &self,
        condition: &Condition,
    ) -> Result<bool, SpmError> {
        match condition {
            Condition::Compare {
                variable,
                operator,
                threshold,
                tolerance,
            } => {
                let value: f64 = self.resolve_variable(variable)?;
                Ok(match operator {
                    CompareOp::Lt => value < *threshold,
                    CompareOp::Le => value <= *threshold,
                    CompareOp::Eq => (value - threshold).abs() <= *tolerance,
                    CompareOp::Ge => value >= *threshold,
                    CompareOp::Gt => value > *threshold,
                    CompareOp::Ne => (value - threshold).abs() > *tolerance,
                })
            }

            Condition::InRange { variable, min, max } => {
                let value: f64 = self.resolve_variable(variable)?;
                Ok(value >= *min && value <= *max)
            }

            Condition::And { conditions } => {
                for c in conditions {
                    if !self.evaluate_condition(c)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            Condition::Or { conditions } => {
                for c in conditions {
                    if self.evaluate_condition(c)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            Condition::Not { condition } => {
                Ok(!self.evaluate_condition(condition)?)
            }
        }
    }

    /// Resolve a variable name to an f64 from the DataStore.
    ///
    /// Handles both raw f64 values and `ActionOutput::Value` stored by actions.
    fn resolve_variable(&self, name: &str) -> Result<f64, SpmError> {
        let json = self.store.get_raw(name).ok_or_else(|| {
            SpmError::Workflow(format!(
                "Variable '{}' not found in store",
                name
            ))
        })?;

        // Try direct f64
        if let Some(v) = json.as_f64() {
            return Ok(v);
        }

        // Try deserializing as ActionOutput
        if let Ok(output) = serde_json::from_value::<ActionOutput>(json.clone())
        {
            match output {
                ActionOutput::Value(v) => return Ok(v),
                ActionOutput::Values(_) => {
                    return Err(SpmError::Workflow(format!(
                        "Variable '{}' holds a Values array, not a single number. \
                         Use a specific signal key instead.",
                        name,
                    )));
                }
                ActionOutput::Data(_) => {
                    return Err(SpmError::Workflow(format!(
                        "Variable '{}' holds structured Data, not a numeric value",
                        name,
                    )));
                }
                ActionOutput::Unit => {
                    return Err(SpmError::Workflow(format!(
                        "Variable '{}' holds Unit (no value) -- the action produced no output",
                        name,
                    )));
                }
            }
        }

        Err(SpmError::Workflow(format!(
            "Variable '{}' is not a numeric value: {}",
            name, json
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionContext, ActionOutput, ActionRegistry};
    use crate::event::ChannelForwarder;
    use crate::spm_controller::{
        AcquisitionMode, Capability, DataStreamStatus, SpmController,
        TriggerSetup,
    };
    use crate::workflow::{
        CompareOp, Condition, Step, Workflow, WorkflowOutcome,
    };
    use nanonis_rs::oscilloscope::OsciData;
    use nanonis_rs::scan::{
        ScanAction, ScanConfig, ScanDirection, ScanProps, ScanPropsBuilder,
    };
    use nanonis_rs::tip_recovery::TipShaperConfig;
    use nanonis_rs::{Position, motor::*};
    use serde::{Deserialize, Serialize};
    use std::collections::HashSet;
    use std::f64;
    use std::time::Duration;

    // ── Mock controller ────────────────────────────────────────────

    struct MockController {
        bias: f64,
    }

    impl MockController {
        fn new() -> Self {
            Self { bias: 0.0 }
        }
    }

    impl SpmController for MockController {
        fn capabilities(&self) -> HashSet<Capability> {
            [Capability::Bias, Capability::Signals].into()
        }

        fn read_signal(
            &mut self,
            index: u32,
            _wait: bool,
        ) -> crate::spm_controller::Result<f64> {
            Ok(index as f64 * 0.1)
        }
        fn read_signals(
            &mut self,
            indices: &[u32],
            _wait: bool,
        ) -> crate::spm_controller::Result<Vec<f64>> {
            Ok(indices.iter().map(|&i| i as f64 * 0.1).collect())
        }
        fn signal_names(
            &mut self,
        ) -> crate::spm_controller::Result<Vec<String>> {
            Ok(vec!["Z".into(), "Current".into()])
        }
        fn get_bias(&mut self) -> crate::spm_controller::Result<f64> {
            Ok(self.bias)
        }
        fn set_bias(
            &mut self,
            voltage: f64,
        ) -> crate::spm_controller::Result<()> {
            self.bias = voltage;
            Ok(())
        }
        fn bias_pulse(
            &mut self,
            _v: f64,
            _w: Duration,
            _z: bool,
            _a: bool,
        ) -> crate::spm_controller::Result<()> {
            Ok(())
        }
        fn withdraw(
            &mut self,
            _w: bool,
            _t: Duration,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn auto_approach(
            &mut self,
            _w: bool,
            _t: Duration,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn set_z_setpoint(
            &mut self,
            _s: f64,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn set_z_home(
            &mut self,
            _m: nanonis_rs::z_ctrl::ZHomeMode,
            _p: f64,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn go_z_home(&mut self) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn z_controller_status(
            &mut self,
        ) -> crate::spm_controller::Result<
            crate::spm_controller::ZControllerStatus,
        > {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn get_position(
            &mut self,
            _w: bool,
        ) -> crate::spm_controller::Result<Position> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn set_position(
            &mut self,
            _p: Position,
            _w: bool,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn move_motor(
            &mut self,
            _d: MotorDirection,
            _s: u16,
            _w: bool,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn move_motor_3d(
            &mut self,
            _d: MotorDisplacement,
            _w: bool,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn move_motor_closed_loop(
            &mut self,
            _t: Position3D,
            _m: MovementMode,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn stop_motor(&mut self) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_action(
            &mut self,
            _a: ScanAction,
            _d: ScanDirection,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_status(&mut self) -> crate::spm_controller::Result<bool> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_props_get(
            &mut self,
        ) -> crate::spm_controller::Result<ScanProps> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_props_set(
            &mut self,
            _p: ScanPropsBuilder,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_speed_get(
            &mut self,
        ) -> crate::spm_controller::Result<ScanConfig> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_speed_set(
            &mut self,
            _c: ScanConfig,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn scan_frame_data_grab(
            &mut self,
            _c: u32,
            _f: bool,
        ) -> crate::spm_controller::Result<(String, Vec<Vec<f32>>, bool)>
        {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn osci_read(
            &mut self,
            _c: i32,
            _t: Option<&TriggerSetup>,
            _m: AcquisitionMode,
        ) -> crate::spm_controller::Result<OsciData> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn tip_shaper(
            &mut self,
            _c: &TipShaperConfig,
            _w: bool,
            _t: Duration,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn pll_center_freq_shift(
            &mut self,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn safe_tip_configure(
            &mut self,
            _a: bool,
            _p: bool,
            _t: f64,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn safe_tip_status(
            &mut self,
        ) -> crate::spm_controller::Result<(bool, bool, f64)> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn safe_tip_set_enabled(
            &mut self,
            _e: bool,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn safe_tip_enabled(&mut self) -> crate::spm_controller::Result<bool> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn data_stream_configure(
            &mut self,
            _c: &[i32],
            _o: i32,
        ) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn data_stream_start(&mut self) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn data_stream_stop(&mut self) -> crate::spm_controller::Result<()> {
            Err(SpmError::Unsupported("mock".into()))
        }
        fn data_stream_status(
            &mut self,
        ) -> crate::spm_controller::Result<DataStreamStatus> {
            Err(SpmError::Unsupported("mock".into()))
        }
    }

    // ── Test action for the registry ───────────────────────────────

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    struct IncrementAction;

    impl Action for IncrementAction {
        fn name(&self) -> &str {
            "increment"
        }
        fn description(&self) -> &str {
            "Increment 'counter' in the store"
        }
        fn execute(
            &self,
            ctx: &mut ActionContext,
        ) -> std::result::Result<ActionOutput, SpmError> {
            let current: f64 = ctx.store.get("counter").unwrap_or(0.0);
            let next = current + 1.0;
            ctx.store.set("counter", &next)?;
            Ok(ActionOutput::Value(next))
        }
    }

    // ── Helper ─────────────────────────────────────────────────────

    fn make_executor() -> WorkflowExecutor {
        let mut reg = ActionRegistry::new();
        reg.register::<IncrementAction>();
        reg.register::<crate::action::bias::ReadBias>();
        reg.register::<crate::action::bias::SetBias>();
        reg.register::<crate::action::util::Wait>();
        WorkflowExecutor::new(reg, Box::new(MockController::new()))
    }

    // ── Basic execution ────────────────────────────────────────────

    #[test]
    fn run_empty_workflow() {
        let mut exec = make_executor();
        let wf = Workflow::new("empty", "Does nothing");
        let result = exec.run(&wf).unwrap();
        assert!(matches!(result, WorkflowOutcome::Completed));
    }

    #[test]
    fn run_single_do_step() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "Read bias")
            .step(Step::action("read_bias", serde_json::Value::Null));
        let result = exec.run(&wf).unwrap();
        assert!(matches!(result, WorkflowOutcome::Completed));
        // Without store_as, result should NOT be stored
        assert!(!exec.store().contains("read_bias"));
    }

    #[test]
    fn do_step_stores_result_under_custom_key() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::action_store(
            "read_bias",
            serde_json::Value::Null,
            "initial_bias",
        ));
        exec.run(&wf).unwrap();
        assert!(exec.store().contains("initial_bias"));
    }

    #[test]
    fn do_step_unknown_action_fails() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "")
            .step(Step::action("nonexistent", serde_json::Value::Null));
        let result = exec.run(&wf);
        assert!(result.is_err());
    }

    // ── Sequence ───────────────────────────────────────────────────

    #[test]
    fn sequence_runs_all_steps() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::sequence(vec![
            Step::action("increment", serde_json::Value::Null),
            Step::action("increment", serde_json::Value::Null),
            Step::action("increment", serde_json::Value::Null),
        ]));
        exec.run(&wf).unwrap();
        let counter: f64 = exec.store().get("counter").unwrap();
        assert!((counter - 3.0).abs() < 1e-10);
    }

    #[test]
    fn sequence_stops_on_error() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::sequence(vec![
            Step::action("increment", serde_json::Value::Null),
            Step::action("nonexistent", serde_json::Value::Null),
            Step::action("increment", serde_json::Value::Null), // should not run
        ]));
        assert!(exec.run(&wf).is_err());
        let counter: f64 = exec.store().get("counter").unwrap();
        assert!(
            (counter - 1.0).abs() < 1e-10,
            "Only first increment should have run"
        );
    }

    // ── SetVar ─────────────────────────────────────────────────────

    #[test]
    fn setvar_stores_literal() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::SetVar {
            key: "threshold".into(),
            value: serde_json::json!(0.5),
        });
        exec.run(&wf).unwrap();
        let val: f64 = exec.store().get("threshold").unwrap();
        assert!((val - 0.5).abs() < 1e-10);
    }

    // ── Conditionals ───────────────────────────────────────────────

    #[test]
    fn if_true_branch() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();
        let wf = Workflow::new("test", "").step(Step::If {
            condition: Condition::Compare {
                variable: "x".into(),
                operator: CompareOp::Gt,
                threshold: 3.0,
                tolerance: 1e-9,
            },
            then: Box::new(Step::SetVar {
                key: "result".into(),
                value: serde_json::json!("big"),
            }),
            otherwise: Some(Box::new(Step::SetVar {
                key: "result".into(),
                value: serde_json::json!("small"),
            })),
        });
        exec.run(&wf).unwrap();
        let result: String = exec.store().get("result").unwrap();
        assert_eq!(result, "big");
    }

    #[test]
    fn if_false_branch() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &1.0f64).unwrap();
        let wf = Workflow::new("test", "").step(Step::If {
            condition: Condition::Compare {
                variable: "x".into(),
                operator: CompareOp::Gt,
                threshold: 3.0,
                tolerance: 1e-9,
            },
            then: Box::new(Step::SetVar {
                key: "result".into(),
                value: serde_json::json!("big"),
            }),
            otherwise: Some(Box::new(Step::SetVar {
                key: "result".into(),
                value: serde_json::json!("small"),
            })),
        });
        exec.run(&wf).unwrap();
        let result: String = exec.store().get("result").unwrap();
        assert_eq!(result, "small");
    }

    #[test]
    fn if_no_otherwise_does_nothing() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &1.0f64).unwrap();
        let wf = Workflow::new("test", "").step(Step::If {
            condition: Condition::Compare {
                variable: "x".into(),
                operator: CompareOp::Gt,
                threshold: 99.0,
                tolerance: 1e-9,
            },
            then: Box::new(Step::SetVar {
                key: "result".into(),
                value: serde_json::json!("ran"),
            }),
            otherwise: None,
        });
        exec.run(&wf).unwrap();
        assert!(!exec.store().contains("result"));
    }

    // ── Conditions ─────────────────────────────────────────────────

    #[test]
    fn condition_compare_all_operators() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();

        let test = |op: CompareOp, thresh: f64| -> bool {
            let cond = Condition::Compare {
                variable: "x".into(),
                operator: op,
                threshold: thresh,
                tolerance: 1e-9,
            };
            exec.evaluate_condition(&cond).unwrap()
        };

        assert!(test(CompareOp::Lt, 10.0));
        assert!(!test(CompareOp::Lt, 3.0));
        assert!(test(CompareOp::Le, 5.0));
        assert!(test(CompareOp::Eq, 5.0));
        assert!(!test(CompareOp::Eq, 5.1));
        assert!(test(CompareOp::Ge, 5.0));
        assert!(test(CompareOp::Gt, 3.0));
        assert!(!test(CompareOp::Gt, 5.0));
        assert!(test(CompareOp::Ne, 3.0));
        assert!(!test(CompareOp::Ne, 5.0));
    }

    #[test]
    fn condition_in_range() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();

        let in_range = Condition::InRange {
            variable: "x".into(),
            min: 3.0,
            max: 7.0,
        };
        assert!(exec.evaluate_condition(&in_range).unwrap());

        let out_of_range = Condition::InRange {
            variable: "x".into(),
            min: 6.0,
            max: 10.0,
        };
        assert!(!exec.evaluate_condition(&out_of_range).unwrap());
    }

    #[test]
    fn condition_and() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();

        let both_true = Condition::And {
            conditions: vec![
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Gt,
                    threshold: 3.0,
                    tolerance: 1e-9,
                },
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Lt,
                    threshold: 10.0,
                    tolerance: 1e-9,
                },
            ],
        };
        assert!(exec.evaluate_condition(&both_true).unwrap());

        let one_false = Condition::And {
            conditions: vec![
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Gt,
                    threshold: 3.0,
                    tolerance: 1e-9,
                },
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Lt,
                    threshold: 2.0,
                    tolerance: 1e-9,
                },
            ],
        };
        assert!(!exec.evaluate_condition(&one_false).unwrap());
    }

    #[test]
    fn condition_or() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();

        let one_true = Condition::Or {
            conditions: vec![
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Gt,
                    threshold: 99.0,
                    tolerance: 1e-9,
                },
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Lt,
                    threshold: 10.0,
                    tolerance: 1e-9,
                },
            ],
        };
        assert!(exec.evaluate_condition(&one_true).unwrap());

        let both_false = Condition::Or {
            conditions: vec![
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Gt,
                    threshold: 99.0,
                    tolerance: 1e-9,
                },
                Condition::Compare {
                    variable: "x".into(),
                    operator: CompareOp::Lt,
                    threshold: 1.0,
                    tolerance: 1e-9,
                },
            ],
        };
        assert!(!exec.evaluate_condition(&both_false).unwrap());
    }

    #[test]
    fn condition_not() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &5.0f64).unwrap();

        let neg = Condition::Not {
            condition: Box::new(Condition::Compare {
                variable: "x".into(),
                operator: CompareOp::Gt,
                threshold: 10.0,
                tolerance: 1e-9,
            }),
        };
        assert!(exec.evaluate_condition(&neg).unwrap());
    }

    #[test]
    fn condition_missing_variable_fails() {
        let exec = make_executor();
        let cond = Condition::Compare {
            variable: "nonexistent".into(),
            operator: CompareOp::Gt,
            threshold: 0.0,
            tolerance: 1e-9,
        };
        assert!(exec.evaluate_condition(&cond).is_err());
    }

    // ── Loops ──────────────────────────────────────────────────────

    #[test]
    fn loop_runs_to_max_iterations() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::repeat(
            Step::action("increment", serde_json::Value::Null),
            5,
        ));
        exec.run(&wf).unwrap();
        let counter: f64 = exec.store().get("counter").unwrap();
        assert!((counter - 5.0).abs() < 1e-10);
    }

    #[test]
    fn loop_exits_on_condition() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::repeat_until(
            Step::action("increment", serde_json::Value::Null),
            Condition::Compare {
                variable: "counter".into(),
                operator: CompareOp::Ge,
                threshold: 3.0,
                tolerance: 1e-9,
            },
            100,
        ));
        exec.run(&wf).unwrap();
        let counter: f64 = exec.store().get("counter").unwrap();
        assert!((counter - 3.0).abs() < 1e-10);
    }

    // ── Shutdown ───────────────────────────────────────────────────

    #[test]
    fn shutdown_stops_workflow() {
        let mut exec = make_executor();
        exec.shutdown_flag().request();
        let wf = Workflow::new("test", "")
            .step(Step::action("increment", serde_json::Value::Null));
        let result = exec.run(&wf).unwrap();
        assert!(matches!(result, WorkflowOutcome::StoppedByUser));
        // The action should not have run
        assert!(!exec.store().contains("counter"));
    }

    #[test]
    fn shutdown_mid_sequence() {
        let flag = ShutdownFlag::new();
        let mut exec = make_executor();
        exec.set_shutdown_flag(flag.clone());

        // Pre-set counter so increment will set it to 1, then we request shutdown
        // We can't easily test mid-sequence shutdown without threads,
        // but we can test that the flag is checked between steps
        // by requesting shutdown and checking it stops
        let wf = Workflow::new("test", "").step(Step::sequence(vec![
            Step::action("increment", serde_json::Value::Null),
            Step::action("increment", serde_json::Value::Null),
        ]));

        // Request shutdown before running
        flag.request();
        let result = exec.run(&wf).unwrap();
        assert!(matches!(result, WorkflowOutcome::StoppedByUser));
    }

    // ── Events ─────────────────────────────────────────────────────

    fn collect_events(rx: &crossbeam_channel::Receiver<Event>) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        events
    }

    #[test]
    fn executor_emits_workflow_events() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut exec = make_executor();
        exec.add_observer(Box::new(ChannelForwarder::new(tx)));

        let wf = Workflow::new("test_wf", "")
            .step(Step::action("read_bias", serde_json::Value::Null));
        exec.run(&wf).unwrap();

        let events = collect_events(&rx);
        // Should have: workflow_started, action_started, action_completed, workflow_completed
        assert!(
            events.len() >= 4,
            "Expected at least 4 events, got {}",
            events.len()
        );

        // Check workflow_started is first
        match &events[0] {
            Event::Custom { kind, data } => {
                assert_eq!(kind, "workflow_started");
                assert_eq!(data["name"], "test_wf");
            }
            _ => panic!("First event should be workflow_started"),
        }

        // Check workflow_completed is last
        match events.last().unwrap() {
            Event::Custom { kind, .. } => {
                assert_eq!(kind, "workflow_completed")
            }
            _ => panic!("Last event should be workflow_completed"),
        }
    }

    #[test]
    fn executor_emits_action_events() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut exec = make_executor();
        exec.add_observer(Box::new(ChannelForwarder::new(tx)));

        let wf = Workflow::new("test", "")
            .step(Step::action("read_bias", serde_json::Value::Null));
        exec.run(&wf).unwrap();

        let events = collect_events(&rx);
        let has_action_started = events.iter().any(|e| matches!(e, Event::ActionStarted { action, .. } if action == "read_bias"));
        let has_action_completed = events.iter().any(|e| matches!(e, Event::ActionCompleted { action, .. } if action == "read_bias"));
        assert!(has_action_started, "Should emit ActionStarted");
        assert!(has_action_completed, "Should emit ActionCompleted");
    }

    #[test]
    fn executor_emits_failure_event() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut exec = make_executor();
        exec.add_observer(Box::new(ChannelForwarder::new(tx)));

        let wf = Workflow::new("test", "")
            .step(Step::action("nonexistent", serde_json::Value::Null));
        let _ = exec.run(&wf); // will fail

        let events = collect_events(&rx);
        let has_workflow_failed = events.iter().any(|e| matches!(e, Event::Custom { kind, .. } if kind == "workflow_failed"));
        assert!(has_workflow_failed, "Should emit workflow_failed event");
    }

    // ── Resolve variable ───────────────────────────────────────────

    #[test]
    fn resolve_raw_f64() {
        let mut exec = make_executor();
        exec.store_mut().set("x", &f64::consts::PI).unwrap();
        let val = exec.resolve_variable("x").unwrap();
        assert!((val - f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn resolve_action_output_value() {
        let mut exec = make_executor();
        // ActionOutput::Value serializes as {"type":"value","data":2.72}
        let output = ActionOutput::Value(2.72);
        exec.store_mut().set("x", &output).unwrap();
        let val = exec.resolve_variable("x").unwrap();
        assert!((val - 2.72).abs() < 1e-10);
    }

    #[test]
    fn resolve_non_numeric_fails() {
        let mut exec = make_executor();
        exec.store_mut()
            .set("x", &"not a number".to_string())
            .unwrap();
        assert!(exec.resolve_variable("x").is_err());
    }

    // ── Workflow serialization ─────────────────────────────────────

    #[test]
    fn workflow_roundtrips_through_json() {
        let wf = Workflow::new("test", "A test workflow")
            .step(Step::action("read_bias", serde_json::Value::Null))
            .step(Step::repeat_until(
                Step::action("increment", serde_json::Value::Null),
                Condition::Compare {
                    variable: "counter".into(),
                    operator: CompareOp::Ge,
                    threshold: 10.0,
                    tolerance: 1e-9,
                },
                100,
            ));

        let json = serde_json::to_string(&wf).unwrap();
        let wf2: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(wf2.name, "test");
        assert_eq!(wf2.steps.len(), 2);
    }

    #[test]
    fn step_convenience_constructors() {
        let action = Step::action("read_bias", serde_json::Value::Null);
        assert!(
            matches!(action, Step::Do { action, .. } if action == "read_bias")
        );

        let action_store =
            Step::action_store("read_bias", serde_json::json!({}), "bias");
        assert!(
            matches!(action_store, Step::Do { store_as: Some(key), .. } if key == "bias")
        );

        let seq = Step::sequence(vec![]);
        assert!(matches!(seq, Step::Sequence { .. }));

        let repeat = Step::repeat(Step::wait(10), 5);
        assert!(matches!(
            repeat,
            Step::Loop {
                max_iterations: 5,
                ..
            }
        ));

        let wait = Step::wait(100);
        assert!(matches!(wait, Step::Wait { duration_ms: 100 }));
    }

    // ── Wait step ──────────────────────────────────────────────────

    #[test]
    fn wait_step_completes() {
        let mut exec = make_executor();
        let wf = Workflow::new("test", "").step(Step::Wait { duration_ms: 1 }); // 1ms
        let start = std::time::Instant::now();
        exec.run(&wf).unwrap();
        assert!(start.elapsed().as_millis() >= 1);
    }

    // ── Integration: set bias then read it back ────────────────────

    #[test]
    fn set_and_read_bias_workflow() {
        let mut exec = make_executor();
        let wf = Workflow::new("bias_test", "Set bias then read it")
            .step(Step::action(
                "set_bias",
                serde_json::json!({"voltage": 1.5}),
            ))
            .step(Step::action_store(
                "read_bias",
                serde_json::Value::Null,
                "final_bias",
            ));
        exec.run(&wf).unwrap();

        // The result is stored by the executor via DataStore::set with ActionOutput.
        // Verify the key exists
        assert!(exec.store().contains("final_bias"));
    }

    // ── MachineState enforcement ───────────────────────────────────

    use crate::machine_state::{MachineState, TipEngagement, ValidationPolicy};

    /// An action that expects the tip to be Approached and declares no capability
    /// requirements so our minimal MockController can run it.
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    struct ApproachedOnly;

    impl Action for ApproachedOnly {
        fn name(&self) -> &str {
            "approached_only"
        }
        fn description(&self) -> &str {
            "test action that requires approached tip"
        }
        fn execute(
            &self,
            _ctx: &mut ActionContext,
        ) -> Result<ActionOutput, SpmError> {
            Ok(ActionOutput::Unit)
        }
        fn expects(&self) -> crate::machine_state::StateRequirements {
            crate::machine_state::StateRequirements::none()
                .tip(TipEngagement::Approached)
        }
    }

    fn exec_with_policy(policy: ValidationPolicy) -> WorkflowExecutor {
        let mut reg = ActionRegistry::new();
        reg.register::<ApproachedOnly>();
        let mut exec =
            WorkflowExecutor::new(reg, Box::new(MockController::new()));
        exec.set_policy(policy);
        exec
    }

    fn approached_only_wf() -> Workflow {
        Workflow::new("t", "")
            .step(Step::action("approached_only", serde_json::Value::Null))
    }

    #[test]
    fn disabled_policy_skips_preconditions() {
        let mut exec = exec_with_policy(ValidationPolicy::Disabled);
        assert_eq!(exec.policy(), ValidationPolicy::Disabled);
        exec.run(&approached_only_wf()).unwrap();
    }

    #[test]
    fn strict_policy_blocks_when_precondition_unmet() {
        let mut exec = exec_with_policy(ValidationPolicy::Strict);
        let err = exec.run(&approached_only_wf()).unwrap_err();
        assert!(matches!(err, SpmError::Workflow(_)));
    }

    #[test]
    fn strict_policy_passes_when_state_seeded() {
        let mut exec = exec_with_policy(ValidationPolicy::Strict);
        let mut state = MachineState::uninitialized();
        state.tip.set(TipEngagement::Approached);
        exec.seed_state(state);
        exec.run(&approached_only_wf()).unwrap();
    }

    #[test]
    fn advisory_policy_warns_but_proceeds() {
        let mut exec = exec_with_policy(ValidationPolicy::Advisory);
        exec.run(&approached_only_wf()).unwrap();
    }

    #[test]
    fn set_bias_updates_state_in_strict_mode() {
        let reg = crate::action::builtin_registry();
        let mut exec =
            WorkflowExecutor::new(reg, Box::new(MockController::new()));
        exec.set_policy(ValidationPolicy::Strict);
        let wf = Workflow::new("t", "").step(Step::action(
            "set_bias",
            serde_json::json!({"voltage": 2.5}),
        ));
        exec.run(&wf).unwrap();
        assert_eq!(exec.state().bias_v.as_known(), Some(&2.5));
    }
}
