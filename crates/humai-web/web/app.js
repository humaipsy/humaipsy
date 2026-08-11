// Plain, dependency-free polling client for the Humai read-only dashboard.
// No build step, no framework — this is intentionally simple.

const POLL_INTERVAL_MS = 1500;

const el = (id) => document.getElementById(id);
const connectionState = el("connection-state");

function setConnectionState(state) {
  connectionState.className = "pill pill-" + state;
  connectionState.textContent =
    state === "connected" ? "connected" : state === "lost" ? "connection lost" : "connecting…";
}

function renderStatus(status) {
  el("status-design").textContent = status.design_name || "—";
  el("status-participant").textContent = status.participant_id || "—";
  el("status-condition").textContent = status.condition_label || "—";
  el("status-trial").textContent =
    status.total_trials > 0
      ? `${status.trial_in_condition} / ${status.total_trials}`
      : "—";
  el("status-timer").textContent = status.timer_display || "—";
  el("status-running").textContent = status.running ? "Running" : "Idle";
}

function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

function renderMessages(messages) {
  const container = el("messages");
  if (!messages || messages.length === 0) {
    container.innerHTML = '<p class="empty-state">No messages yet.</p>';
    return;
  }

  container.innerHTML = messages
    .map((m) => {
      const source = (m.source || "system").toLowerCase();
      const from = m.from || source;
      return `<div class="message source-${escapeHtml(source)}">
        <div class="meta"><span>${escapeHtml(from)}</span><span>${escapeHtml(m.timestamp || "")}</span></div>
        <div class="content">${escapeHtml(m.content)}</div>
      </div>`;
    })
    .join("");

  container.scrollTop = container.scrollHeight;
}

async function pollOnce() {
  try {
    const [statusRes, messagesRes] = await Promise.all([
      fetch("/api/status"),
      fetch("/api/messages"),
    ]);
    if (!statusRes.ok || !messagesRes.ok) {
      throw new Error("bad response");
    }
    renderStatus(await statusRes.json());
    renderMessages(await messagesRes.json());
    setConnectionState("connected");
  } catch (err) {
    setConnectionState("lost");
  }
}

pollOnce();
setInterval(pollOnce, POLL_INTERVAL_MS);
