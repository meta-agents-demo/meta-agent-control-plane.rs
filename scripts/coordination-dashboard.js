(() => {
  const state = {
    plan: null,
    socket: null,
    retry: 0,
    refreshing: false,
    dirty: false,
    refreshTimer: null
  };
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

  function setConnected(connected, text) {
    $('stream-indicator').className = `stream ${connected ? 'online' : 'offline'}`;
    $('stream-label').textContent = text;
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
    if (state.refreshing) {
      state.dirty = true;
      return;
    }
    state.refreshing = true;
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
    } finally {
      state.refreshing = false;
      if (state.dirty) {
        state.dirty = false;
        scheduleRefresh();
      }
    }
  }

  function scheduleRefresh() {
    if (state.refreshTimer) return;
    state.refreshTimer = setTimeout(() => {
      state.refreshTimer = null;
      refresh();
    }, 100);
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

  function connect() {
    if (state.socket) {
      state.socket.onclose = null;
      state.socket.close();
    }
    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${scheme}//${location.host}/ws/ui`);
    state.socket = socket;
    setConnected(false, 'Connecting');

    socket.onopen = () => socket.send(JSON.stringify({ token: token() }));
    socket.onmessage = (event) => {
      try {
        const message = JSON.parse(event.data);
        if (message.kind === 'authenticated') {
          state.retry = 0;
          setConnected(true, 'Live');
          refresh();
          return;
        }
        if (message.error) {
          showError(message.message || message.error);
          setConnected(false, 'Unauthorized');
          return;
        }
        if (message.kind === 'resync_required') {
          scheduleRefresh();
          return;
        }
        if (Number.isInteger(message.revision) && (!state.plan || message.revision > state.plan.revision)) {
          scheduleRefresh();
        }
      } catch (_) {
        scheduleRefresh();
      }
    };
    socket.onerror = () => setConnected(false, 'Connection error');
    socket.onclose = () => {
      if (state.socket !== socket) return;
      setConnected(false, 'Reconnecting');
      const delay = Math.min(15000, 600 * (2 ** state.retry++));
      setTimeout(connect, delay);
    };
  }

  $('save-token').addEventListener('click', () => {
    const value = $('auth-token').value.trim();
    if (value) sessionStorage.setItem('meta-agent-read-token', value);
    else sessionStorage.removeItem('meta-agent-read-token');
    state.retry = 0;
    connect();
    refresh();
  });
  $('refresh').addEventListener('click', refresh);
  $('auth-token').value = token();
  refresh();
  connect();
  setInterval(refresh, 30000);
})();
