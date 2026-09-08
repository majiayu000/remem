#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const { callTool, createServer, toolDescriptors } = require("./server");
const { apiRoute, createSessionActivityBackend } = require("./session-activity");

async function withActivityServer(backend, run) {
  const server = createServer({ backend });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    await run(base);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

test("activity backend maps bounded app calls to native API routes", async () => {
  const calls = [];
  const api = {
    async request(route, options) {
      calls.push({ route, options });
      return { ok: true };
    }
  };
  const backend = createSessionActivityBackend(api);

  await backend.activitySessions({ project: "/repo", limit: 20 });
  await backend.sessionActivity({ session_id: "session-1" });
  await backend.sessionTurn(31);
  await backend.sessionStats({ since_epoch: 100 });
  await backend.projectSession({ source_root: "local", project: "/repo", session_id: "session-1" });

  assert.equal(calls[0].route, "/api/v1/session-activity/sessions?project=%2Frepo&limit=20");
  assert.equal(calls[1].route, "/api/v1/session-activity?session_id=session-1");
  assert.equal(calls[2].route, "/api/v1/session-activity/31");
  assert.equal(calls[3].route, "/api/v1/session-stats?since_epoch=100");
  assert.equal(calls[4].route, "/api/v1/session-activity/project");
  assert.equal(calls[4].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[4].options.body), {
    source_root: "local",
    project: "/repo",
    session_id: "session-1"
  });
});

test("activity app routes expose sessions, turns, stats, detail, and projection", async () => {
  const backend = {
    async activitySessions() {
      return { data: [{ session_id: "session-1", projected_turn_count: 1 }] };
    },
    async sessionActivity() {
      return { data: [{ id: 31, result_status: "done" }] };
    },
    async sessionTurn(id) {
      return { data: { id: Number(id), user_said: "Build it" } };
    },
    async sessionStats() {
      return { data: { sessions: 1, turns: 1, actions: 2 } };
    },
    async projectSession(input) {
      return { data: { changed: true, turn_count: 1, source_digest: input.session_id } };
    },
    stop() {}
  };

  await withActivityServer(backend, async (base) => {
    const sessions = await fetch(`${base}/api/activity-sessions`).then((response) => response.json());
    const turns = await fetch(`${base}/api/session-activity`).then((response) => response.json());
    const detail = await fetch(`${base}/api/session-turn?id=31`).then((response) => response.json());
    const stats = await fetch(`${base}/api/session-stats`).then((response) => response.json());
    const projected = await fetch(`${base}/api/project-session`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: base },
      body: JSON.stringify({ source_root: "local", project: "/repo", session_id: "session-1" })
    }).then((response) => response.json());

    assert.equal(sessions.data[0].projected_turn_count, 1);
    assert.equal(turns.data[0].result_status, "done");
    assert.equal(detail.data.id, 31);
    assert.equal(stats.data.actions, 2);
    assert.equal(projected.data.source_digest, "session-1");
  });
});

test("apiRoute omits empty filters", () => {
  assert.equal(apiRoute("/api/v1/session-stats", { project: "", limit: null }), "/api/v1/session-stats");
});

test("embedded activity tools dispatch to the native API backend", async () => {
  const calls = [];
  const backend = {
    async activitySessions(args) { calls.push(["sessions", args]); return { meta: { count: 1 }, data: [] }; },
    async sessionActivity(args) { calls.push(["turns", args]); return { meta: { count: 1 }, data: [] }; },
    async sessionTurn(id) { calls.push(["turn", id]); return { data: { id } }; },
    async sessionStats(args) { calls.push(["stats", args]); return { data: { sessions: 1 } }; },
    async projectSession(args) { calls.push(["project", args]); return { data: { turn_count: 1 } }; }
  };

  for (const [name, args] of [
    ["remem_activity_sessions", { project: "/repo" }],
    ["remem_session_activity", { session_id: "s1" }],
    ["remem_session_turn", { id: 31 }],
    ["remem_session_stats", { since_epoch: 100 }],
    ["remem_project_session", { source_root: "local", project: "/repo", session_id: "s1" }]
  ]) {
    const descriptor = toolDescriptors().find((tool) => tool.name === name);
    assert.ok(descriptor, `missing descriptor ${name}`);
    assert.equal(descriptor._meta["openai/widgetAccessible"], true);
    const result = await callTool(backend, name, args);
    assert.ok(result.structuredContent);
  }
  assert.deepEqual(calls.map(([kind]) => kind), ["sessions", "turns", "turn", "stats", "project"]);
});

test("project filter invalidates an in-flight session request before refresh", () => {
  const widget = fs.readFileSync(path.join(__dirname, "public", "widget.js"), "utf8");
  const handler = widget.match(/\$\("session-project"\)\.addEventListener\("input", \(\) => \{([\s\S]*?)\n\}\);/);
  assert.ok(handler, "missing project-filter input handler");
  const invalidation = handler[1].indexOf("state.sessionRequestGeneration += 1");
  const refresh = handler[1].indexOf("setTimeout(() => refreshActivity");
  assert.ok(invalidation >= 0, "filter change must invalidate the selected session request");
  assert.ok(refresh > invalidation, "invalidation must happen synchronously before delayed refresh");
  assert.match(handler[1], /state\.selectedSession = null/);
  assert.match(handler[1], /state\.turns = \[\]/);
  assert.match(widget, /page < 5 && sessions\.length < 50/);
  assert.match(widget, /params\.set\("cursor", cursor\)/);
  assert.match(widget, /state\.sessionsTruncated = hasMore/);
});

test("label override bridge preserves reviewed nullable fields and requires local POST", async () => {
  const calls = [];
  const backend = createSessionActivityBackend({ async request(route, options) {
    calls.push({ route, body: JSON.parse(options.body) });
    return route.endsWith("preview") ? { preview_token: "reviewed", changes: [] } : { audit_id: 42 };
  } });
  backend.stop = () => {};
  const proposal = { targets: [{ kind: "session", id: 12 }], session_intent: null, session_topic: null, reason: "Clear an incorrect label" };
  await withActivityServer(backend, async (base) => {
    const options = (body, origin = base) => ({ method: "POST", headers: { "content-type": "application/json", origin }, body: JSON.stringify(body) });
    const preview = await fetch(`${base}/api/session-intent-preview`, options(proposal));
    assert.equal(preview.status, 200);
    assert.equal((await preview.json()).preview_token, "reviewed");
    const apply = await fetch(`${base}/api/session-intent-apply`, options({ preview_token: "reviewed", confirm: true }));
    assert.equal((await apply.json()).audit_id, 42);
    const denied = await fetch(`${base}/api/session-intent-apply`, options({ preview_token: "reviewed", confirm: true }, "https://attacker.example"));
    assert.equal(denied.status, 403);
    const asset = await fetch(`${base}/session-labels.js`);
    assert.equal(asset.status, 200);
  });
  assert.deepEqual(calls, [
    { route: "/api/v1/session-intent/preview", body: proposal },
    { route: "/api/v1/session-intent/apply", body: { preview_token: "reviewed", confirm: true } }
  ]);
  for (const [name, args] of [["remem_session_intent_preview", proposal], ["remem_session_intent_apply", { preview_token: "reviewed", confirm: true }]]) {
    const descriptor = toolDescriptors().find((tool) => tool.name === name);
    assert.equal(descriptor._meta["openai/widgetAccessible"], true);
    assert.equal(descriptor.annotations.readOnlyHint, false);
    assert.ok((await callTool(backend, name, args)).structuredContent);
  }
});

test("session label filters use Shanghai midnight and survive embedded tool dispatch", async () => {
  const { dateEpoch } = require("./public/session-labels");
  assert.equal(dateEpoch("2025-01-01"), 1735660800);
  const calls = [];
  const backend = createSessionActivityBackend({ async request(route) { calls.push(route); return {}; } });
  const filters = { session_intent: "fix", since_epoch: dateEpoch("2025-01-01"), until_epoch: dateEpoch("2025-01-02"), cursor: "opaque" };
  await callTool(backend, "remem_activity_sessions", filters);
  const query = new URL(calls[0], "http://localhost").searchParams;
  assert.equal(query.get("session_intent"), "fix");
  assert.equal(query.get("since_epoch"), "1735660800");
  assert.equal(query.get("until_epoch"), "1735747200");
  assert.equal(query.get("cursor"), "opaque");
});
