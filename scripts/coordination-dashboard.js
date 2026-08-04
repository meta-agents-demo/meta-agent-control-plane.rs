(() => {
  const state = { plan: null };
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value ?? '').replace(/[&<>'"]/g, (character) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;'
  }[character]));
  const token = () => sessionStorage.getItem('meta-agent-read-token') || '';
  const headers = () => token() ? { Authorization: `Bearer ${token()}` } : {};
  const label = (value) => String(value ?? '')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .trim()
    .replace(/^./, (character) => character.toUpperCase());

  function showError(message) {
    const banner = $('error-banner');
    banner.textContent = message;
    banner.hidden = false;
  }

  function clearError() {
    $('error-banner').hidden = true;
  }

  function provenance(item) {
    if (!item.source_events_retained) return 'Some causal events are no longer retained';
    const count = Array.isArray(item.source_event_ids) ? item.source_event_ids.length : 0;
    return `${count} retained source event${count === 1 ? '' : 's'}`;
  }

  function diagnostics(item) {
    const ids = Array.isArray(item.diagnostic_ids) ? item.diagnostic_ids : [];
    return ids.length ? ids.map(esc).join(', ') : 'No linked diagnostic';
  }

  async function refresh() {
    try {
      const response = await fetch('/api/v1/coordination', {
        headers: headers(),
        cache: 'no-store'
      });
      if (!response.ok) throw new Error(`Coordination plan failed: HTTP ${response.status}`);
      state.plan = await response.json();
      render(state.plan);
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
    $('stat-holds').textContent = summary.held_tasks;
    $('stat-suppressed').textContent = summary.suppressed_by_assignment_limits;
    $('stat-omitted').textContent = summary.omitted_interventions + summary.omitted_holds;
    $('revision').textContent = `Revision ${plan.revision} · ${new Date(plan.generated_at).toLocaleTimeString()}`;
    $('policy').textContent = `${plan.planning_policy.max_assignments} total · ${plan.planning_policy.max_assignments_per_agent} per agent · ${plan.planning_policy.max_interventions} interventions · ${plan.planning_policy.max_holds} holds`;
    renderAssignments(plan.assignments);
    renderInterventions(plan.interventions);
    renderHolds(plan.held_tasks);
  }

  function renderAssignments(assignments) {
    const target = $('assignments');
    if (!assignments.length) {
      target.innerHTML = '<p class="empty">No executable assignments in the retained snapshot.</p>';
      return;
    }
    target.innerHTML = assignments.map((item) => `
      <article class="assignment ${item.on_critical_path ? 'critical-path' : ''}">
        <div class="row">
          <div><strong>${esc(label(item.action))}</strong><code>${esc(item.agent_id)} / ${esc(item.task_id)}</code></div>
          <span class="priority">${esc(item.priority)}</span>
        </div>
        <p>${esc(item.rationale)}</p>
        <p class="action"><b>Recommended:</b> ${esc(item.recommended_action)}</p>
        <div class="meta">
          <span>${item.on_critical_path ? 'Critical path' : 'Standard path'}</span>
          <span>${esc(provenance(item))}</span>
          <span>${esc(diagnostics(item))}</span>
        </div>
      </article>`).join('');
  }

  function renderInterventions(interventions) {
    const target = $('interventions');
    if (!interventions.length) {
      target.innerHTML = '<p class="empty">No operator interventions required.</p>';
      return;
    }
    target.innerHTML = interventions.map((item) => `
      <article class="intervention">
        <div class="row">
          <div><strong>${esc(label(item.kind))}</strong><code>${esc(item.agent_id)} / ${esc(item.task_id)}</code></div>
          <span class="priority">${esc(item.priority)}</span>
        </div>
        <p>${esc(item.rationale)}</p>
        <p class="action"><b>Recommended:</b> ${esc(item.recommended_action)}</p>
        <div class="meta"><span>${esc(provenance(item))}</span><span>${esc(diagnostics(item))}</span></div>
      </article>`).join('');
  }

  function renderHolds(holds) {
    const target = $('holds');
    if (!holds.length) {
      target.innerHTML = '<tr><td colspan="5" class="empty">No held tasks.</td></tr>';
      return;
    }
    target.innerHTML = holds.map((item) => `
      <tr>
        <td><strong>${esc(item.task_id)}</strong><small>${esc(item.agent_id)}</small></td>
        <td>${esc(label(item.reason))}</td>
        <td>${esc(item.explanation)}</td>
        <td>${(item.unresolved_dependencies || []).length ? item.unresolved_dependencies.map(esc).join(', ') : '—'}</td>
        <td><small>${esc(provenance(item))}</small><small>${esc(diagnostics(item))}</small></td>
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
