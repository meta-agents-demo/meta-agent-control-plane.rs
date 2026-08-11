use leptos::prelude::*;

pub fn dashboard(reads_protected: bool) -> String {
    let protection = if reads_protected {
        "Read API protected"
    } else {
        "Read API open"
    };
    let body = view! {
        <div class="app-shell">
            <aside class="sidebar">
                <div>
                    <p class="eyebrow">"META-AGENT"</p>
                    <h2>"Runtime panels"</h2>
                    <p class="sidebar-copy">"Choose which real-data panels are visible. The selection stays in this browser."</p>
                </div>
                <nav aria-label="Analytics views">
                    <a href="/">"Overview"</a>
                    <a href="/explorer">"Explorer"</a>
                    <a href="/metacognition">"Metacognition"</a>
                    <a href="/coordination">"Coordination"</a>
                </nav>
                <fieldset class="panel-toggle">
                    <legend>"Visible panels"</legend>
                    <label><input type="checkbox" data-panel-toggle="summary" checked />"Summary"</label>
                    <label><input type="checkbox" data-panel-toggle="collector" checked />"Collector"</label>
                    <label><input type="checkbox" data-panel-toggle="agents" checked />"Agent inventory"</label>
                    <label><input type="checkbox" data-panel-toggle="resources" checked />"CPU and memory"</label>
                    <label><input type="checkbox" data-panel-toggle="activity" checked />"Live activity"</label>
                    <label><input type="checkbox" data-panel-toggle="confidence" checked />"Confidence"</label>
                    <label><input type="checkbox" data-panel-toggle="tokens" checked />"Token usage"</label>
                    <label><input type="checkbox" data-panel-toggle="controls" checked />"Controls"</label>
                    <label><input type="checkbox" data-panel-toggle="hooks" checked />"Hook events"</label>
                    <label><input type="checkbox" data-panel-toggle="processes" checked />"Host processes"</label>
                </fieldset>
                <p class="privacy">"No raw prompts, responses, browser profiles, provider keys, or hidden reasoning are displayed."</p>
            </aside>

            <main>
                <header class="hero">
                    <div>
                        <p class="eyebrow">"REAL HOST + HOOK TELEMETRY"</p>
                        <h1>"Live agent runtime"</h1>
                        <p class="lede">"CPU and RSS from host process counters, plus explicit activity, confidence, token usage, and cooperative controls from Gemini, ChatGPT/OpenAI, and Claude agent hooks."</p>
                    </div>
                    <div id="connection" class="connection offline"><span></span><div><strong id="connection-label">"Connecting"</strong><small>{protection}</small></div></div>
                </header>

                <section class="toolbar panel">
                    <label for="auth-token">"Read/control token"</label>
                    <input id="auth-token" type="password" autocomplete="off" placeholder="Required when protection is enabled" />
                    <button id="save-token" type="button">"Apply token"</button>
                    <button id="refresh" class="secondary" type="button">"Refresh now"</button>
                    <span id="last-updated">"No runtime snapshot yet"</span>
                </section>
                <section id="error-banner" class="error" hidden></section>

                <section class="stats" data-panel-id="summary">
                    <article class="panel"><span>"Agents"</span><strong id="stat-agents">"0"</strong></article>
                    <article class="panel"><span>"Observed CPU"</span><strong id="stat-cpu">"0%"</strong></article>
                    <article class="panel"><span>"Observed RSS"</span><strong id="stat-rss">"0 B"</strong></article>
                    <article class="panel"><span>"Hook-backed"</span><strong id="stat-hooks">"0"</strong></article>
                    <article class="panel"><span>"Confidence reported"</span><strong id="stat-confidence">"0/0"</strong></article>
                    <article class="panel"><span>"Pending controls"</span><strong id="stat-commands">"0"</strong></article>
                </section>

                <section class="panel padded" data-panel-id="collector">
                    <div class="title"><div><h2>"Host collector"</h2><span id="collection-state">"Waiting for collector state"</span></div><button id="collection-toggle" type="button">"Pause collection"</button></div>
                    <div class="collector-grid">
                        <dl><dt>"Process source"</dt><dd id="collection-source">"unknown"</dd></dl>
                        <dl><dt>"Match patterns"</dt><dd id="collection-patterns">"unknown"</dd></dl>
                        <dl><dt>"Latest sample"</dt><dd id="collection-sample">"never"</dd></dl>
                        <dl><dt>"Host capacity"</dt><dd id="collection-host">"unknown"</dd></dl>
                    </div>
                    <p id="collection-error" class="warning" hidden></p>
                    <p class="note">"Linux containers can read a read-only host /proc mount. Docker Desktop on macOS and Windows cannot expose native host processes this way, so those hosts use the same explicit runtime-hook endpoint while the server remains containerized."</p>
                </section>

                <section class="panel padded" data-panel-id="agents">
                    <div class="title"><div><h2>"Agent inventory"</h2><span>"Merged by hook-declared PID when available"</span></div></div>
                    <div class="table-wrap"><table><thead><tr><th>"Agent"</th><th>"Provider / model"</th><th>"PID"</th><th>"Status"</th><th>"CPU"</th><th>"RSS"</th><th>"Confidence"</th><th>"Sources"</th><th>"Last seen"</th></tr></thead><tbody id="agent-rows"><tr><td colspan="9" class="empty">"Loading real agent data…"</td></tr></tbody></table></div>
                </section>

                <section class="panel padded" data-panel-id="resources">
                    <div class="title"><div><h2>"CPU and memory"</h2><span>"Delta CPU from /proc counters; RSS from process status"</span></div></div>
                    <div id="resource-cards" class="metric-grid"><p class="empty">"Loading resource samples…"</p></div>
                </section>

                <section class="two-column">
                    <article class="panel padded" data-panel-id="activity">
                        <div class="title"><div><h2>"Live activity"</h2><span>"Only explicit hook summaries"</span></div></div>
                        <div id="activity-cards" class="card-list"><p class="empty">"Loading hook activity…"</p></div>
                    </article>
                    <article class="panel padded" data-panel-id="confidence">
                        <div class="title"><div><h2>"Reported confidence"</h2><span>"Never inferred from process behavior"</span></div></div>
                        <div id="confidence-cards" class="card-list"><p class="empty">"Loading confidence…"</p></div>
                    </article>
                </section>

                <section class="panel padded" data-panel-id="tokens">
                    <div class="title"><div><h2>"Token usage"</h2><span>"Hook-reported deltas; no pricing assumptions"</span></div></div>
                    <div class="table-wrap"><table><thead><tr><th>"Agent"</th><th>"Input"</th><th>"Output"</th><th>"Total"</th></tr></thead><tbody id="token-rows"><tr><td colspan="4" class="empty">"Loading token counters…"</td></tr></tbody></table></div>
                </section>

                <section class="panel padded" data-panel-id="controls">
                    <div class="title"><div><h2>"Cooperative agent controls"</h2><span>"Pause, resume, and stop commands are queued for hook-aware agents; no arbitrary host signals"</span></div></div>
                    <div id="control-cards" class="control-grid"><p class="empty">"Loading controls…"</p></div>
                    <div class="table-wrap compact"><table><thead><tr><th>"Agent"</th><th>"Action"</th><th>"Status"</th><th>"Created"</th><th>"Agent message"</th></tr></thead><tbody id="command-rows"><tr><td colspan="5" class="empty">"No commands queued."</td></tr></tbody></table></div>
                </section>

                <section class="panel padded" data-panel-id="hooks">
                    <div class="title"><div><h2>"Runtime hook events"</h2><span>"Bounded, newest first"</span></div></div>
                    <div class="table-wrap"><table><thead><tr><th>"Time"</th><th>"Agent"</th><th>"Kind"</th><th>"Tool"</th><th>"Confidence"</th><th>"Tokens"</th><th>"Visible summary"</th></tr></thead><tbody id="hook-rows"><tr><td colspan="7" class="empty">"No runtime hooks received."</td></tr></tbody></table></div>
                </section>

                <section class="panel padded" data-panel-id="processes">
                    <div class="title"><div><h2>"Matched host processes"</h2><span>"Command arguments are used only for matching and are not returned"</span></div></div>
                    <div class="table-wrap"><table><thead><tr><th>"PID"</th><th>"Process"</th><th>"Provider"</th><th>"Pattern"</th><th>"Kernel state"</th><th>"CPU"</th><th>"RSS"</th></tr></thead><tbody id="process-rows"><tr><td colspan="7" class="empty">"Loading process samples…"</td></tr></tbody></table></div>
                </section>
            </main>
        </div>
    }
    .to_html();

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>Live Agent Runtime</title><style>{CSS}</style></head><body>{body}<script>{SCRIPT}</script></body></html>"
    )
}

const SCRIPT: &str = include_str!("../scripts/runtime-dashboard.js");

const CSS: &str = r#"
:root{font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:#edf5ff;background:#071019;--panel:#101c28;--panel2:#0a1621;--line:#2b3f53;--muted:#91a7bb;--accent:#72e0bd;--blue:#74b9ff;--warning:#ffc66f;--danger:#ff7c91}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 30% -10%,#173c59 0,transparent 34%),#071019}.app-shell{display:grid;grid-template-columns:270px minmax(0,1fr);min-height:100vh}.sidebar{position:sticky;top:0;height:100vh;padding:26px 20px;border-right:1px solid var(--line);background:rgba(7,16,25,.97);display:flex;flex-direction:column;gap:20px;overflow:auto}.sidebar h2{margin:.2rem 0}.sidebar-copy,.privacy,.lede,.title span,.note,small{color:var(--muted)}.eyebrow{margin:0;color:var(--accent);font-size:.7rem;font-weight:900;letter-spacing:.17em}.sidebar nav{display:grid;gap:7px}.sidebar nav a{color:#dbe9f7;text-decoration:none;border:1px solid var(--line);border-radius:9px;padding:9px 11px}.panel-toggle{border:1px solid var(--line);border-radius:12px;padding:12px;display:grid;gap:9px}.panel-toggle legend{color:var(--muted);padding:0 5px}.panel-toggle label{display:flex;align-items:center;gap:9px;font-size:.84rem}.panel-toggle input{accent-color:var(--accent)}.privacy{font-size:.76rem;line-height:1.5;margin-top:auto}main{width:min(1560px,calc(100% - 40px));margin:0 auto;padding:30px 0 60px}.hero{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}.hero h1{font-size:clamp(2.3rem,5vw,5.2rem);letter-spacing:-.055em;margin:.2rem 0}.lede{max-width:900px;line-height:1.6}.connection{display:flex;gap:10px;align-items:center;border:1px solid var(--line);background:var(--panel2);padding:13px 15px;border-radius:12px;min-width:180px}.connection>span{width:9px;height:9px;border-radius:50%;background:var(--danger)}.connection.online>span{background:var(--accent);box-shadow:0 0 10px rgba(114,224,189,.6)}.connection strong,.connection small{display:block}.panel{background:linear-gradient(155deg,rgba(19,34,48,.97),rgba(9,21,32,.98));border:1px solid var(--line);border-radius:16px}.padded{padding:19px;margin-top:16px}.toolbar{display:grid;grid-template-columns:auto minmax(220px,1fr) auto auto minmax(180px,auto);gap:10px;align-items:center;padding:13px 15px;margin-top:16px}.toolbar label,.toolbar span{color:var(--muted);font-size:.8rem}.toolbar input{background:#07121c;border:1px solid var(--line);border-radius:9px;color:#edf5ff;padding:10px}.toolbar button,button{border:0;border-radius:9px;padding:10px 13px;background:var(--accent);color:#06130f;font-weight:850;cursor:pointer}.toolbar .secondary,.secondary{background:#21374b;color:#edf5ff}button:disabled{cursor:not-allowed;opacity:.42}.danger{background:#572632;color:#ffdbe1}.error,.warning{border:1px solid var(--danger);background:#3b1720;color:#ffd7de;border-radius:11px;padding:11px 14px;margin-top:12px}.warning{border-color:#8c672c;background:#332814;color:#ffe5b9}.stats{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:11px;margin-top:16px}.stats article{padding:15px}.stats span{display:block;color:var(--muted);font-size:.69rem;text-transform:uppercase;letter-spacing:.07em}.stats strong{display:block;font-size:1.65rem;margin-top:7px}.title{display:flex;justify-content:space-between;gap:14px;align-items:center}.title h2{font-size:1.05rem;margin:.2rem 0}.collector-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin-top:13px}.collector-grid dl{background:var(--panel2);border:1px solid var(--line);border-radius:10px;padding:11px;margin:0}.collector-grid dt{color:var(--muted);font-size:.7rem;text-transform:uppercase}.collector-grid dd{margin:6px 0 0;overflow-wrap:anywhere;font-size:.82rem}.note{font-size:.78rem;line-height:1.5;margin-bottom:0}.table-wrap{overflow:auto;margin-top:12px}.table-wrap.compact{max-height:330px}table{width:100%;border-collapse:collapse;min-width:960px}th,td{text-align:left;padding:10px 9px;border-bottom:1px solid var(--line);font-size:.76rem;vertical-align:top}th{color:var(--muted);font-size:.65rem;text-transform:uppercase;letter-spacing:.06em}td small{display:block;margin-top:4px}.status{display:inline-block;border:1px solid var(--line);border-radius:999px;padding:3px 8px;color:var(--blue);font-size:.7rem}.confidence{color:var(--accent);font-weight:800}.muted,.empty{color:var(--muted)}.metric-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:10px;margin-top:13px}.metric-card,.activity-card,.confidence-card,.control-card{background:var(--panel2);border:1px solid var(--line);border-radius:11px;padding:13px}.metric-title,.activity-card>div,.confidence-card>div,.control-card>div:first-child{display:flex;justify-content:space-between;gap:10px}.metric-title span{color:var(--muted);font-size:.75rem}.metric-card label{display:flex;justify-content:space-between;color:var(--muted);font-size:.72rem;margin:12px 0 5px}.bar{height:7px;border-radius:999px;background:#06101a;overflow:hidden}.bar span{display:block;height:100%;background:linear-gradient(90deg,var(--blue),var(--accent));border-radius:999px}.bar.memory span{background:linear-gradient(90deg,#b49cff,#74b9ff)}.confidence-bar{margin:10px 0}.card-list{display:grid;gap:9px;margin-top:13px}.activity-card p{font-size:.82rem;line-height:1.45}.two-column{display:grid;grid-template-columns:1fr 1fr;gap:16px}.control-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:10px;margin-top:13px}.control-card small{display:block}.button-row{display:flex;gap:8px;margin-top:12px}.button-row button{flex:1}.empty{padding:14px 3px}.privacy{overflow-wrap:anywhere}[hidden]{display:none!important}@media(max-width:1200px){.app-shell{grid-template-columns:220px minmax(0,1fr)}.stats{grid-template-columns:repeat(3,1fr)}.collector-grid{grid-template-columns:repeat(2,1fr)}.two-column{grid-template-columns:1fr}.toolbar{grid-template-columns:1fr 1fr}.toolbar label,.toolbar input,.toolbar span{grid-column:1/-1}}@media(max-width:760px){.app-shell{display:block}.sidebar{position:static;height:auto;border-right:0;border-bottom:1px solid var(--line)}.sidebar nav{grid-template-columns:1fr 1fr}.panel-toggle{grid-template-columns:1fr 1fr}.privacy{margin-top:0}main{width:calc(100% - 20px);padding-top:22px}.hero{display:block}.connection{margin-top:14px;width:max-content}.stats{grid-template-columns:repeat(2,1fr)}.collector-grid{grid-template-columns:1fr}.toolbar{display:flex;flex-direction:column;align-items:stretch}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_panel_sidebar_and_real_runtime_contract() {
        let html = dashboard(true);
        assert!(html.contains("Live agent runtime"));
        assert!(html.contains("panel-toggle"));
        assert!(html.contains("data-panel-toggle=\"resources\""));
        assert!(html.contains("/api/v1/runtime/snapshot"));
        assert!(html.contains("No raw prompts"));
        assert!(html.contains("href=\"/explorer\""));
        assert!(html.contains("href=\"/metacognition\""));
        assert!(html.contains("href=\"/coordination\""));
        assert!(!html.contains("Math.random"));
        assert!(!html.contains("meta-agent-read-token="));
    }
}
