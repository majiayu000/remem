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
    "remem_activation_plan"
  ]) {
    const descriptor = toolDescriptors().find((tool) => tool.name === name);
    assert.deepEqual(descriptor._meta.ui.visibility, ["model", "app"]);
    assert.equal(descriptor._meta["openai/widgetAccessible"], true);
  }
});

test("JSON-RPC tools/list and dashboard call return structured content", async () => {
  const tools = await handleJsonRpc(fakeBackend(), {
    id: 1,
    method: "tools/list",
    params: {}
  });
  assert.ok(tools.tools.some((tool) => tool.name === "remem_save_memory"));

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
    assert.match(await widget.text(), /Remem Dashboard/);

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
  });
});

test("HTTP write routes reject cross-site browser requests", async () => {
  const backend = fakeBackend();
  let saves = 0;
  backend.save = async () => {
    saves += 1;
    return { id: 9 };
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
  }, backend);

  assert.equal(saves, 0);
});

test("widget renders raw archive fallback results", async () => {
  await withServer(async (base) => {
    const widget = await fetch(`${base}/widget.html`).then((response) => response.text());

    assert.match(widget, /payload\.raw_hits/);
    assert.match(widget, /raw_archive/);
    assert.match(widget, /raw archive/);
  });
});

test("widget routes embedded app actions through host tool calls", async () => {
  await withServer(async (base) => {
    const widget = await fetch(`${base}/widget.html`).then((response) => response.text());

    assert.match(widget, /window\.openai\.callTool/);
    assert.match(widget, /remem_dashboard/);
    assert.match(widget, /remem_search/);
    assert.match(widget, /remem_get_memory/);
    assert.match(widget, /remem_save_memory/);
    assert.match(widget, /remem_activation_plan/);
  });
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
