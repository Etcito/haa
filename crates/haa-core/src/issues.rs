//! Validation issues (ARCHITECTURE §12): every loading and validation problem is an `Issue`, never a panic.

use serde::Serialize;

/// How serious an issue is. Only `Error` stops a patch from playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One problem found in a patch, at a dotted path such as `graph.nodes[3].params.cutoff`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    pub message: String,
}

/// True when any issue has `Severity::Error`.
pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|issue| issue.severity == Severity::Error)
}

/// The issues as a compact JSON array, the form the wasm ABI returns.
pub fn issues_json(issues: &[Issue]) -> String {
    serde_json::to_string(issues).unwrap_or_else(|_| "[]".to_string())
}
