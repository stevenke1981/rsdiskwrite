//! Write gating for the MCP server: single-use, time-limited plans and the human
//! confirmation dialog.
//!
//! Layers (PLAN.md section 3): (1) `write_image` exists only with `--allow-write`;
//! (2) a single-use `plan_id` valid for [`PLAN_TTL`], bound to disk identity and image
//! SHA-256; (3) a native topmost dialog (default No) that a human must accept — there is
//! no flag or environment bypass; (4) the hard refusals of [`crate::ops::plan`] plus the
//! identity re-check after locking.

use crate::disk::{Disk, DiskIdentity};
use crate::error::{Error, Result};
use crate::hash::Digest;
use crate::ops::WritePlan;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a `plan_id` stays valid.
pub const PLAN_TTL: Duration = Duration::from_secs(5 * 60);

/// What the human is asked to approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WriteConfirmPrompt {
    pub disk: Disk,
    pub image: PathBuf,
    pub image_sha256: Option<Digest>,
    /// Image file size in bytes.
    pub image_size: u64,
}

impl WriteConfirmPrompt {
    /// Dialog title.
    pub fn title(&self) -> &'static str {
        "rsdiskwrite: an AI agent wants to ERASE a disk"
    }

    /// Dialog body: disk identity (number, model, serial, size, bus) and image.
    pub fn message(&self) -> String {
        let sha = self
            .image_sha256
            .map(|d| format!("\nSHA-256: {d}"))
            .unwrap_or_default();
        format!(
            "An MCP client asked to overwrite this disk. ALL DATA on it will be destroyed and \
             CANNOT BE RECOVERED.\n\nDisk: {}\nSize: {} bytes\n\nImage: {}\nImage size: {} bytes{sha}\n\n\
             Only click Yes if you started this and the disk is correct.",
            self.disk.summary(),
            self.disk.size,
            self.image.display(),
            self.image_size
        )
    }
}

/// Asks a human to approve a write. Production uses [`DialogConfirmer`]; tests inject
/// mocks via [`crate::mcp::ServerConfig`].
pub trait Confirmer: Send + Sync {
    /// Returns true only on an explicit "Yes". Closing or timing out must return false.
    fn confirm(&self, prompt: &WriteConfirmPrompt) -> bool;
}

/// Native topmost Yes/No warning dialog (default No).
#[derive(Debug, Clone, Copy, Default)]
pub struct DialogConfirmer;

impl Confirmer for DialogConfirmer {
    fn confirm(&self, prompt: &WriteConfirmPrompt) -> bool {
        // TODO(phase2-B): show a topmost rfd dialog (default No) and return true on Yes.
        let _ = prompt;
        false
    }
}

/// Opaque, unguessable plan identifier (serialized as a string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(pub String);

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A plan waiting for `write_image`.
#[derive(Debug, Clone)]
pub struct StoredPlan {
    pub id: PlanId,
    pub plan: WritePlan,
    /// SHA-256 of the image file at planning time; re-checked before writing.
    pub image_sha256: Digest,
    pub created: Instant,
}

impl StoredPlan {
    /// The disk identity the plan is bound to.
    pub fn identity(&self) -> DiskIdentity {
        self.plan.disk.identity()
    }
}

/// Thread-safe store of pending plans.
#[derive(Debug)]
pub struct PlanStore {
    ttl: Duration,
    plans: Mutex<HashMap<PlanId, StoredPlan>>,
}

impl PlanStore {
    /// An empty store whose plans expire after `ttl`.
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            plans: Mutex::new(HashMap::new()),
        }
    }

    /// The configured time-to-live.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Stores `plan` and returns a fresh unguessable id. Expired plans are purged.
    pub fn insert(&self, plan: WritePlan, image_sha256: Digest) -> Result<PlanId> {
        // TODO(phase2-B): generate an unguessable id and store the plan.
        let _ = (plan, image_sha256, &self.plans);
        Err(Error::unsupported("MCP write plans"))
    }

    /// Removes and returns the plan (single use). Unknown, already used or expired ids
    /// are [`crate::ErrorKind::PlanInvalid`].
    pub fn take(&self, id: &str) -> Result<StoredPlan> {
        // TODO(phase2-B): single-use lookup with expiry.
        let _ = id;
        Err(Error::unsupported("MCP write plans"))
    }

    /// Number of stored (possibly expired) plans.
    pub fn len(&self) -> usize {
        self.plans.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// True when no plans are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for PlanStore {
    fn default() -> Self {
        Self::new(PLAN_TTL)
    }
}
