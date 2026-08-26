use crate::notification_router::{NotificationRouter, NotificationRouterConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot, Mutex, Notify, OnceCell};
use tokio::time::timeout;

#[derive(Clone, Debug, PartialEq)]
pub struct CodexRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodexRequestError {
    Transport(String),
    Rpc(CodexRpcError),
}

impl std::fmt::Display for CodexRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(message) => formatter.write_str(message),
            Self::Rpc(error) => write!(
                formatter,
                "Codex app-server request failed with JSON-RPC code {}: {}",
                error.code, error.message
            ),
        }
    }
}

impl std::error::Error for CodexRequestError {}

impl CodexRequestError {
    pub fn is_thread_not_found(&self) -> bool {
        matches!(self, Self::Rpc(error) if error.is_thread_not_found())
    }
}

impl CodexRpcError {
    /// Codex app-server 0.146 reports a missing persisted rollout with the
    /// standard invalid-request code and this protocol-owned message prefix.
    /// Keep the check exact and fail closed for every other RPC failure.
    pub fn is_thread_not_found(&self) -> bool {
        self.code == -32600
            && self.data.is_none()
            && self.message.starts_with("no rollout found for thread id ")
    }
}

#[derive(Clone, Debug)]
pub struct CodexProcessConfig {
    pub command: String,
    pub request_timeout: Duration,
}

impl Default for CodexProcessConfig {
    fn default() -> Self {
        Self {
            command: std::env::var("CADGEN_AX_CODEX_COMMAND")
                .map(|command| resolve_executable(&command).unwrap_or(command))
                .unwrap_or_else(|_| {
                    resolve_executable("codex").unwrap_or_else(|| "codex".to_string())
                }),
            request_timeout: duration_from_env("CADGEN_AX_CODEX_REQUEST_TIMEOUT_SECS", 600),
        }
    }
}

#[derive(Clone)]
pub struct CodexProcessClient {
    config: CodexProcessConfig,
    state: Arc<Mutex<Option<Arc<ProcessState>>>>,
    next_request_id: Arc<AtomicU64>,
    latest_connection_generation: Arc<AtomicU64>,
    notifications: broadcast::Sender<Value>,
    notification_router: NotificationRouter,
}

struct ProcessState {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CodexRequestError>>>>>,
    initialization: OnceCell<Result<(), String>>,
    initialization_started: AtomicBool,
    initialization_ready: Notify,
    generation: u64,
}

impl CodexProcessClient {
    pub fn new(config: CodexProcessConfig) -> Self {
        let (notifications, _) = broadcast::channel(256);
        let notification_router = NotificationRouter::new(NotificationRouterConfig::default())
            .expect("default notification router configuration must be valid");
        Self {
            config,
            state: Arc::new(Mutex::new(None)),
            next_request_id: Arc::new(AtomicU64::new(1)),
            latest_connection_generation: Arc::new(AtomicU64::new(0)),
            notifications,
            notification_router,
        }
    }

    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    pub fn notification_router(&self) -> NotificationRouter {
        self.notification_router.clone()
    }

    pub fn latest_connection_generation(&self) -> u64 {
        self.latest_connection_generation.load(Ordering::SeqCst)
    }

    pub async fn current_connection_generation(&self) -> Option<u64> {
        self.state
            .lock()
            .await
            .as_ref()
            .map(|state| state.generation)
    }

    /// Compatibility alias. New run paths should call `ensure_initialized`.
    pub async fn initialize(&self) -> Result<(), String> {
        self.ensure_initialized().await
    }

    pub async fn ensure_initialized(&self) -> Result<(), String> {
        let process_state = self.ensure_started().await?;
        if let Some(result) = process_state.initialization.get() {
            return result.clone();
        }
        if process_state
            .initialization_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let client = self.clone();
            let initializing_state = Arc::clone(&process_state);
            tokio::spawn(async move {
                let result = client.initialize_connection(&initializing_state).await;
                let initialized = initializing_state.initialization.set(result).is_ok();
                initializing_state.initialization_ready.notify_waiters();
                assert!(
                    initialized,
                    "connection initialization completed more than once"
                );
            });
        }
        loop {
            let notified = process_state.initialization_ready.notified();
            if let Some(result) = process_state.initialization.get() {
                return result.clone();
            }
            notified.await;
        }
    }

    async fn initialize_connection(&self, process_state: &Arc<ProcessState>) -> Result<(), String> {
        self.request_on_connection(
            process_state,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "cadgen-ax-tauri-backend",
                    "title": "CADGEN-AX",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
        self.notify_on_connection(process_state, "initialized", json!({}))
            .await
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_detailed(method, params)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn request_detailed(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, CodexRequestError> {
        let process_state = self
            .ensure_started()
            .await
            .map_err(CodexRequestError::Transport)?;
        self.request_on_connection(&process_state, method, params)
            .await
    }

    async fn request_on_connection(
        &self,
        process_state: &Arc<ProcessState>,
        method: &str,
        params: Value,
    ) -> Result<Value, CodexRequestError> {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        process_state.pending.lock().await.insert(id, sender);
        if let Err(error) = self
            .write_json(
                &process_state.stdin,
                json!({ "id": id, "method": method, "params": params }),
            )
            .await
        {
            process_state.pending.lock().await.remove(&id);
            return Err(CodexRequestError::Transport(format!(
                "Failed to write Codex app-server request {method}: {error}"
            )));
        }
        match timeout(self.config.request_timeout, receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Err(CodexRequestError::Transport(format!(
                "Codex app-server response channel closed for {method}."
            ))),
            Err(_) => {
                process_state.pending.lock().await.remove(&id);
                Err(CodexRequestError::Transport(format!(
                    "Timed out waiting for Codex app-server response to {method}."
                )))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let process_state = self.ensure_started().await?;
        self.notify_on_connection(&process_state, method, params)
            .await
    }

    async fn notify_on_connection(
        &self,
        process_state: &Arc<ProcessState>,
        method: &str,
        params: Value,
    ) -> Result<(), String> {
        self.write_json(
            &process_state.stdin,
            json!({ "method": method, "params": params }),
        )
        .await
        .map_err(|error| format!("Failed to write Codex app-server notification {method}: {error}"))
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let process_state = self.state.lock().await.take();
        if let Some(process_state) = process_state {
            fail_pending_requests(
                &process_state.pending,
                "Codex app-server was shut down.".to_string(),
            )
            .await;
            process_state
                .child
                .lock()
                .await
                .kill()
                .await
                .map_err(|error| format!("Failed to stop codex app-server: {error}"))?;
        }
        Ok(())
    }

    async fn ensure_started(&self) -> Result<Arc<ProcessState>, String> {
        let mut state = self.state.lock().await;
        if let Some(process_state) = state.as_ref() {
            return Ok(Arc::clone(process_state));
        }
        let mut command = Command::new(&self.config.command);
        command
            .args(build_app_server_args(&self.config.command))
            .env("PATH", child_path_env())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        let mut child = command.spawn().map_err(|error| {
            format!(
                "Failed to start codex app-server with {:?}: {error}",
                self.config.command
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "codex app-server stdin is not available.".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "codex app-server stdout is not available.".to_string())?;
        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CodexRequestError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let generation = self
            .latest_connection_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let process_state = Arc::new(ProcessState {
            child: Mutex::new(child),
            stdin,
            pending,
            initialization: OnceCell::new(),
            initialization_started: AtomicBool::new(false),
            initialization_ready: Notify::new(),
            generation,
        });
        *state = Some(Arc::clone(&process_state));
        spawn_reader(
            stdout,
            Arc::clone(&process_state),
            self.notifications.clone(),
            self.notification_router.clone(),
            Arc::clone(&self.state),
        );
        Ok(process_state)
    }

    async fn write_json(
        &self,
        stdin: &Arc<Mutex<ChildStdin>>,
        payload: Value,
    ) -> Result<(), String> {
        let mut stdin = stdin.lock().await;
        stdin
            .write_all(format!("{payload}\n").as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        stdin.flush().await.map_err(|error| error.to_string())
    }
}

fn spawn_reader(
    stdout: tokio::process::ChildStdout,
    process_state: Arc<ProcessState>,
    notifications: broadcast::Sender<Value>,
    notification_router: NotificationRouter,
    client_state: Arc<Mutex<Option<Arc<ProcessState>>>>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let connection_error = loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => {
                    break format!(
                        "Codex app-server connection generation {} closed stdout.",
                        process_state.generation
                    )
                }
                Err(error) => {
                    break format!(
                        "Failed to read Codex app-server connection generation {}: {error}",
                        process_state.generation
                    )
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let message = match serde_json::from_str::<Value>(&line) {
                Ok(message) => message,
                Err(error) => {
                    break format!(
                    "Codex app-server emitted invalid JSON on connection generation {}: {error}",
                    process_state.generation
                )
                }
            };
            if let Some(id) = message.get("id").and_then(Value::as_u64) {
                let Some(sender) = process_state.pending.lock().await.remove(&id) else {
                    break format!(
                        "Codex app-server returned response id {id}, but no request is pending on connection generation {}.",
                        process_state.generation
                    );
                };
                let response = if let Some(error) = message.get("error") {
                    let description = error
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| error.to_string());
                    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                    Err(CodexRequestError::Rpc(CodexRpcError {
                        code,
                        message: description,
                        data: error.get("data").cloned(),
                    }))
                } else if let Some(result) = message.get("result") {
                    Ok(result.clone())
                } else {
                    Err(CodexRequestError::Transport(format!(
                        "Codex app-server response {id} contains neither result nor error."
                    )))
                };
                let _ = sender.send(response);
                continue;
            }
            if message.get("method").is_some() {
                if let Err(error) = notification_router.route(message.clone()) {
                    break format!(
                        "Codex notification routing failed on connection generation {}: {error}",
                        process_state.generation
                    );
                }
                let _ = notifications.send(message);
                continue;
            }
            break format!(
                "Codex app-server emitted a message with neither a numeric id nor method on connection generation {}.",
                process_state.generation
            );
        };

        fail_pending_requests(&process_state.pending, connection_error.clone()).await;
        if let Err(error) = notification_router.fail_all_routes() {
            eprintln!(
                "[cadgen-ax:codex-process] generation={} failed to invalidate notification routes: {error}",
                process_state.generation
            );
        }
        let mut state = client_state.lock().await;
        if state
            .as_ref()
            .is_some_and(|current| current.generation == process_state.generation)
        {
            *state = None;
        }
        drop(state);
        if let Err(error) = process_state.child.lock().await.kill().await {
            eprintln!(
                "[cadgen-ax:codex-process] generation={} cleanup_error={error}",
                process_state.generation
            );
        }
    });
}

async fn fail_pending_requests(
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CodexRequestError>>>>>,
    error: String,
) {
    let requests = {
        let mut pending = pending.lock().await;
        pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>()
    };
    for sender in requests {
        let _ = sender.send(Err(CodexRequestError::Transport(error.clone())));
    }
}

fn build_app_server_args(command: &str) -> Vec<String> {
    let mut args = vec!["--listen".to_string(), "stdio://".to_string()];
    if is_standalone_app_server_command(command) {
        return args;
    }
    args.insert(0, "app-server".to_string());
    args
}

fn is_standalone_app_server_command(command: &str) -> bool {
    let stem = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
        .to_lowercase();
    stem == "codex-app-server"
        || stem.starts_with("codex-app-server-aarch64-")
        || stem.starts_with("codex-app-server-x86_64-")
}

fn resolve_executable(command: &str) -> Option<String> {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(command).exists().then(|| command.to_string());
    }
    resolve_executable_in_path(command, &child_path_env())
        .map(|path| path.to_string_lossy().to_string())
}

fn resolve_executable_in_path(command: &str, path_env: &str) -> Option<PathBuf> {
    std::env::split_paths(path_env)
        .map(|directory| directory.join(command))
        .find(|path| path.is_file())
}

fn child_path_env() -> String {
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    for path in cadgen_ax_binary_path_entries() {
        push_unique_path(&mut paths, path);
    }
    for path in configured_extra_path_entries(std::env::var_os("CADGEN_AX_CODEX_EXTRA_PATHS")) {
        push_unique_path(&mut paths, path);
    }
    for path in login_shell_path_entries() {
        push_unique_path(&mut paths, path);
    }
    std::env::join_paths(paths)
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
        .to_string_lossy()
        .to_string()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn cadgen_ax_binary_path_entries() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            paths.push(parent.to_path_buf());
            if parent.file_name().is_some_and(|name| name == "deps") {
                if let Some(debug_dir) = parent.parent() {
                    paths.push(debug_dir.to_path_buf());
                }
            }
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        paths.push(current_dir.join("src-tauri").join("target").join("debug"));
        paths.push(current_dir.join("target").join("debug"));
    }
    paths
        .into_iter()
        .filter(|path| path.is_dir())
        .fold(Vec::new(), |mut unique, path| {
            if !unique.iter().any(|existing| existing == &path) {
                unique.push(path);
            }
            unique
        })
}

fn configured_extra_path_entries(value: Option<OsString>) -> Vec<PathBuf> {
    value
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default()
}

fn login_shell_path_entries() -> Vec<PathBuf> {
    let Some(shell) = std::env::var_os("SHELL").filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let Ok(output) = std::process::Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    std::env::split_paths(&OsString::from(
        String::from_utf8_lossy(&output.stdout).as_ref(),
    ))
    .collect()
}

fn duration_from_env(name: &str, default_secs: u64) -> Duration {
    match std::env::var(name) {
        Ok(value) => match value.parse::<u64>() {
            Ok(seconds) if seconds > 0 => Duration::from_secs(seconds),
            _ => panic!("{name} must be a positive integer number of seconds."),
        },
        Err(std::env::VarError::NotPresent) => Duration::from_secs(default_secs),
        Err(error) => panic!("Failed to read {name}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_the_installed_protocol_missing_rollout_error_as_thread_not_found() {
        let missing = CodexRequestError::Rpc(CodexRpcError {
            code: -32600,
            message: "no rollout found for thread id 019fffff-ffff-7fff-8fff-ffffffffffff"
                .to_string(),
            data: None,
        });
        assert!(missing.is_thread_not_found());
        assert!(!CodexRequestError::Rpc(CodexRpcError {
            code: -32600,
            message: "invalid session id".to_string(),
            data: None,
        })
        .is_thread_not_found());
        assert!(
            !CodexRequestError::Transport("connection closed".to_string()).is_thread_not_found()
        );
    }

    #[cfg(unix)]
    fn test_app_server() -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "cadgen-ax-codex-process-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let marker = directory.join("requests.log");
        let executable = directory.join("codex-app-server");
        let script = format!(
            r#"#!/bin/sh
marker={marker:?}
while IFS= read -r line; do
  case "$line" in
    *\"method\":\"initialize\"*)
      printf '%s\n' initialize >> "$marker"
      id=$(printf '%s\n' "$line" | sed -n 's/.*\"id\":\([0-9][0-9]*\).*/\1/p')
      case "$line" in
        *\"capabilities\":{{\"experimentalApi\":true}}*)
          printf '{{\"id\":%s,\"result\":{{}}}}\n' "$id"
          ;;
        *)
          printf '{{\"id\":%s,\"error\":{{\"code\":-32600,\"message\":\"missing experimentalApi capability\"}}}}\n' "$id"
          ;;
      esac
      ;;
    *\"method\":\"initialized\"*)
      printf '%s\n' initialized >> "$marker"
      ;;
  esac
done
"#,
            marker = marker
        );
        std::fs::write(&executable, script).unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
        (executable, marker)
    }

    #[test]
    fn builds_cli_app_server_args() {
        assert_eq!(
            build_app_server_args("codex"),
            vec!["app-server", "--listen", "stdio://"]
        );
        assert_eq!(
            build_app_server_args("codex-app-server"),
            vec!["--listen", "stdio://"]
        );
    }

    #[test]
    fn resolves_executable_from_supplied_path() {
        let directory = std::env::temp_dir().join(format!(
            "cadgen-ax-codex-path-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("codex");
        std::fs::write(&executable, "#!/bin/sh\n").unwrap();

        assert_eq!(
            resolve_executable_in_path("codex", &directory.to_string_lossy()),
            Some(executable)
        );

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn configured_extra_path_entries_split_platform_path_values() {
        let first =
            std::env::temp_dir().join(format!("cadgen-ax-extra-path-a-{}", uuid::Uuid::new_v4()));
        let second =
            std::env::temp_dir().join(format!("cadgen-ax-extra-path-b-{}", uuid::Uuid::new_v4()));
        let joined = std::env::join_paths([first.as_path(), second.as_path()]).unwrap();

        assert_eq!(
            configured_extra_path_entries(Some(joined)),
            vec![first, second]
        );
    }

    #[test]
    fn child_path_env_includes_cadgen_ax_debug_binary_directory() {
        let path = child_path_env();
        let current_exe = std::env::current_exe().unwrap();
        let debug_dir = current_exe
            .parent()
            .and_then(|parent| {
                if parent.file_name().is_some_and(|name| name == "deps") {
                    parent.parent()
                } else {
                    Some(parent)
                }
            })
            .unwrap()
            .to_string_lossy()
            .to_string();

        assert!(path.contains(&debug_dir));
    }

    #[tokio::test]
    async fn shutdown_is_safe_before_start() {
        let client = CodexProcessClient::new(CodexProcessConfig {
            command: "codex".to_string(),
            request_timeout: Duration::from_millis(10),
        });
        client.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_initialized_is_single_flight_per_connection() {
        let (executable, marker) = test_app_server();
        let test_directory = executable.parent().unwrap().to_path_buf();
        let client = CodexProcessClient::new(CodexProcessConfig {
            command: executable.to_string_lossy().to_string(),
            request_timeout: Duration::from_secs(2),
        });

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            tasks.push(tokio::spawn(
                async move { client.ensure_initialized().await },
            ));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        client.initialize().await.unwrap();

        let requests = timeout(Duration::from_secs(2), async {
            loop {
                let requests = std::fs::read_to_string(&marker).unwrap_or_default();
                if requests.lines().count() == 2 {
                    break requests;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            requests
                .lines()
                .filter(|line| *line == "initialize")
                .count(),
            1
        );
        assert_eq!(
            requests
                .lines()
                .filter(|line| *line == "initialized")
                .count(),
            1
        );
        assert_eq!(client.latest_connection_generation(), 1);
        assert_eq!(client.current_connection_generation().await, Some(1));

        client.shutdown().await.unwrap();
        std::fs::remove_dir_all(test_directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_exit_fails_pending_request_and_next_request_starts_new_generation() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "cadgen-ax-codex-crash-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("codex-app-server");
        let count_file = directory.join("launch-count");
        let script = format!(
            r#"#!/bin/sh
count_file={count_file:?}
count=0
[ -f "$count_file" ] && count=$(cat "$count_file")
count=$((count + 1))
printf '%s' "$count" > "$count_file"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *\"method\":\"initialize\"*) printf '{{"id":%s,"result":{{}}}}\n' "$id" ;;
    *\"method\":\"crash/request\"*) exit 17 ;;
    *\"method\":\"healthy/request\"*) printf '{{"id":%s,"result":{{"ok":true}}}}\n' "$id" ;;
  esac
done
"#,
            count_file = count_file
        );
        std::fs::write(&executable, script).unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let client = CodexProcessClient::new(CodexProcessConfig {
            command: executable.to_string_lossy().to_string(),
            request_timeout: Duration::from_secs(2),
        });
        client.ensure_initialized().await.unwrap();
        let error = client
            .request_detailed("crash/request", json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(error, CodexRequestError::Transport(message) if message.contains("closed stdout"))
        );
        client.ensure_initialized().await.unwrap();
        let result = client.request("healthy/request", json!({})).await.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(client.latest_connection_generation(), 2);
        client.shutdown().await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
