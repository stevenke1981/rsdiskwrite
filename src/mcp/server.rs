//! The stdio MCP server loop.

use crate::error::{Error, ErrorKind, Result};
use crate::mcp::gate::{Confirmer, DialogConfirmer, PLAN_TTL, PlanStore};
use crate::mcp::protocol::{
    Incoming, Notification, Request, RequestId, Response, RpcError, codes, parse_line,
};
use crate::mcp::tools::{self, CallToolResult, ToolContext, names};
use crate::mcp::{SERVER_NAME, negotiate_protocol_version};
use crate::ops::{Phase, Progress};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Server settings.
#[derive(Clone)]
pub struct ServerConfig {
    /// Expose and accept `write_image` (`mcp serve --allow-write`).
    pub allow_write: bool,
    /// Human confirmation for writes. Always [`DialogConfirmer`] in production.
    pub confirmer: Arc<dyn Confirmer>,
    /// `plan_id` lifetime.
    pub plan_ttl: Duration,
}

impl ServerConfig {
    /// Production configuration: dialog confirmer and [`PLAN_TTL`].
    pub fn new(allow_write: bool) -> Self {
        Self {
            allow_write,
            confirmer: Arc::new(DialogConfirmer),
            plan_ttl: PLAN_TTL,
        }
    }
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("allow_write", &self.allow_write)
            .field("plan_ttl", &self.plan_ttl)
            .finish_non_exhaustive()
    }
}

/// Serves MCP over newline-delimited JSON-RPC until `reader` reaches EOF.
///
/// Handles `initialize`, `notifications/initialized`, `ping`, `tools/list` and
/// `tools/call`; unknown methods get `-32601`; notifications are never answered. Every
/// line written to `writer` is one JSON-RPC message (responses and `notifications/progress`);
/// writes run on a worker thread so `ping` stays responsive, which is why `writer` must be
/// `Send + 'static`. Returns `Ok(())` on EOF; I/O errors on `writer` end the loop with
/// [`crate::ErrorKind::Io`].
///
/// Every `tools/call` runs on its own worker thread, so responses may arrive out of
/// order (match them by id). Before returning, the server waits for running tool calls
/// (an in-progress disk write is never abandoned).
pub fn serve<R: BufRead, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    config: ServerConfig,
) -> Result<()> {
    let plans = Arc::new(PlanStore::new(config.plan_ttl));
    serve_with_store(reader, writer, config, plans)
}

/// [`serve`] with an externally owned plan store (tests pre-populate it).
fn serve_with_store<R: BufRead, W: Write + Send + 'static>(
    mut reader: R,
    writer: W,
    config: ServerConfig,
    plans: Arc<PlanStore>,
) -> Result<()> {
    let state = State {
        out: Arc::new(Mutex::new(writer)),
        config,
        plans,
        writing: Arc::new(AtomicBool::new(false)),
    };
    log(&format!(
        "serving MCP on stdio (write_image {})",
        if state.config.allow_write {
            "ENABLED"
        } else {
            "disabled"
        }
    ));
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    let mut buffer = Vec::new();
    let result = loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(Error::io("cannot read from the MCP client", e)),
        }
        workers.retain(|worker| !worker.is_finished());
        let reply = match std::str::from_utf8(&buffer) {
            Err(e) => Some(Response::error(
                None,
                RpcError::new(codes::PARSE_ERROR, format!("Parse error: {e}")),
            )),
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => match parse_line(line.trim()) {
                Err(response) => Some(*response),
                Ok(Incoming::Request(request)) => state.handle_request(request, &mut workers),
                Ok(Incoming::Notification(notification)) => {
                    state.handle_notification(&notification);
                    None
                }
                Ok(Incoming::Response(_)) => None,
            },
        };
        if let Some(reply) = reply
            && let Err(e) = send(&state.out, &reply)
        {
            break Err(Error::io("cannot write to the MCP client", e));
        }
    };
    for worker in workers {
        let _ = worker.join();
    }
    log("MCP client disconnected");
    result
}

/// Shared server state.
struct State<W> {
    out: Arc<Mutex<W>>,
    config: ServerConfig,
    plans: Arc<PlanStore>,
    /// A `write_image` call is running.
    writing: Arc<AtomicBool>,
}

/// Releases the write slot when dropped.
struct WriteSlot(Arc<AtomicBool>);

impl WriteSlot {
    fn acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}

impl Drop for WriteSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn log(message: &str) {
    eprintln!("rsdiskwrite mcp: {message}");
}

/// Writes one message as one line and flushes.
fn send<W: Write, T: Serialize>(out: &Mutex<W>, message: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
    out.write_all(&line)?;
    out.flush()
}

fn invalid_params(id: RequestId, message: impl Into<String>) -> Response {
    Response::error(Some(id), RpcError::new(codes::INVALID_PARAMS, message))
}

fn tool_result(id: RequestId, result: &CallToolResult) -> Response {
    match serde_json::to_value(result) {
        Ok(value) => Response::success(id, value),
        Err(e) => Response::error(
            Some(id),
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("cannot serialize result: {e}"),
            ),
        ),
    }
}

/// The `instructions` string of the `initialize` result.
pub fn instructions(allow_write: bool) -> String {
    let mut text = String::from(
        "rsdiskwrite writes raw disk images (.img, .img.gz) to USB sticks and SD cards on \
         Windows. Writing destroys everything on the target disk, so this server is built around \
         a safety model:\n\
         1. Read-only tools (list_disks, inspect_image, doctor, plan_write, verify_disk) never \
         write anything.\n\
         2. plan_write is a dry run that applies every safety rule (never the system disk, the \
         disk holding the image or temp directory, or fixed/internal disks; disks over 2 GiB \
         only with allow_large) and returns a single-use plan_id valid for 5 minutes, bound to \
         the disk's number, size, model and serial and to the image's SHA-256.\n\
         3. write_image consumes a plan_id and then shows a native warning dialog: a human at \
         this computer must click Yes. There is no argument, flag or environment variable that \
         skips it.\n\
         4. The disk identity and image hash are re-checked after confirmation and again after \
         locking the disk; any change aborts the write.\n\
         Workflow: list_disks -> confirm the target disk with the user (model, serial, size) -> \
         plan_write -> show the plan to the user -> write_image -> tell the user to answer the \
         dialog. Never pick a disk on your own. Failures are returned as isError results with \
         structuredContent {code, message, hint}.",
    );
    if allow_write {
        text.push_str("\nThis server was started with --allow-write: write_image is available.");
    } else {
        text.push_str(
            "\nThis server was started WITHOUT --allow-write: write_image is unavailable and \
             plan_write issues no plan_id. Writing requires the user to restart the server with \
             'rsdiskwrite mcp serve --allow-write'.",
        );
    }
    text
}

impl<W: Write + Send + 'static> State<W> {
    /// Answers a request, or returns `None` when a worker thread will answer it.
    fn handle_request(
        &self,
        request: Request,
        workers: &mut Vec<JoinHandle<()>>,
    ) -> Option<Response> {
        let Request {
            id, method, params, ..
        } = request;
        let result = match method.as_str() {
            "initialize" => {
                let requested = params
                    .as_ref()
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(Value::as_str);
                json!({
                    "protocolVersion": negotiate_protocol_version(requested),
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "title": "rsdiskwrite disk image writer",
                        "version": crate::VERSION
                    },
                    "instructions": instructions(self.config.allow_write)
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools::definitions(self.config.allow_write)}),
            "tools/call" => return self.call_tool(id, params, workers),
            _ => {
                return Some(Response::error(
                    Some(id),
                    RpcError::new(
                        codes::METHOD_NOT_FOUND,
                        format!("Method not found: {method}"),
                    ),
                ));
            }
        };
        Some(Response::success(id, result))
    }

    fn handle_notification(&self, notification: &Notification) {
        match notification.method.as_str() {
            "notifications/initialized" => log("client initialized"),
            "notifications/cancelled" => {
                log("cancellation requested (running disk operations cannot be cancelled)")
            }
            _ => {}
        }
    }

    fn call_tool(
        &self,
        id: RequestId,
        params: Option<Value>,
        workers: &mut Vec<JoinHandle<()>>,
    ) -> Option<Response> {
        let params = params.unwrap_or(Value::Null);
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Some(invalid_params(
                id,
                "tools/call needs params {\"name\": string, \"arguments\": object}",
            ));
        };
        let name = name.to_owned();
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let token = params
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .filter(|token| token.is_string() || token.is_number())
            .cloned();
        // One write at a time, decided here (not on the worker) so it is deterministic.
        let slot = if name == names::WRITE_IMAGE && self.config.allow_write {
            match WriteSlot::acquire(&self.writing) {
                Some(slot) => Some(slot),
                None => {
                    let busy = Error::new(ErrorKind::Busy, "Another write_image call is running")
                        .with_hint("Wait for it to finish, then plan the next write");
                    return Some(tool_result(id, &CallToolResult::failure(&busy)));
                }
            }
        } else {
            None
        };
        log(&format!("tools/call {name}"));
        let out = self.out.clone();
        let plans = self.plans.clone();
        let confirmer = self.config.confirmer.clone();
        let allow_write = self.config.allow_write;
        let worker_id = id.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("mcp-{name}"))
            .spawn(move || {
                let _slot = slot;
                let id = worker_id;
                let reporter = token.map(|token| ProgressReporter::new(token, out.clone()));
                let progress = |p: Progress| {
                    if let Some(reporter) = &reporter {
                        reporter.report(p);
                    }
                };
                let ctx = ToolContext {
                    allow_write,
                    plans: &plans,
                    confirmer: confirmer.as_ref(),
                    progress: reporter.as_ref().map(|_| &progress as &dyn Fn(Progress)),
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tools::call(&name, &arguments, &ctx)
                }));
                let response = match outcome {
                    Ok(Ok(result)) => tool_result(id, &result),
                    Ok(Err(error)) => Response::error(
                        Some(id),
                        RpcError {
                            code: codes::INVALID_PARAMS,
                            message: error.message.clone(),
                            data: serde_json::to_value(&error).ok(),
                        },
                    ),
                    Err(_) => Response::error(
                        Some(id),
                        RpcError::new(codes::INTERNAL_ERROR, format!("tool '{name}' panicked")),
                    ),
                };
                if let Err(e) = send(&out, &response) {
                    log(&format!("cannot send the {name} result: {e}"));
                }
            });
        match spawned {
            Ok(handle) => {
                workers.push(handle);
                None
            }
            Err(e) => Some(Response::error(
                Some(id),
                RpcError::new(
                    codes::INTERNAL_ERROR,
                    format!("cannot start a worker thread: {e}"),
                ),
            )),
        }
    }
}

/// Minimum interval between progress notifications within one phase.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Turns [`Progress`] callbacks into throttled `notifications/progress` whose `progress`
/// value strictly increases across phases (each phase restarts `done` at 0).
struct ProgressReporter<W> {
    token: Value,
    out: Arc<Mutex<W>>,
    state: Mutex<ProgressState>,
}

struct ProgressState {
    phase: Option<Phase>,
    /// Sum of the final `done` values of finished phases.
    base: u64,
    last_done: u64,
    last_sent: Option<Instant>,
    last_value: Option<u64>,
}

impl<W: Write> ProgressReporter<W> {
    fn new(token: Value, out: Arc<Mutex<W>>) -> Self {
        Self {
            token,
            out,
            state: Mutex::new(ProgressState {
                phase: None,
                base: 0,
                last_done: 0,
                last_sent: None,
                last_value: None,
            }),
        }
    }

    /// The notification to send for `p`, if any (throttled, monotonic).
    fn next(&self, p: Progress) -> Option<Notification> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let changed = state.phase != Some(p.phase);
        if changed {
            if state.phase.is_some() {
                state.base = state.base.saturating_add(state.last_done);
            }
            state.phase = Some(p.phase);
        }
        state.last_done = p.done;
        let finished = p.total > 0 && p.done >= p.total;
        let due = state
            .last_sent
            .is_none_or(|sent| sent.elapsed() >= PROGRESS_INTERVAL);
        if !(changed || finished || due) {
            return None;
        }
        let value = state.base.saturating_add(p.done);
        if state.last_value.is_some_and(|last| value <= last) {
            return None;
        }
        state.last_value = Some(value);
        state.last_sent = Some(Instant::now());
        let phase = serde_json::to_value(p.phase)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let mut params = json!({
            "progressToken": self.token,
            "progress": value,
            "message": format!("{phase}: {} of {} bytes", p.done, p.total),
        });
        if p.total > 0 {
            params["total"] = json!(state.base.saturating_add(p.total));
        }
        Some(Notification::new("notifications/progress", Some(params)))
    }

    fn report(&self, p: Progress) {
        if let Some(notification) = self.next(p)
            && let Err(e) = send(&self.out, &notification)
        {
            log(&format!("cannot send progress: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{Digest, sha256_file};
    use crate::mcp::gate::WriteConfirmPrompt;
    use crate::mcp::gate::tests::fake_plan;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    /// A cloneable in-memory writer.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buffer {
        fn lines(&self) -> Vec<Value> {
            let data = self.0.lock().unwrap().clone();
            String::from_utf8(data)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).expect("every line is JSON"))
                .collect()
        }
    }

    /// Answers a fixed value and counts calls.
    #[derive(Default)]
    struct Mock {
        answer: bool,
        calls: AtomicUsize,
    }

    impl Confirmer for Mock {
        fn confirm(&self, _: &WriteConfirmPrompt) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer
        }
    }

    fn config(allow_write: bool, confirmer: Arc<dyn Confirmer>) -> ServerConfig {
        ServerConfig {
            allow_write,
            confirmer,
            plan_ttl: PLAN_TTL,
        }
    }

    fn run(input: &[Value], config: ServerConfig, plans: Arc<PlanStore>) -> Vec<Value> {
        let text: String = input.iter().map(|v| format!("{v}\n")).collect();
        let out = Buffer::default();
        serve_with_store(Cursor::new(text), out.clone(), config, plans).unwrap();
        out.lines()
    }

    fn by_id(lines: &[Value], id: i64) -> &Value {
        lines
            .iter()
            .find(|line| line["id"] == id)
            .unwrap_or_else(|| panic!("no response {id} in {lines:?}"))
    }

    fn write_call(id: i64, plan_id: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": "write_image", "arguments": {"plan_id": plan_id}}})
    }

    /// A temporary 512-byte image and a stored plan for a disk that does not exist.
    fn stored_plan(plans: &PlanStore) -> (tempfile::NamedTempFile, String) {
        let mut image = tempfile::NamedTempFile::new().unwrap();
        image.write_all(&[0u8; 512]).unwrap();
        image.flush().unwrap();
        let digest: Digest = sha256_file(image.path(), &|_| {}).unwrap();
        let id = plans
            .insert(fake_plan(image.path(), digest), digest)
            .unwrap();
        (image, id.0)
    }

    fn error_code(response: &Value) -> &Value {
        assert_eq!(response["result"]["isError"], true, "{response}");
        &response["result"]["structuredContent"]["code"]
    }

    #[test]
    fn lifecycle_methods_and_errors() {
        let lines = run(
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                                  "clientInfo": {"name": "t", "version": "0"}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
                json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"}),
                json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {}}),
            ],
            config(false, Arc::new(Mock::default())),
            Arc::new(PlanStore::default()),
        );
        assert_eq!(
            lines.len(),
            5,
            "the notification is not answered: {lines:?}"
        );
        let init = &by_id(&lines, 1)["result"];
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert_eq!(init["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(init["serverInfo"]["name"], "rsdiskwrite");
        assert_eq!(init["serverInfo"]["version"], crate::VERSION);
        assert!(
            init["instructions"]
                .as_str()
                .unwrap()
                .contains("--allow-write")
        );
        assert_eq!(by_id(&lines, 2)["result"], json!({}));
        let tools = by_id(&lines, 3)["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["name"] != "write_image"));
        assert_eq!(by_id(&lines, 4)["error"]["code"], codes::METHOD_NOT_FOUND);
        assert_eq!(by_id(&lines, 5)["error"]["code"], codes::INVALID_PARAMS);
    }

    #[test]
    fn declined_confirmation_writes_nothing_and_consumes_the_plan() {
        let plans = Arc::new(PlanStore::default());
        let (_image, plan_id) = stored_plan(&plans);
        let mock = Arc::new(Mock::default());
        let lines = run(
            &[write_call(1, &plan_id)],
            config(true, mock.clone()),
            plans.clone(),
        );
        assert_eq!(error_code(by_id(&lines, 1)), "not_confirmed");
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert!(plans.is_empty());

        // Reusing the plan fails before the dialog is shown again.
        let lines = run(
            &[write_call(2, &plan_id)],
            config(true, mock.clone()),
            plans,
        );
        assert_eq!(error_code(by_id(&lines, 2)), "plan_invalid");
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expired_plan_is_rejected_without_a_dialog() {
        let plans = Arc::new(PlanStore::new(Duration::ZERO));
        let (_image, plan_id) = stored_plan(&plans);
        let mock = Arc::new(Mock {
            answer: true,
            ..Mock::default()
        });
        let lines = run(
            &[write_call(1, &plan_id)],
            config(true, mock.clone()),
            plans,
        );
        let response = by_id(&lines, 1);
        assert_eq!(error_code(response), "plan_invalid");
        assert!(
            response["result"]["structuredContent"]["message"]
                .as_str()
                .unwrap()
                .contains("expired")
        );
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn confirmed_write_still_rechecks_the_disk() {
        // The plan targets PhysicalDrive9999, which does not exist: the post-confirmation
        // re-plan must fail before anything is opened for writing.
        let plans = Arc::new(PlanStore::default());
        let (_image, plan_id) = stored_plan(&plans);
        let mock = Arc::new(Mock {
            answer: true,
            ..Mock::default()
        });
        let lines = run(
            &[write_call(1, &plan_id)],
            config(true, mock.clone()),
            plans,
        );
        let code = error_code(by_id(&lines, 1)).clone();
        assert_ne!(code, "not_confirmed");
        assert_ne!(code, "plan_invalid");
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn write_by_name_without_allow_write_is_refused() {
        let plans = Arc::new(PlanStore::default());
        let (_image, plan_id) = stored_plan(&plans);
        let mock = Arc::new(Mock {
            answer: true,
            ..Mock::default()
        });
        let lines = run(
            &[write_call(1, &plan_id)],
            config(false, mock.clone()),
            plans.clone(),
        );
        assert_eq!(error_code(by_id(&lines, 1)), "safety_refusal");
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
        assert_eq!(plans.len(), 1);
    }

    /// Blocks in the dialog until the test releases it, then declines.
    struct Blocking {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Confirmer for Blocking {
        fn confirm(&self, _: &WriteConfirmPrompt) -> bool {
            self.entered.lock().unwrap().send(()).unwrap();
            let _ = self.release.lock().unwrap().recv();
            false
        }
    }

    #[test]
    fn second_write_is_busy_and_ping_stays_responsive() {
        let plans = Arc::new(PlanStore::default());
        let (_a, first) = stored_plan(&plans);
        let (_b, second) = stored_plan(&plans);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let confirmer = Arc::new(Blocking {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        });
        // Feed the server line by line through a pipe-like reader.
        let (line_tx, line_rx) = mpsc::channel::<Vec<u8>>();
        struct Pipe(mpsc::Receiver<Vec<u8>>, Vec<u8>);
        impl std::io::Read for Pipe {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.1.is_empty() {
                    match self.0.recv() {
                        Ok(data) => self.1 = data,
                        Err(_) => return Ok(0),
                    }
                }
                let n = buf.len().min(self.1.len());
                buf[..n].copy_from_slice(&self.1[..n]);
                self.1.drain(..n);
                Ok(n)
            }
        }
        let out = Buffer::default();
        let server = {
            let out = out.clone();
            let plans = plans.clone();
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(Pipe(line_rx, Vec::new()));
                serve_with_store(reader, out, config(true, confirmer), plans)
            })
        };
        let feed = |v: Value| line_tx.send(format!("{v}\n").into_bytes()).unwrap();
        let wait_for = |id: i64| {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(line) = out.lines().into_iter().find(|l| l["id"] == id) {
                    return line;
                }
                assert!(Instant::now() < deadline, "no response {id}");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        feed(write_call(1, &first));
        entered_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("first write reaches the dialog");
        feed(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
        assert_eq!(wait_for(2)["result"], json!({}));
        feed(write_call(3, &second));
        assert_eq!(error_code(&wait_for(3)), "busy");
        assert_eq!(plans.len(), 1, "a busy call does not consume its plan");
        release_tx.send(()).unwrap();
        assert_eq!(error_code(&wait_for(1)), "not_confirmed");
        drop(line_tx);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn progress_is_throttled_and_monotonic() {
        let out = Buffer::default();
        let reporter = ProgressReporter::new(json!("tok"), Arc::new(Mutex::new(out.clone())));
        let p = |phase, done, total| Progress { phase, done, total };
        reporter.report(p(Phase::Writing, 0, 1000));
        reporter.report(p(Phase::Writing, 10, 1000)); // throttled
        reporter.report(p(Phase::Writing, 1000, 1000));
        reporter.report(p(Phase::Verifying, 0, 1000)); // same value as before: skipped
        reporter.report(p(Phase::Verifying, 1000, 1000));
        let values: Vec<u64> = out
            .lines()
            .iter()
            .map(|n| {
                assert_eq!(n["method"], "notifications/progress");
                assert_eq!(n["params"]["progressToken"], "tok");
                n["params"]["progress"].as_u64().unwrap()
            })
            .collect();
        assert_eq!(values, [0, 1000, 2000]);
    }
}
