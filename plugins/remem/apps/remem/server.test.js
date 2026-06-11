#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  activationSummary,
  createServer,
  handleJsonRpc,
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

async function withServer(fn) {
  const server = createServer({ backend: fakeBackend() });
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
  assert.equal(dashboard._meta["openai/outputTemplate"], UI_RESOURCE);
  assert.equal(dashboard.annotations.readOnlyHint, true);
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
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text: "Remember this.", memory_type: "decision" })
    }).then((response) => response.json());
    assert.equal(save.id, 9);
  });
});

test("activation summary is explicit about dry-run content", () => {
  const summary = activationSummary("Would write hooks\nMCP server remem configured\n");

  assert.equal(summary.mentions_hooks, true);
  assert.equal(summary.mentions_mcp, true);
  assert.equal(summary.writes_config, true);
  assert.equal(summary.line_count, 2);
});
