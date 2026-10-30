// Plain, dependency-free polling client for the HumaiPsy read-only dashboard.
// No build step, no framework — this is intentionally simple.

const POLL_INTERVAL_MS = 1500;

const el = (id) => document.getElementById(id);
const connectionState = el("connection-state");

function setConnectionState(state) {
  // "locked" is a real, distinct state from "connected"/"lost": the server
  // is reachable and answering, it's just withholding content because the
  // desktop app's master-password vault isn't unlocked right now.
  connectionState.className = "pill pill-" + state;
  connectionState.textContent =
    state === "connected"
      ? "connected"
      : state === "locked"
        ? "app locked"
        : state === "lost"
          ? "connection lost"
          : "connecting…";
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

// Chat content is plain text except for inline **bold**. Escape first, then
// add only the <strong> tags we generate ourselves, so a model response can
// never inject markup. Unmatched markers are left visible, matching the
// desktop transcript's renderer.
function renderInlineMarkup(text) {
  return escapeHtml(text).replace(/\*\*([\s\S]+?)\*\*/g, "<strong>$1</strong>");
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
        <div class="content">${renderInlineMarkup(m.content)}</div>
      </div>`;
    })
    .join("");

  container.scrollTop = container.scrollHeight;
}

function renderLocked() {
  el("status-design").textContent = "—";
  el("status-participant").textContent = "—";
  el("status-condition").textContent = "—";
  el("status-trial").textContent = "—";
  el("status-timer").textContent = "—";
  el("status-running").textContent = "—";
  el("messages").innerHTML =
    '<p class="empty-state">HumaiPsy is locked. Unlock the desktop app to view the transcript here.</p>';
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
    const status = await statusRes.json();
    const messages = await messagesRes.json();

    if (!status.vault_unlocked) {
      renderLocked();
      setConnectionState("locked");
      return;
    }

    renderStatus(status);
    renderMessages(messages);
    setConnectionState("connected");
  } catch (err) {
    setConnectionState("lost");
  }
}

pollOnce();
setInterval(pollOnce, POLL_INTERVAL_MS);
