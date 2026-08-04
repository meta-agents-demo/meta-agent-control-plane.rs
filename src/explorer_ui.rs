use leptos::prelude::*;

pub fn dashboard(reads_protected: bool) -> String {
    let protection = if reads_protected {
        "Read API protected"
    } else {
        "Read API open"
    };
    let body = view! {
        <main class="shell">
            <header>
                <div>
                    <p class="eyebrow">"META-AGENT CONTROL PLANE"</p>
                    <h1>"Operator explorer"</h1>
                    <p class="lede">"Agents, retained sessions, event timeline, learned lessons, and bounded memory pressure from one coherent snapshot."</p>
                </div>
                <nav><a href="/">"Overview"</a><a href="/metacognition">"Metacognition"</a><a href="/coordination">"Coordination"</a></nav>
            </header>

            <section class="toolbar panel">
                <label for="auth-token">"Read token"</label>
                <input id="auth-token" type="password" autocomplete="off" placeholder="Required when read protection is enabled" />
                <button id="save-token" type="button">"Apply token"</button>
                <button id="refresh" class="secondary" type="button">"Refresh"</button>
                <span id="stream-indicator" class="stream offline"><span id="stream-label">"Connecting"</span></span>
                <span>{protection}</span>
                <span id="revision">"No snapshot yet"</span>
            </section>
            <section class="controls panel">
                <label for="timeline-limit">"Timeline limit"</label><input id="timeline-limit" inputmode="numeric" value="100" />
                <label for="session-limit">"Session limit"</label><input id="session-limit" inputmode="numeric" value="100" />
                <label for="lesson-limit">"Lesson limit"</label><input id="lesson-limit" inputmode="numeric" value="250" />
                <button id="apply-limits" type="button">"Apply limits"</button>
                <label for="search">"Filter retained data"</label><input id="search" type="search" placeholder="Agent, session, task, lesson, event…" />
                <span id="retention">"Waiting for retention summary."</span>
            </section>
            <section id="error-banner" class="error" hidden></section>

            <section class="stats">
                <article class="panel"><span>"Agents"</span><strong id="stat-agents">"0"</strong></article>
                <article class="panel"><span>"Sessions"</span><strong id="stat-sessions">"0"</strong></article>
                <article class="panel"><span>"Events"</span><strong id="stat-events">"0"</strong></article>
                <article class="panel"><span>"Lessons"</span><strong id="stat-lessons">"0"</strong></article>
                <article class="panel"><span>"Tasks"</span><strong id="stat-tasks">"0"</strong></article>
                <article class="panel"><span>"Uptime"</span><strong id="stat-uptime">"0s"</strong></article>
            </section>

            <section class="panel padded">
                <div class="title"><h2>"Agents"</h2><span>"Current bounded agent projection"</span></div>
                <div id="agents" class="card-grid"><p class="empty">"Loading agents…"</p></div>
            </section>

            <section class="panel padded">
                <div class="title"><h2>"Retained sessions"</h2><span>"Derived only from retained recent events"</span></div>
                <div class="table-wrap"><table><thead><tr><th>"Session"</th><th>"Agents"</th><th>"Tasks"</th><th>"Latest"</th><th>"First retained"</th><th>"Last retained"</th><th>"Transports"</th></tr></thead><tbody id="sessions"><tr><td colspan="7" class="empty">"Loading sessions…"</td></tr></tbody></table></div>
            </section>

            <section class="panel padded">
                <div class="title"><h2>"Retained event timeline"</h2><span>"Newest occurrence first; absence is not a historical claim"</span></div>
                <div class="table-wrap"><table><thead><tr><th>"Event"</th><th>"Agent"</th><th>"Session"</th><th>"Task"</th><th>"Transport"</th><th>"Occurred"</th><th>"Visible payload"</th></tr></thead><tbody id="timeline"><tr><td colspan="7" class="empty">"Loading timeline…"</td></tr></tbody></table></div>
            </section>

            <section class="columns">
                <article class="panel padded">
                    <div class="title"><h2>"Lessons"</h2><span>"Retained learned heuristics"</span></div>
                    <div id="lessons" class="cards"><p class="empty">"Loading lessons…"</p></div>
                </article>
                <article class="panel padded">
                    <div class="title"><h2>"System and memory"</h2><span>"LRU pressure and ingestion counters"</span></div>
                    <div class="counter-row"><span>"Accepted" <strong id="accepted">"0"</strong></span><span>"Duplicates" <strong id="duplicates">"0"</strong></span><span>"Rejected" <strong id="rejected">"0"</strong></span></div>
                    <div class="table-wrap"><table><thead><tr><th>"Cache"</th><th>"Length"</th><th>"Capacity"</th><th>"Pressure"</th><th>"Evictions"</th></tr></thead><tbody id="caches"></tbody></table></div>
                </article>
            </section>

            <p class="privacy">"This explorer is read-only and retention-aware. It does not dispatch agents, change ownership, mutate LRU recency, call providers, or reconstruct hidden reasoning."</p>
        </main>
    }
    .to_html();

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>Operator Explorer</title><style>{CSS}</style></head><body>{body}<script>{SCRIPT}</script></body></html>"
    )
}

const SCRIPT: &str = include_str!("../scripts/explorer-dashboard.js");

const CSS: &str = r#"
:root{font-family:Inter,ui-sans-serif,system-ui,sans-serif;color:#edf5ff;background:#071019;--panel:#101c28;--line:#2b3e52;--muted:#92a7bb;--accent:#73e0bd;--blue:#73b9ff;--warning:#ffc46a;--danger:#ff7d91}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 8% 0,#183853 0,transparent 34%),#071019}.shell{width:min(1580px,calc(100% - 32px));margin:auto;padding:32px 0 60px}header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}.eyebrow{color:var(--accent);font-size:.72rem;font-weight:800;letter-spacing:.18em}.lede,.privacy,.title span,.toolbar span,.controls span,small{color:var(--muted)}h1{font-size:clamp(2.2rem,5vw,5rem);margin:.2rem 0;letter-spacing:-.05em}nav{display:flex;gap:8px;flex-wrap:wrap}nav a{color:var(--accent);text-decoration:none;border:1px solid var(--line);border-radius:10px;padding:9px 13px}.panel{background:linear-gradient(155deg,rgba(21,36,52,.97),rgba(10,22,33,.97));border:1px solid var(--line);border-radius:16px}.padded{padding:20px;margin-top:16px}.toolbar,.controls{display:grid;gap:10px;align-items:center;padding:13px 15px;margin-top:16px}.toolbar{grid-template-columns:auto minmax(220px,1fr) auto auto auto auto auto}.controls{grid-template-columns:auto 80px auto 80px auto 80px auto auto minmax(220px,1fr) minmax(260px,auto)}input{background:#08131e;border:1px solid var(--line);color:#edf5ff;padding:10px;border-radius:9px;min-width:0}button{border:0;border-radius:9px;background:var(--accent);padding:10px 14px;font-weight:800;cursor:pointer}.secondary{background:#21364a;color:#edf5ff}.stream{display:inline-flex;align-items:center;gap:7px}.stream::before{content:'';width:8px;height:8px;border-radius:50%;background:var(--danger)}.stream.online::before{background:var(--accent);box-shadow:0 0 9px rgba(115,224,189,.65)}.error{padding:12px 16px;border:1px solid var(--danger);background:#3b1821;border-radius:12px;margin-top:12px}.stats{display:grid;grid-template-columns:repeat(6,1fr);gap:12px;margin-top:16px}.stats article{padding:16px}.stats span{display:block;color:var(--muted);font-size:.72rem;text-transform:uppercase}.stats strong{display:block;font-size:1.8rem;margin-top:7px}.title{display:flex;justify-content:space-between;gap:12px;align-items:baseline}.title h2{font-size:1.05rem}.card-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(290px,1fr));gap:10px}.cards{display:grid;gap:10px}.card{background:#08131e;border:1px solid var(--line);border-radius:12px;padding:14px}.row{display:flex;justify-content:space-between;gap:12px}.row code{display:block;color:var(--accent);font-size:.7rem;margin-top:4px}.status,.confidence{border:1px solid var(--line);color:var(--blue);border-radius:999px;padding:4px 9px;font-size:.73rem;height:max-content}.card p{font-size:.82rem;line-height:1.5}.card dl{display:grid;grid-template-columns:auto 1fr;gap:5px 10px;font-size:.76rem}.card dt{color:var(--muted)}.card dd{margin:0;overflow-wrap:anywhere}.meta-line,.note,.error-note{border-top:1px solid var(--line);padding-top:8px}.error-note{color:#ffc0ca}.columns{display:grid;grid-template-columns:1.1fr 1fr;gap:16px}.counter-row{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin:8px 0 14px}.counter-row span{background:#08131e;border:1px solid var(--line);border-radius:9px;padding:10px;color:var(--muted)}.counter-row strong{display:block;color:#edf5ff;font-size:1.3rem}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;min-width:1080px}th,td{text-align:left;padding:11px 9px;border-bottom:1px solid var(--line);font-size:.77rem;vertical-align:top}th{font-size:.66rem;color:var(--muted);text-transform:uppercase}td small{display:block;margin-top:4px}.payload{display:block;max-width:440px;max-height:110px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;color:#c7d8e9}.empty{color:var(--muted)}.privacy{margin-top:18px;font-size:.82rem}@media(max-width:1150px){.stats{grid-template-columns:repeat(3,1fr)}.columns{grid-template-columns:1fr}.toolbar,.controls{grid-template-columns:1fr 1fr}.toolbar label,.toolbar input,.toolbar span,.controls span{grid-column:1/-1}}@media(max-width:650px){.shell{width:calc(100% - 20px)}header{display:block}nav{margin-top:12px}.stats{grid-template-columns:repeat(2,1fr)}.toolbar,.controls{display:flex;flex-direction:column;align-items:stretch}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_static_retention_aware_shell_without_protected_data() {
        let html = dashboard(true);
        assert!(html.contains("Operator explorer"));
        assert!(html.contains("timeline-limit"));
        assert!(html.contains("stream-indicator"));
        assert!(html.contains("/api/v1/explorer"));
        assert!(html.contains("retention-aware"));
        assert!(!html.contains("meta-agent-read-token="));
    }
}
