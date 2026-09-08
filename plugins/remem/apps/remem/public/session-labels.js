"use strict";

// Shared label controls for the existing Sessions and Workstreams workspaces.
const SessionLabels = (() => {
  const intents = ["fea", "des", "fix", "opt", "rel", "exp", "doc", "res"];
  const $ = (id) => document.getElementById(id);
  function element(tag, text, className) {
    const node = document.createElement(tag);
    if (text !== undefined) node.textContent = text;
    if (className) node.className = className;
    return node;
  }
  function inputLabel(text, input) {
    const label = element("label", text);
    label.append(input);
    return label;
  }
  function dateEpoch(value) {
    return Date.parse(`${value}T00:00:00+08:00`) / 1000;
  }
  function addFilters(params) {
    const intent = $("session-intent-filter").value;
    const from = $("session-date-from").value;
    const through = $("session-date-to").value;
    if (from && through && from > through) throw new Error("Created from must be on or before Created through.");
    if (intent) params.set("session_intent", intent);
    if (from) params.set("since_epoch", String(dateEpoch(from)));
    if (through) params.set("until_epoch", String(dateEpoch(through) + 86400));
  }
  function renderItems(target, sessions, compact, helpers) {
    const { state, sessionKey, projectName, shortSessionId, formatDate } = helpers;
    target.replaceChildren();
    if (!sessions.length) {
      target.append(element("div", "No sessions match this view.", "empty"));
      return;
    }
    if (!compact) target.append(element("div", "LABEL", "eyebrow label-column-heading"));
    for (const session of sessions) {
      const button = element("button", undefined, "session-item");
      button.dataset.sessionKey = sessionKey(session);
      button.classList.toggle("active", Boolean(state.selectedSession && sessionKey(state.selectedSession) === sessionKey(session)));
      const fallback = session.session_topic || `${projectName(session.project)} / ${shortSessionId(session.session_id)}`;
      button.append(element("span", session.display_label || `Abstain · ${fallback}`, "session-item-title"));
      button.append(element("span", session.project || session.source_root, "session-item-project"));
      const meta = element("span", undefined, "session-item-meta");
      meta.append(element("span", `${session.message_count}${session.message_counts_truncated ? "+" : ""} msg`));
      meta.append(element("span", compact ? formatDate(session.last_epoch) : `${session.projected_turn_count} turns · ${formatDate(session.last_epoch)}`));
      button.append(meta);
      if (!compact) button.append(element("span", session.override_available
        ? `Override ID #${session.session_row_id} · ${session.session_intent_source || "No intent source"}`
        : "Override unavailable until a trusted session summary exists", "session-item-project"));
      target.append(button);
    }
  }
  function editor(kind, host, options) {
    const details = element("details");
    details.append(element("summary", "Correct labels · preview before apply"));
    const form = element("div", undefined, "form-grid");
    const ids = element("input"); ids.id = `${kind}-label-ids`; ids.placeholder = "e.g. 12, 18";
    const intent = element("select"); intent.id = `${kind}-label-intent`;
    intent.append(new Option("Abstain (clear intent)", ""));
    intents.forEach((value) => intent.append(new Option(value.toUpperCase(), value)));
    const topic = element("input"); topic.id = `${kind}-label-topic`; topic.maxLength = 80;
    topic.placeholder = "Replacement topic; blank clears it";
    const reason = element("input"); reason.id = `${kind}-label-reason`; reason.maxLength = 1000;
    form.append(inputLabel("Session IDs".replace("Session", kind === "session" ? "Session" : "Workstream"), ids), inputLabel("Replacement intent", intent), inputLabel("Replacement topic", topic), inputLabel("Reason", reason));
    const controls = element("div", undefined, "row");
    const preview = element("button", "Preview changes"); preview.id = `${kind}-label-preview`;
    const confirm = element("input"); confirm.type = "checkbox"; confirm.id = `${kind}-label-confirm`; confirm.disabled = true;
    const apply = element("button", "Apply reviewed changes", "primary"); apply.id = `${kind}-label-apply`; apply.disabled = true;
    controls.append(preview, inputLabel("I reviewed Before / After", confirm), apply);
    const result = element("div", undefined, "label-preview"); result.id = `${kind}-label-result`; result.setAttribute("aria-live", "polite");
    details.append(element("p", "Both fields replace the selected labels. Empty fields explicitly clear them. IDs and workstream aliases remain stable.", "subtle"), form, controls, result);
    host.append(details);
    let token = null;
    let generation = 0;
    let busy = false;
    function invalidate() {
      generation += 1; token = null; confirm.checked = false; confirm.disabled = true; apply.disabled = true;
      result.replaceChildren();
    }
    form.addEventListener("input", invalidate);
    confirm.addEventListener("change", () => { apply.disabled = busy || !token || !confirm.checked; });
    preview.addEventListener("click", async () => {
      invalidate();
      const current = generation;
      try {
        const values = ids.value.split(/[\s,]+/).filter(Boolean).map(Number);
        if (!values.length || values.length > 50 || values.some((id) => !Number.isSafeInteger(id) || id <= 0)) throw new Error("Enter between 1 and 50 positive IDs.");
        if (!reason.value.trim()) throw new Error("A reason is required.");
        busy = true; preview.disabled = true;
        const payload = await options.request("/api/session-intent-preview", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({
          targets: [...new Set(values)].map((id) => ({ kind, id })), session_intent: intent.value || null,
          session_topic: topic.value.trim() || null, reason: reason.value.trim()
        }) });
        if (current !== generation) return;
        if (!payload.preview_token || !Array.isArray(payload.changes) || !payload.changes.length) throw new Error("The server returned an incomplete preview.");
        token = payload.preview_token;
        const table = element("table");
        const header = element("tr"); ["Target", "Before", "After"].forEach((text) => header.append(element("th", text)));
        const thead = element("thead"); thead.append(header); table.append(thead);
        const body = element("tbody");
        for (const change of payload.changes) {
          const row = element("tr"); row.append(element("td", `${change.kind} #${change.id}`));
          for (const value of [change.before, change.after]) {
            row.append(element("td", `${value.session_intent || "Abstain"} ｜ ${value.session_topic || "No topic"} (${value.session_intent_source || "No source"})`));
          }
          body.append(row);
        }
        table.append(body); result.replaceChildren(table);
        confirm.disabled = false;
      } catch (error) {
        if (current === generation) result.replaceChildren(element("p", error.message, "message error"));
      } finally { busy = false; preview.disabled = false; }
    });
    apply.addEventListener("click", async () => {
      if (busy || !token || !confirm.checked) return;
      const reviewedToken = token;
      busy = true; apply.disabled = true; preview.disabled = true;
      try {
        const payload = await options.request("/api/session-intent-apply", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ preview_token: reviewedToken, confirm: true }) });
        invalidate();
        result.append(element("p", `Label overrides applied. Audit ${payload.audit_id ?? "recorded"}.`, "message"));
        try {
          if (kind === "session") await options.refreshActivity($("session-project").value.trim());
          else if ($("workstream-project").value.trim()) await options.loadWorkstreams();
        } catch (error) {
          result.append(element("p", `The override was saved, but refreshing the list failed: ${error.message}`, "message error"));
        }
      } catch (error) {
        invalidate(); result.append(element("p", `${error.message} Review a new preview before applying again.`, "message error"));
      } finally { busy = false; preview.disabled = false; }
    });
  }
  function init(options) {
    intents.forEach((value) => $("session-intent-filter").append(new Option(value.toUpperCase(), value)));
    for (const id of ["session-intent-filter", "session-date-from", "session-date-to"]) {
      $(id).addEventListener("change", () => {
        options.state.sessionRequestGeneration += 1;
        options.state.selectedSession = null;
        options.state.turns = [];
        $("turn-reader").replaceChildren(element("div", "Select a session matching the current filters.", "reader-empty"));
        options.refreshActivity($("session-project").value.trim()).catch((error) => {
          $("session-list").replaceChildren(element("p", error.message, "message error"));
        });
      });
    }
    editor("session", $("session-label-editor"), options);
    editor("workstream", $("workstream-label-editor"), options);
  }
  return { init, addFilters, renderItems, dateEpoch };
})();
if (typeof module !== "undefined") module.exports = SessionLabels;
