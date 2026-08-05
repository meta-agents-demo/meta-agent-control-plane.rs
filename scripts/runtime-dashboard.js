(() => {
  const PANEL_STORAGE_KEY = 'meta-agent-runtime-panels-v1';
  const TOKEN_STORAGE_KEY = 'meta-agent-read-token';
  const REFRESH_INTERVAL_MS = 2000;
  const state = { snapshot: null, refreshTimer: null, refreshing: false };
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value ?? '').replace(/[&<>'"]/g, (char) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;'
  }[char]));
  const token = () => sessionStorage.getItem(TOKEN_STORAGE_KEY) || '';
  const authHeaders = () => token() ? { Authorization: `Bearer ${token()}` } : {};
  const jsonHeaders = () => ({ ...authHeaders(), 'Content-Type': 'application/json' });
  const number = (value, digits = 1) => Number(value || 0).toLocaleString(undefined, {
    maximumFractionDigits: digits
  });
  const percent = (value) => value == null ? 'warming up' : `${number(value)}%`;
  const confidence = (value) => value == null ? 'unreported' : `${Math.round(Number(value) * 100)}%`;
  const bytes = (value) => {
    let amount = Number(value || 0);
    const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    let unit = 0;
    while (amount >= 1024 && unit < units.length - 1) { amount /= 1024; unit += 1; }
    return `${number(amount, amount < 10 ? 1 : 0)} ${units[unit]}`;
  };
  const time = (value) => value ? new Date(value).toLocaleTimeString() : 'never';
  const showError = (message) => {
    const banner = $('error-banner');
    banner.textContent = message;
    banner.hidden = false;
  };
  const clearError = () => { $('error-banner').hidden = true; };
  const setConnection = (ok, label) => {
    $('connection').className = `connection ${ok ? 'online' : 'offline'}`;
    $('connection-label').textContent = label;
  };

  function loadPanels() {
    let stored = {};
    try { stored = JSON.parse(localStorage.getItem(PANEL_STORAGE_KEY) || '{}'); } catch (_) {}
    document.querySelectorAll('[data-panel-toggle]').forEach((input) => {
      const id = input.dataset.panelToggle;
      input.checked = stored[id] !== false;
      applyPanel(id, input.checked);
      input.addEventListener('change', () => {
        applyPanel(id, input.checked);
        const next = {};
        document.querySelectorAll('[data-panel-toggle]').forEach((toggle) => {
          next[toggle.dataset.panelToggle] = toggle.checked;
        });
        localStorage.setItem(PANEL_STORAGE_KEY, JSON.stringify(next));
      });
    });
  }

  function applyPanel(id, enabled) {
    document.querySelectorAll(`[data-panel-id="${id}"]`).forEach((panel) => {
      panel.hidden = !enabled;
    });
  }

  async function api(path, options = {}) {
    const response = await fetch(path, { cache: 'no-store', ...options });
    if (!response.ok) {
      let message = `HTTP ${response.status}`;
      try {
        const payload = await response.json();
        message = payload.message || payload.error || message;
      } catch (_) {}
      throw new Error(message);
    }
    return response.json();
  }

  async function refresh() {
    if (state.refreshing) return;
    state.refreshing = true;
    try {
      state.snapshot = await api('/api/v1/runtime/snapshot', { headers: authHeaders() });
      render(state.snapshot);
      clearError();
      setConnection(true, 'Live data');
    } catch (error) {
      showError(error.message || String(error));
      setConnection(false, 'Unavailable');
    } finally {
      state.refreshing = false;
    }
  }

  function render(snapshot) {
    const totals = snapshot.totals;
    $('stat-agents').textContent = totals.agents;
    $('stat-cpu').textContent = percent(totals.cpu_percent);
    $('stat-rss').textContent = bytes(totals.rss_bytes);
    $('stat-hooks').textContent = totals.hook_backed_agents;
    $('stat-confidence').textContent = `${totals.confidence_reported_agents}/${totals.agents}`;
    $('stat-commands').textContent = totals.pending_commands;
    $('last-updated').textContent = `Updated ${time(snapshot.generated_at)}`;
    renderCollection(snapshot.collection);
    renderAgents(snapshot.agents);
    renderResources(snapshot.agents);
    renderActivity(snapshot.agents);
    renderConfidence(snapshot.agents);
    renderTokens(snapshot.agents);
    renderControls(snapshot.agents, snapshot.recent_commands);
    renderHooks(snapshot.recent_hooks);
    renderProcesses(snapshot.processes);
  }

  function renderCollection(collection) {
    $('collection-state').textContent = collection.enabled ? 'collection enabled' : 'collection paused';
    $('collection-toggle').textContent = collection.enabled ? 'Pause collection' : 'Resume collection';
    $('collection-source').textContent = `${collection.proc_root} · every ${collection.sample_interval_ms} ms`;
    $('collection-patterns').textContent = collection.process_patterns.join(', ');
    $('collection-sample').textContent = time(collection.last_sample_at);
    $('collection-host').textContent = `${collection.cpu_count || 0} CPU(s) · ${collection.memory_total_bytes ? bytes(collection.memory_total_bytes) : 'memory unavailable'}`;
    const error = $('collection-error');
    if (collection.last_error) {
      error.textContent = `${collection.last_error} (${collection.collection_errors} errors)`;
      error.hidden = false;
    } else {
      error.hidden = true;
    }
  }

  function renderAgents(agents) {
    const body = $('agent-rows');
    if (!agents.length) {
      body.innerHTML = '<tr><td colspan="9" class="empty">No matching host agent process or runtime hook has been observed.</td></tr>';
      return;
    }
    body.innerHTML = agents.map((agent) => `<tr>
      <td><strong>${esc(agent.agent_id)}</strong><small>${esc(agent.instance_id || 'no instance id')}</small></td>
      <td>${esc(agent.provider)}<small>${esc(agent.model)}</small></td>
      <td>${agent.pid == null ? 'hook only' : esc(agent.pid)}</td>
      <td><span class="status">${esc(agent.status)}</span></td>
      <td>${percent(agent.cpu_percent)}</td>
      <td>${agent.rss_bytes == null ? 'unavailable' : bytes(agent.rss_bytes)}</td>
      <td><span class="confidence ${agent.reported_confidence == null ? 'muted' : ''}">${confidence(agent.reported_confidence)}</span></td>
      <td>${agent.hook_backed ? 'hook' : 'process'}${agent.process_backed && agent.hook_backed ? ' + process' : ''}</td>
      <td>${time(agent.last_hook_at || agent.last_process_sample_at)}</td>
    </tr>`).join('');
  }

  function renderResources(agents) {
    const target = $('resource-cards');
    if (!agents.length) {
      target.innerHTML = '<p class="empty">No resource samples yet.</p>';
      return;
    }
    target.innerHTML = agents.map((agent) => {
      const cpu = Math.max(0, Math.min(100, Number(agent.cpu_percent || 0)));
      const memory = Math.max(0, Math.min(100, Number(agent.memory_percent || 0)));
      return `<article class="metric-card">
        <div class="metric-title"><strong>${esc(agent.agent_id)}</strong><span>${esc(agent.provider)}</span></div>
        <label>CPU <span>${percent(agent.cpu_percent)}</span></label>
        <div class="bar"><span style="width:${cpu}%"></span></div>
        <label>Host memory <span>${agent.memory_percent == null ? 'unavailable' : percent(agent.memory_percent)}</span></label>
        <div class="bar memory"><span style="width:${memory}%"></span></div>
        <small>${agent.rss_bytes == null ? 'RSS unavailable' : `${bytes(agent.rss_bytes)} RSS`}</small>
      </article>`;
    }).join('');
  }

  function renderActivity(agents) {
    const target = $('activity-cards');
    const active = agents.filter((agent) => agent.hook_backed);
    if (!active.length) {
      target.innerHTML = '<p class="empty">Process discovery cannot reveal semantic activity. Install a runtime hook to populate this panel.</p>';
      return;
    }
    target.innerHTML = active.map((agent) => `<article class="activity-card">
      <div><strong>${esc(agent.agent_id)}</strong><span class="status">${esc(agent.status)}</span></div>
      <p>${esc(agent.current_activity || 'No visible activity summary reported.')}</p>
      <small>${agent.current_tool ? `Tool: ${esc(agent.current_tool)}` : 'No active tool reported'} · session ${esc(agent.session_id || 'unreported')}</small>
    </article>`).join('');
  }

  function renderConfidence(agents) {
    const target = $('confidence-cards');
    if (!agents.length) {
      target.innerHTML = '<p class="empty">No agents observed.</p>';
      return;
    }
    target.innerHTML = agents.map((agent) => {
      const value = agent.reported_confidence;
      const width = value == null ? 0 : Math.round(Number(value) * 100);
      return `<article class="confidence-card">
        <div><strong>${esc(agent.agent_id)}</strong><span>${confidence(value)}</span></div>
        <div class="bar confidence-bar"><span style="width:${width}%"></span></div>
        <small>${value == null ? 'No hook has reported confidence; no estimate was fabricated.' : `Source: ${esc(agent.confidence_source)}`}</small>
      </article>`;
    }).join('');
  }

  function renderTokens(agents) {
    const target = $('token-rows');
    const hooked = agents.filter((agent) => agent.hook_backed);
    if (!hooked.length) {
      target.innerHTML = '<tr><td colspan="4" class="empty">Token usage requires provider or wrapper hooks.</td></tr>';
      return;
    }
    target.innerHTML = hooked.map((agent) => `<tr>
      <td>${esc(agent.agent_id)}</td><td>${number(agent.input_tokens, 0)}</td><td>${number(agent.output_tokens, 0)}</td><td>${number(agent.input_tokens + agent.output_tokens, 0)}</td>
    </tr>`).join('');
  }

  function renderControls(agents, commands) {
    const target = $('control-cards');
    if (!agents.length) {
      target.innerHTML = '<p class="empty">No agents available for control.</p>';
    } else {
      target.innerHTML = agents.map((agent) => `<article class="control-card">
        <div><strong>${esc(agent.agent_id)}</strong><small>${agent.hook_backed ? 'Hook-aware control channel' : 'Observe-only process'}</small></div>
        <div class="button-row">
          <button data-command="pause" data-agent="${esc(agent.agent_id)}" ${agent.hook_backed ? '' : 'disabled'}>Pause</button>
          <button data-command="resume" data-agent="${esc(agent.agent_id)}" ${agent.hook_backed ? '' : 'disabled'}>Resume</button>
          <button class="danger" data-command="stop" data-agent="${esc(agent.agent_id)}" ${agent.hook_backed ? '' : 'disabled'}>Stop</button>
        </div>
      </article>`).join('');
    }
    const recent = $('command-rows');
    if (!commands.length) {
      recent.innerHTML = '<tr><td colspan="5" class="empty">No control commands queued.</td></tr>';
      return;
    }
    recent.innerHTML = commands.slice(0, 50).map((command) => `<tr>
      <td>${esc(command.agent_id)}</td><td>${esc(command.action)}</td><td>${esc(command.status)}</td><td>${time(command.created_at)}</td><td>${esc(command.message || '')}</td>
    </tr>`).join('');
  }

  function renderHooks(hooks) {
    const target = $('hook-rows');
    if (!hooks.length) {
      target.innerHTML = '<tr><td colspan="7" class="empty">No runtime hooks received.</td></tr>';
      return;
    }
    target.innerHTML = hooks.slice(0, 100).map((hook) => `<tr>
      <td>${time(hook.occurred_at)}</td><td>${esc(hook.agent.agent_id)}</td><td>${esc(hook.kind)}</td><td>${esc(hook.tool_name || '')}</td><td>${confidence(hook.confidence)}</td><td>${number(hook.input_tokens_delta + hook.output_tokens_delta, 0)}</td><td>${esc(hook.summary || '')}</td>
    </tr>`).join('');
  }

  function renderProcesses(processes) {
    const target = $('process-rows');
    if (!processes.length) {
      target.innerHTML = '<tr><td colspan="7" class="empty">No configured agent process pattern matched.</td></tr>';
      return;
    }
    target.innerHTML = processes.map((process) => `<tr>
      <td>${process.pid}</td><td>${esc(process.process_name)}</td><td>${esc(process.provider)}</td><td>${esc(process.matched_pattern)}</td><td>${esc(process.process_state)}</td><td>${percent(process.cpu_percent)}</td><td>${bytes(process.rss_bytes)}</td>
    </tr>`).join('');
  }

  async function setCollection() {
    const enabled = !(state.snapshot && state.snapshot.collection.enabled);
    try {
      await api('/api/v1/runtime/collection', {
        method: 'POST', headers: jsonHeaders(), body: JSON.stringify({ enabled })
      });
      await refresh();
    } catch (error) { showError(error.message || String(error)); }
  }

  async function queueCommand(agentId, action) {
    if (action === 'stop' && !window.confirm(`Queue a stop command for ${agentId}?`)) return;
    try {
      await api('/api/v1/runtime/commands', {
        method: 'POST', headers: jsonHeaders(), body: JSON.stringify({ agent_id: agentId, action })
      });
      await refresh();
    } catch (error) { showError(error.message || String(error)); }
  }

  $('save-token').addEventListener('click', () => {
    const value = $('auth-token').value.trim();
    if (value) sessionStorage.setItem(TOKEN_STORAGE_KEY, value);
    else sessionStorage.removeItem(TOKEN_STORAGE_KEY);
    refresh();
  });
  $('refresh').addEventListener('click', refresh);
  $('collection-toggle').addEventListener('click', setCollection);
  $('control-cards').addEventListener('click', (event) => {
    const button = event.target.closest('[data-command]');
    if (button && !button.disabled) queueCommand(button.dataset.agent, button.dataset.command);
  });
  $('auth-token').value = token();
  loadPanels();
  refresh();
  state.refreshTimer = window.setInterval(refresh, REFRESH_INTERVAL_MS);
  window.addEventListener('beforeunload', () => window.clearInterval(state.refreshTimer));
})();
