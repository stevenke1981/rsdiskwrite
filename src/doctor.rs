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
    let mut checks = vec![
        Check::ok("version", format!("rsdiskwrite {}", crate::VERSION)),
        check_exe(),
        check_os(),
        check_elevation(),
        check_system_disk(),
        check_temp_dir(),
    ];
    checks.extend(check_disks());
    checks.push(check_volumes());
    checks.push(check_terminals());
    checks.push(check_mcp_self_test());
    checks
}

/// True when the process token is elevated (`TokenElevation`).
#[cfg(windows)]
pub fn is_elevated() -> Result<bool> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: valid out pointer; the pseudo handle needs no closing.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(Error::io(
            "cannot open the process token",
            std::io::Error::last_os_error(),
        ));
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION and its size is passed exactly.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    let err = std::io::Error::last_os_error();
    // SAFETY: token was opened above and is closed exactly once.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(Error::io("cannot query the process token", err));
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(windows)]
fn check_exe() -> Check {
    match std::env::current_exe() {
        Ok(path) => Check::ok("executable", path.display().to_string()),
        Err(e) => Check::warn(
            "executable",
            format!("cannot determine the executable path: {e}"),
            "MCP client snippets need --exe PATH",
        ),
    }
}

#[cfg(windows)]
fn os_version() -> Option<(u32, u32, u32)> {
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
    }
    // SAFETY: zeroed POD struct with its size field set, as RtlGetVersion requires.
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: valid pointer to an initialised struct.
    (unsafe { RtlGetVersion(&mut info) } == 0).then_some((
        info.dwMajorVersion,
        info.dwMinorVersion,
        info.dwBuildNumber,
    ))
}

#[cfg(windows)]
fn check_os() -> Check {
    let arch = std::env::consts::ARCH;
    match os_version() {
        Some((major, minor, build)) => {
            let name = if major >= 10 && build >= 22000 {
                "Windows 11"
            } else {
                "Windows"
            };
            Check::ok(
                "os",
                format!("{name} {major}.{minor} build {build} ({arch})"),
            )
        }
        None => Check::warn(
            "os",
            format!("Windows, version unknown ({arch})"),
            "Report this if other checks misbehave",
        ),
    }
}

#[cfg(windows)]
fn check_elevation() -> Check {
    match is_elevated() {
        Ok(true) => Check::ok("elevation", "running elevated (Administrator)"),
        Ok(false) => Check::warn(
            "elevation",
            "not elevated",
            "Listing works, but writing and verifying need Administrator: right-click your \
             terminal and choose 'Run as administrator'",
        ),
        Err(e) => Check::warn(
            "elevation",
            e.message,
            "Run from an elevated terminal if writes fail with access denied",
        ),
    }
}

#[cfg(windows)]
fn check_system_disk() -> Check {
    match crate::disk::system_disks() {
        Ok(disks) if !disks.is_empty() => Check::ok(
            "system_disk",
            format!(
                "SystemRoot is on disk {}",
                disks
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Ok(_) => Check::fail(
            "system_disk",
            "SystemRoot did not resolve to any physical disk",
            "The system-disk safety guard cannot work; writes will be refused. Report this",
        ),
        Err(e) => Check::fail(
            "system_disk",
            format!("cannot resolve the system disk: {}", e.message),
            "The system-disk safety guard cannot work; writes will be refused. Report this",
        ),
    }
}

#[cfg(windows)]
fn free_space(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    // SAFETY: NUL-terminated path and a valid out pointer; other outs may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(windows)]
fn check_temp_dir() -> Check {
    use crate::output::format_size;
    let dir = std::env::temp_dir();
    let free = free_space(&dir);
    let disks = crate::disk::volume_disks(&dir).ok();
    let mut detail = format!("{}", dir.display());
    if let Some(free) = free {
        detail.push_str(&format!(", {} free", format_size(free)));
    }
    if let Some(disks) = &disks
        && !disks.is_empty()
    {
        detail.push_str(&format!(
            ", on disk {}",
            disks
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    match free {
        None => Check::warn(
            "temp_dir",
            format!("{detail} (free space unknown)"),
            "Compressed images are decompressed to a temporary file; set TEMP to a valid folder",
        ),
        Some(free) if free < 1024 * 1024 * 1024 => Check::warn(
            "temp_dir",
            detail,
            "Less than 1 GiB free; large compressed images may not fit. Point TEMP at a bigger drive",
        ),
        Some(_) => Check::ok("temp_dir", detail),
    }
}

#[cfg(windows)]
fn check_disks() -> Vec<Check> {
    match crate::disk::enumerate() {
        Ok(e) => {
            let method = match e.method {
                crate::disk::EnumerationMethod::SetupApi => "SetupAPI",
                crate::disk::EnumerationMethod::Probe => "probing PhysicalDrive0-127",
            };
            let mut checks = vec![Check::ok(
                "disk_enumeration",
                format!("{} disk(s) found via {method}", e.disks.len()),
            )];
            if e.disks.is_empty() {
                checks[0] = Check::warn(
                    "disk_enumeration",
                    format!("no disks found via {method}"),
                    "Reconnect the device, or run elevated if listing stays empty",
                );
            } else if e.method == crate::disk::EnumerationMethod::Probe {
                checks[0] = Check::warn(
                    "disk_enumeration",
                    format!("{} disk(s) found via {method}", e.disks.len()),
                    "SetupAPI enumeration was unavailable; the fallback may miss disks",
                );
            }
            checks
        }
        Err(e) => vec![Check::fail(
            "disk_enumeration",
            format!("cannot enumerate disks: {}", e.message),
            "Run from an elevated terminal and check the Windows Storage service",
        )],
    }
}

#[cfg(windows)]
fn check_volumes() -> Check {
    match crate::disk::volumes() {
        Ok(volumes) => {
            let unmapped = volumes.iter().filter(|v| v.disks.is_none()).count();
            if unmapped > 0 {
                Check::warn(
                    "volume_enumeration",
                    format!(
                        "{} volume(s), {unmapped} without readable disk extents",
                        volumes.len()
                    ),
                    "Those volumes cannot be locked before a write; run elevated",
                )
            } else {
                Check::ok(
                    "volume_enumeration",
                    format!("{} volume(s) mapped to disks", volumes.len()),
                )
            }
        }
        Err(e) => Check::fail(
            "volume_enumeration",
            format!("cannot enumerate volumes: {}", e.message),
            "Volumes cannot be locked before writing; run elevated and report this",
        ),
    }
}

#[cfg(windows)]
fn check_terminals() -> Check {
    use std::io::IsTerminal;
    let yes = |b: bool| if b { "terminal" } else { "redirected" };
    let no_color = crate::output::no_color_env();
    Check::ok(
        "terminals",
        format!(
            "stdin {}, stdout {}, stderr {}; NO_COLOR {}",
            yes(std::io::stdin().is_terminal()),
            yes(std::io::stdout().is_terminal()),
            yes(std::io::stderr().is_terminal()),
            if no_color { "set" } else { "not set" }
        ),
    )
}

/// A cloneable in-memory writer for the MCP self-test.
#[cfg(windows)]
#[derive(Clone, Default)]
struct SharedBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

#[cfg(windows)]
impl std::io::Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn mcp_round_trip() -> Result<usize> {
    use serde_json::{Value, json};
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion": crate::mcp::LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "rsdiskwrite-doctor", "version": crate::VERSION}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    ];
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let buffer = SharedBuffer::default();
    let writer = buffer.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Quiet: the self-test must not print the server's stderr log lines into the
        // doctor report.
        let mut config = crate::mcp::ServerConfig::new(false);
        config.quiet = true;
        let _ = tx.send(crate::mcp::serve(
            std::io::Cursor::new(input.into_bytes()),
            writer,
            config,
        ));
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| Error::internal("the MCP server did not finish within 10 seconds"))??;
    let bytes = buffer
        .0
        .lock()
        .map_err(|_| Error::internal("poisoned buffer"))?
        .clone();
    let text = String::from_utf8_lossy(&bytes);
    let messages: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| Error::internal(format!("server wrote a non-JSON line: {e}")))?;
    let find = |id: i64| messages.iter().find(|m| m["id"] == id);
    if find(1)
        .and_then(|m| m["result"]["protocolVersion"].as_str())
        .is_none()
    {
        return Err(Error::internal("initialize returned no protocolVersion"));
    }
    let tools = find(2)
        .and_then(|m| m["result"]["tools"].as_array())
        .ok_or_else(|| Error::internal("tools/list returned no tools array"))?;
    if tools.is_empty() {
        return Err(Error::internal("tools/list returned no tools"));
    }
    Ok(tools.len())
}

#[cfg(windows)]
fn check_mcp_self_test() -> Check {
    match mcp_round_trip() {
        Ok(n) => Check::ok(
            "mcp_self_test",
            format!("in-process initialize + tools/list succeeded ({n} tools)"),
        ),
        Err(e) if e.kind == crate::ErrorKind::Unsupported => Check::warn(
            "mcp_self_test",
            e.message,
            "Update rsdiskwrite to a build that includes the MCP server",
        ),
        Err(e) => Check::fail(
            "mcp_self_test",
            format!("MCP round trip failed: {}", e.message),
            "Run 'rsdiskwrite mcp serve' manually and report this",
        ),
    }
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

    #[test]
    fn check_constructors() {
        let c = Check::warn("elevation", "not elevated", "elevate");
        assert_eq!(c.status, CheckStatus::Warn);
        assert_eq!(c.hint.as_deref(), Some("elevate"));
        assert!(Check::ok("a", "b").hint.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn run_produces_well_formed_checks() {
        let checks = run();
        let ids: Vec<&str> = checks.iter().map(|c| c.id.as_str()).collect();
        for id in [
            "version",
            "elevation",
            "temp_dir",
            "disk_enumeration",
            "mcp_self_test",
        ] {
            assert!(ids.contains(&id), "missing {id}");
        }
        for c in &checks {
            assert!(
                c.id.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "{}",
                c.id
            );
            assert!(!c.detail.is_empty());
            if c.status != CheckStatus::Ok {
                assert!(c.hint.is_some(), "{} lacks a hint", c.id);
            }
        }
    }
}
