pub fn analyze(snapshot: &Snapshot) -> MetacognitionSnapshot {
    analyze_with_policy(snapshot, AnalysisPolicy::default())
}

pub fn analyze_with_policy(
    snapshot: &Snapshot,
    policy: AnalysisPolicy,
) -> MetacognitionSnapshot {
    let policy = policy.normalized();
    let generated_at = snapshot.generated_at;

    let task_lookup: TaskLookup<'_> = snapshot
        .tasks
        .iter()
        .map(|task| {
            (
                (task.agent_id.clone(), task.task.task_id.clone()),
                task,
            )
        })
        .collect();
    let goal_lookup: BTreeSet<(String, String)> = snapshot
        .goals
        .iter()
        .map(|goal| (goal.agent_id.clone(), goal.goal.goal_id.clone()))
        .collect();
    let events_by_task = index_task_events(&snapshot.recent_events);
    let cycle_members = dependency_cycle_members(&task_lookup);
    let remaining_depths = remaining_depths(&task_lookup, &cycle_members);
    let (critical_path_tasks, goal_critical_path_remaining) = critical_paths(
        &snapshot.tasks,
        &cycle_members,
        &remaining_depths,
    );

    let mut diagnostics = Vec::new();
    let mut task_analyses = Vec::with_capacity(snapshot.tasks.len());

    let mut tasks = snapshot.tasks.iter().collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        (&left.agent_id, &left.task.task_id).cmp(&(&right.agent_id, &right.task.task_id))
    });

    for task in tasks {
        let key = (task.agent_id.clone(), task.task.task_id.clone());
        let source_event_ids = source_event_ids(events_by_task.get(&key).map(Vec::as_slice));
        let source_events_retained = !source_event_ids.is_empty();
        let stale_for_seconds = age_seconds(generated_at, task.updated_at);
        let blocker_age_seconds = task
            .blocker
            .as_ref()
            .map(|_| age_seconds(generated_at, task.updated_at));
        let evidence_count = task
            .latest_reflection
            .as_ref()
            .map_or(0, |reflection| reflection.evidence.len())
            .saturating_add(task.artifacts.len());
        let evidence_backed_progress = if evidence_count > 0 {
            task.progress
        } else {
            0.0
        };
        let confidence = task
            .latest_reflection
            .as_ref()
            .map(|reflection| reflection.confidence);

        let (unresolved_dependencies, missing_dependencies) = dependency_state(
            task,
            &task_lookup,
        );
        let mut diagnostic_ids = Vec::new();

        let mut add = |rule: DiagnosticRule,
                       severity: DiagnosticSeverity,
                       summary: String,
                       explanation: String,
                       recommended_action: Option<String>| {
            let diagnostic_id = diagnostic_id(rule, &task.agent_id, &task.task.task_id);
            diagnostic_ids.push(diagnostic_id.clone());
            diagnostics.push(Diagnostic {
                diagnostic_id,
                rule,
                severity,
                agent_id: task.agent_id.clone(),
                goal_id: task.task.goal_id.clone(),
                task_id: Some(task.task.task_id.clone()),
                summary,
                explanation,
                recommended_action,
                source_event_ids: source_event_ids.clone(),
                source_events_retained,
            });
        };

        if is_active(task.status) && stale_for_seconds >= policy.stale_after_seconds {
            let severity = if task.status == TaskStatus::Blocked
                && stale_for_seconds >= policy.stale_after_seconds.saturating_mul(2)
            {
                DiagnosticSeverity::Critical
            } else {
                DiagnosticSeverity::Warning
            };
            add(
                DiagnosticRule::StalledTask,
                severity,
                format!("Task {} has stopped producing observable progress", task.task.task_id),
                format!(
                    "The task is {:?} and its latest retained state is {} seconds old; the configured stall threshold is {} seconds.",
                    task.status, stale_for_seconds, policy.stale_after_seconds
                ),
                Some("Request a checkpoint, verify the blocker, or revise the plan.".to_owned()),
            );
        }

        if task.attempt >= policy.retry_loop_attempts
            && !matches!(task.status, TaskStatus::Succeeded | TaskStatus::Canceled)
        {
            add(
                DiagnosticRule::RetryLoop,
                DiagnosticSeverity::Warning,
                format!("Task {} is in a repeated-attempt loop", task.task.task_id),
                format!(
                    "The task is on attempt {}. The retry-loop threshold is {} attempts.",
                    task.attempt, policy.retry_loop_attempts
                ),
                Some("Change a causal variable before retrying again and record the result.".to_owned()),
            );
        }

        if task.status == TaskStatus::Running && !unresolved_dependencies.is_empty() {
            add(
                DiagnosticRule::BlockedDependency,
                DiagnosticSeverity::Warning,
                format!("Task {} is running before its dependencies are complete", task.task.task_id),
                format!(
                    "Unresolved dependencies: {}.",
                    unresolved_dependencies.join(", ")
                ),
                Some("Pause the task or explicitly revise the dependency graph.".to_owned()),
            );
        }

        if !missing_dependencies.is_empty() {
            add(
                DiagnosticRule::OrphanDependency,
                DiagnosticSeverity::Critical,
                format!("Task {} references unknown dependencies", task.task.task_id),
                format!("Missing scoped task definitions: {}.", missing_dependencies.join(", ")),
                Some("Declare the missing tasks or remove the stale dependency references.".to_owned()),
            );
        }

        if cycle_members.contains(&key) {
            add(
                DiagnosticRule::DependencyCycle,
                DiagnosticSeverity::Critical,
                format!("Task {} participates in a dependency cycle", task.task.task_id),
                "The scoped task graph cannot produce a finite critical path while this cycle exists."
                    .to_owned(),
                Some("Break the cycle by revising at least one dependency edge.".to_owned()),
            );
        }

        if let Some(goal_id) = &task.task.goal_id
            && !goal_lookup.contains(&(task.agent_id.clone(), goal_id.clone()))
        {
            add(
                DiagnosticRule::OrphanGoal,
                DiagnosticSeverity::Warning,
                format!("Task {} references an unknown goal", task.task.task_id),
                format!(
                    "Goal {goal_id} has not been retained or was never declared for agent {}.",
                    task.agent_id
                ),
                Some("Declare the goal or reassign the task to a retained goal.".to_owned()),
            );
        }

        if (task.progress > 0.0 || is_terminal(task.status)) && evidence_count == 0 {
            let severity = if is_terminal(task.status) {
                DiagnosticSeverity::Warning
            } else {
                DiagnosticSeverity::Info
            };
            add(
                DiagnosticRule::MissingEvidence,
                severity,
                format!("Task {} has progress without retained evidence", task.task.task_id),
                format!(
                    "Self-reported progress is {:.0}% but no reflection evidence or completion artifact is retained.",
                    task.progress * 100.0
                ),
                Some("Attach a test, artifact, measurement, or other visible evidence reference.".to_owned()),
            );
        }

        if let Some(confidence) = confidence
            && confidence < policy.low_confidence_threshold
        {
            add(
                DiagnosticRule::LowConfidence,
                DiagnosticSeverity::Warning,
                format!("Task {} has low declared confidence", task.task.task_id),
                format!(
                    "Latest declared confidence is {:.0}%; the policy threshold is {:.0}%.",
                    confidence * 100.0,
                    policy.low_confidence_threshold * 100.0
                ),
                Some("Gather another independent signal or narrow the claim before acting.".to_owned()),
            );
        }

        if is_active(task.status) && task.next_action.is_none() {
            add(
                DiagnosticRule::MissingNextAction,
                if task.status == TaskStatus::Blocked {
                    DiagnosticSeverity::Warning
                } else {
                    DiagnosticSeverity::Info
                },
                format!("Task {} has no explicit next action", task.task.task_id),
                "The latest retained state does not identify the next observable step."
                    .to_owned(),
                Some("Record one concrete, testable next action.".to_owned()),
            );
        }

        let completion_mismatch = match (task.status, task.outcome) {
            (TaskStatus::Succeeded, Some(TaskOutcome::Succeeded)) => task.progress < 1.0,
            (TaskStatus::Failed, Some(TaskOutcome::Failed))
            | (TaskStatus::Canceled, Some(TaskOutcome::Canceled))
            | (TaskStatus::Partial, Some(TaskOutcome::Partial)) => false,
            (status, _) if is_terminal(status) => true,
            (_, Some(_)) => true,
            _ => false,
        };
        if completion_mismatch {
            add(
                DiagnosticRule::CompletionMismatch,
                DiagnosticSeverity::Critical,
                format!("Task {} has contradictory completion state", task.task.task_id),
                format!(
                    "Status {:?}, outcome {:?}, and progress {:.0}% do not form a consistent terminal projection.",
                    task.status,
                    task.outcome,
                    task.progress * 100.0
                ),
                Some("Replay the causal events and publish a corrected authoritative state.".to_owned()),
            );
        }

        diagnostic_ids.sort();
        task_analyses.push(TaskAnalysis {
            agent_id: task.agent_id.clone(),
            task_id: task.task.task_id.clone(),
            goal_id: task.task.goal_id.clone(),
            status: task.status,
            self_reported_progress: task.progress,
            evidence_backed_progress,
            confidence,
            evidence_count,
            attempt: task.attempt,
            stale_for_seconds,
            blocker_age_seconds,
            unresolved_dependencies,
            missing_dependencies,
            on_critical_path: critical_path_tasks.contains(&key),
            diagnostic_ids,
            source_event_ids,
            source_events_retained,
        });
    }

    diagnostics.sort_by(|left, right| {
        right
            .severity
            .rank()
            .cmp(&left.severity.rank())
            .then_with(|| left.diagnostic_id.cmp(&right.diagnostic_id))
    });

    let mut goal_analyses = Vec::with_capacity(snapshot.goals.len());
    let mut goals = snapshot.goals.iter().collect::<Vec<_>>();
    goals.sort_by(|left, right| {
        (&left.agent_id, &left.goal.goal_id).cmp(&(&right.agent_id, &right.goal.goal_id))
    });

    for goal in goals {
        let goal_tasks = task_analyses
            .iter()
            .filter(|task| {
                task.agent_id == goal.agent_id
                    && task.goal_id.as_deref() == Some(goal.goal.goal_id.as_str())
            })
            .collect::<Vec<_>>();
        let total_tasks = goal_tasks.len();
        let active_tasks = goal_tasks.iter().filter(|task| is_active(task.status)).count();
        let blocked_tasks = goal_tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Blocked)
            .count();
        let completed_tasks = goal_tasks
            .iter()
            .filter(|task| is_terminal(task.status))
            .count();
        let stalled_tasks = goal_tasks
            .iter()
            .filter(|task| task.stale_for_seconds >= policy.stale_after_seconds && is_active(task.status))
            .count();
        let tasks_with_evidence = goal_tasks
            .iter()
            .filter(|task| task.evidence_count > 0)
            .count();
        let key = (goal.agent_id.clone(), goal.goal.goal_id.clone());
        let critical_path_remaining = goal_critical_path_remaining.get(&key).copied().flatten();
        let mut critical_path_task_ids = goal_tasks
            .iter()
            .filter(|task| task.on_critical_path)
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>();
        critical_path_task_ids.sort();
        let mut diagnostic_ids = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.agent_id == goal.agent_id
                    && diagnostic.goal_id.as_deref() == Some(goal.goal.goal_id.as_str())
            })
            .map(|diagnostic| diagnostic.diagnostic_id.clone())
            .collect::<Vec<_>>();
        diagnostic_ids.sort();
        let mut data_quality_warnings = Vec::new();
        if total_tasks == 0 {
            data_quality_warnings.push(
                "No retained tasks are mapped to this goal, so progress is indeterminate."
                    .to_owned(),
            );
        }
        if critical_path_remaining.is_none() && total_tasks > 0 {
            data_quality_warnings.push(
                "Critical-path progress is unavailable because the task graph contains a cycle."
                    .to_owned(),
            );
        }
        if goal_tasks
            .iter()
            .any(|task| !task.missing_dependencies.is_empty())
        {
            data_quality_warnings.push(
                "At least one task references a dependency that is not retained.".to_owned(),
            );
        }

        goal_analyses.push(GoalAnalysis {
            agent_id: goal.agent_id.clone(),
            goal_id: goal.goal.goal_id.clone(),
            title: goal.goal.title.clone(),
            success_criteria_count: goal.goal.success_criteria.len(),
            total_tasks,
            active_tasks,
            blocked_tasks,
            completed_tasks,
            stalled_tasks,
            self_reported_progress: mean(goal_tasks.iter().map(|task| task.self_reported_progress)),
            evidence_backed_progress: mean(
                goal_tasks
                    .iter()
                    .map(|task| task.evidence_backed_progress),
            ),
            evidence_coverage: ratio(tasks_with_evidence, total_tasks),
            critical_path_remaining,
            critical_path_task_ids,
            diagnostic_ids,
            data_quality_warnings,
        });
    }

    let total_tasks = task_analyses.len();
    let tasks_with_evidence = task_analyses
        .iter()
        .filter(|task| task.evidence_count > 0)
        .count();
    let summary = MetacognitionSummary {
        total_goals: goal_analyses.len(),
        total_tasks,
        active_tasks: task_analyses
            .iter()
            .filter(|task| is_active(task.status))
            .count(),
        blocked_tasks: task_analyses
            .iter()
            .filter(|task| task.status == TaskStatus::Blocked)
            .count(),
        stalled_tasks: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == DiagnosticRule::StalledTask)
            .count(),
        retry_loops: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == DiagnosticRule::RetryLoop)
            .count(),
        critical_diagnostics: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Critical)
            .count(),
        warning_diagnostics: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
            .count(),
        info_diagnostics: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Info)
            .count(),
        self_reported_progress: mean(
            task_analyses
                .iter()
                .map(|task| task.self_reported_progress),
        ),
        evidence_backed_progress: mean(
            task_analyses
                .iter()
                .map(|task| task.evidence_backed_progress),
        ),
        evidence_coverage: ratio(tasks_with_evidence, total_tasks),
    };

    MetacognitionSnapshot {
        generated_at,
        revision: snapshot.revision,
        policy,
        summary,
        goals: goal_analyses,
        tasks: task_analyses,
        diagnostics,
    }
}
