//! Write gating for the MCP server: single-use, time-limited plans and the human
//! confirmation dialog.
//!
//! Layers (PLAN.md section 3): (1) `write_image` exists only with `--allow-write`;
//! (2) a single-use `plan_id` valid for [`PLAN_TTL`], bound to disk identity and image
//! SHA-256; (3) a native topmost dialog (default No) that a human must accept — there is
//! no flag or environment bypass; (4) the hard refusals of [`crate::ops::plan`] plus the
//! identity re-check after locking.

use crate::disk::{Disk, DiskIdentity};
use crate::error::{Error, ErrorKind, Result};
use crate::hash::Digest;
use crate::ops::WritePlan;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Shows a blocking, topmost Yes/No warning box whose default button is No (pressing
    /// Enter or closing the box declines). Only an explicit click on Yes returns true.
    ///
    /// This calls `MessageBoxW` directly instead of `rfd::MessageDialog` because rfd 0.15
    /// can neither make the box topmost nor make No the default button.
    #[cfg(windows)]
    fn confirm(&self, prompt: &WriteConfirmPrompt) -> bool {
        let body = format!(
            "{}\n\n(Default: No. Pressing Enter or closing this window cancels.)",
            prompt.message()
        );
        dialog::yes_no_default_no(prompt.title(), &body)
    }

    /// No native dialog is available: always declines.
    #[cfg(not(windows))]
    fn confirm(&self, prompt: &WriteConfirmPrompt) -> bool {
        let _ = prompt;
        false
    }
}

#[cfg(windows)]
mod dialog {
    use std::ffi::c_void;

    const MB_YESNO: u32 = 0x0000_0004;
    const MB_ICONWARNING: u32 = 0x0000_0030;
    const MB_DEFBUTTON2: u32 = 0x0000_0100;
    const MB_SETFOREGROUND: u32 = 0x0001_0000;
    const MB_TOPMOST: u32 = 0x0004_0000;
    const IDYES: i32 = 6;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16()
            .filter(|&unit| unit != 0)
            .chain(std::iter::once(0))
            .collect()
    }

    /// Blocks until the human answers; true only for "Yes".
    pub(super) fn yes_no_default_no(title: &str, body: &str) -> bool {
        let title = wide(title);
        let body = wide(body);
        let flags = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST;
        // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call; a
        // null owner window is allowed.
        let answer =
            unsafe { MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), flags) };
        answer == IDYES
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

    /// True once `ttl` has elapsed since the plan was stored.
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created.elapsed() >= ttl
    }
}

/// A fresh, unguessable plan id: `plan_` + 32 hex digits.
///
/// No RNG crate is available, so the id is the SHA-256 of several keys drawn from std's
/// `RandomState` (seeded from the OS CSPRNG) mixed with a counter, the clock and the
/// process id. Ids are never derived from the plan contents.
fn fresh_plan_id() -> PlanId {
    use std::hash::{BuildHasher, Hasher as _};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut digest = crate::hash::Hasher::new();
    for round in 0..4u64 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(round);
        hasher.write_u64(count);
        digest.update(&hasher.finish().to_le_bytes());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    digest.update(&now.as_nanos().to_le_bytes());
    digest.update(&std::process::id().to_le_bytes());
    digest.update(&count.to_le_bytes());
    digest.update(format!("{:?}", std::thread::current().id()).as_bytes());
    let digest = digest.finish();
    PlanId(format!("plan_{}", crate::hash::hex(&digest.0[..16])))
}

fn plan_invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::PlanInvalid, message).with_hint(
        "Call plan_write again to get a fresh plan_id (each id works once, for 5 minutes)",
    )
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
        let mut plans = self.plans.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = self.ttl;
        plans.retain(|_, stored| !stored.is_expired(ttl));
        let id = loop {
            let id = fresh_plan_id();
            if !plans.contains_key(&id) {
                break id;
            }
        };
        plans.insert(
            id.clone(),
            StoredPlan {
                id: id.clone(),
                plan,
                image_sha256,
                created: Instant::now(),
            },
        );
        Ok(id)
    }

    /// Removes and returns the plan (single use). Unknown, already used or expired ids
    /// are [`crate::ErrorKind::PlanInvalid`].
    pub fn take(&self, id: &str) -> Result<StoredPlan> {
        let mut plans = self.plans.lock().unwrap_or_else(|e| e.into_inner());
        let stored = plans.remove(&PlanId(id.trim().to_owned())).ok_or_else(|| {
            plan_invalid(format!("plan_id '{id}' is unknown or was already used"))
        })?;
        if stored.is_expired(self.ttl) {
            return Err(plan_invalid(format!(
                "plan_id '{id}' expired (plans are valid for {} seconds)",
                self.ttl.as_secs()
            )));
        }
        Ok(stored)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::disk::model::test_disk;
    use crate::image::ImageFormat;
    use crate::ops::{PlannedImage, WriteRequest};

    /// A plan for a disk number that does not exist, so nothing can ever be written.
    pub(crate) fn fake_plan(image: &std::path::Path, sha256: Digest) -> WritePlan {
        let disk = test_disk(9999);
        let mut request = WriteRequest::new(image, disk.number);
        request.hash_image = true;
        WritePlan {
            request,
            disk,
            image: PlannedImage {
                path: image.to_path_buf(),
                format: ImageFormat::Raw,
                file_size: 512,
                sha256: Some(sha256),
            },
            refusals: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn plans_are_single_use() {
        let store = PlanStore::default();
        let digest = Digest([7; 32]);
        let id = store
            .insert(fake_plan("x.img".as_ref(), digest), digest)
            .unwrap();
        assert!(id.0.starts_with("plan_") && id.0.len() == 37, "{id}");
        assert_eq!(store.len(), 1);
        let stored = store.take(&id.0).unwrap();
        assert_eq!(stored.image_sha256, digest);
        assert_eq!(stored.identity(), test_disk(9999).identity());
        assert!(store.is_empty());
        assert_eq!(store.take(&id.0).unwrap_err().kind, ErrorKind::PlanInvalid);
        assert_eq!(
            store.take("plan_bogus").unwrap_err().kind,
            ErrorKind::PlanInvalid
        );
    }

    #[test]
    fn expired_plans_are_rejected_and_purged() {
        let store = PlanStore::new(Duration::ZERO);
        let digest = Digest([1; 32]);
        let id = store
            .insert(fake_plan("x.img".as_ref(), digest), digest)
            .unwrap();
        let error = store.take(&id.0).unwrap_err();
        assert_eq!(error.kind, ErrorKind::PlanInvalid);
        assert!(error.message.contains("expired"), "{}", error.message);
        store
            .insert(fake_plan("x.img".as_ref(), digest), digest)
            .unwrap();
        store
            .insert(fake_plan("x.img".as_ref(), digest), digest)
            .unwrap();
        assert_eq!(store.len(), 1, "insert purges expired plans");
    }

    #[test]
    fn plan_ids_are_distinct() {
        let ids: std::collections::HashSet<_> = (0..200).map(|_| fresh_plan_id()).collect();
        assert_eq!(ids.len(), 200);
    }

    #[test]
    fn prompt_names_disk_and_image() {
        let prompt = WriteConfirmPrompt {
            disk: test_disk(3),
            image: "C:/images/x.img".into(),
            image_sha256: Some(Digest([0xab; 32])),
            image_size: 512,
        };
        let message = prompt.message();
        assert!(message.contains("PhysicalDrive3"));
        assert!(message.contains("SN1"));
        assert!(message.contains("x.img"));
        assert!(message.contains(&"ab".repeat(32)));
    }
}
