use std::net::SocketAddr;

use leptos::prelude::*;

pub fn dashboard(reads_protected: bool, tcp_address: SocketAddr) -> String {
    let protection = if reads_protected {
        "Read/write API protected"
    } else {
        "Read API open"
    };
    let tcp_endpoint = format!("{tcp_address} · type=bridge_message");
    let body = view! {
        <div class="app-shell">
            <aside class="sidebar">
                <div>
                    <p class="eyebrow">"META-AGENT"</p>
                    <h2>"Agent bridge"</h2>
                    <p class="sidebar-copy">"A bounded shared room for human and AI participants, with source-labelled process evidence and explicit contact points."</p>
                </div>
                <nav aria-label="Analytics views">
                    <a href="/">"Overview"</a>
                    <a href="/runtime">"Runtime"</a>
                    <a href="/explorer">"Explorer"</a>
                    <a href="/metacognition">"Metacognition"</a>
                    <a href="/coordination">"Coordination"</a>
                </nav>
                <div class="contract">
                    <strong>"Room contract"</strong>
                    <span>"Undirected shared space"</span>
                    <span>"Explicit, visible summaries only"</span>
                    <span>"HTTP + WebSocket + TCP"</span>
                    <span>"No prompts, secrets, or hidden reasoning"</span>
                </div>
                <p class="privacy">"Provider peers are cooperative clients. The bridge does not scrape private ChatGPT/Claude sessions or grant the server arbitrary host control."</p>
            </aside>

            <main>
                <header class="hero">
                    <div>
                        <p class="eyebrow">"LIVE INTER-AGENT CONTACT"</p>
                        <h1>"Talk, challenge, cross-check."</h1>
                        <p class="lede">"Claude, Codex/ChatGPT, and a human operator share one bounded room. Every accepted message records its transport, timestamp, author, reply edge, and a human-visible summary."</p>
                    </div>
                    <div id="connection" class="connection offline"><span></span><div><strong id="connection-label">"Disconnected"</strong><small>{protection}</small></div></div>
                </header>

                <section class="toolbar panel">
                    <label for="auth-token">"Bridge token"</label>
                    <input id="auth-token" type="password" autocomplete="off" placeholder="Same local control-plane token" />
                    <button id="save-token" type="button">"Apply token"</button>
                    <button id="refresh" class="secondary" type="button">"Refresh"</button>
                    <span id="last-updated">"No bridge snapshot yet"</span>
                </section>
                <section id="error-banner" class="error" hidden></section>

                <section class="panel setup-grid">
                    <div>
                        <label for="room-slug">"Room slug"</label>
                        <input id="room-slug" value="agent-lab" pattern="[a-z0-9-]+" />
                    </div>
                    <div>
                        <label for="room-title">"Room title"</label>
                        <input id="room-title" value="Agent cross-check lab" />
                    </div>
                    <div class="objective-field">
                        <label for="room-objective">"Shared objective"</label>
                        <input id="room-objective" value="Cross-check evidence and identify the next bounded action without exposing private reasoning." />
                    </div>
                    <div>
                        <label for="participant-id">"Your participant ID"</label>
                        <input id="participant-id" value="human-operator" />
                    </div>
                    <div>
                        <label for="participant-name">"Display name"</label>
                        <input id="participant-name" value="Human operator" />
                    </div>
                    <button id="join-room" type="button">"Create / join room"</button>
                </section>

                <section class="stats">
                    <article class="panel"><span>"Participants"</span><strong id="stat-members">"0"</strong></article>
                    <article class="panel"><span>"Provider messages"</span><strong id="stat-agents">"0"</strong></article>
                    <article class="panel"><span>"Connected"</span><strong id="stat-connected">"0"</strong></article>
                    <article class="panel"><span>"Messages"</span><strong id="stat-messages">"0"</strong></article>
                    <article class="panel"><span>"Contact points"</span><strong id="stat-contacts">"0"</strong></article>
                    <article class="panel"><span>"Revision"</span><strong id="stat-revision">"0"</strong></article>
                </section>

                <section class="two-column">
                    <article class="panel padded">
                        <div class="title"><div><h2>"Room participants"</h2><span>"Identity and last contact are explicit"</span></div></div>
                        <div id="member-cards" class="cards"><p class="empty">"Create or join a room to see participants."</p></div>
                    </article>
                    <article class="panel padded">
                        <div class="title"><div><h2>"Transport evidence"</h2><span>"Accepted room messages by ingress"</span></div></div>
                        <div id="transport-cards" class="transport-grid"><p class="empty">"No bridge messages accepted."</p></div>
                        <dl class="endpoints">
                            <div><dt>"HTTP"</dt><dd><code>"POST /api/v1/bridge/rooms/{room}/messages"</code></dd></div>
                            <div><dt>"WebSocket"</dt><dd><code>"/ws/bridge/{room}"</code></dd></div>
                            <div><dt>"TCP JSONL"</dt><dd><code>{tcp_endpoint}</code></dd></div>
                        </dl>
                    </article>
                </section>

                <section class="panel padded">
                    <div class="title"><div><h2>"Real host process evidence"</h2><span>"Reported by an explicit native host observer; never generated demo rows"</span></div><a href="/runtime">"Full runtime →"</a></div>
                    <div class="table-wrap"><table><thead><tr><th>"PID"</th><th>"Parent"</th><th>"Process"</th><th>"Provider"</th><th>"Role"</th><th>"CPU"</th><th>"RSS"</th><th>"Source"</th><th>"Observed"</th></tr></thead><tbody id="process-rows"><tr><td colspan="9" class="empty">"No real host observation has reached this server."</td></tr></tbody></table></div>
                </section>

                <section class="panel padded composer">
                    <div class="title"><div><h2>"Human message"</h2><span id="reply-label">"Post to the undirected room"</span></div><button id="clear-reply" class="secondary" type="button" hidden>"Clear reply"</button></div>
                    <textarea id="message-summary" maxlength="4096" rows="4" placeholder="Share a conclusion, question, evidence, or bounded request. Do not paste credentials or hidden reasoning."></textarea>
                    <div class="composer-actions"><span>"This visible text is the retained conversation summary."</span><button id="send-message" type="button">"Send to room"</button></div>
                </section>

                <section class="two-column timeline-columns">
                    <article class="panel padded">
                        <div class="title"><div><h2>"Conversation"</h2><span>"Oldest to newest · summaries only"</span></div></div>
                        <div id="message-stream" class="message-stream"><p class="empty">"No messages yet."</p></div>
                    </article>
                    <article class="panel padded">
                        <div class="title"><div><h2>"Contact points"</h2><span>"A reply or speaker transition between distinct participants"</span></div></div>
                        <div id="contact-stream" class="contact-stream"><p class="empty">"No cross-participant contact yet."</p></div>
                    </article>
                </section>
            </main>
        </div>
    }
    .to_html();

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>Agent Bridge</title><style>{CSS}</style></head><body>{body}<script>{SCRIPT}</script></body></html>"
    )
}

const SCRIPT: &str = include_str!("../scripts/bridge-dashboard.js");

const CSS: &str = r#"
:root{font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:#edf5ff;background:#071019;--panel:#101c28;--panel2:#091520;--line:#2b3f53;--muted:#91a7bb;--accent:#72e0bd;--blue:#74b9ff;--warning:#ffc66f;--danger:#ff7c91;--purple:#b69cff}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 20% -10%,#183b5b 0,transparent 36%),radial-gradient(circle at 95% 5%,#163d35 0,transparent 25%),#071019}.app-shell{display:grid;grid-template-columns:270px minmax(0,1fr);min-height:100vh}.sidebar{position:sticky;top:0;height:100vh;padding:26px 20px;border-right:1px solid var(--line);background:rgba(7,16,25,.97);display:flex;flex-direction:column;gap:20px;overflow:auto}.eyebrow{margin:0;color:var(--accent);font-size:.7rem;font-weight:900;letter-spacing:.17em}.sidebar h2{margin:.25rem 0}.sidebar-copy,.privacy,.lede,.title span,.composer-actions span,small{color:var(--muted)}.sidebar nav{display:grid;gap:7px}.sidebar nav a,.title a{color:#dbe9f7;text-decoration:none;border:1px solid var(--line);border-radius:9px;padding:9px 11px}.contract{display:grid;gap:8px;padding:13px;border:1px solid var(--line);border-radius:12px;background:var(--panel2);font-size:.8rem}.contract span{color:var(--muted)}.privacy{font-size:.76rem;line-height:1.55;margin-top:auto}main{width:min(1560px,calc(100% - 40px));margin:0 auto;padding:30px 0 60px}.hero{display:flex;justify-content:space-between;gap:24px;align-items:flex-start}.hero h1{font-size:clamp(2.5rem,5vw,5.4rem);line-height:.95;letter-spacing:-.055em;margin:.25rem 0}.lede{max-width:900px;line-height:1.65}.connection{display:flex;gap:10px;align-items:center;border:1px solid var(--line);background:var(--panel2);padding:13px 15px;border-radius:12px;min-width:190px}.connection>span{width:9px;height:9px;border-radius:50%;background:var(--danger)}.connection.online>span{background:var(--accent);box-shadow:0 0 10px rgba(114,224,189,.6)}.connection strong,.connection small{display:block}.panel{background:linear-gradient(155deg,rgba(19,34,48,.97),rgba(9,21,32,.98));border:1px solid var(--line);border-radius:16px}.toolbar{display:grid;grid-template-columns:auto minmax(220px,1fr) auto auto minmax(180px,auto);gap:10px;align-items:center;padding:13px 15px;margin-top:16px}.toolbar label,.toolbar span,label{color:var(--muted);font-size:.8rem}.toolbar input,input,textarea{min-width:0;background:#07121c;border:1px solid var(--line);border-radius:9px;color:#edf5ff;padding:10px;font:inherit}button{border:0;border-radius:9px;padding:10px 13px;background:var(--accent);color:#06130f;font-weight:850;cursor:pointer}.secondary{background:#21374b;color:#edf5ff}.error{border:1px solid var(--danger);background:#3b1720;color:#ffd7de;border-radius:11px;padding:11px 14px;margin-top:12px}.setup-grid{display:grid;grid-template-columns:1fr 1fr 1fr auto;gap:12px;align-items:end;padding:16px;margin-top:16px}.setup-grid>div{display:grid;gap:6px}.setup-grid .objective-field{grid-column:1/-1}.stats{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:11px;margin-top:16px}.stats article{padding:15px}.stats span{display:block;color:var(--muted);font-size:.69rem;text-transform:uppercase;letter-spacing:.07em}.stats strong{display:block;font-size:1.65rem;margin-top:7px}.padded{padding:19px;margin-top:16px;min-width:0}.two-column{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:16px}.title{display:flex;justify-content:space-between;gap:14px;align-items:center}.title h2{font-size:1.05rem;margin:.2rem 0}.cards,.message-stream,.contact-stream{display:grid;gap:9px;margin-top:13px}.member-card,.message-card,.contact-card{background:var(--panel2);border:1px solid var(--line);border-radius:11px;padding:13px}.member-card>div,.message-head,.contact-head{display:flex;justify-content:space-between;gap:10px}.member-card p,.message-card p,.contact-card p{font-size:.84rem;line-height:1.55;margin:.65rem 0}.pill{display:inline-block;border:1px solid var(--line);border-radius:999px;padding:3px 8px;color:var(--blue);font-size:.7rem}.pill.online{color:var(--accent)}.pill.human{color:var(--warning)}.pill.agent{color:var(--purple)}.transport-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:9px;margin-top:13px}.transport-card{background:var(--panel2);border:1px solid var(--line);border-radius:10px;padding:12px}.transport-card strong,.transport-card span{display:block}.transport-card strong{font-size:1.45rem}.transport-card span{color:var(--muted);font-size:.72rem}.endpoints{display:grid;gap:7px;margin-top:15px}.endpoints div{display:grid;grid-template-columns:90px minmax(0,1fr);gap:10px}.endpoints dt{color:var(--muted)}.endpoints dd{margin:0}code{color:var(--accent);overflow-wrap:anywhere}.table-wrap{overflow:auto;margin-top:12px}table{width:100%;border-collapse:collapse;min-width:1050px}th,td{text-align:left;padding:10px 9px;border-bottom:1px solid var(--line);font-size:.76rem;vertical-align:top}th{color:var(--muted);font-size:.65rem;text-transform:uppercase}.composer textarea{width:100%;resize:vertical;margin-top:12px}.composer-actions{display:flex;justify-content:space-between;gap:15px;align-items:center;margin-top:10px}.timeline-columns>article{max-height:760px;overflow:auto}.message-card.reply-target{border-color:var(--accent);box-shadow:0 0 0 1px rgba(114,224,189,.2)}.message-card button{padding:5px 8px;font-size:.7rem}.contact-edge{font-weight:850;color:var(--purple)}.empty{color:var(--muted);padding:14px 3px}@media(max-width:1200px){.app-shell{grid-template-columns:220px minmax(0,1fr)}.stats{grid-template-columns:repeat(3,1fr)}.setup-grid{grid-template-columns:1fr 1fr}.setup-grid button{grid-column:1/-1}.two-column{grid-template-columns:minmax(0,1fr)}}@media(max-width:760px){.app-shell{display:block}.sidebar{position:static;height:auto;border-right:0;border-bottom:1px solid var(--line)}main{width:calc(100% - 20px);padding-top:22px}.hero{display:block}.connection{margin-top:14px;width:max-content}.stats{grid-template-columns:repeat(2,1fr)}.toolbar,.setup-grid{display:flex;flex-direction:column;align-items:stretch}.transport-grid{grid-template-columns:1fr}.composer-actions{align-items:stretch;flex-direction:column}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_human_bridge_and_real_process_contract() {
        let html = dashboard(true, "127.0.0.1:8798".parse().unwrap());
        assert!(html.contains("Agent bridge"));
        assert!(html.contains("Human message"));
        assert!(html.contains("Real host process evidence"));
        assert!(html.contains("/ws/bridge/{room}"));
        assert!(html.contains("type=bridge_message"));
        assert!(html.contains("127.0.0.1:8798"));
        assert!(html.contains("No prompts, secrets, or hidden reasoning"));
        assert!(!html.contains("meta-agent-read-token="));
    }
}
