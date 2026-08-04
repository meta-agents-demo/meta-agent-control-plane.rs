(() => {
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value ?? '').replace(/[&<>'"]/g, (character) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;'
  }[character]));
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
      const response = await fetch('/api/v1/coordination', {
        headers: headers(),
        cache: 'no-store'
      });
      if (!response.ok) throw new Error(`Coordination plan failed: HTTP ${response.status}`);
      render(await response.json());
      clearError();
    } catch (error) {
      showError(error.message || String(error));
    }
  }

  function render(plan) {
    const summary = plan.summary;
    $('stat-assignments').textContent = summary.assignments;
    $('stat-agents').textContent = summary.agents_with_assignments;
    $('stat-interventions').textContent = summary.interventions;
    $('stat-held').textContent = summary.held_tasks;
    $('stat-suppressed').textContent = summary.suppressed_by_assignment_limits;
    $('stat-total').textContent = summary.total_tasks;
    $('revision').textContent = `Revision ${plan.revision} · ${new Date(plan.generated_at).toLocaleTimeString()}`;
    renderAssignments(plan.assignments);
    renderInterventions(plan.interventions);
    renderHeld(plan.held_tasks);
  }

  function renderAssignments(assignments) {
    const target = $('assignments');
    if (!assignments.length) {
      target.innerHTML = '<p class="empty">No executable recommendations for this snapshot.</p>';
      return;
    }
    target.innerHTML = assignments.map((item) => `
      <article class="assignment">
        <div class="row"><strong>${esc(item.task_id)}</strong><span class="pill">priority ${esc(item.priority)}</span></div>
        <code>${esc(item.agent_id)} · ${esc(item.action)}${item.goal_id ? ` · ${esc(item.goal_id)}` : ''}</code>
        <p>${esc(item.rationale)}</p>
        <p class="action"><b>Next:</b> ${esc(item.recommended_action)}</p>
        <small>${item.on_critical_path ? 'Critical path · ' : ''}${item.source_events_retained ? `${item.source_event_ids.length} retained source event(s)` : 'Source events no longer retained'} · ${esc(item.assignment_id)}</small>
      </article>`).join('');
  }

  function renderInterventions(interventions) {
    const target = $('interventions');
    if (!interventions.length) {
      target.innerHTML = '<p class="empty">No graph or lifecycle interventions required.</p>';
      return;
    }
    target.innerHTML = interventions.map((item) => `
      <article class="intervention">
        <div class="row"><strong>${esc(item.task_id)}</strong><span class="pill">priority ${esc(item.priority)}</span></div>
        <code>${esc(item.agent_id)} · ${esc(item.kind)}</code>
        <p>${esc(item.rationale)}</p>
        <p class="action"><b>Repair:</b> ${esc(item.recommended_action)}</p>
        <small>${item.source_events_retained ? `${item.source_event_ids.length} retained source event(s)` : 'Source events no longer retained'} · ${esc(item.intervention_id)}</small>
      </article>`).join('');
  }

  function renderHeld(heldTasks) {
    const target = $('held');
    if (!heldTasks.length) {
      target.innerHTML = '<tr><td colspan="5" class="empty">No held work.</td></tr>';
      return;
    }
    target.innerHTML = heldTasks.map((item) => `
      <tr>
        <td><strong>${esc(item.task_id)}</strong>${item.goal_id ? `<small>${esc(item.goal_id)}</small>` : ''}</td>
        <td>${esc(item.agent_id)}</td>
        <td>${esc(item.reason)}</td>
        <td>${esc(item.explanation)}</td>
        <td>${item.unresolved_dependencies.length ? item.unresolved_dependencies.map(esc).join(', ') : '—'}</td>
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
