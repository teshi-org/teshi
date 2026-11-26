use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    process::ChildStdin,
    sync::{Mutex as AsyncMutex, mpsc, oneshot, watch},
    task::JoinHandle,
};

use crate::{
    client::{AcpClient, AcpClientConfig, AcpEvent, InitializeResult},
    command::AcpAgentCommand,
    error::{AcpError, AcpResult, redact_secret},
    permission::{PermissionHost, PermissionPolicy},
};

/// Cache key identifying a reusable ACP child process per workspace and provider config.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AcpConnectionKey {
    pub provider_id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub auth_method: Option<String>,
}

impl std::fmt::Debug for AcpConnectionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_env = self
            .env
            .iter()
            .map(|(k, v)| redact_secret(&format!("{k}={v}")))
            .collect::<Vec<_>>();
        f.debug_struct("AcpConnectionKey")
            .field("provider_id", &self.provider_id)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env", &redacted_env)
            .field("auth_method", &self.auth_method)
            .finish()
    }
}

impl AcpConnectionKey {
    /// Build a canonical key from an [`AcpAgentCommand`], sorting environment variables.
    pub fn from_command(
        provider_id: impl Into<String>,
        command: &AcpAgentCommand,
        auth_method: Option<&str>,
    ) -> Self {
        let mut env = command
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>();
        env.sort();
        Self {
            provider_id: provider_id.into(),
            program: command.program.clone(),
            args: command.args.clone(),
            cwd: command.cwd.clone(),
            env,
            auth_method: auth_method.map(str::to_string),
        }
    }
}

/// Concurrency and reuse strategy for pooled ACP connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AcpPoolMode {
    /// Multiplex multiple concurrent sessions onto one shared ACP process per key.
    ///
    /// Only safe for agents whose ACP implementation isolates concurrent
    /// `session/prompt` turns across sessions within a single process.
    #[default]
    Shared,
    /// Give each active session exclusive ownership of an initialized ACP process,
    /// returning live processes to an idle cache when the session closes and
    /// optionally pre-warming standby processes in the background.
    Exclusive {
        /// Number of initialized idle standby processes to keep warm in the
        /// background for immediate checkout by the next session (`0` disables
        /// background pre-warming).
        prewarm_standby: usize,
        /// Maximum number of idle connections retained per key when leases drop.
        /// Excess idle connections are shut down in the background.
        max_idle: usize,
    },
}

impl AcpPoolMode {
    /// Default exclusive-per-session mode: keeps one background warm standby
    /// process and retains up to two idle connections per key.
    pub const fn exclusive_warm() -> Self {
        Self::Exclusive {
            prewarm_standby: 1,
            max_idle: 2,
        }
    }

    /// Exclusive-per-session mode with idle reuse but no background pre-warming.
    pub const fn exclusive_no_prewarm() -> Self {
        Self::Exclusive {
            prewarm_standby: 0,
            max_idle: 2,
        }
    }
}

#[derive(Clone)]
struct SpawnSpec {
    command: AcpAgentCommand,
    client_config: AcpClientConfig,
    default_policy: PermissionPolicy,
    default_host: Option<Arc<dyn PermissionHost>>,
}

struct PoolEntry {
    conn_id: u64,
    key: AcpConnectionKey,
    client: Arc<AcpClient<ChildStdin>>,
    initialized: InitializeResult,
    active_sessions: AtomicUsize,
    exclusive: bool,
    max_idle: usize,
    invalidated: AtomicBool,
}

struct InFlightPrewarm {
    id: u64,
    claimed: AtomicBool,
    waiter: StdMutex<Option<oneshot::Sender<AcpResult<Arc<PoolEntry>>>>>,
}

struct PoolState {
    entries: HashMap<AcpConnectionKey, Vec<Arc<PoolEntry>>>,
    prewarms: HashMap<AcpConnectionKey, Vec<Arc<InFlightPrewarm>>>,
    prewarm_tasks: Vec<JoinHandle<()>>,
    acquire_tasks: Vec<JoinHandle<()>>,
    shutdown_tasks: Vec<JoinHandle<()>>,
    epoch: u64,
}

impl PoolState {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            prewarms: HashMap::new(),
            prewarm_tasks: Vec::new(),
            acquire_tasks: Vec::new(),
            shutdown_tasks: Vec::new(),
            epoch: 0,
        }
    }

    fn contains_entry(&self, key: &AcpConnectionKey, conn_id: u64) -> bool {
        self.entries
            .get(key)
            .is_some_and(|list| list.iter().any(|e| e.conn_id == conn_id))
    }

    fn remove_entry(&mut self, key: &AcpConnectionKey, conn_id: u64) -> Option<Arc<PoolEntry>> {
        let mut removed = None;
        if let Some(list) = self.entries.get_mut(key) {
            if let Some(pos) = list.iter().position(|e| e.conn_id == conn_id) {
                removed = Some(list.remove(pos));
            }
            if list.is_empty() {
                self.entries.remove(key);
            }
        }
        removed
    }

    fn remove_prewarm(&mut self, key: &AcpConnectionKey, prewarm_id: u64) {
        if let Some(list) = self.prewarms.get_mut(key) {
            list.retain(|p| p.id != prewarm_id);
            if list.is_empty() {
                self.prewarms.remove(key);
            }
        }
    }

    fn drain_all_entries(&mut self) -> Vec<Arc<PoolEntry>> {
        self.entries
            .drain()
            .flat_map(|(_, list)| list.into_iter())
            .collect()
    }

    fn cancel_all_prewarms(&mut self) {
        for (_, list) in self.prewarms.drain() {
            for prewarm in list {
                if let Ok(mut guard) = prewarm.waiter.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(Err(AcpError::Shutdown("ACP pool shut down".into())));
                }
            }
        }
    }

    fn record_shutdown_task(&mut self, task: JoinHandle<()>) {
        self.shutdown_tasks.retain(|t| !t.is_finished());
        self.shutdown_tasks.push(task);
    }

    fn record_prewarm_task(&mut self, task: JoinHandle<()>) {
        self.prewarm_tasks.retain(|t| !t.is_finished());
        self.prewarm_tasks.push(task);
    }

    fn record_acquire_task(&mut self, task: JoinHandle<()>) {
        self.acquire_tasks.retain(|t| !t.is_finished());
        self.acquire_tasks.push(task);
    }
}

struct PoolInner {
    runtime: StdMutex<Option<tokio::runtime::Runtime>>,
    state: Arc<StdMutex<PoolState>>,
    connect_locks: AsyncMutex<HashMap<AcpConnectionKey, Arc<AsyncMutex<()>>>>,
    next_conn_id: Arc<AtomicU64>,
    epoch_tx: watch::Sender<u64>,
}

impl PoolInner {
    fn runtime_handle(&self) -> AcpResult<tokio::runtime::Handle> {
        self.runtime
            .lock()
            .map_err(|_| AcpError::Shutdown("ACP pool runtime lock poisoned".into()))?
            .as_ref()
            .map(|rt| rt.handle().clone())
            .ok_or_else(|| AcpError::Shutdown("ACP pool runtime already shut down".into()))
    }
}

fn spawn_entry_shutdown(handle: &tokio::runtime::Handle, entry: Arc<PoolEntry>) -> JoinHandle<()> {
    handle.spawn(async move {
        if let Err(error) = entry.client.shutdown_shared().await {
            tracing::warn!(
                target: "teshi_acp::pool",
                provider_id = %entry.key.provider_id,
                conn_id = entry.conn_id,
                error = %error,
                "failed to shut down pooled ACP client"
            );
        }
    })
}

fn remove_dead_entries_locked(
    guard: &mut PoolState,
    handle: Option<&tokio::runtime::Handle>,
    key: &AcpConnectionKey,
) -> Vec<Arc<PoolEntry>> {
    let mut removed = Vec::new();
    if let Some(list) = guard.entries.get_mut(key) {
        let mut idx = 0;
        while idx < list.len() {
            let keep = list[idx].client.is_alive() && !list[idx].invalidated.load(Ordering::SeqCst);
            if keep {
                idx += 1;
            } else {
                removed.push(list.remove(idx));
            }
        }
        if list.is_empty() {
            guard.entries.remove(key);
        }
    }
    if let Some(handle) = handle {
        for entry in &removed {
            if entry.active_sessions.load(Ordering::SeqCst) == 0 {
                let task = spawn_entry_shutdown(handle, Arc::clone(entry));
                guard.record_shutdown_task(task);
            }
        }
    }
    removed
}

fn trim_excess_idle_locked(
    guard: &mut PoolState,
    handle: &tokio::runtime::Handle,
    key: &AcpConnectionKey,
    max_idle: usize,
) -> Vec<Arc<PoolEntry>> {
    let removed_dead = remove_dead_entries_locked(guard, Some(handle), key);
    let Some(list) = guard.entries.get_mut(key) else {
        return removed_dead;
    };
    let mut idle_indices = Vec::new();
    for (idx, entry) in list.iter().enumerate() {
        if entry.exclusive && entry.active_sessions.load(Ordering::SeqCst) == 0 {
            idle_indices.push(idx);
        }
    }
    let excess = idle_indices.len().saturating_sub(max_idle);
    let mut evicted = Vec::with_capacity(excess);
    // Remove oldest idle entries first (front of the list), iterating in reverse
    // index order so earlier removals do not shift later indices.
    for idx in idle_indices.into_iter().take(excess).rev() {
        evicted.push(list.remove(idx));
    }
    if list.is_empty() {
        guard.entries.remove(key);
    }
    for entry in evicted {
        let task = spawn_entry_shutdown(handle, entry);
        guard.record_shutdown_task(task);
    }
    removed_dead
}

fn trim_excess_idle_with(
    state: &Arc<StdMutex<PoolState>>,
    handle: &tokio::runtime::Handle,
    key: &AcpConnectionKey,
    max_idle: usize,
) {
    let removed = {
        let Ok(mut guard) = state.lock() else {
            return;
        };
        trim_excess_idle_locked(&mut guard, handle, key, max_idle)
    };
    drop(removed);
}

fn attach_exit_watcher(
    handle: &tokio::runtime::Handle,
    state: Arc<StdMutex<PoolState>>,
    key: AcpConnectionKey,
    conn_id: u64,
    client: &AcpClient<ChildStdin>,
) {
    let mut exit_rx = client.subscribe_exit();
    let shutdown_handle = handle.clone();
    handle.spawn(async move {
        while !*exit_rx.borrow() {
            if exit_rx.changed().await.is_err() {
                break;
            }
        }
        let removed = if let Ok(mut guard) = state.lock() {
            let removed = guard.remove_entry(&key, conn_id);
            if let Some(entry) = &removed
                && entry.active_sessions.load(Ordering::SeqCst) == 0
            {
                let task = spawn_entry_shutdown(&shutdown_handle, Arc::clone(entry));
                guard.record_shutdown_task(task);
            }
            removed
        } else {
            None
        };
        drop(removed);
    });
}

async fn wait_epoch_change(rx: &mut watch::Receiver<u64>, start_epoch: u64) {
    while *rx.borrow() == start_epoch {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

async fn spawn_and_init_client(
    spec: SpawnSpec,
    auth_method: Option<&str>,
    mut epoch_rx: watch::Receiver<u64>,
    start_epoch: u64,
) -> AcpResult<(Arc<AcpClient<ChildStdin>>, InitializeResult)> {
    if *epoch_rx.borrow() != start_epoch {
        return Err(AcpError::Shutdown("ACP pool shut down".into()));
    }
    let (default_events_tx, _default_events_rx) = mpsc::channel::<AcpEvent>(32);
    let client = Arc::new(
        AcpClient::spawn(
            spec.command,
            spec.client_config,
            default_events_tx,
            spec.default_policy,
            spec.default_host,
        )
        .await?,
    );
    if *epoch_rx.borrow() != start_epoch {
        let _ = client.shutdown_shared().await;
        return Err(AcpError::Shutdown("ACP pool shut down".into()));
    }
    let init_res = tokio::select! {
        res = client.initialize_shared() => res,
        () = wait_epoch_change(&mut epoch_rx, start_epoch) => {
            let _ = client.shutdown_shared().await;
            return Err(AcpError::Shutdown("ACP pool shut down".into()));
        }
    };
    let initialized = match init_res {
        Ok(init) => init,
        Err(error) => {
            let _ = client.shutdown_shared().await;
            return Err(error);
        }
    };
    if let Some(method) = auth_method {
        let auth_res = tokio::select! {
            res = client.authenticate(method) => res,
            () = wait_epoch_change(&mut epoch_rx, start_epoch) => {
                let _ = client.shutdown_shared().await;
                return Err(AcpError::Shutdown("ACP pool shut down".into()));
            }
        };
        if let Err(error) = auth_res {
            let _ = client.shutdown_shared().await;
            return Err(error);
        }
    }
    if *epoch_rx.borrow() != start_epoch {
        let _ = client.shutdown_shared().await;
        return Err(AcpError::Shutdown("ACP pool shut down".into()));
    }
    if !client.check_alive().await {
        let _ = client.shutdown_shared().await;
        return Err(AcpError::ProcessExited);
    }
    Ok((client, initialized))
}

fn schedule_exclusive_prewarm(
    inner: &PoolInner,
    handle: &tokio::runtime::Handle,
    key: &AcpConnectionKey,
    spec: &SpawnSpec,
    prewarm_standby: usize,
    max_idle: usize,
    expected_epoch: u64,
) {
    if prewarm_standby == 0 {
        return;
    }
    let effective_max_idle = max_idle.max(prewarm_standby);
    let Ok(mut guard) = inner.state.lock() else {
        return;
    };
    if guard.epoch != expected_epoch {
        return;
    }
    let idle_count = guard
        .entries
        .get(key)
        .map(|list| {
            list.iter()
                .filter(|e| {
                    e.exclusive
                        && e.client.is_alive()
                        && !e.invalidated.load(Ordering::SeqCst)
                        && e.active_sessions.load(Ordering::SeqCst) == 0
                })
                .count()
        })
        .unwrap_or(0);
    let unclaimed_prewarms = guard
        .prewarms
        .get(key)
        .map(|list| {
            list.iter()
                .filter(|p| !p.claimed.load(Ordering::SeqCst))
                .count()
        })
        .unwrap_or(0);
    let needed = prewarm_standby.saturating_sub(idle_count + unclaimed_prewarms);
    if needed == 0 {
        return;
    }
    let start_epoch = guard.epoch;
    let mut to_spawn = Vec::with_capacity(needed);
    {
        let list = guard.prewarms.entry(key.clone()).or_default();
        for _ in 0..needed {
            let id = inner.next_conn_id.fetch_add(1, Ordering::SeqCst);
            let prewarm = Arc::new(InFlightPrewarm {
                id,
                claimed: AtomicBool::new(false),
                waiter: StdMutex::new(None),
            });
            list.push(Arc::clone(&prewarm));
            to_spawn.push(prewarm);
        }
    }

    for prewarm in to_spawn {
        let state = Arc::clone(&inner.state);
        let next_conn_id = Arc::clone(&inner.next_conn_id);
        let epoch_rx = inner.epoch_tx.subscribe();
        let task_handle = handle.clone();
        let key_clone = key.clone();
        let spec = spec.clone();
        let task = handle.spawn(async move {
            let auth_method = key_clone.auth_method.clone();
            let result =
                spawn_and_init_client(spec, auth_method.as_deref(), epoch_rx, start_epoch).await;

            enum Action {
                ShutdownAndNotify(
                    Option<Arc<AcpClient<ChildStdin>>>,
                    Option<oneshot::Sender<AcpResult<Arc<PoolEntry>>>>,
                ),
                Deliver(
                    Arc<PoolEntry>,
                    Option<oneshot::Sender<AcpResult<Arc<PoolEntry>>>>,
                ),
                NotifyErr(AcpError, Option<oneshot::Sender<AcpResult<Arc<PoolEntry>>>>),
            }

            let action = {
                let Ok(mut guard) = state.lock() else {
                    if let Ok((client, _)) = result {
                        let _ = client.shutdown_shared().await;
                    }
                    return;
                };
                guard.remove_prewarm(&key_clone, prewarm.id);
                let waiter = prewarm.waiter.lock().ok().and_then(|mut w| w.take());
                if guard.epoch != start_epoch {
                    let client = result.ok().map(|(c, _)| c);
                    Action::ShutdownAndNotify(client, waiter)
                } else {
                    match result {
                        Ok((client, initialized)) => {
                            let conn_id = next_conn_id.fetch_add(1, Ordering::SeqCst);
                            let initial_sessions = usize::from(waiter.is_some());
                            let entry = Arc::new(PoolEntry {
                                conn_id,
                                key: key_clone.clone(),
                                client,
                                initialized,
                                active_sessions: AtomicUsize::new(initial_sessions),
                                exclusive: true,
                                max_idle: effective_max_idle,
                                invalidated: AtomicBool::new(false),
                            });
                            guard
                                .entries
                                .entry(key_clone.clone())
                                .or_default()
                                .push(Arc::clone(&entry));
                            Action::Deliver(entry, waiter)
                        }
                        Err(err) => Action::NotifyErr(err, waiter),
                    }
                }
            };

            match action {
                Action::ShutdownAndNotify(client, waiter) => {
                    if let Some(client) = client {
                        let _ = client.shutdown_shared().await;
                    }
                    if let Some(tx) = waiter {
                        let _ = tx.send(Err(AcpError::Shutdown("ACP pool shut down".into())));
                    }
                }
                Action::Deliver(entry, waiter) => {
                    attach_exit_watcher(
                        &task_handle,
                        Arc::clone(&state),
                        key_clone.clone(),
                        entry.conn_id,
                        &entry.client,
                    );
                    if let Some(tx) = waiter {
                        if tx.send(Ok(Arc::clone(&entry))).is_err() {
                            // Waiter dropped before receiving the entry; return it to idle or retire under lock.
                            let mut drop_outside_lock = Vec::new();
                            if let Ok(mut guard) = state.lock() {
                                let prev = entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
                                if prev == 1 {
                                    if entry.invalidated.load(Ordering::SeqCst)
                                        || !entry.client.is_alive()
                                        || !entry.client.is_idle_clean()
                                    {
                                        if let Some(removed) =
                                            guard.remove_entry(&key_clone, entry.conn_id)
                                        {
                                            drop_outside_lock.push(removed);
                                        }
                                        let task =
                                            spawn_entry_shutdown(&task_handle, Arc::clone(&entry));
                                        guard.record_shutdown_task(task);
                                    } else {
                                        drop_outside_lock.extend(trim_excess_idle_locked(
                                            &mut guard,
                                            &task_handle,
                                            &key_clone,
                                            effective_max_idle,
                                        ));
                                    }
                                }
                            } else {
                                entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
                            }
                            drop(drop_outside_lock);
                        }
                    } else {
                        trim_excess_idle_with(&state, &task_handle, &key_clone, effective_max_idle);
                    }
                }
                Action::NotifyErr(err, waiter) => {
                    if let Some(tx) = waiter {
                        let _ = tx.send(Err(err));
                    }
                }
            }
        });
        guard.record_prewarm_task(task);
    }
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        let (remaining_entries, remaining_prewarms, remaining_acquires, remaining_tasks) = {
            if let Ok(mut guard) = self.state.lock() {
                guard.epoch = guard.epoch.wrapping_add(1);
                let _ = self.epoch_tx.send(guard.epoch);
                guard.cancel_all_prewarms();
                let entries = guard.drain_all_entries();
                let prewarms = std::mem::take(&mut guard.prewarm_tasks);
                let acquires = std::mem::take(&mut guard.acquire_tasks);
                let tasks = std::mem::take(&mut guard.shutdown_tasks);
                (entries, prewarms, acquires, tasks)
            } else {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
            }
        };
        let runtime = self.runtime.get_mut().ok().and_then(|slot| slot.take());
        let Some(runtime) = runtime else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            for task in &remaining_prewarms {
                task.abort();
            }
            for task in &remaining_acquires {
                task.abort();
            }
            drop(remaining_prewarms);
            drop(remaining_acquires);
            drop(remaining_tasks);
            drop(remaining_entries);
            runtime.shutdown_background();
            return;
        }
        runtime.block_on(async {
            let mut join_set = tokio::task::JoinSet::new();
            for entry in remaining_entries {
                join_set.spawn(async move {
                    let _ = entry.client.shutdown_shared().await;
                });
            }
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                while join_set.join_next().await.is_some() {}
            })
            .await;

            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                for task in remaining_prewarms {
                    let _ = task.await;
                }
                for task in remaining_acquires {
                    let _ = task.await;
                }
                for task in remaining_tasks {
                    let _ = task.await;
                }
            })
            .await;
        });
        runtime.shutdown_timeout(Duration::from_millis(500));
    }
}

/// Shared pool of live ACP agent connections supporting both shared multi-session
/// multiplexing and per-session exclusive ownership with idle reuse / warm standby.
#[derive(Clone)]
pub struct AcpConnectionPool {
    inner: Arc<PoolInner>,
}

impl std::fmt::Debug for AcpConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpConnectionPool")
            .field("active_connections", &self.active_connection_count())
            .field("idle_connections", &self.idle_connection_count())
            .field("active_sessions", &self.total_active_sessions())
            .finish()
    }
}

impl Default for AcpConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl AcpConnectionPool {
    /// Create a pool backed by a dedicated multi-thread Tokio I/O runtime.
    ///
    /// # Panics
    /// Panics if the OS refuses to create the pool's Tokio runtime.
    pub fn new() -> Self {
        Self::try_new().expect("failed to build ACP connection pool Tokio runtime")
    }

    /// Fallible constructor for [`Self::new`].
    ///
    /// # Errors
    /// Returns an error if the Tokio runtime cannot be built.
    pub fn try_new() -> AcpResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("teshi-acp-pool")
            .enable_all()
            .build()
            .map_err(|e| AcpError::Shutdown(format!("failed to create ACP pool runtime: {e}")))?;
        let (epoch_tx, _epoch_rx) = watch::channel(0_u64);
        Ok(Self {
            inner: Arc::new(PoolInner {
                runtime: StdMutex::new(Some(runtime)),
                state: Arc::new(StdMutex::new(PoolState::new())),
                connect_locks: AsyncMutex::new(HashMap::new()),
                next_conn_id: Arc::new(AtomicU64::new(1)),
                epoch_tx,
            }),
        })
    }

    /// Run an async operation synchronously on the pool's shared runtime.
    ///
    /// Using this instead of a per-session `current_thread` runtime ensures that
    /// child processes, reader loops, and dispatchers outlive whichever session
    /// happened to spawn the connection.
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        let handle = self
            .inner
            .runtime_handle()
            .expect("ACP pool runtime missing");
        handle.block_on(future)
    }

    /// Handle to the pool's dedicated Tokio runtime.
    ///
    /// # Errors
    /// Returns an error if the pool has already dropped its runtime.
    pub fn runtime_handle(&self) -> AcpResult<tokio::runtime::Handle> {
        self.inner.runtime_handle()
    }

    /// Reuse a live shared ACP connection for `key`, or spawn, initialize, and cache one.
    ///
    /// Equivalent to [`Self::acquire_with_mode`] with [`AcpPoolMode::Shared`].
    ///
    /// # Errors
    /// Returns spawn, initialize, authentication, or runtime errors.
    pub async fn acquire(
        &self,
        key: AcpConnectionKey,
        command: AcpAgentCommand,
        client_config: AcpClientConfig,
        default_policy: PermissionPolicy,
        default_host: Option<Arc<dyn PermissionHost>>,
    ) -> AcpResult<PooledAcpConnection> {
        self.acquire_with_mode(
            key,
            command,
            client_config,
            default_policy,
            default_host,
            AcpPoolMode::Shared,
        )
        .await
    }

    /// Acquire a pooled ACP connection using the requested [`AcpPoolMode`].
    ///
    /// - In [`AcpPoolMode::Shared`], concurrent callers for the same key share one
    ///   process and serialize initial startup behind a per-key mutex.
    /// - In [`AcpPoolMode::Exclusive`], each active lease owns its own process:
    ///   idle cached entries (or an unclaimed in-flight standby prewarm) are
    ///   checked out immediately; otherwise a new process spawns in parallel
    ///   without blocking other concurrent sessions. Dropping the lease returns
    ///   the live process to the idle cache (bounded by `max_idle`).
    ///
    /// # Errors
    /// Returns spawn, initialize, authentication, or runtime errors.
    pub async fn acquire_with_mode(
        &self,
        key: AcpConnectionKey,
        command: AcpAgentCommand,
        client_config: AcpClientConfig,
        default_policy: PermissionPolicy,
        default_host: Option<Arc<dyn PermissionHost>>,
        mode: AcpPoolMode,
    ) -> AcpResult<PooledAcpConnection> {
        let handle = self.inner.runtime_handle()?;
        let inner = Arc::clone(&self.inner);
        let task_handle = handle.clone();
        let (result_tx, result_rx) = oneshot::channel();
        {
            let mut guard = self
                .inner
                .state
                .lock()
                .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
            let start_epoch = guard.epoch;
            let epoch_rx = self.inner.epoch_tx.subscribe();
            let task = handle.spawn(async move {
                let spec = SpawnSpec {
                    command,
                    client_config,
                    default_policy,
                    default_host,
                };
                let result = match mode {
                    AcpPoolMode::Shared => {
                        acquire_shared(inner, task_handle, key, spec, start_epoch, epoch_rx).await
                    }
                    AcpPoolMode::Exclusive {
                        prewarm_standby,
                        max_idle,
                    } => {
                        acquire_exclusive(
                            inner,
                            task_handle,
                            key,
                            spec,
                            prewarm_standby,
                            max_idle,
                            start_epoch,
                            epoch_rx,
                        )
                        .await
                    }
                };
                let _ = result_tx.send(result);
            });
            guard.record_acquire_task(task);
        }
        result_rx
            .await
            .map_err(|_| AcpError::Shutdown("ACP pool acquire task failed".into()))?
    }

    /// Return an already-live pooled connection for `key` without spawning a new one.
    ///
    /// Prefers a shared connection or an idle clean exclusive connection so active
    /// exclusive sessions are not disturbed.
    pub fn get_live(&self, key: &AcpConnectionKey) -> Option<PooledAcpConnection> {
        let handle = self.inner.runtime_handle().ok();
        let (chosen, removed_dead) = {
            let mut guard = self.inner.state.lock().ok()?;
            let removed_dead = remove_dead_entries_locked(&mut guard, handle.as_ref(), key);
            let chosen = guard.entries.get(key).and_then(|list| {
                list.iter()
                    .find(|e| !e.exclusive && !e.invalidated.load(Ordering::SeqCst))
                    .map(|e| {
                        e.active_sessions.fetch_add(1, Ordering::SeqCst);
                        Arc::clone(e)
                    })
                    .or_else(|| {
                        list.iter().rev().find_map(|e| {
                            (e.exclusive
                                && !e.invalidated.load(Ordering::SeqCst)
                                && e.client.is_idle_clean())
                            .then(|| {
                                e.active_sessions
                                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                                    .ok()
                                    .map(|_| Arc::clone(e))
                            })
                            .flatten()
                        })
                    })
            });
            (chosen, removed_dead)
        };
        drop(removed_dead);
        let entry = chosen?;
        Some(PooledAcpConnection {
            entry,
            pool: Arc::clone(&self.inner),
        })
    }

    /// Find any live pooled connection matching a project workspace path and provider id.
    ///
    /// Returns a shared connection or an idle clean exclusive connection so active
    /// exclusive sessions are never disturbed (used for metadata RPCs such as
    /// `session/list` and `session/delete`).
    pub fn find_live_by_cwd_and_provider(
        &self,
        cwd: &Path,
        provider_id: &str,
    ) -> Option<PooledAcpConnection> {
        let entry = {
            let guard = self.inner.state.lock().ok()?;
            let matching = || {
                guard
                    .entries
                    .values()
                    .flat_map(|list| list.iter())
                    .filter(|entry| {
                        entry.key.provider_id == provider_id
                            && entry.key.cwd == cwd
                            && entry.client.is_alive()
                            && !entry.invalidated.load(Ordering::SeqCst)
                    })
            };
            matching()
                .find(|e| !e.exclusive)
                .map(|e| {
                    e.active_sessions.fetch_add(1, Ordering::SeqCst);
                    Arc::clone(e)
                })
                .or_else(|| {
                    matching().find_map(|e| {
                        (e.exclusive && e.client.is_idle_clean())
                            .then(|| {
                                e.active_sessions
                                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                                    .ok()
                                    .map(|_| Arc::clone(e))
                            })
                            .flatten()
                    })
                })
        }?;
        Some(PooledAcpConnection {
            entry,
            pool: Arc::clone(&self.inner),
        })
    }

    /// Number of currently cached live ACP connections (active and idle).
    pub fn active_connection_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .map(|guard| {
                guard
                    .entries
                    .values()
                    .flat_map(|list| list.iter())
                    .filter(|e| e.client.is_alive())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Number of live cached ACP connections currently idle (`active_sessions == 0`).
    pub fn idle_connection_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .map(|guard| {
                guard
                    .entries
                    .values()
                    .flat_map(|list| list.iter())
                    .filter(|e| {
                        e.client.is_alive() && e.active_sessions.load(Ordering::SeqCst) == 0
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    /// Total active session leases across all cached connections.
    pub fn total_active_sessions(&self) -> usize {
        self.inner
            .state
            .lock()
            .map(|guard| {
                guard
                    .entries
                    .values()
                    .flat_map(|list| list.iter())
                    .filter(|e| e.client.is_alive())
                    .map(|e| e.active_sessions.load(Ordering::SeqCst))
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Begin non-blocking shutdown of all pooled connections and in-flight prewarm/acquire tasks.
    pub fn request_shutdown(&self) {
        let handle = self.inner.runtime_handle().ok();
        let remaining_entries = {
            let Ok(mut guard) = self.inner.state.lock() else {
                return;
            };
            guard.epoch = guard.epoch.wrapping_add(1);
            let _ = self.inner.epoch_tx.send(guard.epoch);
            guard.cancel_all_prewarms();
            let entries = guard.drain_all_entries();
            let prewarms = std::mem::take(&mut guard.prewarm_tasks);
            let acquires = std::mem::take(&mut guard.acquire_tasks);
            guard.shutdown_tasks.retain(|task| !task.is_finished());
            guard.shutdown_tasks.extend(prewarms);
            guard.shutdown_tasks.extend(acquires);
            if let Some(handle) = handle {
                for entry in entries {
                    let task = spawn_entry_shutdown(&handle, entry);
                    guard.shutdown_tasks.push(task);
                }
                Vec::new()
            } else {
                entries
            }
        };
        drop(remaining_entries);
    }

    /// Whether any background shutdown tasks initiated by [`Self::request_shutdown`],
    /// invalidation, or excess-idle eviction are still running.
    pub fn shutdown_pending(&self) -> bool {
        !self.poll_shutdown()
    }

    /// Reap completed shutdown tasks. Returns `true` when none remain.
    pub fn poll_shutdown(&self) -> bool {
        let Ok(mut guard) = self.inner.state.lock() else {
            return true;
        };
        guard.shutdown_tasks.retain(|task| !task.is_finished());
        guard.shutdown_tasks.is_empty()
    }

    /// Shut down all pooled connections and in-flight prewarm/acquire tasks and wait for completion.
    pub async fn shutdown_all(&self) {
        let (entries, prewarms, acquires, tasks) = {
            if let Ok(mut guard) = self.inner.state.lock() {
                guard.epoch = guard.epoch.wrapping_add(1);
                let _ = self.inner.epoch_tx.send(guard.epoch);
                guard.cancel_all_prewarms();
                let entries = guard.drain_all_entries();
                let prewarms = std::mem::take(&mut guard.prewarm_tasks);
                let acquires = std::mem::take(&mut guard.acquire_tasks);
                let tasks = std::mem::take(&mut guard.shutdown_tasks);
                (entries, prewarms, acquires, tasks)
            } else {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
            }
        };
        if let Ok(handle) = self.inner.runtime_handle() {
            let mut shutdown_handles = Vec::with_capacity(entries.len());
            for entry in entries {
                shutdown_handles.push(spawn_entry_shutdown(&handle, entry));
            }
            for task in shutdown_handles {
                let _ = task.await;
            }
        } else {
            for entry in entries {
                if let Err(error) = entry.client.shutdown_shared().await {
                    tracing::warn!(
                        target: "teshi_acp::pool",
                        provider_id = %entry.key.provider_id,
                        conn_id = entry.conn_id,
                        error = %error,
                        "failed to shut down pooled ACP client"
                    );
                }
            }
        }
        for task in prewarms {
            let _ = task.await;
        }
        for task in acquires {
            let _ = task.await;
        }
        for task in tasks {
            let _ = task.await;
        }
        // Drain any shutdown tasks scheduled while awaiting prewarms/acquires.
        let trailing_tasks = self
            .inner
            .state
            .lock()
            .map(|mut guard| std::mem::take(&mut guard.shutdown_tasks))
            .unwrap_or_default();
        for task in trailing_tasks {
            let _ = task.await;
        }
        let mut locks = self.inner.connect_locks.lock().await;
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
}

fn is_deterministic_startup_error(err: &AcpError) -> bool {
    matches!(
        err,
        AcpError::SpawnFailed { .. }
            | AcpError::ExecutableNotFound { .. }
            | AcpError::ConfiguredExecutableMissing { .. }
            | AcpError::AuthenticationFailed(_)
            | AcpError::UnsupportedProtocolVersion(_)
            | AcpError::Shutdown(_)
    )
}

async fn acquire_shared(
    inner: Arc<PoolInner>,
    handle: tokio::runtime::Handle,
    key: AcpConnectionKey,
    spec: SpawnSpec,
    start_epoch: u64,
    mut epoch_rx: watch::Receiver<u64>,
) -> AcpResult<PooledAcpConnection> {
    if *epoch_rx.borrow() != start_epoch {
        return Err(AcpError::Shutdown("ACP pool shut down".into()));
    }
    let key_lock = tokio::select! {
        biased;
        () = wait_epoch_change(&mut epoch_rx, start_epoch) => {
            return Err(AcpError::Shutdown("ACP pool shut down".into()));
        }
        mut locks = inner.connect_locks.lock() => {
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            Arc::clone(
                locks
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        }
    };
    let _connect_guard = tokio::select! {
        biased;
        () = wait_epoch_change(&mut epoch_rx, start_epoch) => {
            return Err(AcpError::Shutdown("ACP pool shut down".into()));
        }
        guard = key_lock.lock() => guard,
    };

    let existing = {
        let guard = inner
            .state
            .lock()
            .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
        if guard.epoch != start_epoch {
            return Err(AcpError::Shutdown("ACP pool shut down".into()));
        }
        guard.entries.get(&key).and_then(|list| {
            list.iter()
                .find(|e| !e.exclusive && !e.invalidated.load(Ordering::SeqCst))
                .cloned()
        })
    };
    if let Some(entry) = existing {
        if entry.client.check_alive().await {
            let still_valid = {
                let guard = inner
                    .state
                    .lock()
                    .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
                if guard.epoch != start_epoch {
                    return Err(AcpError::Shutdown("ACP pool shut down".into()));
                }
                if !entry.invalidated.load(Ordering::SeqCst)
                    && guard.contains_entry(&key, entry.conn_id)
                {
                    entry.active_sessions.fetch_add(1, Ordering::SeqCst);
                    true
                } else {
                    false
                }
            };
            if still_valid {
                return Ok(PooledAcpConnection { entry, pool: inner });
            }
        } else {
            let removed = if let Ok(mut guard) = inner.state.lock() {
                if guard.epoch != start_epoch {
                    return Err(AcpError::Shutdown("ACP pool shut down".into()));
                }
                let removed = guard.remove_entry(&key, entry.conn_id);
                if removed.is_some() && entry.active_sessions.load(Ordering::SeqCst) == 0 {
                    let task = spawn_entry_shutdown(&handle, Arc::clone(&entry));
                    guard.record_shutdown_task(task);
                }
                removed
            } else {
                None
            };
            drop(removed);
        }
    }

    {
        let guard = inner
            .state
            .lock()
            .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
        if guard.epoch != start_epoch {
            return Err(AcpError::Shutdown("ACP pool shut down".into()));
        }
    }
    let (client, initialized) =
        spawn_and_init_client(spec, key.auth_method.as_deref(), epoch_rx, start_epoch).await?;

    let conn_id = inner.next_conn_id.fetch_add(1, Ordering::SeqCst);
    let entry = Arc::new(PoolEntry {
        conn_id,
        key: key.clone(),
        client: Arc::clone(&client),
        initialized,
        active_sessions: AtomicUsize::new(1),
        exclusive: false,
        max_idle: 1,
        invalidated: AtomicBool::new(false),
    });
    let cancelled = {
        let mut guard = inner
            .state
            .lock()
            .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
        if guard.epoch != start_epoch {
            true
        } else {
            guard
                .entries
                .entry(key.clone())
                .or_default()
                .push(Arc::clone(&entry));
            false
        }
    };
    if cancelled {
        let _ = client.shutdown_shared().await;
        return Err(AcpError::Shutdown("ACP pool shut down".into()));
    }

    attach_exit_watcher(&handle, Arc::clone(&inner.state), key, conn_id, &client);
    Ok(PooledAcpConnection { entry, pool: inner })
}

fn release_aborted_checkout(
    inner: &PoolInner,
    handle: &tokio::runtime::Handle,
    key: &AcpConnectionKey,
    entry: &Arc<PoolEntry>,
    remove_from_entries: bool,
) {
    let mut removed = None;
    if let Ok(mut guard) = inner.state.lock() {
        let prev = entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
        if remove_from_entries {
            removed = guard.remove_entry(key, entry.conn_id);
        }
        if prev == 1 {
            let invalidated = entry.invalidated.load(Ordering::SeqCst);
            let dead = !entry.client.is_alive();
            let unclean_exclusive = entry.exclusive && !entry.client.is_idle_clean();
            if invalidated || dead || unclean_exclusive || remove_from_entries {
                if removed.is_none() {
                    removed = guard.remove_entry(key, entry.conn_id);
                }
                let task = spawn_entry_shutdown(handle, Arc::clone(entry));
                guard.record_shutdown_task(task);
            }
        }
    } else {
        entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
    }
    drop(removed);
}

#[allow(clippy::too_many_arguments)]
async fn acquire_exclusive(
    inner: Arc<PoolInner>,
    handle: tokio::runtime::Handle,
    key: AcpConnectionKey,
    spec: SpawnSpec,
    prewarm_standby: usize,
    max_idle: usize,
    start_epoch: u64,
    mut epoch_rx: watch::Receiver<u64>,
) -> AcpResult<PooledAcpConnection> {
    let effective_max_idle = max_idle.max(prewarm_standby);
    let mut prewarm_fallback_used = false;

    // 1. Try to check out an existing live idle exclusive connection immediately.
    loop {
        enum Candidate {
            Idle(Arc<PoolEntry>),
            AdoptedPrewarm(oneshot::Receiver<AcpResult<Arc<PoolEntry>>>),
            SpawnFresh,
        }

        let candidate = {
            let guard = inner
                .state
                .lock()
                .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
            if guard.epoch != start_epoch {
                return Err(AcpError::Shutdown("ACP pool shut down".into()));
            }
            let idle = guard.entries.get(&key).and_then(|list| {
                list.iter().rev().find_map(|entry| {
                    (entry.exclusive
                        && entry.client.is_alive()
                        && !entry.invalidated.load(Ordering::SeqCst)
                        && entry.client.is_idle_clean())
                    .then(|| {
                        entry
                            .active_sessions
                            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                            .ok()
                            .map(|_| Arc::clone(entry))
                    })
                    .flatten()
                })
            });
            if let Some(entry) = idle {
                Candidate::Idle(entry)
            } else if !prewarm_fallback_used
                && let Some(rx) = guard.prewarms.get(&key).and_then(|list| {
                    list.iter().find_map(|prewarm| {
                        if !prewarm.claimed.swap(true, Ordering::SeqCst) {
                            let (tx, rx) = oneshot::channel();
                            if let Ok(mut slot) = prewarm.waiter.lock() {
                                *slot = Some(tx);
                                return Some(rx);
                            }
                        }
                        None
                    })
                })
            {
                Candidate::AdoptedPrewarm(rx)
            } else {
                Candidate::SpawnFresh
            }
        };

        match candidate {
            Candidate::Idle(entry) => {
                if entry.client.check_alive().await {
                    let still_valid = {
                        let guard = inner.state.lock().map_err(|_| {
                            AcpError::Shutdown("ACP pool state lock poisoned".into())
                        })?;
                        if guard.epoch != start_epoch {
                            entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
                            return Err(AcpError::Shutdown("ACP pool shut down".into()));
                        }
                        !entry.invalidated.load(Ordering::SeqCst)
                            && guard.contains_entry(&key, entry.conn_id)
                    };
                    if still_valid {
                        schedule_exclusive_prewarm(
                            &inner,
                            &handle,
                            &key,
                            &spec,
                            prewarm_standby,
                            effective_max_idle,
                            start_epoch,
                        );
                        return Ok(PooledAcpConnection { entry, pool: inner });
                    }
                    release_aborted_checkout(&inner, &handle, &key, &entry, false);
                    continue;
                }
                release_aborted_checkout(&inner, &handle, &key, &entry, true);
                continue;
            }
            Candidate::AdoptedPrewarm(rx) => {
                prewarm_fallback_used = true;
                let recv = tokio::select! {
                    biased;
                    () = wait_epoch_change(&mut epoch_rx, start_epoch) => {
                        return Err(AcpError::Shutdown("ACP pool shut down".into()));
                    }
                    res = rx => res,
                };
                match recv {
                    Ok(Ok(entry)) => {
                        if entry.client.check_alive().await {
                            let still_valid = {
                                let guard = inner.state.lock().map_err(|_| {
                                    AcpError::Shutdown("ACP pool state lock poisoned".into())
                                })?;
                                if guard.epoch != start_epoch {
                                    entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
                                    return Err(AcpError::Shutdown("ACP pool shut down".into()));
                                }
                                !entry.invalidated.load(Ordering::SeqCst)
                                    && guard.contains_entry(&key, entry.conn_id)
                            };
                            if still_valid {
                                schedule_exclusive_prewarm(
                                    &inner,
                                    &handle,
                                    &key,
                                    &spec,
                                    prewarm_standby,
                                    effective_max_idle,
                                    start_epoch,
                                );
                                return Ok(PooledAcpConnection { entry, pool: inner });
                            }
                            release_aborted_checkout(&inner, &handle, &key, &entry, false);
                        } else {
                            release_aborted_checkout(&inner, &handle, &key, &entry, true);
                        }
                    }
                    Ok(Err(err)) => {
                        if is_deterministic_startup_error(&err) {
                            return Err(err);
                        }
                    }
                    Err(_) => {}
                }
                let epoch_changed = inner
                    .state
                    .lock()
                    .map(|g| g.epoch != start_epoch)
                    .unwrap_or(true);
                if epoch_changed {
                    return Err(AcpError::Shutdown("ACP pool shut down".into()));
                }
                // Adopted prewarm failed transiently or died before checkout; fall through once to spawn fresh.
            }
            Candidate::SpawnFresh => {
                // Spawn concurrently without serializing against other exclusive sessions.
                let (client, initialized) = spawn_and_init_client(
                    spec.clone(),
                    key.auth_method.as_deref(),
                    epoch_rx,
                    start_epoch,
                )
                .await?;

                let conn_id = inner.next_conn_id.fetch_add(1, Ordering::SeqCst);
                let entry = Arc::new(PoolEntry {
                    conn_id,
                    key: key.clone(),
                    client: Arc::clone(&client),
                    initialized,
                    active_sessions: AtomicUsize::new(1),
                    exclusive: true,
                    max_idle: effective_max_idle,
                    invalidated: AtomicBool::new(false),
                });
                let cancelled = {
                    let mut guard = inner
                        .state
                        .lock()
                        .map_err(|_| AcpError::Shutdown("ACP pool state lock poisoned".into()))?;
                    if guard.epoch != start_epoch {
                        true
                    } else {
                        guard
                            .entries
                            .entry(key.clone())
                            .or_default()
                            .push(Arc::clone(&entry));
                        false
                    }
                };
                if cancelled {
                    let _ = client.shutdown_shared().await;
                    return Err(AcpError::Shutdown("ACP pool shut down".into()));
                }

                attach_exit_watcher(
                    &handle,
                    Arc::clone(&inner.state),
                    key.clone(),
                    conn_id,
                    &client,
                );
                schedule_exclusive_prewarm(
                    &inner,
                    &handle,
                    &key,
                    &spec,
                    prewarm_standby,
                    effective_max_idle,
                    start_epoch,
                );
                return Ok(PooledAcpConnection { entry, pool: inner });
            }
        }
    }
}

/// Active lease on a pooled [`AcpClient`].
pub struct PooledAcpConnection {
    entry: Arc<PoolEntry>,
    pool: Arc<PoolInner>,
}

impl PooledAcpConnection {
    /// Underlying shared ACP client.
    pub fn client(&self) -> &Arc<AcpClient<ChildStdin>> {
        &self.entry.client
    }

    /// Cached initialization payload negotiated once for this connection.
    pub fn initialized(&self) -> &InitializeResult {
        &self.entry.initialized
    }

    /// OS process id of the external ACP agent process.
    pub fn process_id(&self) -> Option<u32> {
        self.entry.client.process_id()
    }

    /// Whether the underlying process/transport is still alive.
    pub fn is_alive(&self) -> bool {
        self.entry.client.is_alive()
    }

    /// Current number of active session leases sharing this connection.
    pub fn active_session_count(&self) -> usize {
        self.entry.active_sessions.load(Ordering::SeqCst)
    }

    /// Cache key for this pooled connection.
    pub fn key(&self) -> &AcpConnectionKey {
        &self.entry.key
    }

    /// Remove this connection from the pool cache so future acquires spawn a fresh process
    /// and schedule graceful shutdown once the last active lease drops.
    pub fn invalidate(&self) {
        let removed = if let Ok(mut guard) = self.pool.state.lock() {
            self.entry.invalidated.store(true, Ordering::SeqCst);
            guard.remove_entry(&self.entry.key, self.entry.conn_id)
        } else {
            self.entry.invalidated.store(true, Ordering::SeqCst);
            None
        };
        drop(removed);
    }
}

impl Drop for PooledAcpConnection {
    fn drop(&mut self) {
        let handle = self.pool.runtime_handle().ok();
        let mut drop_outside_lock = Vec::new();
        {
            let Ok(mut guard) = self.pool.state.lock() else {
                self.entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
                return;
            };
            let prev = self.entry.active_sessions.fetch_sub(1, Ordering::SeqCst);
            if prev != 1 {
                return;
            }
            let invalidated = self.entry.invalidated.load(Ordering::SeqCst);
            let dead = !self.entry.client.is_alive();
            let unclean_exclusive = self.entry.exclusive && !self.entry.client.is_idle_clean();
            if invalidated || dead || unclean_exclusive {
                if let Some(removed) = guard.remove_entry(&self.entry.key, self.entry.conn_id) {
                    drop_outside_lock.push(removed);
                }
                if let Some(handle) = handle {
                    let task = spawn_entry_shutdown(&handle, Arc::clone(&self.entry));
                    guard.record_shutdown_task(task);
                }
            } else if self.entry.exclusive
                && let Some(handle) = handle
            {
                drop_outside_lock.extend(trim_excess_idle_locked(
                    &mut guard,
                    &handle,
                    &self.entry.key,
                    self.entry.max_idle,
                ));
            }
        }
        drop(drop_outside_lock);
    }
}
