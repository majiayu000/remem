"use strict";

const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);

function normalizeHostName(host) {
  const value = String(host || "").trim().toLowerCase();
  if (value.startsWith("[") && value.endsWith("]")) return value.slice(1, -1);
  return value;
}

function isLoopbackHost(host) {
  return LOOPBACK_HOSTS.has(normalizeHostName(host));
}

function parseHostHeader(host) {
  const value = String(host || "").trim();
  if (!value) return null;
  try {
    const url = new URL(`http://${value}`);
    return {
      host: value.toLowerCase(),
      hostname: normalizeHostName(url.hostname),
      port: url.port ? Number(url.port) : null
    };
  } catch {
    return null;
  }
}

function forbidden(message) {
  return Object.assign(new Error(message), { statusCode: 403 });
}

function assertLocalHostAllowed(req, server) {
  const parsed = parseHostHeader(req.headers.host);
  if (!parsed || !isLoopbackHost(parsed.hostname)) {
    throw forbidden("Local app requests require a loopback Host header");
  }
  const address = server.address();
  const actualPort = address && typeof address === "object" ? Number(address.port) : null;
  const hostPort = parsed.port || 80;
  if (actualPort && hostPort !== actualPort) {
    throw forbidden("Local app requests require the bound Host port");
  }
  return parsed;
}

function assertLocalPostAllowed(req) {
  const contentType = String(req.headers["content-type"] || "").toLowerCase().split(";")[0].trim();
  if (contentType !== "application/json") {
    throw Object.assign(new Error("Local app POST routes require application/json"), { statusCode: 415 });
  }
  const site = String(req.headers["sec-fetch-site"] || "").toLowerCase();
  if (site && site !== "same-origin" && site !== "none") {
    throw forbidden("Cross-site browser requests are not allowed");
  }
  const origin = String(req.headers.origin || "").toLowerCase();
  const expected = req.headers.host && `http://${String(req.headers.host).toLowerCase()}`;
  if (origin && origin !== expected) {
    throw forbidden("Cross-origin browser requests are not allowed");
  }
}

module.exports = {
  assertLocalHostAllowed,
  assertLocalPostAllowed,
  isLoopbackHost,
  parseHostHeader
};
