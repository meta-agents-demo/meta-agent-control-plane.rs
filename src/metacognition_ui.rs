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
                    <h1>"Explainable metacognition"</h1>
                    <p class="lede">"Deterministic progress, dependency, evidence, and retry diagnostics derived only from visible retained state."</p>
                </div>
                <a class="back" href="/">"Overview"</a>
            </header>

            <section class="toolbar panel">
                <label for="auth-token">"Read token"</label>
                <input id="auth-token" type="password" autocomplete="off" placeholder="Required when read protection is enabled" />
                <button id="save-token" type="button">"Apply token"</button>
                <button id="refresh" class="secondary" type="button">"Refresh"</button>
                <span>{protection}</span>
                <span id="revision">"No projection yet"</span>
            </section>
            <section id="error-banner" class="error" hidden></section>

            <section class="stats">
                <article class="panel"><span>"Active"</span><strong id="stat-active">"0"</strong></article>
                <article class="panel"><span>"Blocked"</span><strong id="stat-blocked">"0"</strong></article>
                <article class="panel"><span>"Stalled"</span><strong id="stat-stalled">"0"</strong></article>
                <article class="panel"><span>"Retry loops"</span><strong id="stat-retries">"0"</strong></article>
                <article class="panel"><span>"Evidence coverage"</span><strong id="stat-evidence">"0%"</strong></article>
                <article class="panel"><span>"Evidence / reported"</span><strong id="stat-progress">"0% / 0%"</strong></article>
            </section>

            <section class="columns">
                <article class="panel padded">
                    <div class="title"><h2>"Diagnostics"</h2><span>"Severity, explanation, and recommended action"</span></div>
                    <div id="diagnostics" class="cards"><p class="empty">"Loading diagnostics…"</p></div>
                </article>
                <article class="panel padded">
                    <div class="title"><h2>"Goals"</h2><span>"Evidence-backed progress and critical paths"</span></div>
                    <div id="goals" class="cards"><p class="empty">"Loading goals…"</p></div>
                </article>
            </section>

            <section class="panel padded">
                <div class="title"><h2>"Task analysis"</h2><span>"Evidence, attempts, age, and diagnostics"</span></div>
                <div class="table-wrap"><table><thead><tr><th>"Task"</th><th>"Status"</th><th>"Evidence / reported"</th><th>"Evidence"</th><th>"Attempt"</th><th>"Age"</th><th>"Signals"</th></tr></thead><tbody id="tasks"><tr><td colspan="7" class="empty">"Loading tasks…"</td></tr></tbody></table></div>
            </section>

            <p class="privacy">"This view uses concise summaries, confidence, evidence references, artifacts, dependencies, timestamps, and source event IDs. It never requests or reconstructs private hidden reasoning."</p>
        </main>
    }.to_html();

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>Explainable Metacognition</title><style>{CSS}</style></head><body>{body}<script>{SCRIPT}</script></body></html>"
    )
}

const SCRIPT: &str = include_str!("../scripts/metacognition-dashboard.js");

const CSS: &str = r#"
:root{font-family:Inter,ui-sans-serif,system-ui,sans-serif;color:#eef6ff;background:#071019;--panel:#101c28;--line:#273a4d;--muted:#92a7bb;--accent:#74e3bf;--warning:#ffc46a;--critical:#ff788c}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 10% 0,#17324c 0,transparent 36%),#071019}.shell{width:min(1480px,calc(100% - 32px));margin:auto;padding:32px 0 56px}header{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}.eyebrow{color:var(--accent);letter-spacing:.18em;font-size:.72rem;font-weight:800}.lede,.privacy,.title span,.toolbar span,small{color:var(--muted)}h1{font-size:clamp(2.2rem,5vw,5rem);margin:.2rem 0;letter-spacing:-.05em}.back{color:var(--accent);text-decoration:none;border:1px solid var(--line);padding:10px 14px;border-radius:10px}.panel{background:linear-gradient(155deg,rgba(21,36,52,.97),rgba(11,23,34,.97));border:1px solid var(--line);border-radius:16px}.padded{padding:20px}.toolbar{display:grid;grid-template-columns:auto minmax(220px,1fr) auto auto auto auto;gap:10px;align-items:center;padding:13px 15px;margin:20px 0}.toolbar input{background:#08131e;border:1px solid var(--line);color:#eef6ff;padding:10px;border-radius:9px}.toolbar button{border:0;border-radius:9px;background:var(--accent);padding:10px 14px;font-weight:800}.toolbar .secondary{background:#21364a;color:#eef6ff}.error{padding:12px 16px;border:1px solid var(--critical);background:#3a1720;border-radius:12px;margin-bottom:14px}.stats{display:grid;grid-template-columns:repeat(6,1fr);gap:12px}.stats article{padding:16px}.stats span{display:block;color:var(--muted);font-size:.75rem;text-transform:uppercase}.stats strong{font-size:1.8rem;display:block;margin-top:7px}.columns{display:grid;grid-template-columns:1.2fr 1fr;gap:16px;margin:16px 0}.title{display:flex;justify-content:space-between;gap:12px;align-items:baseline}.title h2{font-size:1.05rem}.cards{display:grid;gap:10px}.diagnostic,.goal{background:#08131e;border:1px solid var(--line);border-radius:12px;padding:13px}.severity-critical{border-color:rgba(255,120,140,.65)}.severity-warning{border-color:rgba(255,196,106,.55)}.row{display:flex;justify-content:space-between;gap:12px}.pill{border:1px solid var(--line);border-radius:999px;padding:3px 8px;font-size:.7rem}.diagnostic code,.goal code{color:var(--accent);font-size:.72rem}.diagnostic p,.goal p{font-size:.84rem;line-height:1.5}.action{color:#d7e6f7}.meter{height:7px;background:#071019;border-radius:999px;overflow:hidden;margin-top:12px}.meter span{display:block;height:100%;background:linear-gradient(90deg,#6eb7ff,var(--accent))}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;min-width:880px}th,td{text-align:left;padding:12px 9px;border-bottom:1px solid var(--line);font-size:.8rem}th{color:var(--muted);text-transform:uppercase;font-size:.7rem}td small{display:block;margin-top:3px}.empty{color:var(--muted)}.privacy{margin-top:18px;font-size:.82rem}@media(max-width:1000px){.stats{grid-template-columns:repeat(3,1fr)}.columns{grid-template-columns:1fr}.toolbar{grid-template-columns:1fr 1fr}.toolbar label,.toolbar input,.toolbar span{grid-column:1/-1}}@media(max-width:620px){.shell{width:calc(100% - 20px)}.stats{grid-template-columns:repeat(2,1fr)}header{display:block}.back{display:inline-block;margin-top:12px}.toolbar{display:flex;flex-direction:column;align-items:stretch}}
"#;
