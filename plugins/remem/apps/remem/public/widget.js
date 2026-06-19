const state = { snapshot: null, results: [], rawError: "", selected: null };
const $ = (id) => document.getElementById(id);
function cls(value, truthy) {
  value.classList.toggle("hidden", !truthy);
}
function valueAt(object, path, fallback = 0) {
  return path.split(".").reduce((acc, key) => acc && acc[key], object) ?? fallback;
}
async function request(route, options = {}) {
  if (canCallHostTool(route)) {
    return requestViaHostTool(route, options);
  }
  const response = await fetch(route, options);
  const payload = await response.json();
  if (!response.ok) {
    throw new Error(payload.error?.message || `Request failed with ${response.status}`);
  }
  return payload;
}
function canCallHostTool(route) {
  return typeof window.openai?.callTool === "function" && String(route).startsWith("/api/");
}
async function requestViaHostTool(route, options = {}) {
  const method = String(options.method || "GET").toUpperCase();
  const url = new URL(route, "http://remem.local");
  const result = await callHostTool(apiToolName(url.pathname, method), apiToolArgs(url, method, options));
  return result?.structuredContent ?? result;
}
async function callHostTool(name, args) {
  if (!name) throw new Error("No host tool is available for this action");
  return window.openai.callTool(name, args);
}
function apiToolName(pathname, method) {
  if (method === "GET" && pathname === "/api/status") return "remem_dashboard";
  if (method === "GET" && pathname === "/api/search") return "remem_search";
  if (method === "GET" && pathname === "/api/memory") return "remem_get_memory";
  if (method === "GET" && pathname === "/api/activation-plan") return "remem_activation_plan";
  if (method === "GET" && pathname === "/api/timeline-around") return "remem_timeline_around";
  if (method === "GET" && pathname === "/api/timeline-report") return "remem_timeline_report";
  if (method === "GET" && pathname === "/api/workstreams") return "remem_workstreams_list";
  if (method === "POST" && pathname === "/api/save") return "remem_save_memory";
  if (method === "POST" && pathname === "/api/governance-preview") return "remem_governance_preview";
  if (method === "POST" && pathname === "/api/workstream-update") return "remem_workstream_update";
  return null;
}
function apiToolArgs(url, method, options) {
  if (method === "GET" && url.pathname === "/api/search") {
    return {
      query: url.searchParams.get("query") || "",
      project: url.searchParams.get("project") || undefined,
      type: url.searchParams.get("type") || undefined,
      limit: numericParam(url.searchParams, "limit", 12),
      offset: numericParam(url.searchParams, "offset", 0),
      include_stale: booleanParam(url.searchParams, "include_stale"),
      multi_hop: booleanParam(url.searchParams, "multi_hop"),
      include_raw_archive: booleanParam(url.searchParams, "include_raw_archive")
    };
  }
  if (method === "GET" && url.pathname === "/api/memory") {
    return { id: Number(url.searchParams.get("id")) };
  }
  if (method === "GET" && url.pathname === "/api/timeline-around") {
    return {
      anchor: optionalNumericParam(url.searchParams, "anchor"),
      query: url.searchParams.get("query") || undefined,
      project: url.searchParams.get("project") || undefined,
      depth_before: optionalNumericParam(url.searchParams, "depth_before"),
      depth_after: optionalNumericParam(url.searchParams, "depth_after")
    };
  }
  if (method === "GET" && url.pathname === "/api/timeline-report") {
    return {
      project: url.searchParams.get("project") || undefined,
      full: booleanParam(url.searchParams, "full")
    };
  }
  if (method === "GET" && url.pathname === "/api/workstreams") {
    return {
      project: url.searchParams.get("project") || undefined,
      status: url.searchParams.get("status") || undefined
    };
  }
  if (method === "POST" && url.pathname === "/api/save") {
    return JSON.parse(options.body || "{}");
  }
  if (method === "POST" && url.pathname === "/api/governance-preview") {
    return JSON.parse(options.body || "{}");
  }
  if (method === "POST" && url.pathname === "/api/workstream-update") {
    return JSON.parse(options.body || "{}");
  }
  return {};
}
function numericParam(params, key, fallback) {
  const value = params.get(key);
  if (value === null || value === "") return fallback;
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}
function booleanParam(params, key) {
  const value = params.get(key);
  if (value === null || value === "") return undefined;
  return value === "true" || value === "1";
}
function optionalNumericParam(params, key) {
  const value = params.get(key);
  if (value === null || value === "") return undefined;
  const number = Number(value);
  return Number.isFinite(number) ? number : undefined;
}
function renderRuntime(snapshot) {
  const runtime = snapshot.runtime || {};
  const selected = runtime.selected || runtime.pathMatch;
  const selectedOk = Boolean(runtime.selected);
  const binarySource = selected?.source || "needs setup";
  $("runtime-status").innerHTML = `
    <span class="pill ${selectedOk ? "ok" : "warn"}">${escapeHtml(binarySource)}</span>
    <div class="mono">${escapeHtml(selected?.path || runtime.managedBinary || "No runtime found")}</div>
    <div class="subtle">version ${escapeHtml(selected?.version || snapshot.expected_version || "unknown")} · schema ${escapeHtml(String(selected?.schemaVersion || "-"))}</div>
    <div class="subtle">plugin data</div>
    <div class="mono">${escapeHtml(snapshot.plugin_data || runtime.pluginData || "")}</div>
  `;
}
function renderCounts(status) {
  $("count-memories").textContent = valueAt(status, "totals.memories");
  $("count-observations").textContent = valueAt(status, "totals.observations");
  $("count-raw").textContent = valueAt(status, "totals.raw_messages");
  const queue = valueAt(status, "capture_pipeline.extract_todo") +
    valueAt(status, "pending_observations.ready") +
    valueAt(status, "jobs.pending");
  $("count-queue").textContent = queue;
}
function renderActivation(activation) {
  const stateClass = activation?.mentions_hooks ? "warn" : "ok";
  const hookLines = activation?.packaged_hooks?.commands?.map(
    (hook) => `${hook.event}: ${hook.command}`
  ) || [];
  $("activation-summary").innerHTML = `
    <span class="pill ${stateClass}">${activation?.mentions_hooks ? "hooks preview" : "no hook plan"}</span>
    <div class="subtle">${activation?.line_count || 0} dry-run line(s)</div>
  `;
  $("activation-plan").textContent = [
    activation?.plan_text || "",
    hookLines.length ? `Packaged plugin hooks:\n${hookLines.join("\n")}` : ""
  ].filter(Boolean).join("\n\n");
}
function renderHealth(snapshot) {
  const doctor = snapshot.doctor || {};
  const failed = Number(doctor.fails || 0);
  const warned = Number(doctor.warns || 0);
  const pill = $("health-pill");
  pill.className = `pill ${failed ? "bad" : warned ? "warn" : "ok"}`;
  pill.textContent = failed ? `${failed} failing check(s)` : warned ? `${warned} warning(s)` : "Healthy";
}
async function refresh() {
  try {
    state.snapshot = await request("/api/status");
    renderRuntime(state.snapshot);
    renderCounts(state.snapshot.status || {});
    renderActivation(state.snapshot.activation || {});
    renderHealth(state.snapshot);
  } catch (error) {
    $("runtime-status").innerHTML = `<div class="message error">${escapeHtml(error.message)}</div>`;
    $("health-pill").className = "pill bad";
    $("health-pill").textContent = "Error";
  }
}
async function runSearch() {
  const query = $("search-query").value.trim();
  if (!query) return;
  const params = new URLSearchParams({
    query,
    limit: "12",
    include_stale: "true"
  });
  const type = $("search-type").value;
  if (type) params.set("type", type);
  if ($("include-raw").checked) params.set("include_raw_archive", "true");
  $("results").innerHTML = `<li class="result"><span class="subtle">Searching...</span></li>`;
  $("detail").innerHTML = "";
  const payload = await request(`/api/search?${params.toString()}`);
  state.results = searchResults(payload);
  state.rawError = payload.raw_hits_error || "";
  $("result-count").textContent = `${state.results.length}`;
  renderResults();
}
function searchResults(payload) {
  const memories = (payload.data || []).map((item) => ({
    ...item,
    result_kind: "memory",
    result_id: String(item.id)
  }));
  const rawHits = (payload.raw_hits || []).map((item) => ({
    ...item,
    id: `raw-${item.id}`,
    raw_id: item.id,
    result_kind: "raw_archive",
    result_id: `raw-${item.id}`,
    title: `${item.role || "raw"} message from raw archive`,
    content: item.preview || "",
    memory_type: "raw archive",
    status: "raw",
    scope: item.source || "",
    project: item.project || ""
  }));
  return memories.concat(rawHits);
}
function renderResults() {
  const results = $("results");
  const warning = state.rawError ? `
    <li class="result raw-error">
      <div class="result-title">
        <span>Raw archive fallback failed</span>
        <span class="pill warn">warning</span>
      </div>
      <div class="preview">${escapeHtml(state.rawError)}</div>
    </li>
  ` : "";
  if (!state.results.length) {
    results.innerHTML = warning || `<li class="result"><span class="subtle">No memories found</span></li>`;
    return;
  }
  results.innerHTML = warning + state.results.map((item) => `
    <li class="result ${item.result_kind === "raw_archive" ? "raw-result" : ""}" data-id="${escapeHtml(item.result_id || item.id)}">
      <div class="result-title">
        <span>${escapeHtml(item.title || `Memory ${item.id}`)}</span>
        <span class="pill">${escapeHtml(item.memory_type || item.type || "memory")}</span>
      </div>
      <div class="preview">${escapeHtml(item.content || item.preview || "")}</div>
    </li>
  `).join("");
}
async function selectResult(id) {
  document.querySelectorAll(".result").forEach((node) => {
    node.classList.toggle("active", node.dataset.id === String(id));
  });
  $("detail").innerHTML = `<div class="message">Loading memory ${id}...</div>`;
  const cached = state.results.find((item) => String(item.result_id || item.id) === String(id));
  if (cached?.result_kind === "raw_archive") {
    state.selected = cached;
    $("detail-type").textContent = "raw archive";
    $("detail").innerHTML = `
      <div class="stack">
        <h3>${escapeHtml(cached.title)}</h3>
        <div class="row">
          <span class="pill">${escapeHtml(cached.role || "")}</span>
          <span class="pill">${escapeHtml(cached.source || "")}</span>
        </div>
        <div class="subtle mono">${escapeHtml(cached.project || "")}</div>
        <div class="detail-text">${escapeHtml(cached.preview || cached.content || "")}</div>
      </div>
    `;
    return;
  }
  const item = await request(`/api/memory?id=${encodeURIComponent(id)}`);
  state.selected = item;
  $("detail-type").textContent = item.memory_type || "memory";
  $("detail").innerHTML = `
    <div class="stack">
      <h3>${escapeHtml(item.title || `Memory ${item.id}`)}</h3>
      <div class="row">
        <span class="pill">${escapeHtml(item.status || "")}</span>
        <span class="pill">${escapeHtml(item.scope || "")}</span>
      </div>
      <div class="subtle mono">${escapeHtml(item.project || "")}</div>
      <div class="detail-text">${escapeHtml(item.content || "")}</div>
    </div>
  `;
}
async function saveMemory() {
  const text = $("save-text").value.trim();
  const target = $("save-result");
  if (!text) {
    target.textContent = "Memory text is required.";
    target.className = "message error";
    return;
  }
  const payload = {
    text,
    title: $("save-title").value.trim() || undefined,
    project: $("save-project").value.trim() || undefined,
    topic_key: $("save-topic").value.trim() || undefined,
    memory_type: $("save-type").value,
    scope: $("save-scope").value
  };
  target.textContent = "Saving...";
  target.className = "message";
  try {
    const saved = await request("/api/save", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload)
    });
    target.textContent = `Saved memory ${saved.id} (${saved.operation}).`;
    target.className = "message";
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
async function refreshPlan() {
  $("activation-plan").textContent = "Generating...";
  const plan = await request("/api/activation-plan");
  renderActivation(plan);
}
function governancePayload() {
  const ids = $("govern-ids").value
    .split(/[,\s]+/)
    .map((value) => value.trim())
    .filter(Boolean)
    .map(Number);
  if (ids.some((id) => !Number.isInteger(id) || id <= 0)) {
    throw new Error("Memory IDs must be positive integers.");
  }
  return {
    action: $("govern-action").value,
    ids,
    project: $("govern-project").value.trim() || undefined,
    query: $("govern-query").value.trim() || undefined,
    memory_type: $("govern-type").value.trim() || undefined,
    status: $("govern-status").value.trim() || undefined,
    limit: Number($("govern-limit").value || 50),
    actor: "codex-remem-app"
  };
}
async function previewGovernance() {
  const target = $("govern-result");
  target.textContent = "Previewing...";
  target.className = "message";
  try {
    const preview = await request("/api/governance-preview", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(governancePayload())
    });
    const affected = preview.affected || [];
    target.className = "message";
    target.innerHTML = `
      <div><strong>${affected.length}</strong> affected memory result(s); dry_run=${escapeHtml(preview.dry_run)}</div>
      ${affected.map((item) => `
        <div class="detail-text mono">#${escapeHtml(item.id)} ${escapeHtml(item.previous_status)} -> ${escapeHtml(item.new_status)} ${escapeHtml(item.title || "")}</div>
      `).join("")}
    `;
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
function setOptionalParam(params, key, value) {
  const text = String(value || "").trim();
  if (text) params.set(key, text);
}
function requireText(id, name) {
  const text = $(id).value.trim();
  if (!text) throw new Error(`${name} is required.`);
  return text;
}
async function loadTimelineAround() {
  const target = $("timeline-result");
  const params = new URLSearchParams();
  setOptionalParam(params, "project", $("timeline-project").value);
  setOptionalParam(params, "anchor", $("timeline-anchor").value);
  setOptionalParam(params, "query", $("timeline-query").value);
  setOptionalParam(params, "depth_before", $("timeline-before").value);
  setOptionalParam(params, "depth_after", $("timeline-after").value);
  if (!params.has("anchor") && !params.has("query")) {
    target.textContent = "Anchor ID or query is required.";
    target.className = "message error";
    return;
  }
  target.textContent = "Loading...";
  target.className = "message";
  try {
    renderTimeline(await request(`/api/timeline-around?${params.toString()}`));
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
async function loadTimelineReport() {
  const target = $("timeline-result");
  const params = new URLSearchParams({ project: requireText("timeline-project", "Project") });
  target.textContent = "Loading...";
  target.className = "message";
  try {
    renderTimeline(await request(`/api/timeline-report?${params.toString()}`));
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
function renderTimeline(payload) {
  const target = $("timeline-result");
  const rows = payload.results || payload.timeline || payload.observations || [];
  const overview = payload.report?.overview || payload.overview;
  target.className = "message";
  target.innerHTML = `
    ${overview ? `<div><strong>${escapeHtml(overview.total_observations ?? rows.length)}</strong> observation(s)</div>` : ""}
    ${rows.map((item) => `
      <div class="detail-text">
        <div><strong>${escapeHtml(item.title || item.summary || `Observation ${item.id || ""}`)}</strong></div>
        <div class="subtle mono">${escapeHtml(item.type || item.memory_type || "")} ${escapeHtml(item.created_at || item.created_at_epoch || "")}</div>
        <div>${escapeHtml(item.content || item.text || item.preview || "")}</div>
      </div>
    `).join("")}
    <pre>${escapeHtml(JSON.stringify(payload.report || payload, null, 2))}</pre>
  `;
}
async function loadWorkstreams() {
  const target = $("workstream-result");
  const params = new URLSearchParams({ project: requireText("workstream-project", "Project") });
  setOptionalParam(params, "status", $("workstream-status").value);
  target.textContent = "Loading...";
  target.className = "message";
  try {
    renderWorkstreams(await request(`/api/workstreams?${params.toString()}`));
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
function workstreamUpdatePayload() {
  const payload = {
    id: Number(requireText("workstream-id", "Workstream ID")),
    project: requireText("workstream-project", "Project"),
    confirm: $("workstream-confirm").checked
  };
  if (!Number.isInteger(payload.id) || payload.id <= 0) throw new Error("Workstream ID must be a positive integer.");
  const status = $("workstream-new-status").value;
  const nextAction = $("workstream-next-action").value.trim();
  const blockers = $("workstream-blockers").value.trim();
  if (status) payload.status = status;
  if (nextAction) payload.next_action = nextAction;
  if (blockers) payload.blockers = blockers;
  if (!payload.status && !payload.next_action && !payload.blockers) {
    throw new Error("Status, next action, or blockers is required.");
  }
  if (!payload.confirm) throw new Error("Confirm update is required.");
  return payload;
}
async function updateWorkstream() {
  const target = $("workstream-result");
  target.textContent = "Updating...";
  target.className = "message";
  try {
    const updated = await request("/api/workstream-update", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(workstreamUpdatePayload())
    });
    target.className = "message";
    target.innerHTML = `<div>Updated workstream ${escapeHtml(updated.id || "")}</div><pre>${escapeHtml(JSON.stringify(updated, null, 2))}</pre>`;
  } catch (error) {
    target.textContent = error.message;
    target.className = "message error";
  }
}
function renderWorkstreams(payload) {
  const target = $("workstream-result");
  const rows = payload.workstreams || payload.data || [];
  target.className = "message";
  target.innerHTML = `
    <div><strong>${escapeHtml(payload.count ?? rows.length)}</strong> workstream(s)</div>
    ${rows.map((item) => `
      <div class="detail-text">
        <div><strong>#${escapeHtml(item.id)} ${escapeHtml(item.title || item.summary || "")}</strong></div>
        <div class="row">
          <span class="pill">${escapeHtml(item.status || "")}</span>
          <span class="pill">${escapeHtml(item.updated_at || item.last_activity_epoch || "")}</span>
        </div>
        <div>${escapeHtml(item.next_action || "")}</div>
        <div class="subtle">${escapeHtml(item.blockers || "")}</div>
      </div>
    `).join("")}
  `;
}
function switchView(name) {
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.classList.toggle("active", tab.dataset.view === name);
  });
  cls($("search-view"), name === "search");
  cls($("save-view"), name === "save");
  cls($("governance-view"), name === "governance");
  cls($("timeline-view"), name === "timeline");
  cls($("workstreams-view"), name === "workstreams");
  cls($("activation-view"), name === "activation");
}
function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
function receiveHostResult(event) {
  const message = event.data;
  if (!message || message.method !== "ui/notifications/tool-result") return;
  const data = message.params?.structuredContent;
  if (!data) return;
  state.snapshot = data;
  renderRuntime(data);
  renderCounts(data.status || {});
  renderActivation(data.activation || {});
  renderHealth(data);
}
window.addEventListener("message", receiveHostResult);
$("refresh").addEventListener("click", refresh);
$("search-button").addEventListener("click", () => runSearch().catch((error) => {
  $("results").innerHTML = `<li class="result"><span class="message error">${escapeHtml(error.message)}</span></li>`;
}));
$("search-query").addEventListener("keydown", (event) => {
  if (event.key === "Enter") runSearch();
});
$("results").addEventListener("click", (event) => {
  const item = event.target.closest(".result");
  if (item?.dataset.id) selectResult(item.dataset.id);
});
$("save-button").addEventListener("click", saveMemory);
$("govern-button").addEventListener("click", previewGovernance);
$("timeline-around-button").addEventListener("click", loadTimelineAround);
$("timeline-report-button").addEventListener("click", () => loadTimelineReport().catch((error) => {
  $("timeline-result").textContent = error.message;
  $("timeline-result").className = "message error";
}));
$("workstream-list-button").addEventListener("click", () => loadWorkstreams().catch((error) => {
  $("workstream-result").textContent = error.message;
  $("workstream-result").className = "message error";
}));
$("workstream-update-button").addEventListener("click", updateWorkstream);
$("plan-button").addEventListener("click", () => refreshPlan().catch((error) => {
  $("activation-plan").textContent = error.message;
}));
document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => switchView(tab.dataset.view));
});
refresh();
