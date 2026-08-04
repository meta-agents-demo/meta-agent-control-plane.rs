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
                    <h1>"Coordination plan"</h1>
                    <p class="lede">"Dependency-safe assignments, operator interventions, and explicit holds derived deterministically from visible retained state."</p>
                </div>
                <nav><a href="/">"Overview"</a><a href="/metacognition">"Metacognition"</a></nav>
            </header>

            <section class="toolbar panel">
                <label for="auth-token">"Read token"</label>
                <input id="auth-token" type="password" autocomplete="off" placeholder="Required when read protection is enabled" />
                <button id="save-token" type="button">"Apply token"</button>
                <button id="refresh" class="secondary" type="button">"Refresh"</button>
                <span id="stream-indicator" class="stream offline"><span id="stream-label">"Connecting"</span></span>
                <span>{protection}</span>
                <span id="revision">"No plan yet"</span>
            </section>
            <section id="error-banner" class="error" hidden></section>

            <section class="stats">
                <article class="panel"><span>"Assignments"</span><strong id="stat-assignments">"0"</strong></article>
                <article class="panel"><span>"Assigned agents"</span><strong id="stat-agents">"0"</strong></article>
                <article class="panel"><span>"Interventions"</span><strong id="stat-interventions">"0"</strong></article>
                <article class="panel"><span>"Held tasks"</span><strong id="stat-holds">"0"</strong></article>
                <article class="panel"><span>"Capacity holds"</span><strong id="stat-suppressed">"0"</strong></article>
                <article class="panel"><span>"Omitted"</span><strong id="stat-omitted">"0"</strong></article>
            </section>

            <section class="policy panel"><strong>"Planning bounds"</strong><span id="policy">"Waiting for plan."</span></section>

            <section class="columns">
                <article class="panel padded">
                    <div class="title"><h2>"Assignments"</h2><span>"Fair-share executable next actions"</span></div>
                    <div id="assignments" class="cards"><p class="empty">"Loading assignments…"</p></div>
                </article>
                <article class="panel padded">
                    <div class="title"><h2>"Operator interventions"</h2><span>"Graph and state repairs before dispatch"</span></div>
                    <div id="interventions" class="cards"><p class="empty">"Loading interventions…"</p></div>
                </article>
            </section>

            <section class="panel padded">
                <div class="title"><h2>"Held tasks"</h2><span>"Offline agents, unresolved dependencies, terminal work, and bounded capacity"</span></div>
                <div class="table-wrap"><table><thead><tr><th>"Task"</th><th>"Reason"</th><th>"Explanation"</th><th>"Dependencies"</th><th>"Provenance"</th></tr></thead><tbody id="holds"><tr><td colspan="5" class="empty">"Loading held tasks…"</td></tr></tbody></table></div>
            </section>

            <p class="privacy">"This view is advisory and read-only. It never dispatches work, changes ownership, calls a provider, or reconstructs hidden reasoning."</p>
        </main>
    }
    .to_html();

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>Coordination Plan</title><style>{CSS}</style></head><body>{body}<script>{SCRIPT}</script></body></html>"
    )
}

const SCRIPT: &str = include_str!("../scripts/coordination-dashboard.js");

const CSS: &str = r#"
:root{font-family:Inter,ui-sans-serif,system-ui,sans-serif;color:#eef6ff;background:#071019;--panel:#101c28;--line:#273a4d;--muted:#92a7bb;--accent:#74e3bf;--blue:#71b8ff;--warning:#ffc46a;--critical:#ff788c}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 12% 0,#17324c 0,transparent 36%),#071019}.shell{width:min(1500px,calc(100% - 32px));margin:auto;padding:32px 0 56px}header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}.eyebrow{color:var(--accent);letter-spacing:.18em;font-size:.72rem;font-weight:800}.lede,.privacy,.title span,.toolbar span,.policy span,small{color:var(--muted)}h1{font-size:clamp(2.2rem,5vw,5rem);margin:.2rem 0;letter-spacing:-.05em}nav{display:flex;gap:9px;flex-wrap:wrap}nav a{color:var(--accent);text-decoration:none;border:1px solid var(--line);padding:10px 14px;border-radius:10px}.panel{background:linear-gradient(155deg,rgba(21,36,52,.97),rgba(11,23,34,.97));border:1px solid var(--line);border-radius:16px}.padded{padding:20px}.toolbar{display:grid;grid-template-columns:auto minmax(220px,1fr) auto auto auto auto auto;gap:10px;align-items:center;padding:13px 15px;margin:20px 0}.toolbar input{background:#08131e;border:1px solid var(--line);color:#eef6ff;padding:10px;border-radius:9px}.toolbar button{border:0;border-radius:9px;background:var(--accent);padding:10px 14px;font-weight:800;cursor:pointer}.toolbar .secondary{background:#21364a;color:#eef6ff}.stream{display:inline-flex;align-items:center;gap:7px}.stream::before{content:'';width:8px;height:8px;border-radius:50%;background:var(--critical);box-shadow:0 0 8px rgba(255,120,140,.45)}.stream.online::before{background:var(--accent);box-shadow:0 0 9px rgba(116,227,191,.65)}.error{padding:12px 16px;border:1px solid var(--critical);background:#3a1720;border-radius:12px;margin-bottom:14px}.stats{display:grid;grid-template-columns:repeat(6,1fr);gap:12px}.stats article{padding:16px}.stats span{display:block;color:var(--muted);font-size:.75rem;text-transform:uppercase}.stats strong{font-size:1.8rem;display:block;margin-top:7px}.policy{display:flex;justify-content:space-between;gap:18px;padding:14px 17px;margin:16px 0}.columns{display:grid;grid-template-columns:1.15fr 1fr;gap:16px;margin:16px 0}.title{display:flex;justify-content:space-between;gap:12px;align-items:baseline}.title h2{font-size:1.05rem}.cards{display:grid;gap:10px}.assignment,.intervention{background:#08131e;border:1px solid var(--line);border-radius:12px;padding:14px}.assignment.critical-path{border-color:rgba(116,227,191,.7);box-shadow:inset 3px 0 var(--accent)}.intervention{border-color:rgba(255,196,106,.55)}.row{display:flex;justify-content:space-between;gap:12px}.row code{display:block;color:var(--accent);font-size:.72rem;margin-top:4px}.priority{border:1px solid var(--line);border-radius:999px;padding:4px 9px;font-size:.75rem;height:max-content;color:var(--blue)}.assignment p,.intervention p{font-size:.84rem;line-height:1.5}.action{color:#d7e6f7}.meta{display:flex;gap:8px;flex-wrap:wrap}.meta span{border:1px solid var(--line);border-radius:999px;padding:3px 8px;font-size:.68rem;color:var(--muted)}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;min-width:1050px}th,td{text-align:left;padding:12px 9px;border-bottom:1px solid var(--line);font-size:.79rem;vertical-align:top}th{color:var(--muted);text-transform:uppercase;font-size:.68rem}td small{display:block;margin-top:4px}.empty{color:var(--muted)}.privacy{margin-top:18px;font-size:.82rem}@media(max-width:1000px){.stats{grid-template-columns:repeat(3,1fr)}.columns{grid-template-columns:1fr}.toolbar{grid-template-columns:1fr 1fr}.toolbar label,.toolbar input,.toolbar span{grid-column:1/-1}}@media(max-width:620px){.shell{width:calc(100% - 20px)}.stats{grid-template-columns:repeat(2,1fr)}header{display:block}nav{margin-top:12px}.toolbar{display:flex;flex-direction:column;align-items:stretch}.policy{display:block}.policy span{display:block;margin-top:6px}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_static_shell_without_protected_plan_data() {
        let html = dashboard(true);
        assert!(html.contains("Coordination plan"));
        assert!(html.contains("Read token"));
        assert!(html.contains("stream-indicator"));
        assert!(html.contains("stream-label"));
        assert!(html.contains("/api/v1/coordination"));
        assert!(html.contains("advisory and read-only"));
    }
}
