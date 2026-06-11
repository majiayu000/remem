#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const {
  ensureRuntime,
  expectedVersion,
  inspectRuntime,
  pluginDataDir,
  pluginRoot
} = require("../../scripts/remem-runtime");
const { governancePreviewArgs } = require("./governance");
const { toolDescriptors, UI_RESOURCE } = require("./tools");
const { callTraceTool, createTraceBackend } = require("./trace");

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 5577;
const JSON_LIMIT_BYTES = 1_000_000;
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);

function dataDir(env = process.env) {
  return path.resolve(env.REMEM_DATA_DIR || path.join(os.homedir(), ".remem"));
}

function apiTokenPath(env = process.env) {
  return path.join(dataDir(env), ".api-token");
}

function runtimeEnv(options = {}) {
  const env = {
    ...process.env,
    ...(options.env || {})
  };
  env.REMEM_PLUGIN_ROOT = pluginRoot(options);
  env.REMEM_PLUGIN_DATA = pluginDataDir(options);
  return env;
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function readWidgetHtml() {
  return fs.readFileSync(path.join(__dirname, "public", "widget.html"), "utf8");
}

const PUBLIC_ASSETS = new Map([
  ["/widget.css", { file: "widget.css", contentType: "text/css; charset=utf-8" }],
  ["/widget.js", { file: "widget.js", contentType: "text/javascript; charset=utf-8" }]
]);

function readPublicAsset(pathname) {
  const asset = PUBLIC_ASSETS.get(pathname);
  if (!asset) return null;
  return {
    content: fs.readFileSync(path.join(__dirname, "public", asset.file), "utf8"),
    contentType: asset.contentType
  };
}

function jsonResponse(res, status, payload) {
  const body = JSON.stringify(payload, null, 2);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store"
  });
  res.end(`${body}\n`);
}

function textResponse(res, status, text, contentType = "text/plain; charset=utf-8") {
  res.writeHead(status, {
    "content-type": contentType,
    "cache-control": "no-store"
  });
  res.end(text);
}

function notFound(res) {
  jsonResponse(res, 404, { error: { code: "not_found", message: "Not found" } });
}

function parseUrl(req) {
  return new URL(req.url, "http://remem.local");
}

function isLoopbackHost(host) {
  return LOOPBACK_HOSTS.has(String(host || "").toLowerCase());
}

function assertLocalPostAllowed(req) {
  const contentType = String(req.headers["content-type"] || "").toLowerCase().split(";")[0].trim();
  if (contentType !== "application/json") {
    throw Object.assign(new Error("Local app POST routes require application/json"), { statusCode: 415 });
  }
  const site = String(req.headers["sec-fetch-site"] || "").toLowerCase();
  if (site && site !== "same-origin" && site !== "none") {
    throw Object.assign(new Error("Cross-site browser requests are not allowed"), { statusCode: 403 });
  }
  const origin = String(req.headers.origin || "").toLowerCase();
  const expected = req.headers.host && `http://${String(req.headers.host).toLowerCase()}`;
  if (origin && origin !== expected) {
    throw Object.assign(new Error("Cross-origin browser requests are not allowed"), { statusCode: 403 });
  }
}

async function readBody(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > JSON_LIMIT_BYTES) {
      throw Object.assign(new Error("Request body is too large"), { statusCode: 413 });
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function readJsonBody(req) {
  const text = await readBody(req);
  if (!text.trim()) return {};
  try {
    return JSON.parse(text);
  } catch (error) {
    throw Object.assign(new Error(`Invalid JSON request body: ${error.message}`), {
      statusCode: 400
    });
  }
}

function runProcess(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd || process.cwd(),
      env: options.env || process.env,
      stdio: ["ignore", "pipe", "pipe"]
    });
    let stdout = "";
    let stderr = "";
    const timeout = setTimeout(() => {
      child.kill("SIGTERM");
      reject(new Error(`${command} ${args.join(" ")} timed out after ${options.timeoutMs}ms`));
    }, options.timeoutMs || 15000);
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.on("exit", (status, signal) => {
      clearTimeout(timeout);
      resolve({ status, signal, stdout, stderr });
    });
  });
}

async function runRemem(args, options = {}) {
  const binary = options.binary || (await ensureRuntime({
    allowDownload: options.allowDownload === true,
    adoptLocal: options.adoptLocal
  }));
  const result = await runProcess(fs.realpathSync(binary), args, {
    env: runtimeEnv(options),
    timeoutMs: options.timeoutMs || 15000,
    cwd: options.cwd || process.cwd()
  });
  if (result.status !== 0 && !options.allowFailure) {
    const detail = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`remem ${args.join(" ")} failed: ${detail}`);
  }
  return result;
}

async function runRememJson(args, options = {}) {
  const result = await runRemem(args, options);
  const text = result.stdout.trim();
  if (!text) {
    throw new Error(`remem ${args.join(" ")} returned empty JSON output`);
  }
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`remem ${args.join(" ")} returned invalid JSON: ${error.message}`);
  }
}

function findFreePort(host = DEFAULT_HOST) {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, host, () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

class RememApiProxy {
  constructor(options = {}) {
    this.host = options.host || DEFAULT_HOST;
    this.port = options.port || null;
    this.env = options.env || process.env;
    this.child = null;
    this.ready = null;
    this.binary = options.binary || null;
  }

  async ensureStarted() {
    if (this.ready) return this.ready;
    this.ready = this.start().catch((error) => {
      this.stop();
      throw error;
    });
    return this.ready;
  }

  async start() {
    const binary = this.binary || (await ensureRuntime({ allowDownload: false }));
    const port = this.port || (await findFreePort(this.host));
    this.port = port;
    this.child = spawn(fs.realpathSync(binary), ["api", "--port", String(port)], {
      env: runtimeEnv({ env: this.env }),
      stdio: ["ignore", "ignore", "pipe"]
    });
    this.child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });
    this.child.on("exit", (status, signal) => {
      this.ready = null;
      this.child = null;
      if (status !== 0 && signal !== "SIGTERM") {
        process.stderr.write(`remem api exited with ${status ?? signal}\n`);
      }
    });
    await this.waitForReady();
    return this;
  }

  async waitForReady() {
    const started = Date.now();
    let lastError;
    while (Date.now() - started < 10000) {
      try {
        const token = this.readToken();
        const response = await fetch(this.url("/api/v1/status"), {
          headers: { authorization: `Bearer ${token}` }
        });
        if (response.ok) return;
        lastError = new Error(`status returned ${response.status}`);
      } catch (error) {
        lastError = error;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error(`remem API did not become ready: ${lastError?.message || "timeout"}`);
  }

  readToken() {
    const token = fs.readFileSync(apiTokenPath(this.env), "utf8").trim();
    if (!token) throw new Error("remem API token is empty");
    return token;
  }

  url(route) {
    return `http://${this.host}:${this.port}${route}`;
  }

  async request(route, options = {}) {
    await this.ensureStarted();
    const headers = {
      authorization: `Bearer ${this.readToken()}`,
      ...(options.headers || {})
    };
    const response = await fetch(this.url(route), {
      ...options,
      headers
    });
    const text = await response.text();
    let payload = null;
    if (text.trim()) {
      try {
        payload = JSON.parse(text);
      } catch (_error) {
        payload = { raw: text };
      }
    }
    if (!response.ok) {
      const message = payload?.error?.message || text || `HTTP ${response.status}`;
      throw Object.assign(new Error(message), {
        statusCode: response.status,
        payload
      });
    }
    return payload;
  }

  stop() {
    if (this.child) {
      this.child.kill("SIGTERM");
      this.child = null;
    }
    this.ready = null;
  }
}

function activationSummary(text) {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  return {
    plan_text: text,
    line_count: lines.length,
    writes_config: lines.some((line) => /write|would write|update|install/i.test(line)),
    mentions_hooks: lines.some((line) => /hook/i.test(line)),
    mentions_mcp: lines.some((line) => /mcp/i.test(line))
  };
}

function createBackend(options = {}) {
  const api = options.api || new RememApiProxy(options.apiOptions || {});
  return {
    async runtime() {
      return inspectRuntime();
    },
    async status() {
      return runRememJson(["status", "--json"], {
        allowDownload: false,
        allowFailure: true
      });
    },
    async doctor() {
      try {
        return await runRememJson(["doctor", "--json"], {
          allowDownload: false,
          allowFailure: true,
          timeoutMs: 20000
        });
      } catch (error) {
        return {
          status: "error",
          error: error.message,
          checks: []
        };
      }
    },
    async search(params) {
      const query = new URLSearchParams();
      for (const [key, value] of Object.entries(params)) {
        if (value !== undefined && value !== null && value !== "") query.set(key, String(value));
      }
      return api.request(`/api/v1/search?${query.toString()}`);
    },
    async memory(id) {
      return api.request(`/api/v1/memory?id=${encodeURIComponent(String(id))}`);
    },
    async save(input) {
      return api.request("/api/v1/memories", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          ...input,
          host: input.host || "codex-remem-app",
          claim_source: input.claim_source || "codex_app_save"
        })
      });
    },
    async activationPlan() {
      const result = await runRemem(
        ["install", "--target", "codex", "--hooks-only", "--dry-run"],
        {
          allowDownload: false,
          allowFailure: true,
          timeoutMs: 15000
        }
      );
      return activationSummary(result.stdout || result.stderr || "");
    },
    async governancePreview(input) {
      const { args, requested } = governancePreviewArgs(input);
      const result = await runRememJson(args, {
        allowDownload: false,
        timeoutMs: 20000
      });
      return {
        ...result,
        requested
      };
    },
    ...createTraceBackend(runRememJson),
    stop() {
      api.stop?.();
    }
  };
}

async function buildSnapshot(backend) {
  const [runtime, status, doctor, activation] = await Promise.all([
    recoverableField("runtime", () => backend.runtime()),
    recoverableField("status", () => backend.status(), setupStatus),
    recoverableField("doctor", () => backend.doctor(), setupDoctor),
    recoverableField("activation", () => backend.activationPlan(), setupActivation)
  ]);
  return {
    expected_version: expectedVersion(),
    plugin_data: pluginDataDir(),
    runtime,
    status,
    doctor,
    activation
  };
}

async function recoverableField(name, load, fallback = setupField) {
  try {
    return await load();
  } catch (error) {
    return fallback(name, error);
  }
}

function setupField(name, error) {
  return {
    status: "setup_required",
    unavailable: true,
    error: {
      field: name,
      message: error.message
    }
  };
}

function setupStatus(name, error) {
  return {
    ...setupField(name, error),
    totals: {
      memories: 0,
      observations: 0,
      raw_messages: 0
    },
    capture_pipeline: {
      extract_todo: 0
    },
    pending_observations: {
      ready: 0
    },
    jobs: {
      pending: 0
    }
  };
}

function setupDoctor(name, error) {
  return {
    ...setupField(name, error),
    fails: 0,
    warns: 1,
    checks: []
  };
}

function setupActivation(name, error) {
  return {
    ...setupField(name, error),
    plan_text: "",
    line_count: 0,
    writes_config: false,
    mentions_hooks: false,
    mentions_mcp: false
  };
}

async function callTool(backend, name, args = {}) {
  if (name === "remem_dashboard") {
    const snapshot = await buildSnapshot(backend);
    return toolResult("Remem dashboard ready.", snapshot, { rendered_at: new Date().toISOString() });
  }
  if (name === "remem_search") {
    const result = await backend.search({
      query: args.query,
      project: args.project,
      type: args.type,
      limit: args.limit || 10,
      offset: args.offset || 0,
      include_stale: args.include_stale,
      multi_hop: args.multi_hop
    });
    return toolResult(`Found ${result.meta?.count ?? 0} memory result(s).`, result);
  }
  if (name === "remem_get_memory") {
    const result = await backend.memory(args.id);
    return toolResult(`Memory ${args.id} loaded.`, result);
  }
  if (name === "remem_save_memory") {
    const result = await backend.save(args);
    return toolResult(`Saved memory ${result.id}.`, result);
  }
  if (name === "remem_activation_plan") {
    const result = await backend.activationPlan();
    return toolResult("Activation plan generated without writing config.", result);
  }
  if (name === "remem_governance_preview") {
    const result = await backend.governancePreview(args);
    return toolResult(
      `Governance dry-run found ${result.affected?.length ?? 0} affected memory result(s).`,
      result
    );
  }
  const traceResult = await callTraceTool(backend, name, args, toolResult);
  if (traceResult) return traceResult;
  throw Object.assign(new Error(`Unknown tool: ${name}`), { code: -32602 });
}

function toolResult(text, structuredContent, meta = {}) {
  return {
    content: [{ type: "text", text }],
    structuredContent,
    _meta: meta
  };
}

async function handleJsonRpc(backend, message) {
  if (!message || typeof message !== "object") {
    throw Object.assign(new Error("Invalid JSON-RPC request"), { code: -32600 });
  }
  const method = message.method;
  if (method === "initialize") {
    return {
      protocolVersion: message.params?.protocolVersion || "2025-06-18",
      capabilities: { tools: {}, resources: {} },
      serverInfo: { name: "remem-app", version: expectedVersion() },
      instructions: "Use Remem to inspect project memory, search details, save explicit durable memories, preview governance, and preview hook activation."
    };
  }
  if (method === "tools/list") return { tools: toolDescriptors() };
  if (method === "tools/call") {
    return callTool(backend, message.params?.name, message.params?.arguments || {});
  }
  if (method === "resources/list") {
    return {
      resources: [
        {
          uri: UI_RESOURCE,
          name: "Remem Dashboard",
          mimeType: "text/html;profile=mcp-app"
        }
      ]
    };
  }
  if (method === "resources/read") {
    if (message.params?.uri !== UI_RESOURCE) {
      throw Object.assign(new Error(`Unknown resource: ${message.params?.uri}`), { code: -32602 });
    }
    return {
      contents: [
        {
          uri: UI_RESOURCE,
          mimeType: "text/html;profile=mcp-app",
          text: readWidgetHtml()
        }
      ]
    };
  }
  if (method === "notifications/initialized") return undefined;
  throw Object.assign(new Error(`Method not found: ${method}`), { code: -32601 });
}

function jsonRpcSuccess(id, result) {
  return { jsonrpc: "2.0", id, result };
}

function jsonRpcError(id, error) {
  return {
    jsonrpc: "2.0",
    id,
    error: {
      code: error.code || -32000,
      message: error.message
    }
  };
}

function createServer(options = {}) {
  const backend = options.backend || createBackend(options);
  const server = http.createServer(async (req, res) => {
    try {
      const url = parseUrl(req);
      if (req.method === "GET" && (url.pathname === "/" || url.pathname === "/widget.html")) {
        return textResponse(res, 200, readWidgetHtml(), "text/html; charset=utf-8");
      }
      if (req.method === "GET") {
        const asset = readPublicAsset(url.pathname);
        if (asset) return textResponse(res, 200, asset.content, asset.contentType);
      }
      if (req.method === "GET" && url.pathname === "/healthz") {
        return jsonResponse(res, 200, { ok: true, name: "remem-app" });
      }
      if (req.method === "GET" && url.pathname === "/api/status") {
        return jsonResponse(res, 200, await buildSnapshot(backend));
      }
      if (req.method === "GET" && url.pathname === "/api/search") {
        return jsonResponse(res, 200, await backend.search(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/memory") {
        return jsonResponse(res, 200, await backend.memory(url.searchParams.get("id")));
      }
      if (req.method === "GET" && url.pathname === "/api/activation-plan") {
        return jsonResponse(res, 200, await backend.activationPlan());
      }
      if (req.method === "GET" && url.pathname === "/api/current-state") {
        return jsonResponse(res, 200, await backend.currentState(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/commit") {
        return jsonResponse(res, 200, await backend.commitLookup(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/session-commits") {
        return jsonResponse(res, 200, await backend.sessionCommits(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/timeline-around") {
        return jsonResponse(res, 200, await backend.timelineAround(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/timeline-report") {
        return jsonResponse(res, 200, await backend.timelineReport(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "GET" && url.pathname === "/api/workstreams") {
        return jsonResponse(res, 200, await backend.workstreamsList(Object.fromEntries(url.searchParams)));
      }
      if (req.method === "POST" && url.pathname === "/api/save") {
        assertLocalPostAllowed(req);
        return jsonResponse(res, 201, await backend.save(await readJsonBody(req)));
      }
      if (req.method === "POST" && url.pathname === "/api/governance-preview") {
        assertLocalPostAllowed(req);
        return jsonResponse(res, 200, await backend.governancePreview(await readJsonBody(req)));
      }
      if (req.method === "POST" && url.pathname === "/api/workstream-update") {
        assertLocalPostAllowed(req);
        return jsonResponse(res, 200, await backend.workstreamUpdate(await readJsonBody(req)));
      }
      if (req.method === "POST" && url.pathname === "/mcp") {
        assertLocalPostAllowed(req);
        const payload = await readJsonBody(req);
        const items = Array.isArray(payload) ? payload : [payload];
        const replies = [];
        for (const item of items) {
          try {
            const result = await handleJsonRpc(backend, item);
            if (item.id !== undefined && result !== undefined) {
              replies.push(jsonRpcSuccess(item.id, result));
            }
          } catch (error) {
            if (item.id !== undefined) replies.push(jsonRpcError(item.id, error));
          }
        }
        if (Array.isArray(payload)) return jsonResponse(res, 200, replies);
        return jsonResponse(res, 200, replies[0] || {});
      }
      return notFound(res);
    } catch (error) {
      return jsonResponse(res, error.statusCode || 500, {
        error: {
          code: error.statusCode === 400 ? "bad_request" : "app_error",
          message: error.message
        }
      });
    }
  });
  server.stopBackend = () => backend.stop?.();
  return server;
}

function parseArgs(argv) {
  const args = { host: DEFAULT_HOST, port: DEFAULT_PORT };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--host") args.host = argv[++i];
    else if (arg === "--port") args.port = Number(argv[++i]);
    else if (arg === "--json") args.json = true;
    else if (arg === "--help" || arg === "-h") args.help = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  if (!Number.isInteger(args.port) || args.port <= 0) {
    throw new Error("--port must be a positive integer");
  }
  if (!isLoopbackHost(args.host)) {
    throw new Error("--host must be a loopback address because this local app has no HTTP auth");
  }
  return args;
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  if (args.help) {
    process.stdout.write("Usage: node plugins/remem/apps/remem/server.js [--host 127.0.0.1] [--port 5577]\n");
    return 0;
  }
  const server = createServer({ host: args.host });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(args.port, args.host, resolve);
  });
  const url = `http://${args.host}:${args.port}/`;
  if (args.json) {
    process.stdout.write(`${JSON.stringify({ url, mcp: `${url}mcp` })}\n`);
  } else {
    process.stdout.write(`Remem app listening on ${url}\n`);
  }
  const stop = () => {
    server.stopBackend();
    server.close(() => process.exit(0));
  };
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  return 0;
}

module.exports = {
  RememApiProxy,
  activationSummary,
  apiTokenPath,
  buildSnapshot,
  callTool,
  createBackend,
  createServer,
  dataDir,
  handleJsonRpc,
  isLoopbackHost,
  parseArgs,
  readJson,
  runProcess,
  toolDescriptors,
  UI_RESOURCE
};

if (require.main === module) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exit(1);
  });
}
