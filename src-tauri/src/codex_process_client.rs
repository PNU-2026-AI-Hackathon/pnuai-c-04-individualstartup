use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct CodexProcessConfig {
    pub command: String,
    pub request_timeout: Duration,
}

impl Default for CodexProcessConfig {
    fn default() -> Self {
        Self {
            command: std::env::var("CADASTROPHE_CODEX_COMMAND")
                .map(|command| resolve_executable(&command).unwrap_or(command))
                .unwrap_or_else(|_| {
                    resolve_executable("codex").unwrap_or_else(|| "codex".to_string())
                }),
            request_timeout: duration_from_env("CADASTROPHE_CODEX_REQUEST_TIMEOUT_SECS", 30),
        }
    }
}

#[derive(Clone)]
pub struct CodexProcessClient {
    config: CodexProcessConfig,
    state: Arc<Mutex<Option<ProcessState>>>,
    next_request_id: Arc<AtomicU64>,
    notifications: broadcast::Sender<Value>,
}

struct ProcessState {
    #[allow(dead_code)]
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
}

impl CodexProcessClient {
    pub fn new(config: CodexProcessConfig) -> Self {
        let (notifications, _) = broadcast::channel(256);
        Self {
            config,
            state: Arc::new(Mutex::new(None)),
            next_request_id: Arc::new(AtomicU64::new(1)),
            notifications,
        }
    }

    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    pub async fn initialize(&self) -> Result<(), String> {
        self.ensure_started().await?;
        let _ = self
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "cadastrophe-tauri-backend",
                        "title": "Cadastrophe",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {}
                }),
            )
            .await?;
        self.notify("initialized", json!({})).await
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ensure_started().await?;
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        let stdin = {
            let mut state = self.state.lock().await;
            let process_state = state
                .as_mut()
                .ok_or_else(|| "codex app-server is not running.".to_string())?;
            process_state.pending.lock().await.insert(id, sender);
            Arc::clone(&process_state.stdin)
        };
        self.write_json(
            &stdin,
            json!({ "id": id, "method": method, "params": params }),
        )
        .await?;
        timeout(self.config.request_timeout, receiver)
            .await
            .map_err(|_| format!("Timed out waiting for Codex app-server response to {method}."))?
            .map_err(|_| format!("Codex app-server response channel closed for {method}."))?
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.ensure_started().await?;
        let stdin = {
            let mut state = self.state.lock().await;
            let process_state = state
                .as_mut()
                .ok_or_else(|| "codex app-server is not running.".to_string())?;
            Arc::clone(&process_state.stdin)
        };
        self.write_json(&stdin, json!({ "method": method, "params": params }))
            .await
    }

    #[allow(dead_code)]
    pub async fn shutdown(&self) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if let Some(mut process_state) = state.take() {
            let _ = process_state.child.kill().await;
        }
        Ok(())
    }

    async fn ensure_started(&self) -> Result<(), String> {
        if self.state.lock().await.is_some() {
            return Ok(());
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
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        spawn_reader(stdout, Arc::clone(&pending), self.notifications.clone());
        *self.state.lock().await = Some(ProcessState {
            child,
            stdin,
            pending,
        });
        Ok(())
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
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    notifications: broadcast::Sender<Value>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(id) = message.get("id").and_then(Value::as_u64) {
                if let Some(sender) = pending.lock().await.remove(&id) {
                    let response = if let Some(error) = message.get("error") {
                        Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Codex app-server request failed.")
                            .to_string())
                    } else {
                        Ok(message.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = sender.send(response);
                }
                continue;
            }
            if message.get("method").is_some() {
                let _ = notifications.send(message);
            }
        }
    });
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
    for fallback in fallback_path_entries() {
        let path = PathBuf::from(fallback);
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    std::env::join_paths(paths)
        .unwrap_or_else(|_| std::ffi::OsString::from(fallback_path_entries().join(":")))
        .to_string_lossy()
        .to_string()
}

fn fallback_path_entries() -> Vec<&'static str> {
    vec![
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ]
}

fn duration_from_env(name: &str, default_secs: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "cadastrophe-codex-path-test-{}",
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
    fn child_path_env_includes_homebrew_locations() {
        let path = child_path_env();
        assert!(path.contains("/opt/homebrew/bin"));
        assert!(path.contains("/usr/local/bin"));
    }

    #[tokio::test]
    async fn shutdown_is_safe_before_start() {
        let client = CodexProcessClient::new(CodexProcessConfig {
            command: "codex".to_string(),
            request_timeout: Duration::from_millis(10),
        });
        client.shutdown().await.unwrap();
    }
}
