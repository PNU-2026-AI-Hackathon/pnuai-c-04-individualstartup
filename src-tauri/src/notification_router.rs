use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationIdentifiers {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutedNotification {
    pub transport_sequence: u64,
    pub method: String,
    pub identifiers: NotificationIdentifiers,
    pub raw: Value,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NotificationRouteKey {
    pub thread_id: String,
    pub turn_id: String,
}

impl NotificationRouteKey {
    pub fn new(thread_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRouteHandle {
    thread_id: String,
    registration_id: u64,
}

impl PendingRouteHandle {
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationRouteOutcome {
    Exact,
    BufferedPending,
    Global,
    Orphan,
    DuplicateTerminalSuppressed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotificationRouteError {
    InvalidNotification(String),
    ConflictingIdentifier {
        kind: &'static str,
        first: String,
        second: String,
    },
    DuplicateExactRoute(NotificationRouteKey),
    DuplicatePendingRoute(String),
    UnknownPendingRoute,
    RouteChannelClosed(NotificationRouteKey),
    RouteChannelFull(NotificationRouteKey),
    PendingBufferFull(String),
    PendingTurnMismatch {
        expected_turn_id: String,
        actual_turn_id: String,
    },
}

impl fmt::Display for NotificationRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNotification(message) => write!(formatter, "{message}"),
            Self::ConflictingIdentifier {
                kind,
                first,
                second,
            } => write!(
                formatter,
                "Notification contains conflicting {kind} values: {first:?} and {second:?}."
            ),
            Self::DuplicateExactRoute(key) => write!(
                formatter,
                "A notification route is already registered for thread {:?}, turn {:?}.",
                key.thread_id, key.turn_id
            ),
            Self::DuplicatePendingRoute(thread_id) => write!(
                formatter,
                "A pending notification route is already registered for thread {thread_id:?}."
            ),
            Self::UnknownPendingRoute => {
                write!(
                    formatter,
                    "The pending notification route is no longer registered."
                )
            }
            Self::RouteChannelClosed(key) => write!(
                formatter,
                "The notification route channel is closed for thread {:?}, turn {:?}.",
                key.thread_id, key.turn_id
            ),
            Self::RouteChannelFull(key) => write!(
                formatter,
                "The notification route channel is full for thread {:?}, turn {:?}.",
                key.thread_id, key.turn_id
            ),
            Self::PendingBufferFull(thread_id) => write!(
                formatter,
                "The early notification buffer is full for thread {thread_id:?}."
            ),
            Self::PendingTurnMismatch {
                expected_turn_id,
                actual_turn_id,
            } => write!(
                formatter,
                "Pending notifications belong to turn {actual_turn_id:?}, not promoted turn {expected_turn_id:?}."
            ),
        }
    }
}

impl std::error::Error for NotificationRouteError {}

#[derive(Clone, Debug)]
pub struct NotificationRouterConfig {
    pub route_capacity: usize,
    pub pending_buffer_capacity: usize,
    pub diagnostic_capacity: usize,
}

impl Default for NotificationRouterConfig {
    fn default() -> Self {
        Self {
            route_capacity: 256,
            pending_buffer_capacity: 64,
            diagnostic_capacity: 256,
        }
    }
}

#[derive(Clone)]
pub struct NotificationRouter {
    inner: Arc<Mutex<RouterState>>,
    next_sequence: Arc<AtomicU64>,
    next_registration_id: Arc<AtomicU64>,
    config: NotificationRouterConfig,
    global: broadcast::Sender<RoutedNotification>,
    orphan: broadcast::Sender<RoutedNotification>,
}

struct RouterState {
    exact_routes: HashMap<NotificationRouteKey, mpsc::Sender<RoutedNotification>>,
    pending_routes: HashMap<String, PendingRoute>,
    terminal_routes: HashSet<NotificationRouteKey>,
}

struct PendingRoute {
    registration_id: u64,
    notifications: VecDeque<RoutedNotification>,
    terminal_turn_ids: HashSet<String>,
}

impl NotificationRouter {
    pub fn new(config: NotificationRouterConfig) -> Result<Self, NotificationRouteError> {
        if config.route_capacity == 0 {
            return Err(NotificationRouteError::InvalidNotification(
                "Notification route capacity must be greater than zero.".to_string(),
            ));
        }
        if config.pending_buffer_capacity == 0 {
            return Err(NotificationRouteError::InvalidNotification(
                "Pending notification buffer capacity must be greater than zero.".to_string(),
            ));
        }
        if config.pending_buffer_capacity > config.route_capacity {
            return Err(NotificationRouteError::InvalidNotification(
                "Pending notification buffer capacity must not exceed route capacity.".to_string(),
            ));
        }
        if config.diagnostic_capacity == 0 {
            return Err(NotificationRouteError::InvalidNotification(
                "Notification diagnostic capacity must be greater than zero.".to_string(),
            ));
        }
        let (global, _) = broadcast::channel(config.diagnostic_capacity);
        let (orphan, _) = broadcast::channel(config.diagnostic_capacity);
        Ok(Self {
            inner: Arc::new(Mutex::new(RouterState {
                exact_routes: HashMap::new(),
                pending_routes: HashMap::new(),
                terminal_routes: HashSet::new(),
            })),
            next_sequence: Arc::new(AtomicU64::new(1)),
            next_registration_id: Arc::new(AtomicU64::new(1)),
            config,
            global,
            orphan,
        })
    }

    pub fn subscribe_global(&self) -> broadcast::Receiver<RoutedNotification> {
        self.global.subscribe()
    }

    pub fn subscribe_orphans(&self) -> broadcast::Receiver<RoutedNotification> {
        self.orphan.subscribe()
    }

    pub fn register_route(
        &self,
        key: NotificationRouteKey,
    ) -> Result<mpsc::Receiver<RoutedNotification>, NotificationRouteError> {
        validate_route_key(&key)?;
        let (sender, receiver) = mpsc::channel(self.config.route_capacity);
        let mut state = self.lock_state()?;
        if state.exact_routes.contains_key(&key) {
            return Err(NotificationRouteError::DuplicateExactRoute(key));
        }
        state.terminal_routes.remove(&key);
        state.exact_routes.insert(key, sender);
        Ok(receiver)
    }

    pub fn unregister_route(
        &self,
        key: &NotificationRouteKey,
    ) -> Result<bool, NotificationRouteError> {
        let mut state = self.lock_state()?;
        let removed = state.exact_routes.remove(key).is_some();
        state.terminal_routes.remove(key);
        Ok(removed)
    }

    /// Drops every run-specific sender so collectors wake immediately when the
    /// underlying app-server connection is lost. Pending early events are also
    /// discarded because they belong to the failed connection generation.
    pub fn fail_all_routes(&self) -> Result<(usize, usize), NotificationRouteError> {
        let mut state = self.lock_state()?;
        let exact = state.exact_routes.len();
        let pending = state.pending_routes.len();
        state.exact_routes.clear();
        state.pending_routes.clear();
        state.terminal_routes.clear();
        Ok((exact, pending))
    }

    pub fn begin_pending_route(
        &self,
        thread_id: impl Into<String>,
    ) -> Result<PendingRouteHandle, NotificationRouteError> {
        let thread_id = thread_id.into();
        validate_identifier("threadId", &thread_id)?;
        let registration_id = self.next_registration_id.fetch_add(1, Ordering::SeqCst);
        let mut state = self.lock_state()?;
        if state.pending_routes.contains_key(&thread_id) {
            return Err(NotificationRouteError::DuplicatePendingRoute(thread_id));
        }
        state.pending_routes.insert(
            thread_id.clone(),
            PendingRoute {
                registration_id,
                notifications: VecDeque::new(),
                terminal_turn_ids: HashSet::new(),
            },
        );
        Ok(PendingRouteHandle {
            thread_id,
            registration_id,
        })
    }

    pub fn promote_pending_route(
        &self,
        handle: PendingRouteHandle,
        turn_id: impl Into<String>,
    ) -> Result<mpsc::Receiver<RoutedNotification>, NotificationRouteError> {
        let key = NotificationRouteKey::new(handle.thread_id.clone(), turn_id);
        validate_route_key(&key)?;
        let (sender, receiver) = mpsc::channel(self.config.route_capacity);
        let mut state = self.lock_state()?;
        if state.exact_routes.contains_key(&key) {
            return Err(NotificationRouteError::DuplicateExactRoute(key));
        }
        let pending = state
            .pending_routes
            .get(&handle.thread_id)
            .filter(|route| route.registration_id == handle.registration_id)
            .ok_or(NotificationRouteError::UnknownPendingRoute)?;
        for notification in &pending.notifications {
            if let Some(actual_turn_id) = notification.identifiers.turn_id.as_ref() {
                if actual_turn_id != &key.turn_id {
                    return Err(NotificationRouteError::PendingTurnMismatch {
                        expected_turn_id: key.turn_id.clone(),
                        actual_turn_id: actual_turn_id.clone(),
                    });
                }
            }
        }
        if pending.notifications.len() > self.config.route_capacity {
            return Err(NotificationRouteError::RouteChannelFull(key));
        }
        let mut pending = state
            .pending_routes
            .remove(&handle.thread_id)
            .expect("pending route was validated while the router lock was held");
        while let Some(notification) = pending.notifications.pop_front() {
            sender
                .try_send(notification)
                .map_err(|error| map_try_send_error(error, &key))?;
        }
        state.terminal_routes.remove(&key);
        if pending.terminal_turn_ids.contains(&key.turn_id) {
            state.terminal_routes.insert(key.clone());
        }
        state.exact_routes.insert(key, sender);
        Ok(receiver)
    }

    pub fn cancel_pending_route(
        &self,
        handle: &PendingRouteHandle,
    ) -> Result<usize, NotificationRouteError> {
        let mut state = self.lock_state()?;
        let pending = state
            .pending_routes
            .get(&handle.thread_id)
            .filter(|route| route.registration_id == handle.registration_id)
            .ok_or(NotificationRouteError::UnknownPendingRoute)?;
        let discarded = pending.notifications.len();
        state.pending_routes.remove(&handle.thread_id);
        Ok(discarded)
    }

    pub fn route(&self, raw: Value) -> Result<NotificationRouteOutcome, NotificationRouteError> {
        let notification =
            parse_notification(self.next_sequence.fetch_add(1, Ordering::SeqCst), raw)?;
        let thread_id = notification.identifiers.thread_id.clone();
        let turn_id = notification.identifiers.turn_id.clone();

        if let (Some(thread_id), Some(turn_id)) = (thread_id.as_ref(), turn_id.as_ref()) {
            let key = NotificationRouteKey::new(thread_id, turn_id);
            let mut state = self.lock_state()?;
            if is_terminal_method(&notification.method) && state.terminal_routes.contains(&key) {
                return Ok(NotificationRouteOutcome::DuplicateTerminalSuppressed);
            }
            if let Some(sender) = state.exact_routes.get(&key) {
                sender
                    .try_send(notification.clone())
                    .map_err(|error| map_try_send_error(error, &key))?;
                if is_terminal_method(&notification.method) {
                    state.terminal_routes.insert(key);
                }
                return Ok(NotificationRouteOutcome::Exact);
            }
            if let Some(pending) = state.pending_routes.get_mut(thread_id) {
                if is_terminal_method(&notification.method)
                    && !pending.terminal_turn_ids.insert(turn_id.clone())
                {
                    return Ok(NotificationRouteOutcome::DuplicateTerminalSuppressed);
                }
                if pending.notifications.len() >= self.config.pending_buffer_capacity {
                    if is_terminal_method(&notification.method) {
                        pending.terminal_turn_ids.remove(turn_id);
                    }
                    return Err(NotificationRouteError::PendingBufferFull(thread_id.clone()));
                }
                pending.notifications.push_back(notification);
                return Ok(NotificationRouteOutcome::BufferedPending);
            }
        }

        if thread_id.is_none() && turn_id.is_none() {
            if self.global.send(notification.clone()).is_err() {
                log_unobserved_diagnostic("global", &notification);
            }
            return Ok(NotificationRouteOutcome::Global);
        }
        if self.orphan.send(notification.clone()).is_err() {
            log_unobserved_diagnostic("orphan", &notification);
        }
        Ok(NotificationRouteOutcome::Orphan)
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, RouterState>, NotificationRouteError> {
        self.inner.lock().map_err(|_| {
            NotificationRouteError::InvalidNotification(
                "Notification router lock is poisoned.".to_string(),
            )
        })
    }
}

fn log_unobserved_diagnostic(kind: &str, notification: &RoutedNotification) {
    eprintln!(
        "[cadastrophe:router] unobserved {kind} notification method={:?} transportSequence={} threadId={:?} turnId={:?} itemId={:?}",
        notification.method,
        notification.transport_sequence,
        notification.identifiers.thread_id,
        notification.identifiers.turn_id,
        notification.identifiers.item_id,
    );
}

pub fn extract_notification_identifiers(
    raw: &Value,
) -> Result<NotificationIdentifiers, NotificationRouteError> {
    let params = raw.get("params").unwrap_or(&Value::Null);
    let thread_id = unique_identifier(
        "threadId",
        params,
        &[
            &["threadId"],
            &["thread", "id"],
            &["turn", "threadId"],
            &["item", "threadId"],
        ],
    )?;
    let turn_id = unique_identifier(
        "turnId",
        params,
        &[&["turnId"], &["turn", "id"], &["item", "turnId"]],
    )?;
    let item_id = unique_identifier("itemId", params, &[&["itemId"], &["item", "id"]])?;
    Ok(NotificationIdentifiers {
        thread_id,
        turn_id,
        item_id,
    })
}

fn parse_notification(
    transport_sequence: u64,
    raw: Value,
) -> Result<RoutedNotification, NotificationRouteError> {
    let method = raw
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.trim().is_empty())
        .ok_or_else(|| {
            NotificationRouteError::InvalidNotification(
                "Codex notification must contain a non-empty string method.".to_string(),
            )
        })?
        .to_string();
    let identifiers = extract_notification_identifiers(&raw)?;
    validate_method_identifiers(&method, &identifiers)?;
    Ok(RoutedNotification {
        transport_sequence,
        method,
        identifiers,
        raw,
    })
}

fn validate_method_identifiers(
    method: &str,
    identifiers: &NotificationIdentifiers,
) -> Result<(), NotificationRouteError> {
    if method.starts_with("thread/") && identifiers.thread_id.is_none() {
        return Err(missing_identifier(method, "threadId"));
    }
    if method.starts_with("turn/") {
        if identifiers.thread_id.is_none() {
            return Err(missing_identifier(method, "threadId"));
        }
        if identifiers.turn_id.is_none() {
            return Err(missing_identifier(method, "turnId"));
        }
    }
    if method.starts_with("item/") {
        if identifiers.thread_id.is_none() {
            return Err(missing_identifier(method, "threadId"));
        }
        if identifiers.turn_id.is_none() {
            return Err(missing_identifier(method, "turnId"));
        }
        if identifiers.item_id.is_none() {
            return Err(missing_identifier(method, "itemId"));
        }
    }
    Ok(())
}

fn missing_identifier(method: &str, identifier: &str) -> NotificationRouteError {
    NotificationRouteError::InvalidNotification(format!(
        "Codex notification method {method:?} requires {identifier}."
    ))
}

fn unique_identifier(
    kind: &'static str,
    root: &Value,
    paths: &[&[&str]],
) -> Result<Option<String>, NotificationRouteError> {
    let mut found: Option<String> = None;
    for path in paths {
        let Some(value) = value_at_path(root, path) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let candidate = value.as_str().ok_or_else(|| {
            NotificationRouteError::InvalidNotification(format!(
                "Notification {kind} must be a string when present."
            ))
        })?;
        validate_identifier(kind, candidate)?;
        if let Some(previous) = found.as_ref() {
            if previous != candidate {
                return Err(NotificationRouteError::ConflictingIdentifier {
                    kind,
                    first: previous.clone(),
                    second: candidate.to_string(),
                });
            }
        } else {
            found = Some(candidate.to_string());
        }
    }
    Ok(found)
}

fn value_at_path<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, key| value.get(*key))
}

fn validate_route_key(key: &NotificationRouteKey) -> Result<(), NotificationRouteError> {
    validate_identifier("threadId", &key.thread_id)?;
    validate_identifier("turnId", &key.turn_id)
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), NotificationRouteError> {
    if value.trim().is_empty() {
        return Err(NotificationRouteError::InvalidNotification(format!(
            "Notification {kind} must not be empty."
        )));
    }
    Ok(())
}

fn is_terminal_method(method: &str) -> bool {
    matches!(
        method,
        "turn/completed" | "turn/failed" | "turn/interrupted"
    )
}

fn map_try_send_error(
    error: mpsc::error::TrySendError<RoutedNotification>,
    key: &NotificationRouteKey,
) -> NotificationRouteError {
    match error {
        mpsc::error::TrySendError::Closed(_) => {
            NotificationRouteError::RouteChannelClosed(key.clone())
        }
        mpsc::error::TrySendError::Full(_) => NotificationRouteError::RouteChannelFull(key.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn router() -> NotificationRouter {
        NotificationRouter::new(NotificationRouterConfig {
            route_capacity: 8,
            pending_buffer_capacity: 2,
            diagnostic_capacity: 8,
        })
        .unwrap()
    }

    #[test]
    fn extracts_ids_from_flat_and_nested_notifications() {
        let flat = extract_notification_identifiers(&json!({
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1"
            }
        }))
        .unwrap();
        assert_eq!(
            flat,
            NotificationIdentifiers {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("item-1".to_string()),
            }
        );

        let nested = extract_notification_identifiers(&json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-2",
                "turn": {"id": "turn-2"},
                "item": {"id": "item-2", "turnId": "turn-2"}
            }
        }))
        .unwrap();
        assert_eq!(nested.thread_id.as_deref(), Some("thread-2"));
        assert_eq!(nested.turn_id.as_deref(), Some("turn-2"));
        assert_eq!(nested.item_id.as_deref(), Some("item-2"));
    }

    #[test]
    fn rejects_conflicting_identifiers() {
        let error = extract_notification_identifiers(&json!({
            "method": "turn/started",
            "params": {
                "threadId": "thread-1",
                "turn": {"id": "turn-1", "threadId": "thread-2"}
            }
        }))
        .unwrap_err();
        assert!(matches!(
            error,
            NotificationRouteError::ConflictingIdentifier {
                kind: "threadId",
                ..
            }
        ));
    }

    #[test]
    fn rejects_routable_notifications_with_missing_identifiers() {
        let error = router()
            .route(json!({
                "method": "item/agentMessage/delta",
                "params": {"threadId": "thread-1", "turnId": "turn-1", "delta": "hello"}
            }))
            .unwrap_err();
        assert_eq!(
            error,
            NotificationRouteError::InvalidNotification(
                "Codex notification method \"item/agentMessage/delta\" requires itemId."
                    .to_string()
            )
        );
    }

    #[tokio::test]
    async fn routes_only_to_the_exact_thread_and_turn() {
        let router = router();
        let key = NotificationRouteKey::new("thread-1", "turn-1");
        let mut receiver = router.register_route(key).unwrap();
        let mut orphans = router.subscribe_orphans();

        assert_eq!(
            router
                .route(json!({
                    "method": "item/started",
                    "params": {"threadId": "thread-2", "turnId": "turn-2", "itemId": "item-2"}
                }))
                .unwrap(),
            NotificationRouteOutcome::Orphan
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            orphans
                .recv()
                .await
                .unwrap()
                .identifiers
                .thread_id
                .as_deref(),
            Some("thread-2")
        );

        assert_eq!(
            router
                .route(json!({
                    "method": "item/started",
                    "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": "item-1"}
                }))
                .unwrap(),
            NotificationRouteOutcome::Exact
        );
        assert_eq!(receiver.recv().await.unwrap().transport_sequence, 2);
    }

    #[tokio::test]
    async fn buffers_early_events_and_flushes_them_on_promotion() {
        let router = router();
        let handle = router.begin_pending_route("thread-1").unwrap();
        router
            .route(json!({
                "method": "turn/started",
                "params": {"threadId": "thread-1", "turn": {"id": "turn-1"}}
            }))
            .unwrap();
        router
            .route(json!({
                "method": "item/started",
                "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": "item-1"}
            }))
            .unwrap();

        let mut receiver = router.promote_pending_route(handle, "turn-1").unwrap();
        assert_eq!(receiver.recv().await.unwrap().method, "turn/started");
        assert_eq!(receiver.recv().await.unwrap().method, "item/started");
    }

    #[tokio::test]
    async fn keeps_thread_scoped_events_out_of_pending_turn_routes() {
        let router = router();
        let handle = router.begin_pending_route("thread-1").unwrap();
        let mut orphans = router.subscribe_orphans();

        assert_eq!(
            router
                .route(json!({
                    "method": "thread/status/changed",
                    "params": {
                        "threadId": "thread-1",
                        "status": {"type": "active", "activeFlags": []}
                    }
                }))
                .unwrap(),
            NotificationRouteOutcome::Orphan
        );
        let diagnostic = orphans.recv().await.unwrap();
        assert_eq!(diagnostic.method, "thread/status/changed");
        assert_eq!(
            diagnostic.identifiers.thread_id.as_deref(),
            Some("thread-1")
        );
        assert_eq!(diagnostic.identifiers.turn_id, None);

        let mut receiver = router.promote_pending_route(handle, "turn-1").unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn refuses_to_promote_early_events_for_a_different_turn() {
        let router = router();
        let handle = router.begin_pending_route("thread-1").unwrap();
        router
            .route(json!({
                "method": "turn/started",
                "params": {"threadId": "thread-1", "turnId": "turn-other"}
            }))
            .unwrap();
        assert_eq!(
            router
                .promote_pending_route(handle.clone(), "turn-expected")
                .unwrap_err(),
            NotificationRouteError::PendingTurnMismatch {
                expected_turn_id: "turn-expected".to_string(),
                actual_turn_id: "turn-other".to_string(),
            }
        );
        assert_eq!(router.cancel_pending_route(&handle).unwrap(), 1);
    }

    #[test]
    fn fails_when_pending_buffer_is_full() {
        let router = router();
        router.begin_pending_route("thread-1").unwrap();
        for item_id in ["item-1", "item-2"] {
            router
                .route(json!({
                    "method": "item/started",
                    "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": item_id}
                }))
                .unwrap();
        }
        assert_eq!(
            router
                .route(json!({
                    "method": "item/started",
                    "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": "item-3"}
                }))
                .unwrap_err(),
            NotificationRouteError::PendingBufferFull("thread-1".to_string())
        );
    }

    #[tokio::test]
    async fn isolates_global_and_orphan_notifications() {
        let router = router();
        let mut global = router.subscribe_global();
        let mut orphans = router.subscribe_orphans();
        assert_eq!(
            router
                .route(json!({"method": "config/warning", "params": {"message": "warning"}}))
                .unwrap(),
            NotificationRouteOutcome::Global
        );
        assert_eq!(global.recv().await.unwrap().method, "config/warning");
        assert!(orphans.try_recv().is_err());

        assert_eq!(
            router
                .route(json!({
                    "method": "turn/started",
                    "params": {"threadId": "unregistered", "turnId": "turn-1"}
                }))
                .unwrap(),
            NotificationRouteOutcome::Orphan
        );
        assert_eq!(orphans.recv().await.unwrap().method, "turn/started");
        assert!(global.try_recv().is_err());
    }

    #[tokio::test]
    async fn suppresses_duplicate_terminal_events_per_route() {
        let router = router();
        let key = NotificationRouteKey::new("thread-1", "turn-1");
        let mut receiver = router.register_route(key).unwrap();
        let terminal = json!({
            "method": "turn/completed",
            "params": {"threadId": "thread-1", "turnId": "turn-1"}
        });
        assert_eq!(
            router.route(terminal.clone()).unwrap(),
            NotificationRouteOutcome::Exact
        );
        assert_eq!(
            router.route(terminal).unwrap(),
            NotificationRouteOutcome::DuplicateTerminalSuppressed
        );
        assert_eq!(receiver.recv().await.unwrap().method, "turn/completed");
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn suppresses_duplicate_terminal_events_while_pending() {
        let router = router();
        let handle = router.begin_pending_route("thread-1").unwrap();
        let terminal = json!({
            "method": "turn/completed",
            "params": {"threadId": "thread-1", "turnId": "turn-1"}
        });
        assert_eq!(
            router.route(terminal.clone()).unwrap(),
            NotificationRouteOutcome::BufferedPending
        );
        assert_eq!(
            router.route(terminal.clone()).unwrap(),
            NotificationRouteOutcome::DuplicateTerminalSuppressed
        );
        let mut receiver = router.promote_pending_route(handle, "turn-1").unwrap();
        assert_eq!(receiver.recv().await.unwrap().method, "turn/completed");
        assert_eq!(
            router.route(terminal).unwrap(),
            NotificationRouteOutcome::DuplicateTerminalSuppressed
        );
        assert!(receiver.try_recv().is_err());
    }
}
