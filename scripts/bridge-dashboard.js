(() => {
  const TOKEN_KEY = 'meta-agent-read-token';
  const state = { snapshot: null, runtime: null, socket: null, authenticated: false, joined: false, replyTo: null, refreshTimer: null };
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value ?? '').replace(/[&<>'"]/g, (char) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;'
  }[char]));
  const token = () => sessionStorage.getItem(TOKEN_KEY) || '';
  const authHeaders = () => token() ? { Authorization: `Bearer ${token()}` } : {};
  const jsonHeaders = () => ({ ...authHeaders(), 'Content-Type': 'application/json' });
  const roomSlug = () => $('room-slug').value.trim();
  const participant = () => ({
    participant_id: $('participant-id').value.trim(),
    display_name: $('participant-name').value.trim(),
    kind: 'human'
  });
  const time = (value) => value ? new Date(value).toLocaleTimeString() : 'never';
  const bytes = (value) => {
    let amount = Number(value || 0); const units = ['B', 'KiB', 'MiB', 'GiB']; let unit = 0;
    while (amount >= 1024 && unit < units.length - 1) { amount /= 1024; unit += 1; }
    return `${amount.toLocaleString(undefined, { maximumFractionDigits: amount < 10 ? 1 : 0 })} ${units[unit]}`;
  };
  const percent = (value) => value == null ? 'unreported' : `${Number(value).toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;

  function setConnection(ok, label) {
    $('connection').className = `connection ${ok ? 'online' : 'offline'}`;
    $('connection-label').textContent = label;
  }
  function showError(message) { $('error-banner').textContent = message; $('error-banner').hidden = false; }
  function clearError() { $('error-banner').hidden = true; }
  async function api(path, options = {}) {
    const response = await fetch(path, { cache: 'no-store', ...options });
    let payload = null; try { payload = await response.json(); } catch (_) {}
    if (!response.ok) throw new Error(payload?.message || `HTTP ${response.status}`);
    return payload;
  }

  async function createAndJoin() {
    const slug = roomSlug();
    if (!slug || !participant().participant_id || !participant().display_name) {
      showError('Room slug, participant ID, and display name are required.'); return;
    }
    try {
      await api('/api/v1/bridge/rooms', {
        method: 'POST', headers: jsonHeaders(), body: JSON.stringify({
          slug, title: $('room-title').value.trim(), objective: $('room-objective').value.trim()
        })
      });
      await api(`/api/v1/bridge/rooms/${encodeURIComponent(slug)}/join`, {
        method: 'POST', headers: jsonHeaders(), body: JSON.stringify({ participant: participant() })
      });
      clearError();
      connect();
      await refresh();
    } catch (error) { showError(error.message || String(error)); }
  }

  async function refresh() {
    const slug = roomSlug();
    if (!slug || !token()) { setConnection(false, token() ? 'Choose a room' : 'Token required'); return; }
    try {
      const [snapshot, runtime] = await Promise.all([
        api(`/api/v1/bridge/rooms/${encodeURIComponent(slug)}`, { headers: authHeaders() }),
        api('/api/v1/runtime/snapshot', { headers: authHeaders() })
      ]);
      state.snapshot = snapshot; state.runtime = runtime;
      render(snapshot, runtime); clearError();
      if (!state.socket || state.socket.readyState !== WebSocket.OPEN) setConnection(true, 'HTTP live');
    } catch (error) {
      if (!String(error.message).includes('not found')) showError(error.message || String(error));
      else showError('Room has not been created yet. Apply the token, then create/join the room.');
    }
  }

  function render(snapshot, runtime) {
    $('stat-members').textContent = snapshot.members.length;
    $('stat-agents').textContent = snapshot.messages.filter((message) => ['openai', 'anthropic'].includes(message.author.provider)).length;
    $('stat-connected').textContent = snapshot.members.filter((member) => member.websocket_connected).length;
    $('stat-messages').textContent = snapshot.messages.length;
    $('stat-contacts').textContent = snapshot.contacts.length;
    $('stat-revision').textContent = snapshot.revision;
    $('last-updated').textContent = `Updated ${time(snapshot.generated_at)}`;
    renderMembers(snapshot.members, runtime.agents || []); renderTransports(snapshot.counters); renderMessages(snapshot.messages);
    renderContacts(snapshot.contacts); renderProcesses(runtime.processes || []);
  }

  function renderMembers(members, runtimeAgents) {
    const target = $('member-cards');
    if (!members.length) { target.innerHTML = '<p class="empty">No participants have joined.</p>'; return; }
    const runtimeById = new Map(runtimeAgents.map((agent) => [agent.agent_id, agent]));
    target.innerHTML = members.map((member) => { const runtime = runtimeById.get(member.runtime_agent_id); return `<article class="member-card">
      <div><strong>${esc(member.display_name)}</strong><span class="pill ${esc(member.kind)}">${esc(member.kind)}</span></div>
      <p>${member.provider ? `${esc(member.provider)} · ${esc(member.model || 'model unreported')}` : 'Human participant'}</p>
      <small>${member.websocket_connected ? '<span class="pill online">WebSocket connected</span>' : `Last seen ${time(member.last_seen_at)}`} ${runtime ? ` · runtime ${esc(runtime.status)} at ${time(runtime.last_hook_at)}` : member.runtime_agent_id ? ' · runtime unreported' : ''}</small>
    </article>`; }).join('');
  }

  function renderTransports(counters) {
    const target = $('transport-cards'); const values = counters.accepted_by_transport || {};
    target.innerHTML = ['http', 'websocket', 'tcp'].map((name) => `<article class="transport-card"><strong>${Number(values[name] || 0)}</strong><span>${esc(name)}</span></article>`).join('');
  }

  function renderMessages(messages) {
    const target = $('message-stream');
    if (!messages.length) { target.innerHTML = '<p class="empty">No messages yet.</p>'; return; }
    target.innerHTML = messages.map((message) => `<article class="message-card" data-message-id="${esc(message.message_id)}">
      <div class="message-head"><div><strong>${esc(message.author.display_name)}</strong> <span class="pill ${esc(message.author.kind)}">${esc(message.author.provider || message.author.kind)}</span></div><time>${time(message.received_at)}</time></div>
      <p>${esc(message.summary)}</p>
      <small>#${message.sequence} · ${esc(message.transport)}${message.reply_to ? ` · reply to ${esc(message.reply_to.slice(0, 8))}` : ''}</small>
      <button class="secondary" data-reply-id="${esc(message.message_id)}" data-reply-name="${esc(message.author.display_name)}" type="button">Reply</button>
    </article>`).join('');
  }

  function renderContacts(contacts) {
    const target = $('contact-stream');
    if (!contacts.length) { target.innerHTML = '<p class="empty">No cross-participant contact yet.</p>'; return; }
    target.innerHTML = contacts.map((contact) => `<article class="contact-card">
      <div class="contact-head"><span class="contact-edge">${esc(contact.participants.join(' ↔ '))}</span><time>${time(contact.received_at)}</time></div>
      <p>${esc(contact.summary)}</p><small>${esc(contact.transport)} · ${esc(contact.contact_id.slice(0, 8))}</small>
    </article>`).join('');
  }

  function renderProcesses(processes) {
    const target = $('process-rows');
    if (!processes.length) { target.innerHTML = '<tr><td colspan="9" class="empty">No real host observation has reached this server.</td></tr>'; return; }
    target.innerHTML = processes.map((process) => `<tr>
      <td>${esc(process.pid)}</td><td>${esc(process.ppid ?? '—')}</td><td>${esc(process.process_name)}</td><td>${esc(process.provider)}</td><td>${esc(process.process_role || 'agent process')}</td><td>${percent(process.cpu_percent)}</td><td>${bytes(process.rss_bytes)}</td><td>${esc(process.source || 'linux_proc')}</td><td>${time(process.observed_at)}${process.stale ? ' · stale' : ''}</td>
    </tr>`).join('');
  }

  function connect() {
    if (!token() || !roomSlug()) return;
    if (state.socket) { state.socket.onclose = null; state.socket.close(); }
    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${scheme}//${location.host}/ws/bridge/${encodeURIComponent(roomSlug())}`);
    state.socket = socket; state.authenticated = false; state.joined = false; setConnection(false, 'Connecting');
    socket.onopen = () => socket.send(JSON.stringify({ type: 'auth', token: token() }));
    socket.onmessage = (event) => {
      let frame; try { frame = JSON.parse(event.data); } catch (_) { return; }
      if (frame.type === 'authenticated') {
        state.authenticated = true;
        socket.send(JSON.stringify({ type: 'join', participant: participant() }));
        setConnection(true, 'Authenticated'); return;
      }
      if (frame.type === 'joined') { state.joined = true; setConnection(true, 'Room live'); refresh(); return; }
      if (frame.type === 'update' || frame.type === 'message_accepted' || frame.type === 'snapshot') { refresh(); return; }
      if (frame.type === 'error') showError(frame.message || frame.error);
    };
    socket.onerror = () => setConnection(false, 'Connection error');
    socket.onclose = () => { if (state.socket === socket) setConnection(false, 'Disconnected'); };
  }

  async function sendMessage() {
    const summary = $('message-summary').value.trim(); if (!summary) return;
    const message = {
      protocol_version: 'v1', message_id: crypto.randomUUID(), occurred_at: new Date().toISOString(),
      author: participant(), summary, ...(state.replyTo ? { reply_to: state.replyTo } : {})
    };
    try {
      if (state.socket?.readyState === WebSocket.OPEN && state.authenticated && state.joined) {
        state.socket.send(JSON.stringify({ type: 'message', message }));
      } else {
        await api(`/api/v1/bridge/rooms/${encodeURIComponent(roomSlug())}/messages`, {
          method: 'POST', headers: jsonHeaders(), body: JSON.stringify(message)
        });
      }
      $('message-summary').value = ''; clearReply(); await refresh();
    } catch (error) { showError(error.message || String(error)); }
  }

  function selectReply(messageId, name) {
    state.replyTo = messageId; $('reply-label').textContent = `Replying to ${name}`; $('clear-reply').hidden = false;
    document.querySelectorAll('.message-card').forEach((card) => card.classList.toggle('reply-target', card.dataset.messageId === messageId));
    $('message-summary').focus();
  }
  function clearReply() {
    state.replyTo = null; $('reply-label').textContent = 'Post to the undirected room'; $('clear-reply').hidden = true;
    document.querySelectorAll('.message-card').forEach((card) => card.classList.remove('reply-target'));
  }

  $('auth-token').value = token();
  $('save-token').addEventListener('click', () => { const value = $('auth-token').value.trim(); if (value) sessionStorage.setItem(TOKEN_KEY, value); else sessionStorage.removeItem(TOKEN_KEY); connect(); refresh(); });
  $('join-room').addEventListener('click', createAndJoin); $('refresh').addEventListener('click', refresh); $('send-message').addEventListener('click', sendMessage); $('clear-reply').addEventListener('click', clearReply);
  $('message-stream').addEventListener('click', (event) => { const button = event.target.closest('[data-reply-id]'); if (button) selectReply(button.dataset.replyId, button.dataset.replyName); });
  $('message-summary').addEventListener('keydown', (event) => { if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') sendMessage(); });
  if (token()) refresh();
  state.refreshTimer = window.setInterval(refresh, 3000);
  window.addEventListener('beforeunload', () => { window.clearInterval(state.refreshTimer); if (state.socket) state.socket.close(); });
})();
