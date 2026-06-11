"use strict";

const GOVERNANCE_ACTIONS = new Set(["delete", "reject", "stale"]);

function cleanOptionalString(value) {
  if (value === undefined || value === null) return undefined;
  const text = String(value).trim();
  return text ? text : undefined;
}

function cleanOptionalInteger(value, fallback, { min = 0, max = Number.MAX_SAFE_INTEGER } = {}) {
  if (value === undefined || value === null || value === "") return fallback;
  const number = Number(value);
  if (!Number.isInteger(number) || number < min || number > max) {
    throw Object.assign(new Error(`Expected integer between ${min} and ${max}`), {
      statusCode: 400
    });
  }
  return number;
}

function cleanRequiredInteger(value, { min = 0, max = Number.MAX_SAFE_INTEGER } = {}) {
  const number = Number(value);
  if (!Number.isInteger(number) || number < min || number > max) {
    throw Object.assign(new Error(`Expected integer between ${min} and ${max}`), {
      statusCode: 400
    });
  }
  return number;
}

function normalizeGovernancePreviewInput(input = {}) {
  const action = cleanOptionalString(input.action) || "stale";
  if (!GOVERNANCE_ACTIONS.has(action)) {
    throw Object.assign(new Error("Governance action must be delete, reject, or stale"), {
      statusCode: 400
    });
  }
  const rawIds = Array.isArray(input.ids)
    ? input.ids
    : input.id !== undefined && input.id !== null
      ? [input.id]
      : [];
  const ids = rawIds.map((id) => cleanRequiredInteger(id, { min: 1 }));
  const query = cleanOptionalString(input.query);
  const memoryType = cleanOptionalString(input.memory_type || input.type);
  const status = cleanOptionalString(input.status);
  const project = cleanOptionalString(input.project);
  const limit = cleanOptionalInteger(input.limit, 50, { min: 1, max: 200 });
  const offset = cleanOptionalInteger(input.offset, 0, { min: 0, max: 1_000_000 });
  const hasSelector = Boolean(query || memoryType || status);
  if (!ids.length && !hasSelector) {
    throw Object.assign(
      new Error("Governance preview requires memory IDs or a selector"),
      { statusCode: 400 }
    );
  }
  return {
    action,
    actor: cleanOptionalString(input.actor),
    ids,
    limit,
    memory_type: memoryType,
    offset,
    project,
    query,
    reason: cleanOptionalString(input.reason),
    status
  };
}

function pushOptionalArg(args, name, value) {
  if (value !== undefined && value !== null && value !== "") {
    args.push(name, String(value));
  }
}

function governancePreviewArgs(input = {}) {
  const requested = normalizeGovernancePreviewInput(input);
  const args = ["govern", "--action", requested.action, "--dry-run", "--json"];
  pushOptionalArg(args, "--project", requested.project);
  pushOptionalArg(args, "--reason", requested.reason);
  pushOptionalArg(args, "--actor", requested.actor);
  pushOptionalArg(args, "--query", requested.query);
  pushOptionalArg(args, "--memory-type", requested.memory_type);
  pushOptionalArg(args, "--status", requested.status);
  pushOptionalArg(args, "--limit", requested.limit);
  pushOptionalArg(args, "--offset", requested.offset);
  for (const id of requested.ids) args.push(String(id));
  return { args, requested };
}

module.exports = {
  governancePreviewArgs,
  normalizeGovernancePreviewInput
};
