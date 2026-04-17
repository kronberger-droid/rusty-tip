mod executor;
mod shutdown;

pub use executor::WorkflowExecutor;
pub use shutdown::ShutdownFlag;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::action::ActionOutput;

/// A complete workflow definition -- serializable to/from JSON/TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Workflow {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            steps: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Step {
    /// Execute a single action by name.
    Do {
        action: String,
        #[serde(default)]
        params: serde_json::Value,
        #[serde(default)]
        store_as: Option<String>,
        #[serde(default)]
        label: Option<String>,
    },

    /// Execute steps in order, stop on first error.
    Sequence {
        steps: Vec<Step>,
        #[serde(default)]
        label: Option<String>,
    },

    /// Conditional execution.
    If {
        condition: Condition,
        then: Box<Step>,
        #[serde(default)]
        otherwise: Option<Box<Step>>,
    },

    /// Repeat until condition is met or max iterations reached.
    Loop {
        body: Box<Step>,
        #[serde(default)]
        until: Option<Condition>,
        #[serde(default = "default_max_iterations")]
        max_iterations: u32,
        #[serde(default)]
        label: Option<String>,
    },

    /// Store a literal value into the DataStore.
    SetVar {
        key: String,
        value: serde_json::Value,
    },

    /// Wait for a duration.
    Wait { duration_ms: u64 },
}

fn default_max_iterations() -> u32 {
    1000
}

fn default_compare_tolerance() -> f64 {
    1e-9
}

impl Step {
    /// Convenience: create a Do step.
    pub fn action(name: &str, params: serde_json::Value) -> Self {
        Step::Do {
            action: name.into(),
            params,
            store_as: None,
            label: None,
        }
    }

    /// Convenience: create a Do step that stores its result.
    pub fn action_store(
        name: &str,
        params: serde_json::Value,
        store_as: &str,
    ) -> Self {
        Step::Do {
            action: name.into(),
            params,
            store_as: Some(store_as.into()),
            label: None,
        }
    }

    /// Convenience: create a Sequence step.
    pub fn sequence(steps: Vec<Step>) -> Self {
        Step::Sequence { steps, label: None }
    }

    /// Convenience: create a Loop step.
    pub fn repeat(body: Step, max_iterations: u32) -> Self {
        Step::Loop {
            body: Box::new(body),
            until: None,
            max_iterations,
            label: None,
        }
    }

    /// Convenience: create a Loop step with an exit condition.
    pub fn repeat_until(
        body: Step,
        until: Condition,
        max_iterations: u32,
    ) -> Self {
        Step::Loop {
            body: Box::new(body),
            until: Some(until),
            max_iterations,
            label: None,
        }
    }

    /// Convenience: create a Wait step.
    pub fn wait(duration_ms: u64) -> Self {
        Step::Wait { duration_ms }
    }
}

/// Conditions for If and Loop steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Condition {
    /// Compare a stored value to a threshold.
    ///
    /// `tolerance` is the absolute tolerance applied to `Eq`/`Ne` comparisons.
    /// For physical measurements (bias in V, frequency shift in Hz, etc.)
    /// `f64::EPSILON` is never useful — supply a tolerance meaningful in the
    /// variable's units. Defaults to `1e-9`.
    Compare {
        variable: String,
        operator: CompareOp,
        threshold: f64,
        #[serde(default = "default_compare_tolerance")]
        tolerance: f64,
    },
    /// Check if a value is within bounds (inclusive).
    InRange {
        variable: String,
        min: f64,
        max: f64,
    },
    /// All conditions must be true.
    And { conditions: Vec<Condition> },
    /// At least one condition must be true.
    Or { conditions: Vec<Condition> },
    /// Negate a condition.
    Not { condition: Box<Condition> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
    Ne,
}

/// What a step produces after execution.
#[derive(Debug)]
pub enum StepOutcome {
    /// Step completed with an action output.
    Completed(ActionOutput),
    /// Workflow was shut down gracefully.
    Shutdown,
    /// Loop reached max iterations without exit condition being met.
    CycleLimit { label: Option<String>, max: u32 },
}

/// Final result of a workflow run.
#[derive(Debug)]
pub enum WorkflowOutcome {
    /// Workflow ran to completion.
    Completed,
    /// Workflow was stopped by shutdown flag.
    StoppedByUser,
    /// Loop reached its max iteration count without the exit condition being met.
    CycleLimit { label: Option<String>, max: u32 },
}
