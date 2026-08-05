use std::{fs, path::Path};

use super::RuntimeConfig;

#[derive(Clone, Debug)]
pub(super) struct RawProcSnapshot {
    pub(super) total_ticks: u64,
    pub(super) cpu_count: usize,
    pub(super) memory_total_bytes: Option<u64>,
    pub(super) processes: Vec<RawProcess>,
}

#[derive(Clone, Debug)]
pub(super) struct RawProcess {
    pub(super) pid: u32,
    pub(super) provider: String,
    pub(super) process_name: String,
    pub(super) matched_pattern: String,
    pub(super) process_state: String,
    pub(super) process_ticks: u64,
    pub(super) rss_bytes: u64,
}

pub(super) fn read_proc_snapshot(config: &RuntimeConfig) -> Result<RawProcSnapshot, String> {
    let (total_ticks, cpu_count) = read_total_cpu(&config.proc_root)?;
    let memory_total_bytes = read_memory_total(&config.proc_root);
    let entries = fs::read_dir(&config.proc_root).map_err(|error| {
        format!(
            "cannot read host process root {}: {error}",
            config.proc_root.display()
        )
    })?;
    let mut processes = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(process) = read_process(&config.proc_root, pid, &config.process_patterns) {
            processes.push(process);
        }
    }
    Ok(RawProcSnapshot {
        total_ticks,
        cpu_count,
        memory_total_bytes,
        processes,
    })
}

fn read_total_cpu(proc_root: &Path) -> Result<(u64, usize), String> {
    let content = fs::read_to_string(proc_root.join("stat"))
        .map_err(|error| format!("cannot read host CPU counters: {error}"))?;
    let mut lines = content.lines();
    let aggregate = lines
        .next()
        .ok_or_else(|| "host CPU counters are empty".to_owned())?;
    let mut aggregate_fields = aggregate.split_whitespace();
    if aggregate_fields.next() != Some("cpu") {
        return Err("host CPU counters do not start with the aggregate cpu row".to_owned());
    }
    // Linux reports guest and guest_nice inside user/nice as well as in their own
    // columns. Sum user through steal only so the denominator is not double-counted.
    let total_ticks = aggregate_fields
        .take(8)
        .filter_map(|value| value.parse::<u64>().ok())
        .sum();
    let cpu_count = lines
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| {
            name.strip_prefix("cpu").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|value| value.is_ascii_digit())
            })
        })
        .count()
        .max(1);
    Ok((total_ticks, cpu_count))
}

fn read_memory_total(proc_root: &Path) -> Option<u64> {
    let content = fs::read_to_string(proc_root.join("meminfo")).ok()?;
    let line = content.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1_024)
}

fn read_process(proc_root: &Path, pid: u32, patterns: &[String]) -> Option<RawProcess> {
    let process_root = proc_root.join(pid.to_string());
    let stat = fs::read_to_string(process_root.join("stat")).ok()?;
    let stat_close = stat.rfind(')')?;
    let fields = stat
        .get(stat_close + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let process_state = fields.first()?.to_string();
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    let process_ticks = user_ticks.checked_add(system_ticks)?;

    let comm = fs::read_to_string(process_root.join("comm"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    let cmdline = fs::read(process_root.join("cmdline")).unwrap_or_default();
    let cmdline_match = cmdline
        .split(|byte| *byte == 0)
        .filter_map(|part| std::str::from_utf8(part).ok())
        .take(12)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let match_haystack = format!("{} {cmdline_match}", comm.to_ascii_lowercase());
    let matched_pattern = patterns
        .iter()
        .find(|pattern| match_haystack.contains(pattern.as_str()))?
        .clone();
    let provider = provider_for_pattern(&matched_pattern).to_owned();
    let process_name = display_process_name(&comm, &matched_pattern);
    let rss_bytes = read_process_rss(&process_root).unwrap_or(0);

    Some(RawProcess {
        pid,
        provider,
        process_name,
        matched_pattern,
        process_state,
        process_ticks,
        rss_bytes,
    })
}

fn provider_for_pattern(pattern: &str) -> &'static str {
    if pattern.contains("claude") || pattern.contains("anthropic") {
        "anthropic"
    } else if pattern.contains("gemini") || pattern.contains("google") {
        "google"
    } else if pattern.contains("codex") || pattern.contains("chatgpt") || pattern.contains("openai")
    {
        "openai"
    } else {
        "unknown"
    }
}

fn display_process_name(comm: &str, matched_pattern: &str) -> String {
    if comm.is_empty() || matches!(comm, "node" | "python" | "python3" | "bash" | "sh") {
        format!("{matched_pattern}-agent")
    } else {
        comm.to_owned()
    }
}

fn read_process_rss(process_root: &Path) -> Option<u64> {
    let status = fs::read_to_string(process_root.join("status")).ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1_024)
}
