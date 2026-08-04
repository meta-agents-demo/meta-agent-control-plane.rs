(() => {
  const state = {
    snapshot: null,
    socket: null,
    retry: 0,
    refreshing: false,
    dirty: false,
    refreshTimer: null,
    query: ''
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
  const date = (value) => value ? new Date(value).toLocaleString() : '—';
  const list = (values) => Array.isArray(values) && values.length
    ? values.map(esc).join(', ')
    : '—';
  const searchable = (...values) => values.flat(Infinity).filter((value) => value != null)
    .map((value) => typeof value === 'object' ? JSON.stringify(value) : String(value))
    .join(' ')
    .toLowerCase();

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

  function boundedInput(id, minimum, maximum, fallback) {
    const raw = $(id).value.trim();
    if (!raw) return fallback;
    if (!/^\d+$/.test(raw)) throw new Error(`${label(id)} must be a base-10 integer.`);
    const value = Number(raw);
    if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
      throw new Error(`${label(id)} must be between ${minimum} and ${maximum}.`);
    }
    return value;
  }

  function explorerUrl() {
    const parameters = new URLSearchParams({
      timeline_limit: String(boundedInput('timeline-limit', 1, 250, 100)),
      session_limit: String(boundedInput('session-limit', 1, 250, 100)),
      lesson_limit: String(boundedInput('lesson-limit', 1, 1000, 250))
    });
    return `/api/v1/explorer?${parameters.toString()}`;
  }

  async function refresh() {
    if (state.refreshing) {
      state.dirty = true;
      return;
    }
    state.refreshing = true;
    try {
      const response = await fetch(explorerUrl(), {
        headers: headers(),
        cache: 'no-store'
      });
      if (!response.ok) throw new Error(`Explorer read failed: HTTP ${response.status}`);
      state.snapshot = await response.json();
      render();
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

  function matches(...values) {
    return !state.query || searchable(values).includes(state.query);
  }

  function render() {
    const snapshot = state.snapshot;
    if (!snapshot) return;
    $('revision').textContent = `Revision ${snapshot.revision} · ${date(snapshot.generated_at)}`;
    $('stat-agents').textContent = snapshot.system.agents;
    $('stat-sessions').textContent = snapshot.system.retained_sessions;
    $('stat-events').textContent = snapshot.system.retained_events;
    $('stat-lessons').textContent = snapshot.system.lessons;
    $('stat-tasks').textContent = snapshot.system.tasks;
    $('stat-uptime').textContent = `${snapshot.system.uptime_seconds}s`;
    $('retention').textContent = `${snapshot.retention.returned_timeline_events}/${snapshot.retention.total_timeline_events} events · ${snapshot.retention.returned_sessions}/${snapshot.retention.total_sessions} sessions · ${snapshot.retention.returned_lessons}/${snapshot.retention.total_lessons} lessons`;
    renderAgents(snapshot.agents.filter((agent) => matches(
      agent.agent.agent_id, agent.display_name, agent.agent.provider, agent.agent.model,
      agent.status, agent.session_id, agent.current_goal_id, agent.active_task_id,
      agent.capabilities, agent.metadata, agent.latest_reflection, agent.latest_error
    )));
    renderSessions(snapshot.sessions.filter((session) => matches(
      session.session_id, session.agent_ids, session.task_ids, session.event_kinds,
      session.transports, session.latest_event_kind, session.latest_task_id
    )));
    renderTimeline(snapshot.timeline.filter((record) => matches(
      record.event.event_id, record.event.kind, record.event.agent.agent_id,
      record.event.agent.provider, record.event.agent.model, record.event.session_id,
      record.event.correlation_id, record.event.data, record.transport
    )));
    renderLessons(snapshot.lessons.filter((lesson) => matches(
      lesson.lesson.lesson_id, lesson.lesson.statement, lesson.agent_id,
      lesson.lesson.source_task_id, lesson.lesson.tags, lesson.lesson.applicability,
      lesson.lesson.evidence
    )));
    renderSystem(snapshot.system);
  }

  function renderAgents(agents) {
    const target = $('agents');
    if (!agents.length) {
      target.innerHTML = '<p class="empty">No matching retained agents.</p>';
      return;
    }
    target.innerHTML = agents.map((agent) => `
      <article class="card agent-card">
        <div class="row">
          <div><strong>${esc(agent.display_name || agent.agent.agent_id)}</strong><code>${esc(agent.agent.agent_id)}</code></div>
          <span class="status">${esc(label(agent.status))}</span>
        </div>
        <p>${esc(agent.agent.provider)} · ${esc(agent.agent.model)}${agent.agent.instance_id ? ` · ${esc(agent.agent.instance_id)}` : ''}</p>
        <dl>
          <dt>Session</dt><dd>${esc(agent.session_id || '—')}</dd>
          <dt>Goal</dt><dd>${esc(agent.current_goal_id || '—')}</dd>
          <dt>Task</dt><dd>${esc(agent.active_task_id || '—')}</dd>
          <dt>Last seen</dt><dd>${esc(date(agent.last_seen_at))}</dd>
          <dt>Completed / failed</dt><dd>${esc(agent.completed_tasks)} / ${esc(agent.failed_tasks)}</dd>
        </dl>
        <p class="meta-line"><b>Capabilities:</b> ${list(agent.capabilities)}</p>
        ${agent.latest_reflection ? `<p class="note"><b>Reflection:</b> ${esc(agent.latest_reflection.summary)} · confidence ${esc(agent.latest_reflection.confidence)}</p>` : ''}
        ${agent.latest_error ? `<p class="error-note"><b>Latest error:</b> ${esc(agent.latest_error.code)} — ${esc(agent.latest_error.message)}</p>` : ''}
      </article>`).join('');
  }

  function renderSessions(sessions) {
    const target = $('sessions');
    if (!sessions.length) {
      target.innerHTML = '<tr><td colspan="7" class="empty">No matching retained sessions.</td></tr>';
      return;
    }
    target.innerHTML = sessions.map((session) => `
      <tr>
        <td><strong>${esc(session.session_id)}</strong><small>${esc(session.event_count)} retained events</small></td>
        <td>${list(session.agent_ids)}</td>
        <td>${list(session.task_ids)}</td>
        <td>${esc(session.latest_event_kind)}<small>${esc(session.latest_task_id || 'No task')}</small></td>
        <td>${esc(date(session.first_occurred_at))}</td>
        <td>${esc(date(session.last_occurred_at))}</td>
        <td><small>${esc(Object.entries(session.transports || {}).map(([name, count]) => `${name}:${count}`).join(' · ') || '—')}</small></td>
      </tr>`).join('');
  }

  function renderTimeline(records) {
    const target = $('timeline');
    if (!records.length) {
      target.innerHTML = '<tr><td colspan="7" class="empty">No matching retained events.</td></tr>';
      return;
    }
    target.innerHTML = records.map((record) => `
      <tr>
        <td><strong>${esc(label(record.event.kind))}</strong><small>${esc(record.event.event_id)}</small></td>
        <td>${esc(record.event.agent.agent_id)}<small>${esc(record.event.agent.provider)} / ${esc(record.event.agent.model)}</small></td>
        <td>${esc(record.event.session_id || '—')}</td>
        <td>${esc(record.event.data?.task_id || '—')}</td>
        <td>${esc(record.transport)}</td>
        <td>${esc(date(record.event.occurred_at))}</td>
        <td><code class="payload">${esc(JSON.stringify(record.event.data))}</code></td>
      </tr>`).join('');
  }

  function renderLessons(lessons) {
    const target = $('lessons');
    if (!lessons.length) {
      target.innerHTML = '<p class="empty">No matching retained lessons.</p>';
      return;
    }
    target.innerHTML = lessons.map((item) => `
      <article class="card lesson-card">
        <div class="row">
          <div><strong>${esc(item.lesson.statement)}</strong><code>${esc(item.agent_id)} / ${esc(item.lesson.lesson_id)}</code></div>
          <span class="confidence">${esc(Math.round(item.lesson.confidence * 100))}%</span>
        </div>
        <p>${esc(item.lesson.applicability || 'No applicability note')}</p>
        <p class="meta-line"><b>Source task:</b> ${esc(item.lesson.source_task_id || '—')} · <b>Observations:</b> ${esc(item.observations)} · <b>Learned:</b> ${esc(date(item.learned_at))}</p>
        <p class="meta-line"><b>Tags:</b> ${list(item.lesson.tags)}</p>
      </article>`).join('');
  }

  function renderSystem(system) {
    $('accepted').textContent = system.counters.accepted;
    $('duplicates').textContent = system.counters.duplicate;
    $('rejected').textContent = system.counters.rejected;
    const target = $('caches');
    target.innerHTML = Object.entries(system.caches).map(([name, cache]) => `
      <tr>
        <td>${esc(label(name))}</td>
        <td>${esc(cache.length)}</td>
        <td>${esc(cache.capacity)}</td>
        <td>${esc(Math.round(cache.pressure * 100))}%</td>
        <td>${esc(cache.evictions)}</td>
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
        if (Number.isInteger(message.revision) && (!state.snapshot || message.revision > state.snapshot.revision)) {
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
  $('apply-limits').addEventListener('click', refresh);
  $('refresh').addEventListener('click', refresh);
  $('search').addEventListener('input', (event) => {
    state.query = event.target.value.trim().toLowerCase();
    render();
  });
  $('auth-token').value = token();
  refresh();
  connect();
  setInterval(refresh, 30000);
})();
