//! Environment diagnostics (`rsdiskwrite doctor`, MCP `doctor` tool).

#[cfg(windows)]
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// One diagnostic check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Stable snake_case id, e.g. `"elevation"`, `"system_disk"`, `"mcp_self_test"`.
    pub id: String,
    pub status: CheckStatus,
    /// What was found, e.g. `"not elevated"`.
    pub detail: String,
    /// What to do about a warning/failure.
    pub hint: Option<String>,
}

impl Check {
    /// An [`CheckStatus::Ok`] check.
    pub fn ok(id: &str, detail: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Ok, detail, None)
    }
    /// A [`CheckStatus::Warn`] check with a hint.
    pub fn warn(id: &str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Warn, detail, Some(hint.into()))
    }
    /// A [`CheckStatus::Fail`] check with a hint.
    pub fn fail(id: &str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Fail, detail, Some(hint.into()))
    }
    fn new(id: &str, status: CheckStatus, detail: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            id: id.to_owned(),
            status,
            detail: detail.into(),
            hint,
        }
    }
}

/// The `doctor` command's JSON `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// rsdiskwrite version.
    pub version: String,
    pub checks: Vec<Check>,
}

impl DoctorReport {
    /// Wraps `checks` with the crate version.
    pub fn new(checks: Vec<Check>) -> Self {
        Self {
            version: crate::VERSION.to_owned(),
            checks,
        }
    }
    /// True when any check is [`CheckStatus::Fail`] (`doctor` then exits 1).
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }
}

/// Runs every check (version, exe path, OS build/arch, elevation, system disk, temp dir
/// disk + free space, disk enumeration, volume enumeration, terminals/NO_COLOR, MCP
/// self-test). Never writes anything and never fails as a whole.
#[cfg(windows)]
pub fn run() -> Vec<Check> {
    // TODO(phase2-D): implement the full check list from PLAN.md section 4.
    vec![
        Check::ok("version", format!("rsdiskwrite {}", crate::VERSION)),
        Check::warn(
            "checks",
            "most diagnostics are not implemented in this build",
            "Update rsdiskwrite",
        ),
    ]
}

/// True when the process token is elevated (`TokenElevation`).
#[cfg(windows)]
pub fn is_elevated() -> Result<bool> {
    // TODO(phase2-D): OpenProcessToken + GetTokenInformation(TokenElevation).
    Err(Error::unsupported("elevation detection"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_failure_detection() {
        let mut report = DoctorReport::new(vec![Check::ok("a", "fine")]);
        assert!(!report.has_failures());
        report.checks.push(Check::fail("b", "bad", "fix"));
        assert!(report.has_failures());
        assert_eq!(
            serde_json::to_value(&report.checks[1]).unwrap()["status"],
            "fail"
        );
    }
}
