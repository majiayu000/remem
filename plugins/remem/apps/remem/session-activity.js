"use strict";

const { assertLocalPostAllowed } = require("./request-security");

function apiRoute(pathname, params = {}) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== "") query.set(key, String(value));
  }
  const encoded = query.toString();
  return encoded ? `${pathname}?${encoded}` : pathname;
}

function createSessionActivityBackend(api) {
  return {
    async sessionIntentPreview(input) {
      return api.request("/api/v1/session-intent/preview", {
        method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(input)
      });
    },
    async sessionIntentApply(input) {
      return api.request("/api/v1/session-intent/apply", {
        method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(input)
      });
    },
    async activitySessions(params = {}) {
      return api.request(apiRoute("/api/v1/session-activity/sessions", params));
    },
    async sessionActivity(params = {}) {
      return api.request(apiRoute("/api/v1/session-activity", params));
    },
    async sessionTurn(id) {
      return api.request(`/api/v1/session-activity/${encodeURIComponent(String(id))}`);
    },
    async sessionStats(params = {}) {
      return api.request(apiRoute("/api/v1/session-stats", params));
    },
    async projectSession(input) {
      return api.request("/api/v1/session-activity/project", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input)
      });
    }
  };
}

async function handleSessionActivityRoute({ req, res, url, backend, jsonResponse, readJsonBody }) {
  if (req.method === "POST" && ["/api/session-intent-preview", "/api/session-intent-apply"].includes(url.pathname)) {
    assertLocalPostAllowed(req);
    const input = await readJsonBody(req);
    const result = url.pathname.endsWith("-preview")
      ? await backend.sessionIntentPreview(input) : await backend.sessionIntentApply(input);
    jsonResponse(res, 200, result);
    return true;
  }
  if (req.method === "GET" && url.pathname === "/api/activity-sessions") {
    jsonResponse(res, 200, await backend.activitySessions(Object.fromEntries(url.searchParams)));
    return true;
  }
  if (req.method === "GET" && url.pathname === "/api/session-activity") {
    jsonResponse(res, 200, await backend.sessionActivity(Object.fromEntries(url.searchParams)));
    return true;
  }
  if (req.method === "GET" && url.pathname === "/api/session-turn") {
    jsonResponse(res, 200, await backend.sessionTurn(url.searchParams.get("id")));
    return true;
  }
  if (req.method === "GET" && url.pathname === "/api/session-stats") {
    jsonResponse(res, 200, await backend.sessionStats(Object.fromEntries(url.searchParams)));
    return true;
  }
  if (req.method === "POST" && url.pathname === "/api/project-session") {
    assertLocalPostAllowed(req);
    jsonResponse(res, 200, await backend.projectSession(await readJsonBody(req)));
    return true;
  }
  return false;
}

module.exports = {
  apiRoute,
  createSessionActivityBackend,
  handleSessionActivityRoute
};
