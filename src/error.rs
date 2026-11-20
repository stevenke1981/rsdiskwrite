//! Crate-wide error type with stable machine-readable codes and process exit codes.
//!
//! Every fallible public function in this crate returns [`Result<T>`]. An [`Error`]
//! carries an [`ErrorKind`] (which determines the JSON `code` and the process exit
//! code), a human-readable message and an optional hint telling the user what to do.

use serde::Serialize;
use std::fmt;

/// Classification of an [`Error`].
///
/// The snake_case [`ErrorKind::code`] string is part of the JSON/MCP contract and the
/// [`ErrorKind::exit_code`] mapping is part of the CLI contract; neither may change
/// without a schema bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Invalid command line or arguments (exit 2).
    Usage,
    /// A safety rule refused the operation, e.g. system disk, fixed disk,
    /// large disk without `--allow-large`, image larger than the disk (exit 3).
    SafetyRefusal,
    /// The user did not confirm a destructive action, or the confirmation phrase
    /// was wrong or could not be read from a terminal (exit 4).
    NotConfirmed,
    /// The operation needs an elevated (Administrator) process (exit 5).
    ElevationRequired,
    /// Read-back verification found the disk differs from the image (exit 6).
    VerifyMismatch,
    /// A SHA-256 digest did not match the expected value (exit 6).
    HashMismatch,
    /// The requested physical disk does not exist or cannot be opened (exit 7).
    DiskNotFound,
    /// The disk at the requested number is no longer the disk that was
    /// planned/confirmed (number, size, model or serial changed) (exit 7).
    IdentityChanged,
    /// The image file is empty, corrupt or not decodable (exit 1).
    InvalidImage,
    /// An MCP `plan_id` is unknown, expired or already used (exit 1).
    PlanInvalid,
    /// Another write is already running (MCP server) (exit 1).
    Busy,
    /// Operating-system or file I/O failure (exit 1).
    Io,
    /// A bug or unexpected internal state (exit 1).
    Internal,
    /// The feature is not implemented or not available on this system (exit 1).
    Unsupported,
}

impl ErrorKind {
    /// Every variant, in declaration order. Useful for exhaustive tests and docs.
    pub const ALL: [ErrorKind; 14] = [
        ErrorKind::Usage,
        ErrorKind::SafetyRefusal,
        ErrorKind::NotConfirmed,
        ErrorKind::ElevationRequired,
        ErrorKind::VerifyMismatch,
        ErrorKind::HashMismatch,
        ErrorKind::DiskNotFound,
        ErrorKind::IdentityChanged,
        ErrorKind::InvalidImage,
        ErrorKind::PlanInvalid,
        ErrorKind::Busy,
        ErrorKind::Io,
        ErrorKind::Internal,
        ErrorKind::Unsupported,
    ];

    /// Stable snake_case identifier used as `error.code` in JSON output and MCP results.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Usage => "usage",
            ErrorKind::SafetyRefusal => "safety_refusal",
            ErrorKind::NotConfirmed => "not_confirmed",
            ErrorKind::ElevationRequired => "elevation_required",
            ErrorKind::VerifyMismatch => "verify_mismatch",
            ErrorKind::HashMismatch => "hash_mismatch",
            ErrorKind::DiskNotFound => "disk_not_found",
            ErrorKind::IdentityChanged => "identity_changed",
            ErrorKind::InvalidImage => "invalid_image",
            ErrorKind::PlanInvalid => "plan_invalid",
            ErrorKind::Busy => "busy",
            ErrorKind::Io => "io",
            ErrorKind::Internal => "internal",
            ErrorKind::Unsupported => "unsupported",
        }
    }

    /// Process exit code for the CLI:
    /// 1 I/O/internal/unsupported (and other generic failures), 2 usage,
    /// 3 safety refusal, 4 not confirmed, 5 elevation required,
    /// 6 verify/hash mismatch, 7 disk not found / identity changed.
    /// Success is 0 and is never produced by an error.
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorKind::Usage => 2,
            ErrorKind::SafetyRefusal => 3,
            ErrorKind::NotConfirmed => 4,
            ErrorKind::ElevationRequired => 5,
            ErrorKind::VerifyMismatch | ErrorKind::HashMismatch => 6,
            ErrorKind::DiskNotFound | ErrorKind::IdentityChanged => 7,
            ErrorKind::InvalidImage
            | ErrorKind::PlanInvalid
            | ErrorKind::Busy
            | ErrorKind::Io
            | ErrorKind::Internal
            | ErrorKind::Unsupported => 1,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The crate error. Serializes as `{"code": "...", "message": "...", "hint": ... }`
/// (the exact shape of the JSON envelope's `error` object and of MCP tool errors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    /// Classification; serialized as `code` using [`ErrorKind::code`].
    #[serde(rename = "code")]
    pub kind: ErrorKind,
    /// One-line human-readable description (no trailing period required).
    pub message: String,
    /// Optional advice on how to fix or work around the problem.
    pub hint: Option<String>,
    /// The command already rendered its result (e.g. `doctor` printed a report with failing
    /// checks); `main` must only exit with [`Error::exit_code`] and print nothing more.
    /// Set with [`Error::into_reported`]. Not serialized.
    #[serde(skip)]
    pub reported: bool,
}

impl Error {
    /// Creates an error of `kind` with `message` and no hint.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
            reported: false,
        }
    }

    /// Marks the error as already reported to the user (see [`Error::reported`]).
    pub fn into_reported(mut self) -> Self {
        self.reported = true;
        self
    }

    /// Returns the error with `hint` attached (replacing any previous hint).
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Shorthand for `Error::new(ErrorKind::Usage, message)`.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, message)
    }

    /// Shorthand for `Error::new(ErrorKind::SafetyRefusal, message)`.
    pub fn refusal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::SafetyRefusal, message)
    }

    /// Shorthand for `Error::new(ErrorKind::Internal, message)`.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    /// An [`ErrorKind::Unsupported`] error for a feature that is not implemented yet
    /// (`what` names the feature, e.g. `"read-back verification"`).
    pub fn unsupported(what: impl fmt::Display) -> Self {
        Self::new(
            ErrorKind::Unsupported,
            format!("{what} is not implemented in this build"),
        )
    }

    /// Wraps an I/O error as [`ErrorKind::Io`] with `"{context}: {err}"` as the message.
    pub fn io(context: impl fmt::Display, err: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, format!("{context}: {err}"))
    }

    /// Like [`Error::io`], but maps `PermissionDenied` to [`ErrorKind::ElevationRequired`]
    /// with a "run as Administrator" hint. Use for opening raw disks and volumes.
    pub fn io_elevation(context: impl fmt::Display, err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            Self::new(ErrorKind::ElevationRequired, format!("{context}: {err}"))
                .with_hint("Run rsdiskwrite from an elevated (Administrator) terminal")
        } else {
            Self::io(context, err)
        }
    }

    /// Shorthand for `self.kind.exit_code()`.
    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    /// Converts to [`ErrorKind::Io`] using the I/O error's own message.
    fn from(err: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, err.to_string())
    }
}

/// `std::result::Result` specialised to the crate [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_cli_contract() {
        let expected = [
            (ErrorKind::Usage, 2, "usage"),
            (ErrorKind::SafetyRefusal, 3, "safety_refusal"),
            (ErrorKind::NotConfirmed, 4, "not_confirmed"),
            (ErrorKind::ElevationRequired, 5, "elevation_required"),
            (ErrorKind::VerifyMismatch, 6, "verify_mismatch"),
            (ErrorKind::HashMismatch, 6, "hash_mismatch"),
            (ErrorKind::DiskNotFound, 7, "disk_not_found"),
            (ErrorKind::IdentityChanged, 7, "identity_changed"),
            (ErrorKind::InvalidImage, 1, "invalid_image"),
            (ErrorKind::PlanInvalid, 1, "plan_invalid"),
            (ErrorKind::Busy, 1, "busy"),
            (ErrorKind::Io, 1, "io"),
            (ErrorKind::Internal, 1, "internal"),
            (ErrorKind::Unsupported, 1, "unsupported"),
        ];
        assert_eq!(expected.len(), ErrorKind::ALL.len());
        for (kind, code, name) in expected {
            assert_eq!(kind.exit_code(), code, "{kind:?}");
            assert_eq!(kind.code(), name);
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(name.into())
            );
        }
    }

    #[test]
    fn error_serializes_as_code_message_hint() {
        let error = Error::refusal("no").with_hint("try again");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({"code": "safety_refusal", "message": "no", "hint": "try again"})
        );
        assert_eq!(error.exit_code(), 3);
        assert_eq!(error.to_string(), "no");
    }

    #[test]
    fn permission_denied_maps_to_elevation_required() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            Error::io_elevation("open", denied).kind,
            ErrorKind::ElevationRequired
        );
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(Error::io_elevation("open", missing).kind, ErrorKind::Io);
    }
}
