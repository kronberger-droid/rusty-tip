use std::time::Instant;

use crate::action::{ActionContext, ActionOutput, ActionRegistry, DataStore};
use crate::event::{Event, EventBus, EventEmitter, Observer};
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

use super::{
    CompareOp, Condition, ShutdownFlag, Step, StepOutcome, Workflow, WorkflowOutcome,
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
}

impl WorkflowExecutor {
    pub fn new(registry: ActionRegistry, controller: Box<dyn SpmController>) -> Self {
        Self {
            registry,
            controller,
            events: EventBus::new(),
            store: DataStore::new(),
            shutdown: ShutdownFlag::new(),
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

    /// Execute a complete workflow.
    pub fn run(&mut self, workflow: &Workflow) -> Result<WorkflowOutcome, SpmError> {
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
                        StepOutcome::Shutdown => return Ok(StepOutcome::Shutdown),
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
                label: _,
            } => {
                for _i in 0..*max_iterations {
                    match self.execute_step(body)? {
                        StepOutcome::Completed(_) => {}
                        StepOutcome::Shutdown => return Ok(StepOutcome::Shutdown),
                    }
                    if let Some(condition) = until {
                        if self.evaluate_condition(condition)? {
                            return Ok(StepOutcome::Completed(ActionOutput::Unit));
                        }
                    }
                }
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }

            Step::SetVar { key, value } => {
                self.store.set(key, value);
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }

            Step::Wait { duration_ms } => {
                std::thread::sleep(std::time::Duration::from_millis(*duration_ms));
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

        // Emit start event
        self.events
            .emit(Event::action_started(action.name(), params.clone()));

        let start = Instant::now();
        let mut ctx = ActionContext {
            controller: &mut *self.controller,
            store: &mut self.store,
            events: &self.events,
        };

        match action.execute(&mut ctx) {
            Ok(output) => {
                let duration = start.elapsed();
                self.events
                    .emit(Event::action_completed(action.name(), &output, duration));

                // Store result
                let key = store_as.unwrap_or(action.name());
                self.store.set(key, &output);

                Ok(StepOutcome::Completed(output))
            }
            Err(e) => {
                let duration = start.elapsed();
                self.events.emit(Event::action_failed(
                    action.name(),
                    &e.to_string(),
                    duration,
                ));
                Err(e)
            }
        }
    }

    /// Evaluate a condition against the current DataStore.
    fn evaluate_condition(&self, condition: &Condition) -> Result<bool, SpmError> {
        match condition {
            Condition::Compare {
                variable,
                operator,
                threshold,
            } => {
                let value: f64 = self.resolve_variable(variable)?;
                Ok(match operator {
                    CompareOp::Lt => value < *threshold,
                    CompareOp::Le => value <= *threshold,
                    CompareOp::Eq => (value - threshold).abs() < f64::EPSILON,
                    CompareOp::Ge => value >= *threshold,
                    CompareOp::Gt => value > *threshold,
                    CompareOp::Ne => (value - threshold).abs() >= f64::EPSILON,
                })
            }

            Condition::InRange {
                variable,
                min,
                max,
            } => {
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
    /// Handles both raw f64 values and ActionOutput::Value stored by actions.
    fn resolve_variable(&self, name: &str) -> Result<f64, SpmError> {
        let json = self
            .store
            .get_raw(name)
            .ok_or_else(|| SpmError::Workflow(format!("Variable '{}' not found in store", name)))?;

        // Try direct f64
        if let Some(v) = json.as_f64() {
            return Ok(v);
        }

        // Try ActionOutput::Value (tagged as {"type":"value","0": n})
        if let Some(obj) = json.as_object() {
            if obj.get("type").and_then(|t| t.as_str()) == Some("value") {
                if let Some(v) = obj.get("0").and_then(|v| v.as_f64()) {
                    return Ok(v);
                }
            }
        }

        Err(SpmError::Workflow(format!(
            "Variable '{}' is not a numeric value: {}",
            name, json
        )))
    }
}
