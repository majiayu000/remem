#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  RememApiProxy,
  activationSummary,
  buildSnapshot,
  createServer,
  handleJsonRpc,
  parseArgs,
  toolDescriptors,
  UI_RESOURCE
} = require("./server");
const { governancePreviewArgs } = require("./governance");
const { createTraceBackend } = require("./trace");
const pluginManifest = require("../../.codex-plugin/plugin.json");

function fakeBackend() {
  return {
    async runtime() {
      return {
        expectedVersion: pluginManifest.version,
        pluginData: "/tmp/remem-plugin",
        managedBinary: "/tmp/remem-plugin/bin/remem",
        selected: {
          source: "managed",
          path: "/tmp/remem-plugin/bin/remem",
          version: pluginManifest.version,
          schemaVersion: 34
        },
        candidates: []
      };
    },
    async status() {
      return {
        version: `${pluginManifest.version} (schema v34)`,
        totals: {
          memories: 3,
          observations: 5,
          raw_messages: 8
        },
        capture_pipeline: {
          extract_todo: 1
        },
        pending_observations: {
          ready: 2
        },
        jobs: {
          pending: 4
        }
      };
    },
    async doctor() {
      return {
        status: "ok",
        fails: 0,
        warns: 0,
        checks: []
      };
    },
    async search(params) {
      return {
        data: [
          {
            id: 7,
            title: `Result for ${params.query}`,
            content: "Compact preview",
            memory_type: "decision"
          }
        ],
        meta: {
          count: 1,
          limit: 10,
          offset: 0,
          has_more: false
        }
      };
    },
    async memory(id) {
      return {
        id: Number(id),
        title: "Full memory",
        content: "Detailed memory body",
        memory_type: "decision",
        status: "active",
        scope: "project"
      };
    },
    async save(input) {
      return {
        id: 9,
        status: "saved",
        memory_type: input.memory_type || "decision",
        operation: "add",
        next_step: {
          tool: "get_observations",
          ids: [9],
          source: "memory"
        }
      };
    },
    async activationPlan() {
      return activationSummary("Would write Codex hooks\nWould update MCP config\n");
    },
    async governancePreview(input) {
      return {
        dry_run: true,
        action: input.action || "stale",
        reason: input.reason || null,
        requested: input,
        affected: [
          {
            id: Number(input.ids?.[0] || 7),
            title: "Old decision",
            previous_status: "active",
            new_status: "stale"
          }
        ]
      };
    },
    async currentState(input) {
      return {
        state_key: input.state_key,
        status: "current",
        current: {
          id: 11,
          title: "Current deployment target"
        },
        history: []
      };
    },
    async commitLookup(input) {
      return [
        {
          git: {
            sha: input.sha,
            short_sha: String(input.sha).slice(0, 7),
            message: "Wire Remem app trace tools"
          },
          sessions: []
        }
      ];
    },
    async sessionCommits(input) {
      return [
        {
          link: {
            session_id: input.session_id,
            source: "test"
          },
          git: {
            sha: "abcdef123456",
            short_sha: "abcdef1"
          }
        }
      ];
    },
    async timelineAround(input) {
      return {
        anchor_id: Number(input.anchor || 15),
        query: input.query || null,
        project: input.project || null,
        count: 1,
        results: [
          {
            id: Number(input.anchor || 15),
            title: "Release manifest",
            type: "decision"
          }
        ]
      };
    },
    async timelineReport(input) {
      return {
        project: input.project,
        full: input.full === true || input.full === "true",
        report: {
          overview: {
            total_observations: 2
          },
          activity_by_type: [],
          token_economics: {}
        }
      };
    },
    async workstreamsList(input) {
      return {
        project: input.project,
        status: input.status || null,
        count: 1,
        workstreams: [
          {
            id: 21,
            title: "Wire app routes",
            status: "active"
          }
        ]
      };
    },
    async workstreamUpdate(input) {
      return {
        id: Number(input.id),
        project: input.project,
        updated: input.confirm === true
      };
    },
    stop() {}
  };
}

async function withServer(fn, backend = fakeBackend()) {
  const server = createServer({ backend });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  try {
    await fn(`http://127.0.0.1:${port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

test("tool descriptors expose the dashboard UI resource", () => {
  const dashboard = toolDescriptors().find((tool) => tool.name === "remem_dashboard");

  assert.equal(dashboard._meta.ui.resourceUri, UI_RESOURCE);
  assert.deepEqual(dashboard._meta.ui.visibility, ["model", "app"]);
  assert.equal(dashboard._meta["openai/outputTemplate"], UI_RESOURCE);
  assert.equal(dashboard._meta["openai/widgetAccessible"], true);
  assert.equal(dashboard.annotations.readOnlyHint, true);
});

test("widget-callable tools are exposed to the app surface", () => {
  for (const name of [
    "remem_dashboard",
    "remem_search",
    "remem_get_memory",
    "remem_save_memory",
    "remem_activation_plan",
    "remem_governance_preview",
    "remem_current_state",
    "remem_commit_lookup",
    "remem_session_commits",
    "remem_timeline_around",
    "remem_timeline_report",
    "remem_workstreams_list",
    "remem_workstream_update"
  ]) {
    const descriptor = toolDescriptors().find((tool) => tool.name === name);
    assert.deepEqual(descriptor._meta.ui.visibility, ["model", "app"]);
    assert.equal(descriptor._meta["openai/widgetAccessible"], true);
  }
  const timelineAround = toolDescriptors().find((tool) => tool.name === "remem_timeline_around");
  assert.deepEqual(timelineAround.inputSchema.anyOf, [
    { required: ["anchor"] },
    { required: ["query"] }
  ]);
  const workstreamUpdate = toolDescriptors().find((tool) => tool.name === "remem_workstream_update");
  assert.deepEqual(workstreamUpdate.inputSchema.anyOf, [
    { required: ["status"] },
    { required: ["next_action"] },
    { required: ["blockers"] }
  ]);
});

test("JSON-RPC tools/list and dashboard call return structured content", async () => {
  const tools = await handleJsonRpc(fakeBackend(), {
    id: 1,
    method: "tools/list",
    params: {}
  });
  assert.ok(tools.tools.some((tool) => tool.name === "remem_save_memory"));
  assert.ok(tools.tools.some((tool) => tool.name === "remem_governance_preview"));
  assert.ok(tools.tools.some((tool) => tool.name === "remem_current_state"));
  assert.ok(tools.tools.some((tool) => tool.name === "remem_commit_lookup"));

  const result = await handleJsonRpc(fakeBackend(), {
    id: 2,
    method: "tools/call",
    params: { name: "remem_dashboard", arguments: {} }
  });

  assert.equal(result.structuredContent.expected_version, pluginManifest.version);
  assert.equal(result.structuredContent.status.totals.memories, 3);
});

test("HTTP API serves widget, status, search, memory detail, and save", async () => {
  await withServer(async (base) => {
    const widget = await fetch(`${base}/widget.html`);
    assert.equal(widget.status, 200);
    const widgetHtml = await widget.text();
    assert.match(widgetHtml, /Remem Dashboard/);
    assert.match(widgetHtml, /href="\/widget\.css"/);
    assert.match(widgetHtml, /src="\/widget\.js"/);

    const widgetCss = await fetch(`${base}/widget.css`);
    assert.equal(widgetCss.status, 200);
    assert.match(widgetCss.headers.get("content-type"), /^text\/css/);
    assert.match(await widgetCss.text(), /\.shell/);

    const widgetJs = await fetch(`${base}/widget.js`);
    assert.equal(widgetJs.status, 200);
    assert.match(widgetJs.headers.get("content-type"), /^text\/javascript/);
    assert.match(await widgetJs.text(), /window\.openai\.callTool/);

    const status = await fetch(`${base}/api/status`).then((response) => response.json());
    assert.equal(status.runtime.selected.schemaVersion, 34);

    const search = await fetch(`${base}/api/search?query=runtime`).then((response) =>
      response.json()
    );
    assert.equal(search.data[0].id, 7);

    const memory = await fetch(`${base}/api/memory?id=7`).then((response) => response.json());
    assert.equal(memory.content, "Detailed memory body");

    const save = await fetch(`${base}/api/save`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: base },
      body: JSON.stringify({ text: "Remember this.", memory_type: "decision" })
    }).then((response) => response.json());
    assert.equal(save.id, 9);

    const preview = await fetch(`${base}/api/governance-preview`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: base },
      body: JSON.stringify({ action: "stale", ids: [7], project: "/tmp/remem" })
    }).then((response) => response.json());
    assert.equal(preview.dry_run, true);
    assert.equal(preview.affected[0].id, 7);

    const current = await fetch(`${base}/api/current-state?state_key=deploy-target`).then((response) =>
      response.json()
    );
    assert.equal(current.current.id, 11);

    const commit = await fetch(`${base}/api/commit?sha=abcdef123456`).then((response) =>
      response.json()
    );
    assert.equal(commit[0].git.short_sha, "abcdef1");

    const sessionCommits = await fetch(`${base}/api/session-commits?session_id=session-1`).then(
      (response) => response.json()
    );
    assert.equal(sessionCommits[0].link.session_id, "session-1");

    const timelineAround = await fetch(`${base}/api/timeline-around?anchor=15`).then((response) =>
      response.json()
    );
    assert.equal(timelineAround.anchor_id, 15);

    const timelineReport = await fetch(
      `${base}/api/timeline-report?project=/tmp/remem&full=true`
    ).then((response) => response.json());
    assert.equal(timelineReport.report.overview.total_observations, 2);

    const workstreams = await fetch(`${base}/api/workstreams?project=/tmp/remem&status=active`).then(
      (response) => response.json()
    );
    assert.equal(workstreams.workstreams[0].id, 21);

    const workstreamUpdate = await fetch(`${base}/api/workstream-update`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: base },
      body: JSON.stringify({
        id: 21,
        project: "/tmp/remem",
        status: "paused",
        confirm: true
      })
    }).then((response) => response.json());
    assert.equal(workstreamUpdate.updated, true);
  });
});

test("HTTP write routes reject cross-site browser requests", async () => {
  const backend = fakeBackend();
  let saves = 0;
  backend.save = async () => {
    saves += 1;
    return { id: 9 };
  };
  let previews = 0;
  backend.governancePreview = async () => {
    previews += 1;
    return { dry_run: true, affected: [] };
  };
  let workstreamUpdates = 0;
  backend.workstreamUpdate = async () => {
    workstreamUpdates += 1;
    return { id: 21, updated: true };
  };

  await withServer(async (base) => {
    const apiSave = await fetch(`${base}/api/save`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: "https://attacker.example" },
      body: JSON.stringify({ text: "poison" })
    });
    assert.equal(apiSave.status, 403);

    const simplePost = await fetch(`${base}/api/save`, {
      method: "POST",
      headers: { "content-type": "text/plain" },
      body: JSON.stringify({ text: "poison" })
    });
    assert.equal(simplePost.status, 415);

    const mcpSave = await fetch(`${base}/mcp`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: "https://attacker.example" },
      body: JSON.stringify({
        id: 1,
        method: "tools/call",
        params: { name: "remem_save_memory", arguments: { text: "poison" } }
      })
    });
    assert.equal(mcpSave.status, 403);

    const apiGovernance = await fetch(`${base}/api/governance-preview`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: "https://attacker.example" },
      body: JSON.stringify({ action: "delete", ids: [7] })
    });
    assert.equal(apiGovernance.status, 403);

    const apiWorkstreamUpdate = await fetch(`${base}/api/workstream-update`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: "https://attacker.example" },
      body: JSON.stringify({ id: 21, project: "/tmp/remem", status: "paused", confirm: true })
    });
    assert.equal(apiWorkstreamUpdate.status, 403);
  }, backend);

  assert.equal(saves, 0);
  assert.equal(previews, 0);
  assert.equal(workstreamUpdates, 0);
});

test("widget renders raw archive fallback results", async () => {
  await withServer(async (base) => {
    const widget = await fetch(`${base}/widget.js`).then((response) => response.text());

    assert.match(widget, /payload\.raw_hits/);
    assert.match(widget, /raw_archive/);
    assert.match(widget, /raw archive/);
  });
});

test("widget routes embedded app actions through host tool calls", async () => {
  await withServer(async (base) => {
    const html = await fetch(`${base}/widget.html`).then((response) => response.text());
    const widget = await fetch(`${base}/widget.js`).then((response) => response.text());

    assert.match(html, /data-view="timeline"/);
    assert.match(html, /data-view="workstreams"/);
    assert.match(widget, /window\.openai\.callTool/);
    assert.match(widget, /remem_dashboard/);
    assert.match(widget, /remem_search/);
    assert.match(widget, /remem_get_memory/);
    assert.match(widget, /remem_save_memory/);
    assert.match(widget, /remem_activation_plan/);
    assert.match(widget, /remem_governance_preview/);
    assert.match(widget, /remem_timeline_around/);
    assert.match(widget, /remem_timeline_report/);
    assert.match(widget, /remem_workstreams_list/);
    assert.match(widget, /remem_workstream_update/);
    assert.match(widget, /\/api\/governance-preview/);
    assert.match(widget, /\/api\/timeline-around/);
    assert.match(widget, /\/api\/workstream-update/);
  });
});

test("JSON-RPC trace tools return structured content", async () => {
  const current = await handleJsonRpc(fakeBackend(), {
    id: 1,
    method: "tools/call",
    params: { name: "remem_current_state", arguments: { state_key: "deploy-target" } }
  });
  assert.equal(current.structuredContent.current.id, 11);

  const commit = await handleJsonRpc(fakeBackend(), {
    id: 2,
    method: "tools/call",
    params: { name: "remem_commit_lookup", arguments: { sha: "abcdef123456" } }
  });
  assert.equal(commit.structuredContent.results[0].git.short_sha, "abcdef1");

  const sessionCommits = await handleJsonRpc(fakeBackend(), {
    id: 3,
    method: "tools/call",
    params: { name: "remem_session_commits", arguments: { session_id: "session-1" } }
  });
  assert.equal(sessionCommits.structuredContent.results[0].link.session_id, "session-1");

  const timelineAround = await handleJsonRpc(fakeBackend(), {
    id: 4,
    method: "tools/call",
    params: { name: "remem_timeline_around", arguments: { anchor: 15 } }
  });
  assert.equal(timelineAround.structuredContent.anchor_id, 15);

  const timelineReport = await handleJsonRpc(fakeBackend(), {
    id: 5,
    method: "tools/call",
    params: { name: "remem_timeline_report", arguments: { project: "/tmp/remem" } }
  });
  assert.equal(timelineReport.structuredContent.report.overview.total_observations, 2);

  const workstreams = await handleJsonRpc(fakeBackend(), {
    id: 6,
    method: "tools/call",
    params: { name: "remem_workstreams_list", arguments: { project: "/tmp/remem" } }
  });
  assert.equal(workstreams.structuredContent.workstreams[0].id, 21);

  const update = await handleJsonRpc(fakeBackend(), {
    id: 7,
    method: "tools/call",
    params: {
      name: "remem_workstream_update",
      arguments: { id: 21, project: "/tmp/remem", status: "paused", confirm: true }
    }
  });
  assert.equal(update.structuredContent.updated, true);
});

test("trace backend builds guarded timeline and workstream CLI args", async () => {
  const calls = [];
  const backend = createTraceBackend(async (args) => {
    calls.push(args);
    return { ok: true };
  });

  await backend.timelineAround({ query: "release manifest", project: "/tmp/remem", depth_before: 2 });
  await backend.timelineReport({ project: "/tmp/remem", full: true });
  await backend.workstreamsList({ project: "/tmp/remem", status: "active" });
  await backend.workstreamUpdate({
    id: 21,
    project: "/tmp/remem",
    status: "paused",
    confirm: true
  });

  assert.deepEqual(calls[0], [
    "timeline",
    "around",
    "--json",
    "--query",
    "release manifest",
    "--project",
    "/tmp/remem",
    "--depth-before",
    "2"
  ]);
  assert.deepEqual(calls[1], ["timeline", "report", "/tmp/remem", "--json", "--full"]);
  assert.deepEqual(calls[2], [
    "workstreams",
    "list",
    "--project",
    "/tmp/remem",
    "--json",
    "--status",
    "active"
  ]);
  assert.deepEqual(calls[3], [
    "workstreams",
    "update",
    "21",
    "--project",
    "/tmp/remem",
    "--json",
    "--status",
    "paused",
    "--confirm"
  ]);
  assert.throws(
    () => backend.workstreamUpdate({ id: 21, project: "/tmp/remem", confirm: true }),
    /required/
  );
  assert.throws(
    () => backend.workstreamUpdate({ id: 21, project: "/tmp/remem", status: "paused" }),
    /confirm/
  );
});

test("governance preview args are always dry-run JSON CLI calls", () => {
  const { args, requested } = governancePreviewArgs({
    action: "delete",
    ids: [7, "8"],
    project: "/tmp/remem",
    query: "old plan",
    memory_type: "decision",
    status: "active",
    limit: 12,
    actor: "codex-remem-app"
  });

  assert.deepEqual(args, [
    "govern",
    "--action",
    "delete",
    "--dry-run",
    "--json",
    "--project",
    "/tmp/remem",
    "--actor",
    "codex-remem-app",
    "--query",
    "old plan",
    "--memory-type",
    "decision",
    "--status",
    "active",
    "--limit",
    "12",
    "--offset",
    "0",
    "7",
    "8"
  ]);
  assert.equal(requested.action, "delete");
  assert.deepEqual(requested.ids, [7, 8]);
});

test("governance preview rejects unsafe or empty requests before CLI execution", () => {
  assert.throws(() => governancePreviewArgs({ action: "archive", ids: [7] }), /delete, reject, or stale/);
  assert.throws(() => governancePreviewArgs({ action: "stale" }), /requires memory IDs or a selector/);
  assert.throws(() => governancePreviewArgs({ action: "stale", ids: [null] }), /Expected integer/);
});

test("API proxy clears failed readiness so later requests can retry", async () => {
  const proxy = new RememApiProxy();
  let attempts = 0;
  let kills = 0;
  proxy.start = async function start() {
    attempts += 1;
    this.child = {
      kill(signal) {
        if (signal === "SIGTERM") kills += 1;
      }
    };
    throw new Error("startup failed");
  };

  await assert.rejects(() => proxy.ensureStarted(), /startup failed/);
  await assert.rejects(() => proxy.ensureStarted(), /startup failed/);

  assert.equal(attempts, 2);
  assert.equal(kills, 2);
  assert.equal(proxy.ready, null);
  assert.equal(proxy.child, null);
});

test("buildSnapshot keeps setup details available when status commands fail", async () => {
  const backend = fakeBackend();
  backend.status = async () => {
    throw new Error("database not found");
  };
  backend.doctor = async () => {
    throw new Error("database not found");
  };

  const snapshot = await buildSnapshot(backend);

  assert.equal(snapshot.status.status, "setup_required");
  assert.equal(snapshot.status.totals.memories, 0);
  assert.equal(snapshot.doctor.status, "setup_required");
  assert.equal(snapshot.doctor.warns, 1);
  assert.equal(snapshot.runtime.selected.version, pluginManifest.version);
});

test("CLI host binding is restricted to loopback addresses", () => {
  assert.equal(parseArgs(["--host", "127.0.0.1"]).host, "127.0.0.1");
  assert.equal(parseArgs(["--host", "localhost"]).host, "localhost");
  assert.equal(parseArgs(["--host", "::1"]).host, "::1");
  assert.throws(
    () => parseArgs(["--host", "0.0.0.0"]),
    /loopback address/
  );
});

test("activation summary is explicit about dry-run content", () => {
  const summary = activationSummary("Would write hooks\nMCP server remem configured\n");

  assert.equal(summary.mentions_hooks, true);
  assert.equal(summary.mentions_mcp, true);
  assert.equal(summary.writes_config, true);
  assert.equal(summary.line_count, 2);
});
