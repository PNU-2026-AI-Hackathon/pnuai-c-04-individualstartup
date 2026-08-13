use crate::codex_process_client::{CodexProcessClient, CodexRequestError};
use crate::notification_router::{
    NotificationRouteKey, NotificationRouter, PendingRouteHandle, RoutedNotification,
};
use crate::protocol::{
    CadAgentPlane, CadAgentRecoveryStatus, CadAgentRunHistoryOutcome,
    CadAgentRunHistoryRecoveryInput, CadAgentRunStatus, CadAgentThread, CadAgentThreadStatus,
    CadConversationPhase, CadRecoveredAgentMessage, Metadata, StartNewAgentConversationResult,
    ThreadScope,
};
use crate::session_service::{timestamp, SessionService};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};
use uuid::Uuid;

const EXTERNAL_AGENT: &str = "codex";

#[derive(Clone, Debug)]
pub struct AgentThreadManagerConfig {
    pub interrupt_terminal_timeout: Duration,
}

impl Default for AgentThreadManagerConfig {
    fn default() -> Self {
        Self {
            interrupt_terminal_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartManagedTurn {
    pub session_id: String,
    pub run_id: String,
    pub thread_start_params: Value,
    /// Complete `turn/start` parameters except `threadId`.
    pub turn_start_params: Value,
    /// Context appended to the first text input when a missing persisted thread is replaced.
    pub replacement_context: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ScopedTurnBinding {
    pub scope: ThreadScope,
    pub agent_thread_id: String,
    pub external_thread_id: String,
    pub external_turn_id: String,
    pub connection_generation: u64,
}

pub type ScopedTurnBindCallback =
    Arc<dyn Fn(&ScopedTurnBinding) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct StartScopedTurn {
    pub scope: ThreadScope,
    pub thread_start_params: Value,
    /// Complete `turn/start` parameters except `threadId`.
    pub turn_start_params: Value,
    /// Atomically binds the owning plane record after the external turn route exists.
    pub bind: ScopedTurnBindCallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedThreadActivation {
    Started,
    Reused,
    Resumed,
    Replaced { previous_external_thread_id: String },
}

pub struct ManagedAgentTurn {
    pub scope: ThreadScope,
    pub session_id: String,
    pub run_id: String,
    pub agent_thread_id: String,
    pub external_thread_id: String,
    pub external_turn_id: String,
    pub connection_generation: u64,
    pub activation: ManagedThreadActivation,
    pub notifications: mpsc::Receiver<RoutedNotification>,
    route_key: NotificationRouteKey,
    router: NotificationRouter,
    lease: Option<ScopedTurnLease>,
}

impl ManagedAgentTurn {
    fn release_route_and_lease(&mut self) -> Result<(), AgentThreadManagerError> {
        self.router
            .unregister_route(&self.route_key)
            .map_err(|error| AgentThreadManagerError::Routing(error.to_string()))?;
        self.lease.take();
        Ok(())
    }
}

impl Drop for ManagedAgentTurn {
    fn drop(&mut self) {
        // Route cleanup is best-effort in Drop only. Call `finish_turn` to observe errors.
        let _ = self.router.unregister_route(&self.route_key);
        self.lease.take();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveredTurnStatus {
    Completed,
    Failed { message: String },
    Interrupted,
    InProgress,
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredAgentMessage {
    pub external_item_id: String,
    pub text: String,
    pub phase: Option<CadConversationPhase>,
    pub sequence: u64,
    pub is_final: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnReconciliation {
    pub status: RecoveredTurnStatus,
    pub messages: Vec<RecoveredAgentMessage>,
}

#[derive(Debug)]
pub struct InterruptReconciliation {
    /// Notifications observed after the interrupt request. The collector must ingest these.
    pub observed_notifications: Vec<RoutedNotification>,
    pub reconciliation: TurnReconciliation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentThreadManagerError {
    InvalidInput(String),
    Persistence(String),
    Transport(String),
    Routing(String),
    Protocol(String),
    ActiveTurn(String),
}

impl fmt::Display for AgentThreadManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::Persistence(message)
            | Self::Transport(message)
            | Self::Routing(message)
            | Self::Protocol(message)
            | Self::ActiveTurn(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AgentThreadManagerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportRequestError {
    ThreadNotFound,
    Rejected(String),
    Connection(String),
}

#[async_trait]
pub(crate) trait AgentThreadTransport: Send + Sync {
    async fn ensure_initialized(&self) -> Result<(), String>;
    async fn current_connection_generation(&self) -> Option<u64>;
    fn notification_router(&self) -> NotificationRouter;
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportRequestError>;
}

#[async_trait]
impl AgentThreadTransport for CodexProcessClient {
    async fn ensure_initialized(&self) -> Result<(), String> {
        CodexProcessClient::ensure_initialized(self).await
    }

    async fn current_connection_generation(&self) -> Option<u64> {
        CodexProcessClient::current_connection_generation(self).await
    }

    fn notification_router(&self) -> NotificationRouter {
        CodexProcessClient::notification_router(self)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportRequestError> {
        self.request_detailed(method, params)
            .await
            .map_err(|error| {
                if error.is_thread_not_found() {
                    TransportRequestError::ThreadNotFound
                } else {
                    match error {
                        CodexRequestError::Rpc(error) => TransportRequestError::Rejected(format!(
                            "JSON-RPC code {}: {}",
                            error.code, error.message
                        )),
                        CodexRequestError::Transport(message) => {
                            TransportRequestError::Connection(message)
                        }
                    }
                }
            })
    }
}

#[derive(Clone)]
pub struct AgentThreadManager {
    service: Arc<SessionService>,
    transport: Arc<dyn AgentThreadTransport>,
    router: NotificationRouter,
    config: AgentThreadManagerConfig,
    runtime: Arc<Mutex<ManagerRuntime>>,
}

#[derive(Default)]
struct ManagerRuntime {
    active_scopes: HashSet<ScopeLeaseKey>,
    loaded_threads: HashSet<(u64, String)>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScopeLeaseKey {
    session_id: String,
    plane: &'static str,
}

impl From<&ThreadScope> for ScopeLeaseKey {
    fn from(scope: &ThreadScope) -> Self {
        Self {
            session_id: scope.session_id.clone(),
            plane: match scope.plane {
                CadAgentPlane::Modeling => "modeling",
                CadAgentPlane::Validation => "validation",
            },
        }
    }
}

struct ScopedTurnLease {
    key: ScopeLeaseKey,
    runtime: Arc<Mutex<ManagerRuntime>>,
}

impl Drop for ScopedTurnLease {
    fn drop(&mut self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active_scopes.remove(&self.key);
        }
    }
}

impl AgentThreadManager {
    pub fn new(
        service: Arc<SessionService>,
        client: CodexProcessClient,
        config: AgentThreadManagerConfig,
    ) -> Result<Self, AgentThreadManagerError> {
        Self::with_transport(service, Arc::new(client), config)
    }

    pub(crate) fn with_transport(
        service: Arc<SessionService>,
        transport: Arc<dyn AgentThreadTransport>,
        config: AgentThreadManagerConfig,
    ) -> Result<Self, AgentThreadManagerError> {
        if config.interrupt_terminal_timeout.is_zero() {
            return Err(AgentThreadManagerError::InvalidInput(
                "Interrupt terminal timeout must be greater than zero.".to_string(),
            ));
        }
        let router = transport.notification_router();
        Ok(Self {
            service,
            transport,
            router,
            config,
            runtime: Arc::new(Mutex::new(ManagerRuntime::default())),
        })
    }

    pub async fn start_turn(
        &self,
        input: StartManagedTurn,
    ) -> Result<ManagedAgentTurn, AgentThreadManagerError> {
        validate_start_input(&input)?;
        let scope = modeling_scope(&input.session_id);
        let lease = self.acquire_scope_lease(&scope)?;
        let run = self
            .service
            .get_agent_run(&input.session_id, &input.run_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .ok_or_else(|| {
                AgentThreadManagerError::Persistence(format!(
                    "Agent run not found: {}",
                    input.run_id
                ))
            })?;
        if run.external_turn_id.is_some() {
            return Err(AgentThreadManagerError::InvalidInput(format!(
                "Agent run {} is already bound to an external turn.",
                input.run_id
            )));
        }
        if matches!(
            run.status,
            CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
        ) {
            return Err(AgentThreadManagerError::InvalidInput(format!(
                "Terminal agent run {} cannot start a Codex turn.",
                input.run_id
            )));
        }

        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        let generation = self
            .transport
            .current_connection_generation()
            .await
            .ok_or_else(|| {
                AgentThreadManagerError::Transport(
                    "Codex process has no active connection after initialization.".to_string(),
                )
            })?;

        let (mut thread, activation) = self.activate_thread(&input, generation).await?;
        thread.status = CadAgentThreadStatus::Active;
        thread.updated_at = timestamp();
        self.service
            .upsert_agent_thread(thread.clone())
            .map_err(AgentThreadManagerError::Persistence)?;

        let pending = match self
            .router
            .begin_pending_route(thread.external_thread_id.clone())
        {
            Ok(pending) => pending,
            Err(error) => {
                self.set_thread_status(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(AgentThreadManagerError::Routing(error.to_string()));
            }
        };
        let turn_params = match prepare_turn_params(
            input.turn_start_params,
            &thread.external_thread_id,
            matches!(activation, ManagedThreadActivation::Replaced { .. })
                .then_some(input.replacement_context)
                .flatten(),
        ) {
            Ok(params) => params,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.set_thread_status(&mut thread, CadAgentThreadStatus::Ready)?;
                return Err(error);
            }
        };
        let turn_response = match self.transport.request("turn/start", turn_params).await {
            Ok(response) => response,
            Err(error) => {
                self.cancel_pending(&pending)?;
                let status = match error {
                    TransportRequestError::Rejected(_) | TransportRequestError::ThreadNotFound => {
                        CadAgentThreadStatus::Ready
                    }
                    TransportRequestError::Connection(_) => CadAgentThreadStatus::Failed,
                };
                self.set_thread_status(&mut thread, status)?;
                return Err(map_transport_error("turn/start", error));
            }
        };
        let turn_id = match required_nested_id(&turn_response, "turn", "turn/start") {
            Ok(turn_id) => turn_id,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.set_thread_status(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(error);
            }
        };
        let notifications = match self
            .router
            .promote_pending_route(pending.clone(), turn_id.clone())
        {
            Ok(receiver) => receiver,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.set_thread_status(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(AgentThreadManagerError::Routing(error.to_string()));
            }
        };
        if let Err(error) = self.service.bind_agent_run_to_thread(
            &input.session_id,
            &input.run_id,
            &thread.id,
            Some(turn_id.clone()),
            Some(generation),
            recovery_status_for_activation(&activation),
        ) {
            self.router
                .unregister_route(&NotificationRouteKey::new(
                    &thread.external_thread_id,
                    &turn_id,
                ))
                .map_err(|route_error| AgentThreadManagerError::Routing(route_error.to_string()))?;
            self.set_thread_status(&mut thread, CadAgentThreadStatus::Failed)?;
            return Err(AgentThreadManagerError::Persistence(error));
        }

        Ok(ManagedAgentTurn {
            scope,
            session_id: input.session_id,
            run_id: input.run_id,
            agent_thread_id: thread.id,
            external_thread_id: thread.external_thread_id.clone(),
            external_turn_id: turn_id.clone(),
            connection_generation: generation,
            activation,
            notifications,
            route_key: NotificationRouteKey::new(thread.external_thread_id, turn_id),
            router: self.router.clone(),
            lease: Some(lease),
        })
    }

    /// Starts a fresh Codex thread and turn for a non-modeling owner. This path
    /// deliberately has no resume, reuse, or replacement behavior.
    pub async fn start_scoped_turn(
        &self,
        input: StartScopedTurn,
    ) -> Result<ManagedAgentTurn, AgentThreadManagerError> {
        validate_scoped_start_input(&input)?;
        let lease = self.acquire_scope_lease(&input.scope)?;
        let active_in_plane = self
            .service
            .list_agent_threads(&input.scope.session_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .into_iter()
            .find(|thread| {
                thread.external_agent == EXTERNAL_AGENT
                    && thread.plane == input.scope.plane
                    && thread.archived_at.is_none()
                    && thread.replaced_by_id.is_none()
            });
        if let Some(active) = active_in_plane {
            return Err(AgentThreadManagerError::InvalidInput(format!(
                "Session {} already has active {:?} Codex thread {} owned by {}; scoped turns require an empty plane.",
                input.scope.session_id, input.scope.plane, active.id, active.owner_id
            )));
        }

        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        let generation = self
            .transport
            .current_connection_generation()
            .await
            .ok_or_else(|| {
                AgentThreadManagerError::Transport(
                    "Codex process has no active connection after initialization.".to_string(),
                )
            })?;
        let thread_response = self
            .transport
            .request("thread/start", input.thread_start_params)
            .await
            .map_err(|error| map_transport_error("thread/start", error))?;
        let external_thread_id = required_nested_id(&thread_response, "thread", "thread/start")?;
        let now = timestamp();
        let mut thread = CadAgentThread {
            id: Uuid::new_v4().to_string(),
            session_id: input.scope.session_id.clone(),
            plane: input.scope.plane.clone(),
            owner_id: input.scope.owner_id.clone(),
            external_agent: EXTERNAL_AGENT.to_string(),
            external_thread_id: external_thread_id.clone(),
            status: CadAgentThreadStatus::Active,
            connection_generation: Some(generation),
            created_at: now.clone(),
            updated_at: now,
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: None,
        };
        thread = self
            .service
            .upsert_agent_thread(thread)
            .map_err(|error| orphan_thread_error(&external_thread_id, error))?;
        self.mark_loaded(generation, &external_thread_id)?;

        let pending = match self.router.begin_pending_route(external_thread_id.clone()) {
            Ok(pending) => pending,
            Err(error) => {
                self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(AgentThreadManagerError::Routing(error.to_string()));
            }
        };
        let turn_params =
            match prepare_turn_params(input.turn_start_params, &external_thread_id, None) {
                Ok(params) => params,
                Err(error) => {
                    self.cancel_pending(&pending)?;
                    self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
                    return Err(error);
                }
            };
        let turn_response = match self.transport.request("turn/start", turn_params).await {
            Ok(response) => response,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(map_transport_error("turn/start", error));
            }
        };
        let turn_id = match required_nested_id(&turn_response, "turn", "turn/start") {
            Ok(turn_id) => turn_id,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(error);
            }
        };
        let route_key = NotificationRouteKey::new(&external_thread_id, &turn_id);
        let notifications = match self
            .router
            .promote_pending_route(pending.clone(), turn_id.clone())
        {
            Ok(receiver) => receiver,
            Err(error) => {
                self.cancel_pending(&pending)?;
                self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
                return Err(AgentThreadManagerError::Routing(error.to_string()));
            }
        };
        let binding = ScopedTurnBinding {
            scope: input.scope.clone(),
            agent_thread_id: thread.id.clone(),
            external_thread_id: external_thread_id.clone(),
            external_turn_id: turn_id.clone(),
            connection_generation: generation,
        };
        if let Err(error) = (input.bind)(&binding) {
            self.router
                .unregister_route(&route_key)
                .map_err(|route_error| AgentThreadManagerError::Routing(route_error.to_string()))?;
            self.archive_scoped_thread(&mut thread, CadAgentThreadStatus::Failed)?;
            return Err(AgentThreadManagerError::Persistence(error));
        }

        Ok(ManagedAgentTurn {
            scope: input.scope.clone(),
            session_id: input.scope.session_id,
            run_id: input.scope.owner_id,
            agent_thread_id: thread.id,
            external_thread_id,
            external_turn_id: turn_id,
            connection_generation: generation,
            activation: ManagedThreadActivation::Started,
            notifications,
            route_key,
            router: self.router.clone(),
            lease: Some(lease),
        })
    }

    pub async fn start_new_conversation(
        &self,
        session_id: &str,
        thread_start_params: Value,
    ) -> Result<StartNewAgentConversationResult, AgentThreadManagerError> {
        if session_id.trim().is_empty() {
            return Err(AgentThreadManagerError::InvalidInput(
                "Session id cannot be empty when starting a new agent conversation.".to_string(),
            ));
        }
        if !thread_start_params.is_object() {
            return Err(AgentThreadManagerError::InvalidInput(
                "thread_start_params must be an object.".to_string(),
            ));
        }
        let scope = modeling_scope(session_id);
        let _lease = self.acquire_scope_lease(&scope)?;
        let preparation = self
            .service
            .prepare_agent_thread_replacement(&scope, EXTERNAL_AGENT)
            .map_err(AgentThreadManagerError::Persistence)?;
        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        let generation = self
            .transport
            .current_connection_generation()
            .await
            .ok_or_else(|| {
                AgentThreadManagerError::Transport(
                    "Codex process has no active connection after initialization.".to_string(),
                )
            })?;
        let response = self
            .transport
            .request("thread/start", thread_start_params)
            .await
            .map_err(|error| map_transport_error("thread/start", error))?;
        let external_thread_id = required_nested_id(&response, "thread", "thread/start")?;
        let now = timestamp();
        let replacement = CadAgentThread {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            plane: CadAgentPlane::Modeling,
            owner_id: session_id.to_string(),
            external_agent: EXTERNAL_AGENT.to_string(),
            external_thread_id: external_thread_id.clone(),
            status: CadAgentThreadStatus::Ready,
            connection_generation: Some(generation),
            created_at: now.clone(),
            updated_at: now,
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: None,
        };

        self.mark_loaded(generation, &external_thread_id)
            .map_err(|error| orphan_thread_error(&external_thread_id, error.to_string()))?;
        let persisted = match preparation.active_thread {
            Some(previous) => self
                .service
                .replace_active_agent_thread(
                    &previous.id,
                    replacement,
                    "user_started_new_agent_conversation".to_string(),
                    None,
                )
                .map(|result| (Some(result.archived_thread), result.active_thread)),
            None => self
                .service
                .install_first_agent_thread(replacement)
                .map(|thread| (None, thread)),
        };
        let (archived_thread, active_thread) = match persisted {
            Ok(result) => result,
            Err(error) => {
                self.unmark_loaded(generation, &external_thread_id);
                return Err(orphan_thread_error(&external_thread_id, error));
            }
        };
        let state = self
            .service
            .get_session_state(session_id)
            .map_err(AgentThreadManagerError::Persistence)?;
        Ok(StartNewAgentConversationResult {
            archived_thread,
            active_thread,
            state,
        })
    }

    pub fn finish_turn(&self, turn: &mut ManagedAgentTurn) -> Result<(), AgentThreadManagerError> {
        let mut thread = self
            .service
            .list_agent_threads(&turn.session_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .into_iter()
            .find(|thread| thread.id == turn.agent_thread_id)
            .ok_or_else(|| {
                AgentThreadManagerError::Persistence(format!(
                    "Agent thread not found: {}",
                    turn.agent_thread_id
                ))
            })?;
        if thread.session_id != turn.scope.session_id
            || thread.plane != turn.scope.plane
            || thread.owner_id != turn.scope.owner_id
            || thread.external_thread_id != turn.external_thread_id
        {
            return Err(AgentThreadManagerError::Persistence(format!(
                "Managed turn scope or external identity does not match agent thread {}.",
                turn.agent_thread_id
            )));
        }
        if thread.replaced_by_id.is_none() && thread.archived_at.is_none() {
            let now = timestamp();
            match turn.scope.plane {
                CadAgentPlane::Modeling => thread.status = CadAgentThreadStatus::Ready,
                CadAgentPlane::Validation => {
                    thread.status = CadAgentThreadStatus::Archived;
                    thread.archived_at = Some(now.clone());
                }
            }
            thread.updated_at = now;
            self.service
                .upsert_agent_thread(thread)
                .map_err(AgentThreadManagerError::Persistence)?;
        }
        turn.release_route_and_lease()
    }

    pub async fn interrupt_and_reconcile(
        &self,
        turn: &mut ManagedAgentTurn,
    ) -> Result<InterruptReconciliation, AgentThreadManagerError> {
        self.transport
            .request(
                "turn/interrupt",
                json!({
                    "threadId": turn.external_thread_id,
                    "turnId": turn.external_turn_id,
                }),
            )
            .await
            .map_err(|error| map_transport_error("turn/interrupt", error))?;

        let deadline = Instant::now() + self.config.interrupt_terminal_timeout;
        let mut observed = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, turn.notifications.recv()).await {
                Ok(Some(notification)) => {
                    let terminal = terminal_status(&notification);
                    observed.push(notification);
                    if let Some(status) = terminal {
                        return Ok(InterruptReconciliation {
                            observed_notifications: observed,
                            reconciliation: TurnReconciliation {
                                status,
                                messages: Vec::new(),
                            },
                        });
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }
        let reconciliation = match turn.scope.plane {
            CadAgentPlane::Modeling => {
                self.recover_run_from_history(&turn.session_id, &turn.run_id)
                    .await?
            }
            CadAgentPlane::Validation => {
                self.recover_turn_from_history(&turn.external_thread_id, &turn.external_turn_id)
                    .await?
            }
        };
        Ok(InterruptReconciliation {
            observed_notifications: observed,
            reconciliation,
        })
    }

    pub async fn reconcile_run(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<TurnReconciliation, AgentThreadManagerError> {
        self.recover_run_from_history(session_id, run_id).await
    }

    pub async fn reconcile_after_connection_loss(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<TurnReconciliation, AgentThreadManagerError> {
        self.service
            .mark_agent_run_reconciling(
                session_id,
                run_id,
                "Codex app-server connection closed; reconciling turn history.".to_string(),
            )
            .map_err(AgentThreadManagerError::Persistence)?;
        let reconciliation = self.recover_run_from_history(session_id, run_id).await?;
        if matches!(reconciliation.status, RecoveredTurnStatus::InProgress) {
            self.service
                .mark_agent_run_unknown_outcome(
                    session_id,
                    run_id,
                    "Codex turn remained in progress after app-server reconnection, but live event reattachment is unavailable; outcome is unknown and the run was not retried."
                        .to_string(),
                )
                .map_err(AgentThreadManagerError::Persistence)?;
            return Err(AgentThreadManagerError::Transport(
                "Codex turn remained in progress after app-server reconnection and history read; live event reattachment is unavailable."
                    .to_string(),
            ));
        }
        Ok(reconciliation)
    }

    pub async fn interrupt_run_and_reconcile(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<(), AgentThreadManagerError> {
        let run = self
            .service
            .get_agent_run(session_id, run_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .ok_or_else(|| {
                AgentThreadManagerError::Persistence(format!("Agent run not found: {run_id}"))
            })?;
        let thread_id = run.external_thread_id.ok_or_else(|| {
            AgentThreadManagerError::InvalidInput(format!(
                "Agent run {run_id} has no external thread id."
            ))
        })?;
        let turn_id = run.external_turn_id.ok_or_else(|| {
            AgentThreadManagerError::InvalidInput(format!(
                "Agent run {run_id} has no external turn id."
            ))
        })?;
        self.transport
            .request(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            )
            .await
            .map_err(|error| map_transport_error("turn/interrupt", error))?;
        let deadline = Instant::now() + self.config.interrupt_terminal_timeout;
        loop {
            let reconciliation = self.recover_run_from_history(session_id, run_id).await?;
            match reconciliation.status {
                RecoveredTurnStatus::Completed
                | RecoveredTurnStatus::Failed { .. }
                | RecoveredTurnStatus::Interrupted
                | RecoveredTurnStatus::NotFound => return Ok(()),
                RecoveredTurnStatus::InProgress if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                RecoveredTurnStatus::InProgress => {
                    return Err(AgentThreadManagerError::Transport(format!(
                        "Timed out waiting for interrupted Codex turn {turn_id} to reach a terminal state."
                    )))
                }
            }
        }
    }

    pub async fn recover_run_from_history(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<TurnReconciliation, AgentThreadManagerError> {
        let run = self
            .service
            .get_agent_run(session_id, run_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .ok_or_else(|| {
                AgentThreadManagerError::Persistence(format!("Agent run not found: {run_id}"))
            })?;
        let thread_id = run.external_thread_id.as_deref().ok_or_else(|| {
            AgentThreadManagerError::InvalidInput(format!(
                "Agent run {run_id} has no external thread id."
            ))
        })?;
        let turn_id = run.external_turn_id.as_deref().ok_or_else(|| {
            AgentThreadManagerError::InvalidInput(format!(
                "Agent run {run_id} has no external turn id."
            ))
        })?;

        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        let response = match self
            .transport
            .request(
                "thread/read",
                json!({"threadId": thread_id, "includeTurns": true}),
            )
            .await
        {
            Ok(response) => response,
            Err(TransportRequestError::ThreadNotFound) => {
                self.service
                    .apply_agent_run_history_recovery(CadAgentRunHistoryRecoveryInput {
                        session_id: session_id.to_string(),
                        run_id: run_id.to_string(),
                        outcome: CadAgentRunHistoryOutcome::NotFound,
                    })
                    .map_err(AgentThreadManagerError::Persistence)?;
                return Ok(TurnReconciliation {
                    status: RecoveredTurnStatus::NotFound,
                    messages: Vec::new(),
                });
            }
            Err(error) => return Err(map_transport_error("thread/read", error)),
        };
        let reconciliation = parse_turn_history(&response, turn_id)?;
        self.persist_reconciliation(session_id, run_id, &reconciliation)?;
        Ok(reconciliation)
    }

    pub async fn recover_turn_from_history(
        &self,
        external_thread_id: &str,
        external_turn_id: &str,
    ) -> Result<TurnReconciliation, AgentThreadManagerError> {
        if external_thread_id.trim().is_empty() || external_turn_id.trim().is_empty() {
            return Err(AgentThreadManagerError::InvalidInput(
                "External thread and turn ids must not be empty when reading history.".to_string(),
            ));
        }
        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        let response = match self
            .transport
            .request(
                "thread/read",
                json!({"threadId": external_thread_id, "includeTurns": true}),
            )
            .await
        {
            Ok(response) => response,
            Err(TransportRequestError::ThreadNotFound) => {
                return Ok(TurnReconciliation {
                    status: RecoveredTurnStatus::NotFound,
                    messages: Vec::new(),
                })
            }
            Err(error) => return Err(map_transport_error("thread/read", error)),
        };
        parse_turn_history(&response, external_turn_id)
    }

    pub async fn read_thread_history(
        &self,
        external_thread_id: &str,
    ) -> Result<Option<Value>, AgentThreadManagerError> {
        if external_thread_id.trim().is_empty() {
            return Err(AgentThreadManagerError::InvalidInput(
                "External thread id must not be empty when reading history.".to_string(),
            ));
        }
        self.transport
            .ensure_initialized()
            .await
            .map_err(AgentThreadManagerError::Transport)?;
        match self
            .transport
            .request(
                "thread/read",
                json!({"threadId": external_thread_id, "includeTurns": true}),
            )
            .await
        {
            Ok(response) => Ok(Some(response)),
            Err(TransportRequestError::ThreadNotFound) => Ok(None),
            Err(error) => Err(map_transport_error("thread/read", error)),
        }
    }

    fn persist_reconciliation(
        &self,
        session_id: &str,
        run_id: &str,
        reconciliation: &TurnReconciliation,
    ) -> Result<(), AgentThreadManagerError> {
        if reconciliation.status == RecoveredTurnStatus::InProgress {
            self.service
                .mark_agent_run_reconciling(
                    session_id,
                    run_id,
                    "External turn remains in progress according to thread history.".to_string(),
                )
                .map_err(AgentThreadManagerError::Persistence)?;
            return Ok(());
        }
        let outcome = match &reconciliation.status {
            RecoveredTurnStatus::Completed => {
                let messages = reconciliation
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(index, message)| {
                        let mut metadata = Metadata::new();
                        metadata.insert("recoveredFromHistory".to_string(), Value::Bool(true));
                        let phase = if message.phase.is_none() {
                            metadata.insert("phaseInferred".to_string(), Value::Bool(true));
                            Some(if index + 1 == reconciliation.messages.len() {
                                CadConversationPhase::FinalAnswer
                            } else {
                                CadConversationPhase::Commentary
                            })
                        } else {
                            message.phase.clone()
                        };
                        CadRecoveredAgentMessage {
                            external_item_id: message.external_item_id.clone(),
                            content: message.text.clone(),
                            phase,
                            sequence: Some(message.sequence),
                            is_final: true,
                            created_at: timestamp(),
                            metadata: Some(metadata),
                        }
                    })
                    .collect();
                CadAgentRunHistoryOutcome::Completed { messages }
            }
            RecoveredTurnStatus::Failed { message } => CadAgentRunHistoryOutcome::Failed {
                error: message.clone(),
            },
            RecoveredTurnStatus::Interrupted => CadAgentRunHistoryOutcome::Interrupted {
                reason: "Codex turn was interrupted.".to_string(),
            },
            RecoveredTurnStatus::NotFound => CadAgentRunHistoryOutcome::NotFound,
            RecoveredTurnStatus::InProgress => unreachable!("handled above"),
        };
        self.service
            .apply_agent_run_history_recovery(CadAgentRunHistoryRecoveryInput {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                outcome,
            })
            .map_err(AgentThreadManagerError::Persistence)?;
        Ok(())
    }

    async fn activate_thread(
        &self,
        input: &StartManagedTurn,
        generation: u64,
    ) -> Result<(CadAgentThread, ManagedThreadActivation), AgentThreadManagerError> {
        let active = self
            .service
            .get_active_agent_thread(&modeling_scope(&input.session_id), EXTERNAL_AGENT)
            .map_err(AgentThreadManagerError::Persistence)?;
        let Some(mut thread) = active else {
            let response = self
                .transport
                .request("thread/start", input.thread_start_params.clone())
                .await
                .map_err(|error| map_transport_error("thread/start", error))?;
            let external_thread_id = required_nested_id(&response, "thread", "thread/start")?;
            let now = timestamp();
            let thread = CadAgentThread {
                id: Uuid::new_v4().to_string(),
                session_id: input.session_id.clone(),
                plane: CadAgentPlane::Modeling,
                owner_id: input.session_id.clone(),
                external_agent: EXTERNAL_AGENT.to_string(),
                external_thread_id: external_thread_id.clone(),
                status: CadAgentThreadStatus::Ready,
                connection_generation: Some(generation),
                created_at: now.clone(),
                updated_at: now,
                last_resumed_at: None,
                archived_at: None,
                replaced_by_id: None,
                metadata: None,
            };
            let thread = self
                .service
                .upsert_agent_thread(thread)
                .map_err(AgentThreadManagerError::Persistence)?;
            self.mark_loaded(generation, &external_thread_id)?;
            return Ok((thread, ManagedThreadActivation::Started));
        };

        if self.is_loaded(generation, &thread.external_thread_id)?
            && thread.connection_generation == Some(generation)
        {
            return Ok((thread, ManagedThreadActivation::Reused));
        }

        match self
            .transport
            .request(
                "thread/resume",
                json!({"threadId": thread.external_thread_id, "excludeTurns": true}),
            )
            .await
        {
            Ok(response) => {
                let resumed_id = required_nested_id(&response, "thread", "thread/resume")?;
                if resumed_id != thread.external_thread_id {
                    return Err(AgentThreadManagerError::Protocol(format!(
                        "Codex thread/resume returned thread {resumed_id}, expected {}.",
                        thread.external_thread_id
                    )));
                }
                let now = timestamp();
                thread.status = CadAgentThreadStatus::Ready;
                thread.connection_generation = Some(generation);
                thread.updated_at = now.clone();
                thread.last_resumed_at = Some(now);
                let thread = self
                    .service
                    .upsert_agent_thread(thread)
                    .map_err(AgentThreadManagerError::Persistence)?;
                self.mark_loaded(generation, &thread.external_thread_id)?;
                Ok((thread, ManagedThreadActivation::Resumed))
            }
            Err(TransportRequestError::ThreadNotFound) => {
                self.replace_missing_thread(thread, input, generation).await
            }
            Err(error) => Err(map_transport_error("thread/resume", error)),
        }
    }

    async fn replace_missing_thread(
        &self,
        previous: CadAgentThread,
        input: &StartManagedTurn,
        generation: u64,
    ) -> Result<(CadAgentThread, ManagedThreadActivation), AgentThreadManagerError> {
        if input
            .replacement_context
            .as_deref()
            .is_none_or(|context| context.trim().is_empty())
        {
            return Err(AgentThreadManagerError::InvalidInput(
                "Replacing a missing Codex thread requires non-empty replacement context."
                    .to_string(),
            ));
        }
        let response = self
            .transport
            .request("thread/start", input.thread_start_params.clone())
            .await
            .map_err(|error| map_transport_error("thread/start", error))?;
        let external_thread_id = required_nested_id(&response, "thread", "thread/start")?;
        let now = timestamp();
        let mut metadata = Metadata::new();
        metadata.insert(
            "replacementReason".to_string(),
            Value::String("resume_thread_not_found".to_string()),
        );
        metadata.insert(
            "replacesExternalThreadId".to_string(),
            Value::String(previous.external_thread_id.clone()),
        );
        if let Some(last_normal_turn_id) = self
            .service
            .list_agent_runs(&input.session_id)
            .map_err(AgentThreadManagerError::Persistence)?
            .into_iter()
            .rev()
            .find(|run| {
                run.agent_thread_id.as_deref() == Some(previous.id.as_str())
                    && run.status == CadAgentRunStatus::Completed
            })
            .and_then(|run| run.external_turn_id)
        {
            metadata.insert(
                "lastNormalTurnId".to_string(),
                Value::String(last_normal_turn_id),
            );
        }
        let replacement = CadAgentThread {
            id: Uuid::new_v4().to_string(),
            session_id: input.session_id.clone(),
            plane: CadAgentPlane::Modeling,
            owner_id: input.session_id.clone(),
            external_agent: EXTERNAL_AGENT.to_string(),
            external_thread_id: external_thread_id.clone(),
            status: CadAgentThreadStatus::Ready,
            connection_generation: Some(generation),
            created_at: now.clone(),
            updated_at: now.clone(),
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: Some(metadata),
        };
        let replacement = self
            .service
            .replace_active_agent_thread(
                &previous.id,
                replacement,
                "resume_thread_not_found".to_string(),
                Some(&input.run_id),
            )
            .map_err(AgentThreadManagerError::Persistence)?;
        let replacement = replacement.active_thread;
        self.mark_loaded(generation, &external_thread_id)?;
        Ok((
            replacement,
            ManagedThreadActivation::Replaced {
                previous_external_thread_id: previous.external_thread_id,
            },
        ))
    }

    fn acquire_scope_lease(
        &self,
        scope: &ThreadScope,
    ) -> Result<ScopedTurnLease, AgentThreadManagerError> {
        validate_scope(scope)?;
        let key = ScopeLeaseKey::from(scope);
        let mut runtime = self.runtime.lock().map_err(|_| {
            AgentThreadManagerError::ActiveTurn(
                "Agent thread manager runtime lock is poisoned.".to_string(),
            )
        })?;
        if !runtime.active_scopes.insert(key.clone()) {
            return Err(AgentThreadManagerError::ActiveTurn(format!(
                "Scope {}/{:?}/{} already has an active Codex turn.",
                scope.session_id, scope.plane, scope.owner_id
            )));
        }
        Ok(ScopedTurnLease {
            key,
            runtime: Arc::clone(&self.runtime),
        })
    }

    fn is_loaded(
        &self,
        generation: u64,
        external_thread_id: &str,
    ) -> Result<bool, AgentThreadManagerError> {
        let runtime = self.runtime.lock().map_err(|_| {
            AgentThreadManagerError::ActiveTurn(
                "Agent thread manager runtime lock is poisoned.".to_string(),
            )
        })?;
        Ok(runtime
            .loaded_threads
            .contains(&(generation, external_thread_id.to_string())))
    }

    fn mark_loaded(
        &self,
        generation: u64,
        external_thread_id: &str,
    ) -> Result<(), AgentThreadManagerError> {
        let mut runtime = self.runtime.lock().map_err(|_| {
            AgentThreadManagerError::ActiveTurn(
                "Agent thread manager runtime lock is poisoned.".to_string(),
            )
        })?;
        runtime
            .loaded_threads
            .retain(|(loaded_generation, _)| *loaded_generation == generation);
        runtime
            .loaded_threads
            .insert((generation, external_thread_id.to_string()));
        Ok(())
    }

    fn unmark_loaded(&self, generation: u64, external_thread_id: &str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime
                .loaded_threads
                .remove(&(generation, external_thread_id.to_string()));
        }
    }

    fn cancel_pending(&self, pending: &PendingRouteHandle) -> Result<(), AgentThreadManagerError> {
        self.router
            .cancel_pending_route(pending)
            .map(|_| ())
            .map_err(|error| AgentThreadManagerError::Routing(error.to_string()))
    }

    fn set_thread_status(
        &self,
        thread: &mut CadAgentThread,
        status: CadAgentThreadStatus,
    ) -> Result<(), AgentThreadManagerError> {
        thread.status = status;
        thread.updated_at = timestamp();
        self.service
            .upsert_agent_thread(thread.clone())
            .map(|_| ())
            .map_err(AgentThreadManagerError::Persistence)
    }

    fn archive_scoped_thread(
        &self,
        thread: &mut CadAgentThread,
        status: CadAgentThreadStatus,
    ) -> Result<(), AgentThreadManagerError> {
        let now = timestamp();
        thread.status = status;
        thread.updated_at = now.clone();
        thread.archived_at = Some(now);
        self.service
            .upsert_agent_thread(thread.clone())
            .map(|_| ())
            .map_err(AgentThreadManagerError::Persistence)
    }
}

fn orphan_thread_error(external_thread_id: &str, error: String) -> AgentThreadManagerError {
    AgentThreadManagerError::Persistence(format!(
        "Codex thread/start created external thread {external_thread_id}, but its session mapping was not committed; the external thread is orphaned: {error}"
    ))
}

fn modeling_scope(session_id: &str) -> ThreadScope {
    ThreadScope {
        session_id: session_id.to_string(),
        plane: CadAgentPlane::Modeling,
        owner_id: session_id.to_string(),
    }
}

fn validate_scope(scope: &ThreadScope) -> Result<(), AgentThreadManagerError> {
    if scope.session_id.trim().is_empty() || scope.owner_id.trim().is_empty() {
        return Err(AgentThreadManagerError::InvalidInput(
            "Thread scope session and owner ids must not be empty.".to_string(),
        ));
    }
    Ok(())
}

fn validate_scoped_start_input(input: &StartScopedTurn) -> Result<(), AgentThreadManagerError> {
    validate_scope(&input.scope)?;
    if input.scope.plane != CadAgentPlane::Validation {
        return Err(AgentThreadManagerError::InvalidInput(
            "start_scoped_turn is reserved for validation scope; modeling must use start_turn."
                .to_string(),
        ));
    }
    if !input.thread_start_params.is_object() || !input.turn_start_params.is_object() {
        return Err(AgentThreadManagerError::InvalidInput(
            "Thread and turn parameters must be JSON objects.".to_string(),
        ));
    }
    if input.turn_start_params.get("threadId").is_some() {
        return Err(AgentThreadManagerError::InvalidInput(
            "turn_start_params must not contain threadId; the manager binds it exactly."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_start_input(input: &StartManagedTurn) -> Result<(), AgentThreadManagerError> {
    if input.session_id.trim().is_empty() || input.run_id.trim().is_empty() {
        return Err(AgentThreadManagerError::InvalidInput(
            "Session and run ids must not be empty.".to_string(),
        ));
    }
    if !input.thread_start_params.is_object() || !input.turn_start_params.is_object() {
        return Err(AgentThreadManagerError::InvalidInput(
            "Thread and turn parameters must be JSON objects.".to_string(),
        ));
    }
    if input.turn_start_params.get("threadId").is_some() {
        return Err(AgentThreadManagerError::InvalidInput(
            "turn_start_params must not contain threadId; the manager binds it exactly."
                .to_string(),
        ));
    }
    Ok(())
}

fn prepare_turn_params(
    mut params: Value,
    thread_id: &str,
    replacement_context: Option<String>,
) -> Result<Value, AgentThreadManagerError> {
    let object = params.as_object_mut().ok_or_else(|| {
        AgentThreadManagerError::InvalidInput("turn_start_params must be an object.".to_string())
    })?;
    if object
        .insert("threadId".to_string(), Value::String(thread_id.to_string()))
        .is_some()
    {
        return Err(AgentThreadManagerError::InvalidInput(
            "turn_start_params already contained threadId.".to_string(),
        ));
    }
    if let Some(context) = replacement_context {
        let inputs = object
            .get_mut("input")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                AgentThreadManagerError::InvalidInput(
                    "Replacement turn parameters require an input array.".to_string(),
                )
            })?;
        let text = inputs
            .iter_mut()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|item| item.get_mut("text"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                AgentThreadManagerError::InvalidInput(
                    "Replacement turn parameters require a text input item.".to_string(),
                )
            })?
            .to_string();
        let first_text = inputs
            .iter_mut()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(Value::as_object_mut)
            .expect("text input was validated above");
        first_text.insert(
            "text".to_string(),
            Value::String(format!("{text}\n\n{context}")),
        );
    }
    Ok(params)
}

fn required_nested_id(
    response: &Value,
    container: &str,
    method: &str,
) -> Result<String, AgentThreadManagerError> {
    response
        .get(container)
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AgentThreadManagerError::Protocol(format!(
                "Codex {method} response did not include {container}.id."
            ))
        })
}

fn map_transport_error(method: &str, error: TransportRequestError) -> AgentThreadManagerError {
    match error {
        TransportRequestError::ThreadNotFound => AgentThreadManagerError::Transport(format!(
            "Codex {method} reported thread-not-found in an unsupported context."
        )),
        TransportRequestError::Rejected(message) | TransportRequestError::Connection(message) => {
            AgentThreadManagerError::Transport(format!("Codex {method} failed: {message}"))
        }
    }
}

fn recovery_status_for_activation(activation: &ManagedThreadActivation) -> CadAgentRecoveryStatus {
    match activation {
        ManagedThreadActivation::Started | ManagedThreadActivation::Reused => {
            CadAgentRecoveryStatus::None
        }
        ManagedThreadActivation::Resumed => CadAgentRecoveryStatus::Resumed,
        ManagedThreadActivation::Replaced { .. } => CadAgentRecoveryStatus::OrphanedThread,
    }
}

fn terminal_status(notification: &RoutedNotification) -> Option<RecoveredTurnStatus> {
    match notification.method.as_str() {
        "turn/completed" => match notification
            .raw
            .pointer("/params/turn/status")
            .and_then(Value::as_str)
        {
            Some("completed") => Some(RecoveredTurnStatus::Completed),
            Some("interrupted") => Some(RecoveredTurnStatus::Interrupted),
            Some("failed") => Some(RecoveredTurnStatus::Failed {
                message: notification
                    .raw
                    .pointer("/params/turn/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex turn failed.")
                    .to_string(),
            }),
            _ => None,
        },
        "turn/interrupted" => Some(RecoveredTurnStatus::Interrupted),
        "turn/failed" => Some(RecoveredTurnStatus::Failed {
            message: notification
                .raw
                .pointer("/params/turn/error/message")
                .or_else(|| notification.raw.pointer("/params/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed.")
                .to_string(),
        }),
        _ => None,
    }
}

fn parse_turn_history(
    response: &Value,
    expected_turn_id: &str,
) -> Result<TurnReconciliation, AgentThreadManagerError> {
    let turns = response
        .pointer("/thread/turns")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AgentThreadManagerError::Protocol(
                "Codex thread/read response did not include thread.turns.".to_string(),
            )
        })?;
    let Some(turn) = turns
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(expected_turn_id))
    else {
        return Ok(TurnReconciliation {
            status: RecoveredTurnStatus::NotFound,
            messages: Vec::new(),
        });
    };
    let status = match turn.get("status").and_then(Value::as_str) {
        Some("completed") => RecoveredTurnStatus::Completed,
        Some("interrupted") => RecoveredTurnStatus::Interrupted,
        Some("inProgress") => RecoveredTurnStatus::InProgress,
        Some("failed") => RecoveredTurnStatus::Failed {
            message: turn
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed.")
                .to_string(),
        },
        Some(other) => {
            return Err(AgentThreadManagerError::Protocol(format!(
                "Codex turn history returned unknown status {other:?}."
            )))
        }
        None => {
            return Err(AgentThreadManagerError::Protocol(
                "Codex turn history omitted turn.status.".to_string(),
            ))
        }
    };
    let items = turn.get("items").and_then(Value::as_array).ok_or_else(|| {
        AgentThreadManagerError::Protocol("Codex turn history omitted turn.items.".to_string())
    })?;
    let agent_items = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .collect::<Vec<_>>();
    let mut messages = Vec::with_capacity(agent_items.len());
    for (index, item) in agent_items.into_iter().enumerate() {
        let external_item_id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                AgentThreadManagerError::Protocol(
                    "Codex agentMessage history item omitted id.".to_string(),
                )
            })?
            .to_string();
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AgentThreadManagerError::Protocol(format!(
                    "Codex agentMessage history item {external_item_id} omitted text."
                ))
            })?
            .to_string();
        let phase = match item.get("phase").and_then(Value::as_str) {
            Some("commentary") => Some(CadConversationPhase::Commentary),
            Some("final_answer") => Some(CadConversationPhase::FinalAnswer),
            Some(other) => {
                return Err(AgentThreadManagerError::Protocol(format!(
                "Codex agentMessage history item {external_item_id} has unknown phase {other:?}."
            )))
            }
            None => None,
        };
        // A history item exists only after Codex has completed that item. `phase`
        // separately distinguishes commentary from the final answer.
        let is_final = true;
        messages.push(RecoveredAgentMessage {
            external_item_id,
            text,
            phase,
            sequence: index as u64,
            is_final,
        });
    }
    Ok(TurnReconciliation { status, messages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification_router::NotificationRouterConfig;
    use crate::protocol::{
        CadArtifactKind, CadDiagnostics, CadSourceLanguage, CadValidationEvaluation,
        CadValidationEvaluationKind, CadValidationEvaluationStatus, CreateCadSessionInput,
        PersistRuntimeArtifactInput, UpdateModelSourceInput,
    };
    use base64::Engine;
    use std::collections::VecDeque;

    #[derive(Clone)]
    struct ScriptedTransport {
        router: NotificationRouter,
        generation: Arc<Mutex<Option<u64>>>,
        requests: Arc<Mutex<Vec<(String, Value)>>>,
        responses: Arc<Mutex<VecDeque<Result<Value, TransportRequestError>>>>,
        early_notifications: Arc<Mutex<VecDeque<Value>>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Result<Value, TransportRequestError>>) -> Self {
            Self {
                router: NotificationRouter::new(NotificationRouterConfig::default()).unwrap(),
                generation: Arc::new(Mutex::new(Some(1))),
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses.into())),
                early_notifications: Arc::new(Mutex::new(VecDeque::new())),
            }
        }

        fn methods(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|(method, _)| method.clone())
                .collect()
        }
    }

    #[async_trait]
    impl AgentThreadTransport for ScriptedTransport {
        async fn ensure_initialized(&self) -> Result<(), String> {
            Ok(())
        }

        async fn current_connection_generation(&self) -> Option<u64> {
            *self.generation.lock().unwrap()
        }

        fn notification_router(&self) -> NotificationRouter {
            self.router.clone()
        }

        async fn request(
            &self,
            method: &str,
            params: Value,
        ) -> Result<Value, TransportRequestError> {
            self.requests
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            if method == "turn/start" {
                let notifications = self
                    .early_notifications
                    .lock()
                    .unwrap()
                    .drain(..)
                    .collect::<Vec<_>>();
                for notification in notifications {
                    self.router.route(notification).unwrap();
                }
            }
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted response missing")
        }
    }

    fn setup(
        responses: Vec<Result<Value, TransportRequestError>>,
    ) -> (AgentThreadManager, ScriptedTransport, String, String) {
        let root = std::env::temp_dir().join(format!("thread-manager-{}", Uuid::new_v4()));
        let service = Arc::new(SessionService::new(root));
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let run = service
            .create_agent_run(
                &session.state.session.id,
                "make a cube".to_string(),
                None,
                Some(EXTERNAL_AGENT.to_string()),
                None,
            )
            .unwrap();
        let transport = ScriptedTransport::new(responses);
        let manager = AgentThreadManager::with_transport(
            service,
            Arc::new(transport.clone()),
            AgentThreadManagerConfig::default(),
        )
        .unwrap();
        (manager, transport, session.state.session.id, run.0.id)
    }

    fn start_input(session_id: &str, run_id: &str) -> StartManagedTurn {
        StartManagedTurn {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            thread_start_params: json!({"cwd": "/tmp"}),
            turn_start_params: json!({
                "input": [{"type": "text", "text": "prompt", "text_elements": []}]
            }),
            replacement_context: Some("Recovered CAD state".to_string()),
        }
    }

    fn create_validation_evaluation(
        manager: &AgentThreadManager,
        session_id: &str,
        run_id: &str,
        evaluation_id: &str,
    ) -> CadValidationEvaluation {
        let revision_id = manager
            .service
            .update_model_source(UpdateModelSourceInput {
                session_id: session_id.to_string(),
                source_language: CadSourceLanguage::Openscad,
                source: "cube([1,1,1]);".to_string(),
                parent_revision_id: None,
                parameters: None,
            })
            .unwrap()
            .revision_id;
        manager
            .service
            .link_agent_run_output_revision(session_id, run_id, revision_id.clone())
            .unwrap();
        let artifact = manager
            .service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: session_id.to_string(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::RenderImage,
                format: "png".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(b"png"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 1,
                    items: Vec::new(),
                },
                metadata: Metadata::new(),
            })
            .unwrap()
            .artifact;
        manager
            .service
            .create_validation_evaluation(CadValidationEvaluation {
                id: evaluation_id.to_string(),
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                revision_id,
                artifact_id: artifact.id,
                kind: CadValidationEvaluationKind::Vlm,
                attempt: 1,
                status: CadValidationEvaluationStatus::Queued,
                evaluator_thread_id: None,
                external_turn_id: None,
                input_contract: json!({}),
                report: None,
                passed: None,
                score: None,
                pass_threshold: 0.8,
                error: None,
                created_at: timestamp(),
                started_at: None,
                completed_at: None,
            })
            .unwrap()
    }

    fn bind_scripted_run(manager: &AgentThreadManager, session_id: &str, run_id: &str) {
        let now = timestamp();
        let thread = manager
            .service
            .upsert_agent_thread(CadAgentThread {
                id: Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                plane: CadAgentPlane::Modeling,
                owner_id: session_id.to_string(),
                external_agent: EXTERNAL_AGENT.to_string(),
                external_thread_id: "thread-1".to_string(),
                status: CadAgentThreadStatus::Active,
                connection_generation: Some(1),
                created_at: now.clone(),
                updated_at: now,
                last_resumed_at: None,
                archived_at: None,
                replaced_by_id: None,
                metadata: None,
            })
            .unwrap();
        manager
            .service
            .bind_agent_run_to_thread(
                session_id,
                run_id,
                &thread.id,
                Some("turn-1".to_string()),
                Some(1),
                CadAgentRecoveryStatus::None,
            )
            .unwrap();
    }

    #[tokio::test]
    async fn connection_loss_with_in_progress_history_marks_unknown_outcome() {
        let (manager, _transport, session_id, run_id) = setup(vec![Ok(json!({
            "thread": {"turns": [{"id": "turn-1", "status": "inProgress", "items": []}]}
        }))]);
        bind_scripted_run(&manager, &session_id, &run_id);

        let error = manager
            .reconcile_after_connection_loss(&session_id, &run_id)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("live event reattachment is unavailable"));
        let run = manager
            .service
            .get_agent_run(&session_id, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, CadAgentRunStatus::Failed);
        assert_eq!(run.recovery_status, CadAgentRecoveryStatus::UnknownOutcome);
    }

    #[tokio::test]
    async fn connection_loss_with_terminal_history_recovers_run() {
        let (manager, _transport, session_id, run_id) = setup(vec![Ok(json!({
            "thread": {"turns": [{
                "id": "turn-1", "status": "completed",
                "items": [{"type": "agentMessage", "id": "item-1", "text": "recovered", "phase": "final_answer"}]
            }]}
        }))]);
        bind_scripted_run(&manager, &session_id, &run_id);

        let reconciliation = manager
            .reconcile_after_connection_loss(&session_id, &run_id)
            .await
            .unwrap();
        assert_eq!(reconciliation.status, RecoveredTurnStatus::Completed);
        let run = manager
            .service
            .get_agent_run(&session_id, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, CadAgentRunStatus::Completed);
        assert_eq!(
            run.recovery_status,
            CadAgentRecoveryStatus::RecoveredFromHistory
        );
    }

    #[tokio::test]
    async fn starts_a_real_new_conversation_for_a_session_without_a_thread() {
        let root = std::env::temp_dir().join(format!("thread-manager-{}", Uuid::new_v4()));
        let service = Arc::new(SessionService::new(root));
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let transport = ScriptedTransport::new(vec![Ok(json!({
            "thread": {"id": "new-conversation-thread"}
        }))]);
        let manager = AgentThreadManager::with_transport(
            Arc::clone(&service),
            Arc::new(transport.clone()),
            AgentThreadManagerConfig::default(),
        )
        .unwrap();

        let result = manager
            .start_new_conversation(&session.session_id, json!({"cwd": "/tmp"}))
            .await
            .unwrap();

        assert!(result.archived_thread.is_none());
        assert_eq!(
            result.active_thread.external_thread_id,
            "new-conversation-thread"
        );
        assert_eq!(result.state.agent_threads, vec![result.active_thread]);
        assert_eq!(transport.methods(), vec!["thread/start"]);
    }

    #[tokio::test]
    async fn rejects_new_conversation_before_transport_when_a_run_is_active() {
        let (manager, transport, session_id, _) = setup(Vec::new());

        let error = manager
            .start_new_conversation(&session_id, json!({"cwd": "/tmp"}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("active"));
        assert!(transport.methods().is_empty());
    }

    #[tokio::test]
    async fn reports_the_external_orphan_id_when_mapping_persistence_fails() {
        let app_data_dir = std::env::temp_dir().join(format!("thread-manager-{}", Uuid::new_v4()));
        let layout = crate::storage::StorageLayout::from_app_data_dir(app_data_dir);
        crate::storage::initialize_storage(&layout).unwrap();
        let service = Arc::new(
            SessionService::with_repository(
                layout.clone(),
                Arc::new(crate::session_repository::SqliteSessionRepository::new(
                    layout.clone(),
                )),
            )
            .unwrap(),
        );
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let connection = rusqlite::Connection::open(layout.database_path()).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TRIGGER fail_new_conversation_mapping
                BEFORE INSERT ON agent_threads
                BEGIN
                  SELECT RAISE(ABORT, 'forced agent thread mapping failure');
                END;
                "#,
            )
            .unwrap();
        drop(connection);
        let transport = ScriptedTransport::new(vec![Ok(json!({
            "thread": {"id": "orphaned-external-thread"}
        }))]);
        let manager = AgentThreadManager::with_transport(
            Arc::clone(&service),
            Arc::new(transport),
            AgentThreadManagerConfig::default(),
        )
        .unwrap();

        let error = manager
            .start_new_conversation(&session.session_id, json!({"cwd": "/tmp"}))
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("orphaned-external-thread"));
        assert!(error.contains("orphaned"));
        assert!(error.contains("forced agent thread mapping failure"));
        assert!(service
            .get_session_state(&session.session_id)
            .unwrap()
            .agent_threads
            .is_empty());
    }

    #[tokio::test]
    async fn atomically_replaces_conversation_and_reuses_the_loaded_thread_on_next_run() {
        let root = std::env::temp_dir().join(format!("thread-manager-{}", Uuid::new_v4()));
        let service = Arc::new(SessionService::new(root));
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let transport = ScriptedTransport::new(vec![
            Ok(json!({"thread": {"id": "conversation-1"}})),
            Ok(json!({"thread": {"id": "conversation-2"}})),
            Ok(json!({"turn": {"id": "turn-2"}})),
        ]);
        let manager = AgentThreadManager::with_transport(
            Arc::clone(&service),
            Arc::new(transport.clone()),
            AgentThreadManagerConfig::default(),
        )
        .unwrap();
        manager
            .start_new_conversation(&session.session_id, json!({"cwd": "/tmp"}))
            .await
            .unwrap();
        let replaced = manager
            .start_new_conversation(&session.session_id, json!({"cwd": "/tmp"}))
            .await
            .unwrap();
        assert_eq!(
            replaced
                .archived_thread
                .as_ref()
                .and_then(|thread| thread.replaced_by_id.as_deref()),
            Some(replaced.active_thread.id.as_str())
        );

        let run = service
            .create_agent_run(
                &session.session_id,
                "continue in the new conversation".to_string(),
                None,
                Some(EXTERNAL_AGENT.to_string()),
                None,
            )
            .unwrap();
        let turn = manager
            .start_turn(start_input(&session.session_id, &run.0.id))
            .await
            .unwrap();

        assert_eq!(turn.activation, ManagedThreadActivation::Reused);
        assert_eq!(turn.external_thread_id, "conversation-2");
        assert_eq!(
            transport.methods(),
            vec!["thread/start", "thread/start", "turn/start"]
        );
    }

    #[tokio::test]
    async fn starts_lazily_then_reuses_loaded_thread_in_same_generation() {
        let (manager, transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread": {"id": "thread-1"}})),
            Ok(json!({"turn": {"id": "turn-1"}})),
            Ok(json!({"turn": {"id": "turn-2"}})),
        ]);
        let mut first = manager
            .start_turn(start_input(&session_id, &run_id))
            .await
            .unwrap();
        assert_eq!(first.activation, ManagedThreadActivation::Started);
        manager.finish_turn(&mut first).unwrap();
        manager
            .service
            .update_agent_run(
                &session_id,
                &run_id,
                Some(CadAgentRunStatus::Completed),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let second_run = manager
            .service
            .create_agent_run(
                &session_id,
                "make it taller".to_string(),
                None,
                Some(EXTERNAL_AGENT.to_string()),
                None,
            )
            .unwrap();
        let second = manager
            .start_turn(start_input(&session_id, &second_run.0.id))
            .await
            .unwrap();
        assert_eq!(second.activation, ManagedThreadActivation::Reused);
        assert_eq!(second.external_thread_id, "thread-1");
        assert_eq!(
            transport.methods(),
            vec!["thread/start", "turn/start", "turn/start"]
        );
    }

    #[tokio::test]
    async fn a_new_manager_resumes_persisted_thread_even_when_generation_number_matches() {
        let (manager, transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread": {"id": "thread-1"}})),
            Ok(json!({"turn": {"id": "turn-1"}})),
        ]);
        let mut first = manager
            .start_turn(start_input(&session_id, &run_id))
            .await
            .unwrap();
        manager.finish_turn(&mut first).unwrap();
        manager
            .service
            .update_agent_run(
                &session_id,
                &run_id,
                Some(CadAgentRunStatus::Completed),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let second_run = manager
            .service
            .create_agent_run(
                &session_id,
                "again".to_string(),
                None,
                Some(EXTERNAL_AGENT.to_string()),
                None,
            )
            .unwrap();
        transport.responses.lock().unwrap().extend([
            Ok(json!({"thread": {"id": "thread-1"}})),
            Ok(json!({"turn": {"id": "turn-2"}})),
        ]);
        let restarted = AgentThreadManager::with_transport(
            Arc::clone(&manager.service),
            Arc::new(transport.clone()),
            AgentThreadManagerConfig::default(),
        )
        .unwrap();
        let second = restarted
            .start_turn(start_input(&session_id, &second_run.0.id))
            .await
            .unwrap();
        assert_eq!(second.activation, ManagedThreadActivation::Resumed);
        assert_eq!(transport.methods()[2..], ["thread/resume", "turn/start"]);
    }

    #[tokio::test]
    async fn missing_resume_preserves_old_mapping_and_promotes_replacement_route() {
        let (manager, transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread": {"id": "old-thread"}})),
            Ok(json!({"turn": {"id": "turn-1"}})),
        ]);
        let mut first = manager
            .start_turn(start_input(&session_id, &run_id))
            .await
            .unwrap();
        manager.finish_turn(&mut first).unwrap();
        manager
            .service
            .update_agent_run(
                &session_id,
                &run_id,
                Some(CadAgentRunStatus::Completed),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let second_run = manager
            .service
            .create_agent_run(
                &session_id,
                "again".to_string(),
                None,
                Some(EXTERNAL_AGENT.to_string()),
                None,
            )
            .unwrap();
        transport.responses.lock().unwrap().extend([
            Err(TransportRequestError::ThreadNotFound),
            Ok(json!({"thread": {"id": "new-thread"}})),
            Ok(json!({"turn": {"id": "turn-2"}})),
        ]);
        *transport.generation.lock().unwrap() = Some(2);
        let second = manager
            .start_turn(start_input(&session_id, &second_run.0.id))
            .await
            .unwrap();
        assert_eq!(
            second.activation,
            ManagedThreadActivation::Replaced {
                previous_external_thread_id: "old-thread".to_string()
            }
        );
        let threads = manager.service.list_agent_threads(&session_id).unwrap();
        let old = threads
            .iter()
            .find(|thread| thread.external_thread_id == "old-thread")
            .unwrap();
        let new = threads
            .iter()
            .find(|thread| thread.external_thread_id == "new-thread")
            .unwrap();
        assert_eq!(old.status, CadAgentThreadStatus::Replaced);
        assert_eq!(old.replaced_by_id.as_deref(), Some(new.id.as_str()));
        assert!(old.archived_at.is_some());
        assert!(new.archived_at.is_none());
        let turn_start = transport.requests.lock().unwrap().last().unwrap().1.clone();
        assert_eq!(
            turn_start.get("threadId").and_then(Value::as_str),
            Some("new-thread")
        );
        assert!(turn_start
            .pointer("/input/0/text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Recovered CAD state"));
    }

    #[tokio::test]
    async fn notification_before_turn_ack_is_promoted_to_the_exact_receiver() {
        let (manager, transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread": {"id": "thread-1"}})),
            Ok(json!({"turn": {"id": "turn-1"}})),
        ]);
        transport
            .early_notifications
            .lock()
            .unwrap()
            .push_back(json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-1",
                    "turn": {"id": "turn-1"}
                }
            }));
        let mut turn = manager
            .start_turn(start_input(&session_id, &run_id))
            .await
            .unwrap();
        let notification = turn.notifications.recv().await.unwrap();
        assert_eq!(notification.method, "turn/started");
        assert_eq!(notification.identifiers.turn_id.as_deref(), Some("turn-1"));
    }

    #[tokio::test]
    async fn interrupt_without_terminal_notification_recovers_terminal_and_messages_from_history() {
        let (mut manager, _transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread": {"id": "thread-1"}})),
            Ok(json!({"turn": {"id": "turn-1"}})),
            Ok(json!({})),
            Ok(json!({
                "thread": {"turns": [{
                    "id": "turn-1",
                    "status": "completed",
                    "items": [{
                        "type": "agentMessage",
                        "id": "item-1",
                        "text": "recovered answer",
                        "phase": "final_answer"
                    }]
                }]}
            })),
        ]);
        manager.config.interrupt_terminal_timeout = Duration::from_millis(1);
        let mut turn = manager
            .start_turn(start_input(&session_id, &run_id))
            .await
            .unwrap();
        let result = manager.interrupt_and_reconcile(&mut turn).await.unwrap();
        assert_eq!(result.reconciliation.status, RecoveredTurnStatus::Completed);
        let run = manager
            .service
            .get_agent_run(&session_id, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, CadAgentRunStatus::Completed);
        assert_eq!(
            run.recovery_status,
            CadAgentRecoveryStatus::RecoveredFromHistory
        );
        let state = manager.service.get_session_state(&session_id).unwrap();
        assert!(state
            .conversation
            .iter()
            .any(|message| message.content == "recovered answer"));
    }

    #[test]
    fn parses_terminal_history_and_preserves_all_agent_messages() {
        let recovered = parse_turn_history(
            &json!({
                "thread": {"turns": [{
                    "id": "turn-1",
                    "status": "completed",
                    "items": [
                        {"type": "agentMessage", "id": "item-1", "text": "working", "phase": "commentary"},
                        {"type": "agentMessage", "id": "item-2", "text": "done", "phase": "final_answer"}
                    ]
                }]}
            }),
            "turn-1",
        )
        .unwrap();
        assert_eq!(recovered.status, RecoveredTurnStatus::Completed);
        assert_eq!(recovered.messages.len(), 2);
        assert!(recovered.messages[0].is_final);
        assert!(recovered.messages[1].is_final);
    }

    #[test]
    fn phase_less_history_preserves_unknown_phase_for_persistence_inference() {
        let recovered = parse_turn_history(
            &json!({
                "thread": {"turns": [{
                    "id": "turn-1",
                    "status": "completed",
                    "items": [
                        {"type": "agentMessage", "id": "item-1", "text": "working"},
                        {"type": "agentMessage", "id": "item-2", "text": "done"}
                    ]
                }]}
            }),
            "turn-1",
        )
        .unwrap();
        assert!(recovered
            .messages
            .iter()
            .all(|message| message.phase.is_none()));
    }

    #[test]
    fn history_unknown_status_fails_fast() {
        let error = parse_turn_history(
            &json!({"thread": {"turns": [{"id": "turn-1", "status": "mystery", "items": []}]}}),
            "turn-1",
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown status"));
    }

    #[test]
    fn modeling_and_validation_leases_are_concurrent_but_validation_plane_is_serial() {
        let (manager, _transport, session_id, _run_id) = setup(Vec::new());
        let modeling = modeling_scope(&session_id);
        let validation_one = ThreadScope {
            session_id: session_id.clone(),
            plane: CadAgentPlane::Validation,
            owner_id: "evaluation-1".to_string(),
        };
        let validation_two = ThreadScope {
            session_id,
            plane: CadAgentPlane::Validation,
            owner_id: "evaluation-2".to_string(),
        };

        let modeling_lease = manager.acquire_scope_lease(&modeling).unwrap();
        let validation_lease = manager.acquire_scope_lease(&validation_one).unwrap();
        assert!(manager.acquire_scope_lease(&validation_two).is_err());

        drop(validation_lease);
        assert!(manager.acquire_scope_lease(&validation_two).is_ok());
        drop(modeling_lease);
    }

    #[test]
    fn scoped_start_rejects_modeling_plane() {
        let bind: ScopedTurnBindCallback = Arc::new(|_| Ok(()));
        let error = validate_scoped_start_input(&StartScopedTurn {
            scope: ThreadScope {
                session_id: "session-1".to_string(),
                plane: CadAgentPlane::Modeling,
                owner_id: "session-1".to_string(),
            },
            thread_start_params: json!({}),
            turn_start_params: json!({}),
            bind,
        })
        .unwrap_err();
        assert!(error.to_string().contains("reserved for validation"));
    }

    #[tokio::test]
    async fn scoped_validation_turn_is_fresh_and_archived_on_finish() {
        let (manager, transport, session_id, run_id) = setup(vec![
            Ok(json!({"thread":{"id":"validation-thread-1"}})),
            Ok(json!({"turn":{"id":"validation-turn-1"}})),
        ]);
        let evaluation =
            create_validation_evaluation(&manager, &session_id, &run_id, "evaluation-1");
        let service = Arc::clone(&manager.service);
        let bind_session = session_id.clone();
        let mut turn = manager
            .start_scoped_turn(StartScopedTurn {
                scope: ThreadScope {
                    session_id: session_id.clone(),
                    plane: CadAgentPlane::Validation,
                    owner_id: evaluation.id.clone(),
                },
                thread_start_params: json!({"cwd":"/tmp"}),
                turn_start_params: json!({"input":[]}),
                bind: Arc::new(move |binding| {
                    service
                        .bind_validation_evaluation(
                            &bind_session,
                            "evaluation-1",
                            &binding.agent_thread_id,
                            &binding.external_turn_id,
                        )
                        .map(|_| ())
                }),
            })
            .await
            .unwrap();

        assert_eq!(transport.methods(), vec!["thread/start", "turn/start"]);
        assert_eq!(turn.activation, ManagedThreadActivation::Started);
        manager.finish_turn(&mut turn).unwrap();
        let thread = manager
            .service
            .list_agent_threads(&session_id)
            .unwrap()
            .into_iter()
            .find(|thread| thread.id == turn.agent_thread_id)
            .unwrap();
        assert_eq!(thread.status, CadAgentThreadStatus::Archived);
        assert!(thread.archived_at.is_some());
    }
}
