fn diagnostic_id(rule: DiagnosticRule, agent_id: &str, task_id: &str) -> String {
    format!("{}:{agent_id}:{task_id}", rule.as_str())
}

fn age_seconds(now: DateTime<Utc>, then: DateTime<Utc>) -> i64 {
    now.signed_duration_since(then).num_seconds().max(0)
}

const fn is_active(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Pending | TaskStatus::Running | TaskStatus::Blocked
    )
}

const fn is_terminal(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Partial
    )
}

fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f32 / denominator as f32
    }
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0_f32, 0_usize), |(sum, count), value| {
        (sum + value, count.saturating_add(1))
    });
    if count == 0 {
        0.0
    } else {
        sum / count as f32
    }
}

fn index_task_events(events: &[EventRecord]) -> TaskEventIndex<'_> {
    let mut indexed: TaskEventIndex<'_> = BTreeMap::new();
    for record in events {
        if let Some(task_id) = record.event.task_id() {
            indexed
                .entry((record.event.agent.agent_id.clone(), task_id.to_owned()))
                .or_default()
                .push(record);
        }
    }
    for records in indexed.values_mut() {
        records.sort_by(|left, right| {
            (left.event.occurred_at, left.event.event_id)
                .cmp(&(right.event.occurred_at, right.event.event_id))
        });
    }
    indexed
}

fn source_event_ids(records: Option<&[&EventRecord]>) -> Vec<Uuid> {
    records.map_or_else(Vec::new, |records| {
        records
            .iter()
            .rev()
            .take(MAX_SOURCE_EVENT_IDS)
            .map(|record| record.event.event_id)
            .collect()
    })
}

fn dependency_state(
    task: &TaskState,
    task_lookup: &TaskLookup<'_>,
) -> (Vec<String>, Vec<String>) {
    let mut unresolved = Vec::new();
    let mut missing = Vec::new();
    for dependency_id in &task.task.depends_on {
        let key = (task.agent_id.clone(), dependency_id.clone());
        match task_lookup.get(&key) {
            Some(dependency) if dependency.status == TaskStatus::Succeeded => {}
            Some(_) => unresolved.push(dependency_id.clone()),
            None => missing.push(dependency_id.clone()),
        }
    }
    unresolved.sort();
    missing.sort();
    (unresolved, missing)
}

fn dependency_cycle_members(task_lookup: &TaskLookup<'_>) -> BTreeSet<TaskKey> {
    let mut by_agent: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for ((agent_id, task_id), task) in task_lookup {
        let dependencies = task
            .task
            .depends_on
            .iter()
            .filter(|dependency_id| {
                task_lookup.contains_key(&(agent_id.clone(), (*dependency_id).clone()))
            })
            .cloned()
            .collect::<Vec<_>>();
        by_agent
            .entry(agent_id.clone())
            .or_default()
            .insert(task_id.clone(), dependencies);
    }

    let mut members = BTreeSet::new();
    for (agent_id, graph) in by_agent {
        let mut states = BTreeMap::new();
        let mut stack = Vec::new();
        for task_id in graph.keys() {
            visit_cycle(
                task_id,
                &graph,
                &mut states,
                &mut stack,
                &mut members,
                &agent_id,
            );
        }
    }
    members
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Done,
}

fn visit_cycle(
    task_id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    states: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
    members: &mut BTreeSet<(String, String)>,
    agent_id: &str,
) {
    match states.get(task_id).copied() {
        Some(VisitState::Done) => return,
        Some(VisitState::Visiting) => {
            if let Some(position) = stack.iter().position(|value| value == task_id) {
                for cycle_task_id in &stack[position..] {
                    members.insert((agent_id.to_owned(), cycle_task_id.clone()));
                }
            }
            return;
        }
        None => {}
    }

    states.insert(task_id.to_owned(), VisitState::Visiting);
    stack.push(task_id.to_owned());
    if let Some(dependencies) = graph.get(task_id) {
        for dependency_id in dependencies {
            visit_cycle(
                dependency_id,
                graph,
                states,
                stack,
                members,
                agent_id,
            );
        }
    }
    stack.pop();
    states.insert(task_id.to_owned(), VisitState::Done);
}

fn remaining_depths(
    task_lookup: &TaskLookup<'_>,
    cycle_members: &BTreeSet<(String, String)>,
) -> BTreeMap<(String, String), usize> {
    let mut memo = BTreeMap::new();
    for key in task_lookup.keys() {
        remaining_depth(key, task_lookup, cycle_members, &mut memo);
    }
    memo
}

fn remaining_depth(
    key: &(String, String),
    task_lookup: &TaskLookup<'_>,
    cycle_members: &BTreeSet<(String, String)>,
    memo: &mut BTreeMap<(String, String), usize>,
) -> usize {
    if let Some(depth) = memo.get(key).copied() {
        return depth;
    }
    let Some(task) = task_lookup.get(key) else {
        return 0;
    };
    if task.status == TaskStatus::Succeeded {
        memo.insert(key.clone(), 0);
        return 0;
    }
    if cycle_members.contains(key) {
        memo.insert(key.clone(), 1);
        return 1;
    }

    memo.insert(key.clone(), 1);
    let dependency_depth = task
        .task
        .depends_on
        .iter()
        .map(|dependency_id| (key.0.clone(), dependency_id.clone()))
        .filter(|dependency_key| task_lookup.contains_key(dependency_key))
        .map(|dependency_key| remaining_depth(&dependency_key, task_lookup, cycle_members, memo))
        .max()
        .unwrap_or(0);
    let depth = dependency_depth.saturating_add(1);
    memo.insert(key.clone(), depth);
    depth
}

fn critical_paths(
    tasks: &[TaskState],
    cycle_members: &BTreeSet<TaskKey>,
    remaining_depths: &BTreeMap<TaskKey, usize>,
) -> CriticalPathResult {
    let mut grouped: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    for task in tasks {
        if let Some(goal_id) = &task.task.goal_id {
            grouped
                .entry((task.agent_id.clone(), goal_id.clone()))
                .or_default()
                .push((task.agent_id.clone(), task.task.task_id.clone()));
        }
    }

    let mut critical_tasks = BTreeSet::new();
    let mut remaining = BTreeMap::new();
    for (goal_key, task_keys) in grouped {
        if task_keys.iter().any(|key| cycle_members.contains(key)) {
            remaining.insert(goal_key, None);
            continue;
        }
        let max_depth = task_keys
            .iter()
            .filter_map(|key| remaining_depths.get(key).copied())
            .max()
            .unwrap_or(0);
        for key in task_keys {
            if max_depth > 0 && remaining_depths.get(&key).copied() == Some(max_depth) {
                critical_tasks.insert(key);
            }
        }
        remaining.insert(goal_key, Some(max_depth));
    }
    (critical_tasks, remaining)
}
