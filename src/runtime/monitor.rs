use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    RuntimeConfig,
    collector::{RawProcSnapshot, read_proc_snapshot},
    model::{
        ControlCommand, ControlCommandAck, ControlCommandRequest, ControlStatus,
        HostObserverStatus, HostProcessObservationEnvelope, RuntimeAgentRef, RuntimeAgentTelemetry,
        RuntimeCollectionStatus, RuntimeError, RuntimeHookEnvelope, RuntimeHookKind,
        RuntimeProcessTelemetry, RuntimeSnapshot, RuntimeTotals, validate_text,
    },
};

const SNAPSHOT_HOOK_LIMIT: usize = 250;
const SNAPSHOT_COMMAND_LIMIT: usize = 250;
const HOST_OBSERVATION_ID_LIMIT: usize = 8_192;
const HOST_OBSERVER_STALE_AFTER_SECONDS: i64 = 15;

#[derive(Clone, Debug)]
pub struct RuntimeMonitor {
    config: Arc<RuntimeConfig>,
    collection_enabled: Arc<AtomicBool>,
    state: Arc<RwLock<RuntimeState>>,
}

#[derive(Debug)]
struct RuntimeState {
    hooks: VecDeque<RuntimeHookEnvelope>,
    hook_ids: VecDeque<Uuid>,
    hook_agents: HashMap<String, HookAgentState>,
    commands: VecDeque<ControlCommand>,
    processes: HashMap<u32, RuntimeProcessTelemetry>,
    host_observations: HashMap<String, HostProcessObservationEnvelope>,
    host_observation_ids: VecDeque<Uuid>,
    previous_total_ticks: Option<u64>,
    previous_process_ticks: HashMap<u32, u64>,
    cpu_count: usize,
    memory_total_bytes: Option<u64>,
    last_sample_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    collection_errors: u64,
}

#[derive(Clone, Debug)]
struct HookAgentState {
    agent: RuntimeAgentRef,
    session_id: Option<String>,
    pid: Option<u32>,
    status: String,
    current_activity: Option<String>,
    current_tool: Option<String>,
    reported_confidence: Option<f32>,
    cpu_percent: Option<f64>,
    rss_bytes: Option<u64>,
    memory_percent: Option<f64>,
    control_capable: bool,
    input_tokens: u64,
    output_tokens: u64,
    last_hook_at: DateTime<Utc>,
}

impl RuntimeMonitor {
    pub fn new(config: RuntimeConfig) -> Self {
        let collection_enabled = config.process_collection_enabled;
        Self {
            config: Arc::new(config),
            collection_enabled: Arc::new(AtomicBool::new(collection_enabled)),
            state: Arc::new(RwLock::new(RuntimeState {
                hooks: VecDeque::new(),
                hook_ids: VecDeque::new(),
                hook_agents: HashMap::new(),
                commands: VecDeque::new(),
                processes: HashMap::new(),
                host_observations: HashMap::new(),
                host_observation_ids: VecDeque::new(),
                previous_total_ticks: None,
                previous_process_ticks: HashMap::new(),
                cpu_count: 0,
                memory_total_bytes: None,
                last_sample_at: None,
                last_error: None,
                collection_errors: 0,
            })),
        }
    }

    pub fn from_env() -> Self {
        Self::new(RuntimeConfig::from_env())
    }

    pub fn collection_enabled(&self) -> bool {
        self.collection_enabled.load(Ordering::Relaxed)
    }

    pub fn set_collection_enabled(&self, enabled: bool) {
        self.collection_enabled.store(enabled, Ordering::Relaxed);
    }

    pub async fn run(self, cancellation: CancellationToken) {
        let mut interval = tokio::time::interval(self.config.sample_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return,
                _ = interval.tick() => {
                    if self.collection_enabled() {
                        self.collect_once().await;
                    }
                }
            }
        }
    }

    pub async fn collect_once(&self) {
        let config = Arc::clone(&self.config);
        let result = tokio::task::spawn_blocking(move || read_proc_snapshot(&config)).await;
        match result {
            Ok(Ok(snapshot)) => self.apply_proc_snapshot(snapshot).await,
            Ok(Err(error)) => self.record_collection_error(error).await,
            Err(error) => {
                self.record_collection_error(format!("runtime collector task failed: {error}"))
                    .await
            }
        }
    }

    async fn record_collection_error(&self, error: String) {
        let mut state = self.state.write().await;
        state.collection_errors = state.collection_errors.saturating_add(1);
        state.last_error = Some(error);
    }

    async fn apply_proc_snapshot(&self, snapshot: RawProcSnapshot) {
        let observed_at = Utc::now();
        let mut state = self.state.write().await;
        let total_delta = state
            .previous_total_ticks
            .and_then(|previous| snapshot.total_ticks.checked_sub(previous));
        let mut next_previous = HashMap::with_capacity(snapshot.processes.len());
        let mut processes = HashMap::with_capacity(snapshot.processes.len());

        for process in snapshot.processes {
            let cpu_percent = total_delta.and_then(|total_delta| {
                if total_delta == 0 {
                    return None;
                }
                let previous = state.previous_process_ticks.get(&process.pid)?;
                let process_delta = process.process_ticks.checked_sub(*previous)?;
                Some(
                    (process_delta as f64 / total_delta as f64)
                        * snapshot.cpu_count.max(1) as f64
                        * 100.0,
                )
            });
            let memory_percent = snapshot.memory_total_bytes.and_then(|total| {
                (total > 0).then(|| process.rss_bytes as f64 / total as f64 * 100.0)
            });
            next_previous.insert(process.pid, process.process_ticks);
            processes.insert(
                process.pid,
                RuntimeProcessTelemetry {
                    pid: process.pid,
                    ppid: None,
                    pgid: None,
                    provider: process.provider,
                    process_name: process.process_name,
                    matched_pattern: process.matched_pattern,
                    process_role: None,
                    process_state: process.process_state,
                    cpu_percent,
                    rss_bytes: process.rss_bytes,
                    memory_percent,
                    observed_at,
                    source: "linux_proc".to_owned(),
                    observer_id: None,
                    host_id: None,
                    platform: Some("linux".to_owned()),
                    stale: false,
                },
            );
        }

        state.previous_total_ticks = Some(snapshot.total_ticks);
        state.previous_process_ticks = next_previous;
        state.processes = processes;
        state.cpu_count = snapshot.cpu_count;
        state.memory_total_bytes = snapshot.memory_total_bytes;
        state.last_sample_at = Some(observed_at);
        state.last_error = None;
    }

    pub async fn ingest_host_observation(
        &self,
        observation: HostProcessObservationEnvelope,
    ) -> Result<(), RuntimeError> {
        observation.validate()?;
        let mut state = self.state.write().await;
        if state
            .host_observation_ids
            .contains(&observation.observation_id)
        {
            return Err(RuntimeError::DuplicateHostObservation);
        }
        if state
            .host_observations
            .get(&observation.observer_id)
            .is_some_and(|current| current.observed_at > observation.observed_at)
        {
            return Err(RuntimeError::OutOfOrderHostObservation);
        }
        state
            .host_observation_ids
            .push_front(observation.observation_id);
        while state.host_observation_ids.len() > HOST_OBSERVATION_ID_LIMIT {
            state.host_observation_ids.pop_back();
        }
        state
            .host_observations
            .insert(observation.observer_id.clone(), observation);
        Ok(())
    }

    pub async fn ingest_hook(&self, hook: RuntimeHookEnvelope) -> Result<(), RuntimeError> {
        hook.validate()?;
        let mut state = self.state.write().await;
        if state.hook_ids.contains(&hook.event_id) {
            return Err(RuntimeError::DuplicateHook);
        }

        let agent = state
            .hook_agents
            .entry(hook.agent.agent_id.clone())
            .or_insert_with(|| HookAgentState {
                agent: hook.agent.clone(),
                session_id: hook.session_id.clone(),
                pid: hook.pid,
                status: "observed".to_owned(),
                current_activity: None,
                current_tool: None,
                reported_confidence: None,
                cpu_percent: hook.cpu_percent,
                rss_bytes: hook.rss_bytes,
                memory_percent: hook.memory_percent,
                control_capable: hook.control_capable,
                input_tokens: 0,
                output_tokens: 0,
                last_hook_at: hook.occurred_at,
            });
        let updates_current_state = hook.occurred_at >= agent.last_hook_at;
        agent.input_tokens = agent.input_tokens.saturating_add(hook.input_tokens_delta);
        agent.output_tokens = agent.output_tokens.saturating_add(hook.output_tokens_delta);
        agent.control_capable |= hook.control_capable;
        if updates_current_state {
            agent.agent = hook.agent.clone();
            if hook.session_id.is_some() {
                agent.session_id = hook.session_id.clone();
            }
            if hook.pid.is_some() {
                agent.pid = hook.pid;
            }
            agent.last_hook_at = hook.occurred_at;
            if hook.confidence.is_some() {
                agent.reported_confidence = hook.confidence;
            }
            if hook.cpu_percent.is_some() {
                agent.cpu_percent = hook.cpu_percent;
            }
            if hook.rss_bytes.is_some() {
                agent.rss_bytes = hook.rss_bytes;
            }
            if hook.memory_percent.is_some() {
                agent.memory_percent = hook.memory_percent;
            }
            let preserve_failure =
                hook.kind == RuntimeHookKind::SessionFinished && agent.status == "failed";
            if hook.summary.is_some() && !preserve_failure {
                agent.current_activity = hook.summary.clone();
            }
            match hook.kind {
                RuntimeHookKind::SessionStarted => agent.status = "running".to_owned(),
                RuntimeHookKind::Heartbeat => {}
                RuntimeHookKind::Activity | RuntimeHookKind::ModelResponse => {
                    agent.status = "running".to_owned();
                }
                RuntimeHookKind::ToolStarted => {
                    agent.status = "running".to_owned();
                    agent.current_tool = hook.tool_name.clone();
                }
                RuntimeHookKind::ToolFinished => {
                    agent.status = "running".to_owned();
                    agent.current_tool = None;
                }
                RuntimeHookKind::ConfidenceReported => {}
                RuntimeHookKind::ErrorObserved => agent.status = "failed".to_owned(),
                RuntimeHookKind::SessionFinished => {
                    if !preserve_failure {
                        agent.status = "idle".to_owned();
                    }
                    agent.current_tool = None;
                }
            }
        }

        let event_id = hook.event_id;
        state.hooks.push_front(hook);
        state.hook_ids.push_front(event_id);
        while state.hooks.len() > self.config.hook_capacity {
            state.hooks.pop_back();
        }
        while state.hook_ids.len() > self.config.hook_capacity {
            state.hook_ids.pop_back();
        }
        Ok(())
    }

    pub async fn enqueue_command(
        &self,
        request: ControlCommandRequest,
    ) -> Result<ControlCommand, RuntimeError> {
        request.validate()?;
        let mut state = self.state.write().await;
        if !state
            .hook_agents
            .get(&request.agent_id)
            .is_some_and(|agent| agent.control_capable)
        {
            return Err(RuntimeError::AgentNotHookBacked);
        }
        let command = ControlCommand {
            command_id: Uuid::new_v4(),
            created_at: Utc::now(),
            agent_id: request.agent_id,
            action: request.action,
            status: ControlStatus::Pending,
            acknowledged_at: None,
            message: None,
        };
        state.commands.push_front(command.clone());
        while state.commands.len() > self.config.command_capacity {
            state.commands.pop_back();
        }
        Ok(command)
    }

    pub async fn pending_commands(
        &self,
        agent_id: &str,
    ) -> Result<Vec<ControlCommand>, RuntimeError> {
        validate_text("agent_id", agent_id, 256)?;
        let state = self.state.read().await;
        Ok(state
            .commands
            .iter()
            .filter(|command| {
                command.agent_id == agent_id && command.status == ControlStatus::Pending
            })
            .cloned()
            .collect())
    }

    pub async fn acknowledge_command(
        &self,
        ack: ControlCommandAck,
    ) -> Result<ControlCommand, RuntimeError> {
        ack.validate()?;
        let mut state = self.state.write().await;
        let command = state
            .commands
            .iter_mut()
            .find(|command| command.command_id == ack.command_id)
            .ok_or(RuntimeError::CommandNotFound)?;
        if command.agent_id != ack.agent_id {
            return Err(RuntimeError::CommandAgentMismatch);
        }
        command.status = if ack.accepted {
            ControlStatus::Acknowledged
        } else {
            ControlStatus::Rejected
        };
        command.acknowledged_at = Some(Utc::now());
        command.message = ack.message;
        Ok(command.clone())
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        let state = self.state.read().await;
        let generated_at = Utc::now();
        let mut processes = state.processes.values().cloned().collect::<Vec<_>>();
        let mut host_observers = Vec::with_capacity(state.host_observations.len());
        for observation in state.host_observations.values() {
            let stale = generated_at
                .signed_duration_since(observation.observed_at)
                .num_seconds()
                > HOST_OBSERVER_STALE_AFTER_SECONDS;
            host_observers.push(HostObserverStatus {
                observer_id: observation.observer_id.clone(),
                host_id: observation.host_id.clone(),
                platform: observation.platform.clone(),
                last_observed_at: observation.observed_at,
                process_count: observation.processes.len(),
                stale,
            });
            processes.extend(
                observation
                    .processes
                    .iter()
                    .map(|process| RuntimeProcessTelemetry {
                        pid: process.pid,
                        ppid: process.ppid,
                        pgid: process.pgid,
                        provider: process.provider.clone(),
                        process_name: process.process_name.clone(),
                        matched_pattern: "host_observer".to_owned(),
                        process_role: Some(process.process_role.clone()),
                        process_state: process.process_state.clone(),
                        cpu_percent: process.cpu_percent,
                        rss_bytes: process.rss_bytes,
                        memory_percent: process.memory_percent,
                        observed_at: observation.observed_at,
                        source: "host_observer".to_owned(),
                        observer_id: Some(observation.observer_id.clone()),
                        host_id: Some(observation.host_id.clone()),
                        platform: Some(observation.platform.clone()),
                        stale,
                    }),
            );
        }
        host_observers.sort_by(|left, right| left.observer_id.cmp(&right.observer_id));
        processes.sort_by(|left, right| {
            left.host_id
                .cmp(&right.host_id)
                .then(left.pid.cmp(&right.pid))
        });

        let mut agents = Vec::<RuntimeAgentTelemetry>::with_capacity(state.hook_agents.len());
        for hook_agent in state.hook_agents.values() {
            let process = hook_agent.pid.and_then(|pid| {
                processes
                    .iter()
                    .filter(|process| process.pid == pid && !process.stale)
                    .max_by_key(|process| process.observed_at)
            });
            let hook_has_resource = hook_agent.cpu_percent.is_some()
                || hook_agent.rss_bytes.is_some()
                || hook_agent.memory_percent.is_some();
            agents.push(RuntimeAgentTelemetry {
                agent_id: hook_agent.agent.agent_id.clone(),
                provider: hook_agent.agent.provider.clone(),
                model: hook_agent.agent.model.clone(),
                instance_id: hook_agent.agent.instance_id.clone(),
                session_id: hook_agent.session_id.clone(),
                pid: hook_agent.pid,
                status: hook_agent.status.clone(),
                current_activity: hook_agent.current_activity.clone(),
                current_tool: hook_agent.current_tool.clone(),
                reported_confidence: hook_agent.reported_confidence,
                confidence_source: if hook_agent.reported_confidence.is_some() {
                    "hook".to_owned()
                } else {
                    "unreported".to_owned()
                },
                cpu_percent: process
                    .and_then(|value| value.cpu_percent)
                    .or(hook_agent.cpu_percent),
                rss_bytes: process
                    .map(|value| value.rss_bytes)
                    .or(hook_agent.rss_bytes),
                memory_percent: process
                    .and_then(|value| value.memory_percent)
                    .or(hook_agent.memory_percent),
                resource_source: process.map_or_else(
                    || {
                        if hook_has_resource {
                            "hook".to_owned()
                        } else {
                            "unreported".to_owned()
                        }
                    },
                    |process| process.source.clone(),
                ),
                input_tokens: hook_agent.input_tokens,
                output_tokens: hook_agent.output_tokens,
                process_backed: process.is_some(),
                hook_backed: true,
                control_capable: hook_agent.control_capable,
                last_hook_at: Some(hook_agent.last_hook_at),
                last_process_sample_at: process.map(|value| value.observed_at),
            });
        }

        agents.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
        let recent_commands = state.commands.iter().cloned().collect::<Vec<_>>();
        let totals = RuntimeTotals {
            agents: agents.len(),
            observed_processes: processes.len(),
            process_backed_agents: agents.iter().filter(|agent| agent.process_backed).count(),
            hook_backed_agents: agents.iter().filter(|agent| agent.hook_backed).count(),
            cpu_percent: processes
                .iter()
                .filter(|process| !process.stale)
                .filter_map(|process| process.cpu_percent)
                .sum(),
            rss_bytes: processes
                .iter()
                .filter(|process| !process.stale)
                .map(|process| process.rss_bytes)
                .sum(),
            input_tokens: agents.iter().map(|agent| agent.input_tokens).sum(),
            output_tokens: agents.iter().map(|agent| agent.output_tokens).sum(),
            confidence_reported_agents: agents
                .iter()
                .filter(|agent| agent.reported_confidence.is_some())
                .count(),
            confidence_unreported_agents: agents
                .iter()
                .filter(|agent| agent.reported_confidence.is_none())
                .count(),
            pending_commands: recent_commands
                .iter()
                .filter(|command| command.status == ControlStatus::Pending)
                .count(),
        };
        RuntimeSnapshot {
            generated_at,
            collection: RuntimeCollectionStatus {
                configured: self.config.process_collection_enabled,
                enabled: self.collection_enabled(),
                proc_root: self.config.proc_root.display().to_string(),
                sample_interval_ms: duration_millis(self.config.sample_interval),
                process_patterns: self.config.process_patterns.clone(),
                cpu_count: state.cpu_count,
                memory_total_bytes: state.memory_total_bytes,
                last_sample_at: state.last_sample_at,
                last_error: state.last_error.clone(),
                collection_errors: state.collection_errors,
            },
            totals,
            agents,
            processes,
            host_observers,
            recent_hooks: state
                .hooks
                .iter()
                .take(SNAPSHOT_HOOK_LIMIT)
                .cloned()
                .collect(),
            recent_commands: recent_commands
                .into_iter()
                .take(SNAPSHOT_COMMAND_LIMIT)
                .collect(),
        }
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
