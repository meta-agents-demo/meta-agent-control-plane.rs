use std::{
    collections::{BTreeMap, HashMap, VecDeque},
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
        ControlCommand, ControlCommandAck, ControlCommandRequest, ControlStatus, RuntimeAgentRef,
        RuntimeAgentTelemetry, RuntimeCollectionStatus, RuntimeError, RuntimeHookEnvelope,
        RuntimeHookKind, RuntimeProcessTelemetry, RuntimeSnapshot, RuntimeTotals, validate_text,
    },
};

const SNAPSHOT_HOOK_LIMIT: usize = 250;
const SNAPSHOT_COMMAND_LIMIT: usize = 250;

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
                    provider: process.provider,
                    process_name: process.process_name,
                    matched_pattern: process.matched_pattern,
                    process_state: process.process_state,
                    cpu_percent,
                    rss_bytes: process.rss_bytes,
                    memory_percent,
                    observed_at: observed_at.clone(),
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
                input_tokens: 0,
                output_tokens: 0,
                last_hook_at: hook.occurred_at.clone(),
            });
        let updates_current_state = hook.occurred_at >= agent.last_hook_at;
        agent.input_tokens = agent.input_tokens.saturating_add(hook.input_tokens_delta);
        agent.output_tokens = agent.output_tokens.saturating_add(hook.output_tokens_delta);
        if updates_current_state {
            agent.agent = hook.agent.clone();
            if hook.session_id.is_some() {
                agent.session_id = hook.session_id.clone();
            }
            if hook.pid.is_some() {
                agent.pid = hook.pid;
            }
            agent.last_hook_at = hook.occurred_at.clone();
            if hook.confidence.is_some() {
                agent.reported_confidence = hook.confidence;
            }
            if hook.summary.is_some() {
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
                    agent.status = "idle".to_owned();
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
        if !state.hook_agents.contains_key(&request.agent_id) {
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
        let mut agents = BTreeMap::<String, RuntimeAgentTelemetry>::new();
        let process_agent_ids = state
            .hook_agents
            .values()
            .filter_map(|agent| agent.pid.map(|pid| (pid, agent.agent.agent_id.clone())))
            .collect::<HashMap<_, _>>();

        for process in state.processes.values() {
            let agent_id = process_agent_ids
                .get(&process.pid)
                .cloned()
                .unwrap_or_else(|| format!("process:{}:{}", process.provider, process.pid));
            agents.insert(
                agent_id.clone(),
                RuntimeAgentTelemetry {
                    agent_id,
                    provider: process.provider.clone(),
                    model: "unreported".to_owned(),
                    instance_id: None,
                    session_id: None,
                    pid: Some(process.pid),
                    status: process_state_label(&process.process_state).to_owned(),
                    current_activity: None,
                    current_tool: None,
                    reported_confidence: None,
                    confidence_source: "unreported".to_owned(),
                    cpu_percent: process.cpu_percent,
                    rss_bytes: Some(process.rss_bytes),
                    memory_percent: process.memory_percent,
                    input_tokens: 0,
                    output_tokens: 0,
                    process_backed: true,
                    hook_backed: false,
                    last_hook_at: None,
                    last_process_sample_at: Some(process.observed_at.clone()),
                },
            );
        }

        for hook_agent in state.hook_agents.values() {
            let process = hook_agent.pid.and_then(|pid| state.processes.get(&pid));
            agents
                .entry(hook_agent.agent.agent_id.clone())
                .and_modify(|agent| {
                    agent.provider = hook_agent.agent.provider.clone();
                    agent.model = hook_agent.agent.model.clone();
                    agent.instance_id = hook_agent.agent.instance_id.clone();
                    agent.session_id = hook_agent.session_id.clone();
                    agent.pid = hook_agent.pid.or(agent.pid);
                    agent.status = hook_agent.status.clone();
                    agent.current_activity = hook_agent.current_activity.clone();
                    agent.current_tool = hook_agent.current_tool.clone();
                    agent.reported_confidence = hook_agent.reported_confidence;
                    agent.confidence_source = if hook_agent.reported_confidence.is_some() {
                        "hook".to_owned()
                    } else {
                        "unreported".to_owned()
                    };
                    agent.input_tokens = hook_agent.input_tokens;
                    agent.output_tokens = hook_agent.output_tokens;
                    agent.hook_backed = true;
                    agent.last_hook_at = Some(hook_agent.last_hook_at.clone());
                })
                .or_insert_with(|| RuntimeAgentTelemetry {
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
                    cpu_percent: process.and_then(|value| value.cpu_percent),
                    rss_bytes: process.map(|value| value.rss_bytes),
                    memory_percent: process.and_then(|value| value.memory_percent),
                    input_tokens: hook_agent.input_tokens,
                    output_tokens: hook_agent.output_tokens,
                    process_backed: process.is_some(),
                    hook_backed: true,
                    last_hook_at: Some(hook_agent.last_hook_at.clone()),
                    last_process_sample_at: process.map(|value| value.observed_at.clone()),
                });
        }

        let agents = agents.into_values().collect::<Vec<_>>();
        let recent_commands = state.commands.iter().cloned().collect::<Vec<_>>();
        let totals = RuntimeTotals {
            agents: agents.len(),
            process_backed_agents: agents.iter().filter(|agent| agent.process_backed).count(),
            hook_backed_agents: agents.iter().filter(|agent| agent.hook_backed).count(),
            cpu_percent: agents.iter().filter_map(|agent| agent.cpu_percent).sum(),
            rss_bytes: agents.iter().filter_map(|agent| agent.rss_bytes).sum(),
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
        let mut processes = state.processes.values().cloned().collect::<Vec<_>>();
        processes.sort_by_key(|process| process.pid);

        RuntimeSnapshot {
            generated_at: Utc::now(),
            collection: RuntimeCollectionStatus {
                configured: self.config.process_collection_enabled,
                enabled: self.collection_enabled(),
                proc_root: self.config.proc_root.display().to_string(),
                sample_interval_ms: duration_millis(self.config.sample_interval),
                process_patterns: self.config.process_patterns.clone(),
                cpu_count: state.cpu_count,
                memory_total_bytes: state.memory_total_bytes,
                last_sample_at: state.last_sample_at.clone(),
                last_error: state.last_error.clone(),
                collection_errors: state.collection_errors,
            },
            totals,
            agents,
            processes,
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

fn process_state_label(state: &str) -> &'static str {
    match state {
        "R" => "running",
        "S" | "D" | "I" => "waiting",
        "T" | "t" => "stopped",
        "Z" => "zombie",
        "X" | "x" => "dead",
        _ => "observed",
    }
}
