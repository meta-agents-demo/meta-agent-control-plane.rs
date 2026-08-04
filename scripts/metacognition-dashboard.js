(() => {
  const state = { projection: null };
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value ?? '').replace(/[&<>'"]/g, (character) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;'
  }[character]));
  const pct = (value) => `${Math.round(Number(value || 0) * 100)}%`;
  const token = () => sessionStorage.getItem('meta-agent-read-token') || '';
  const headers = () => token() ? { Authorization: `Bearer ${token()}` } : {};

  function showError(message) {
    const banner = $('error-banner');
    banner.textContent = message;
    banner.hidden = false;
  }

  function clearError() {
    $('error-banner').hidden = true;
  }

  async function refresh() {
    try {
      const response = await fetch('/api/v1/metacognition', {
        headers: headers(),
        cache: 'no-store'
      });
      if (!response.ok) throw new Error(`Projection failed: HTTP ${response.status}`);
      state.projection = await response.json();
      render(state.projection);
      clearError();
    } catch (error) {
      showError(error.message || String(error));
    }
  }

  function render(projection) {
    const summary = projection.summary;
    $('stat-active').textContent = summary.active_tasks;
    $('stat-blocked').textContent = summary.blocked_tasks;
    $('stat-stalled').textContent = summary.stalled_tasks;
    $('stat-retries').textContent = summary.retry_loops;
    $('stat-evidence').textContent = pct(summary.evidence_coverage);
    $('stat-progress').textContent = `${pct(summary.evidence_backed_progress)} / ${pct(summary.self_reported_progress)}`;
    $('revision').textContent = `Revision ${projection.revision} · ${new Date(projection.generated_at).toLocaleTimeString()}`;
    renderDiagnostics(projection.diagnostics);
    renderGoals(projection.goals);
    renderTasks(projection.tasks);
  }

  function renderDiagnostics(diagnostics) {
    const target = $('diagnostics');
    if (!diagnostics.length) {
      target.innerHTML = '<p class="empty">No diagnostics. The retained state is internally consistent.</p>';
      return;
    }
    target.innerHTML = diagnostics.map((item) => `
      <article class="diagnostic severity-${esc(item.severity)}">
        <div class="row"><strong>${esc(item.summary)}</strong><span class="pill">${esc(item.severity)}</span></div>
        <code>${esc(item.rule)} · ${esc(item.agent_id)}${item.task_id ? ` / ${esc(item.task_id)}` : ''}</code>
        <p>${esc(item.explanation)}</p>
        ${item.recommended_action ? `<p class="action"><b>Next:</b> ${esc(item.recommended_action)}</p>` : ''}
        <small>${item.source_events_retained ? `${item.source_event_ids.length} retained causal event(s)` : 'Causal events no longer retained'}</small>
      </article>`).join('');
  }

  function renderGoals(goals) {
    const target = $('goals');
    if (!goals.length) {
      target.innerHTML = '<p class="empty">No retained goals.</p>';
      return;
    }
    target.innerHTML = goals.map((goal) => `
      <article class="goal">
        <div class="row"><strong>${esc(goal.title)}</strong><span class="pill">${goal.blocked_tasks} blocked</span></div>
        <code>${esc(goal.agent_id)} / ${esc(goal.goal_id)}</code>
        <div class="meter"><span style="width:${pct(goal.evidence_backed_progress)}"></span></div>
        <p>${pct(goal.evidence_backed_progress)} evidence-backed of ${pct(goal.self_reported_progress)} self-reported · ${pct(goal.evidence_coverage)} evidence coverage</p>
        <small>${goal.critical_path_remaining == null ? 'Critical path unavailable' : `Critical path depth ${goal.critical_path_remaining}`}${goal.critical_path_task_ids.length ? ` · ${goal.critical_path_task_ids.map(esc).join(', ')}` : ''}</small>
      </article>`).join('');
  }

  function renderTasks(tasks) {
    const target = $('tasks');
    if (!tasks.length) {
      target.innerHTML = '<tr><td colspan="7" class="empty">No retained tasks.</td></tr>';
      return;
    }
    target.innerHTML = tasks.map((task) => `
      <tr>
        <td><strong>${esc(task.task_id)}</strong><small>${esc(task.agent_id)}</small></td>
        <td>${esc(task.status)}</td>
        <td>${pct(task.evidence_backed_progress)} / ${pct(task.self_reported_progress)}</td>
        <td>${task.evidence_count}</td>
        <td>${task.attempt}</td>
        <td>${task.stale_for_seconds}s</td>
        <td>${task.diagnostic_ids.length}${task.on_critical_path ? ' · critical path' : ''}</td>
      </tr>`).join('');
  }

  $('save-token').addEventListener('click', () => {
    sessionStorage.setItem('meta-agent-read-token', $('auth-token').value.trim());
    refresh();
  });
  $('refresh').addEventListener('click', refresh);
  $('auth-token').value = token();
  refresh();
  setInterval(refresh, 5000);
})();
