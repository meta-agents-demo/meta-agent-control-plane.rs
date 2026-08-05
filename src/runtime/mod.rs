use std::{env, path::PathBuf, time::Duration};

mod collector;
mod model;
mod monitor;
#[cfg(test)]
mod tests;

pub use model::*;
pub use monitor::RuntimeMonitor;

pub const RUNTIME_PROTOCOL_VERSION: &str = "v1";
const DEFAULT_SAMPLE_INTERVAL_MS: u64 = 1_000;
const MIN_SAMPLE_INTERVAL_MS: u64 = 250;
const MAX_SAMPLE_INTERVAL_MS: u64 = 60_000;
const DEFAULT_HOOK_CAPACITY: usize = 2_048;
const DEFAULT_COMMAND_CAPACITY: usize = 512;
const MAX_HOOK_CAPACITY: usize = 65_536;
const MAX_COMMAND_CAPACITY: usize = 8_192;
const DEFAULT_PROCESS_PATTERNS: &str = "claude,gemini,codex,chatgpt,openai";
const MAX_PROCESS_PATTERNS: usize = 64;
const MAX_PROCESS_PATTERN_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub process_collection_enabled: bool,
    pub proc_root: PathBuf,
    pub sample_interval: Duration,
    pub process_patterns: Vec<String>,
    pub hook_capacity: usize,
    pub command_capacity: usize,
}

impl RuntimeConfig {
    pub fn from_env() -> Self {
        let process_collection_enabled = env_bool("META_AGENT_RUNTIME_DISCOVERY_ENABLED", true);
        let proc_root = env::var_os("META_AGENT_HOST_PROC_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/proc"));
        let sample_interval_ms = env_u64(
            "META_AGENT_RUNTIME_SAMPLE_INTERVAL_MS",
            DEFAULT_SAMPLE_INTERVAL_MS,
        )
        .clamp(MIN_SAMPLE_INTERVAL_MS, MAX_SAMPLE_INTERVAL_MS);
        let process_patterns = env::var("META_AGENT_PROCESS_PATTERNS")
            .unwrap_or_else(|_| DEFAULT_PROCESS_PATTERNS.to_owned())
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= MAX_PROCESS_PATTERN_BYTES)
            .take(MAX_PROCESS_PATTERNS)
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let process_patterns = if process_patterns.is_empty() {
            DEFAULT_PROCESS_PATTERNS
                .split(',')
                .map(str::to_owned)
                .collect()
        } else {
            process_patterns
        };
        let hook_capacity = env_usize("META_AGENT_RUNTIME_HOOK_CAPACITY", DEFAULT_HOOK_CAPACITY)
            .clamp(1, MAX_HOOK_CAPACITY);
        let command_capacity = env_usize(
            "META_AGENT_RUNTIME_COMMAND_CAPACITY",
            DEFAULT_COMMAND_CAPACITY,
        )
        .clamp(1, MAX_COMMAND_CAPACITY);

        Self {
            process_collection_enabled,
            proc_root,
            sample_interval: Duration::from_millis(sample_interval_ms),
            process_patterns,
            hook_capacity,
            command_capacity,
        }
    }

    #[cfg(test)]
    pub(super) fn test(proc_root: PathBuf) -> Self {
        Self {
            process_collection_enabled: true,
            proc_root,
            sample_interval: Duration::from_millis(250),
            process_patterns: vec!["claude".to_owned(), "gemini".to_owned()],
            hook_capacity: 16,
            command_capacity: 16,
        }
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}
