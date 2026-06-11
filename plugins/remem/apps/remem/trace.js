"use strict";

function badRequest(message) {
  return Object.assign(new Error(message), { statusCode: 400 });
}

function requiredText(value, name) {
  const text = String(value || "").trim();
  if (!text) throw badRequest(`${name} is required`);
  return text;
}

function positiveInteger(value, name) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) {
    throw badRequest(`${name} must be a positive integer`);
  }
  return number;
}

function optionalPositiveInteger(value, name) {
  if (value === undefined || value === null || String(value).trim() === "") return null;
  return positiveInteger(value, name);
}

function truthy(value) {
  return value === true || value === "true" || value === "1" || value === 1;
}

function pushOptional(args, flag, value) {
  if (value !== undefined && value !== null && String(value).trim() !== "") {
    args.push(flag, String(value));
  }
}

function currentStateArgs(input) {
  const args = ["current", requiredText(input.state_key, "state_key"), "--json"];
  pushOptional(args, "--project", input.project);
  pushOptional(args, "--type", input.memory_type);
  pushOptional(args, "--owner-scope", input.owner_scope);
  pushOptional(args, "--owner-key", input.owner_key);
  pushOptional(args, "--as-of-epoch", input.as_of_epoch);
  return args;
}

function commitLookupArgs(input) {
  const args = ["commit", "show", requiredText(input.sha, "sha"), "--json"];
  pushOptional(args, "--project", input.project);
  return args;
}

function sessionCommitsArgs(input) {
  const args = ["commit", "session", requiredText(input.session_id, "session_id"), "--json"];
  pushOptional(args, "--project", input.project);
  pushOptional(args, "--limit", input.limit || 20);
  return args;
}

function timelineAroundArgs(input) {
  const args = ["timeline", "around", "--json"];
  const anchor = optionalPositiveInteger(input.anchor, "anchor");
  const query = String(input.query || "").trim();
  if (anchor !== null) args.push("--anchor", String(anchor));
  if (query) args.push("--query", query);
  if (anchor === null && !query) throw badRequest("anchor or query is required");
  pushOptional(args, "--project", input.project);
  pushOptional(args, "--depth-before", input.depth_before);
  pushOptional(args, "--depth-after", input.depth_after);
  return args;
}

function timelineReportArgs(input) {
  const args = ["timeline", "report", requiredText(input.project, "project"), "--json"];
  if (truthy(input.full)) args.push("--full");
  return args;
}

function workstreamsListArgs(input) {
  const args = ["workstreams", "list", "--project", requiredText(input.project, "project"), "--json"];
  pushOptional(args, "--status", input.status);
  return args;
}

function workstreamUpdateArgs(input) {
  const args = [
    "workstreams",
    "update",
    String(positiveInteger(input.id, "id")),
    "--project",
    requiredText(input.project, "project"),
    "--json"
  ];
  pushOptional(args, "--status", input.status);
  pushOptional(args, "--next-action", input.next_action);
  pushOptional(args, "--blockers", input.blockers);
  if (!args.includes("--status") && !args.includes("--next-action") && !args.includes("--blockers")) {
    throw badRequest("status, next_action, or blockers is required");
  }
  if (!truthy(input.confirm)) throw badRequest("confirm is required");
  args.push("--confirm");
  return args;
}

function createTraceBackend(runRememJson) {
  const run = (args, timeoutMs = 15000) => runRememJson(args, {
    allowDownload: false,
    timeoutMs
  });
  return {
    currentState: (input) => run(currentStateArgs(input)),
    commitLookup: (input) => run(commitLookupArgs(input)),
    sessionCommits: (input) => run(sessionCommitsArgs(input)),
    timelineAround: (input) => run(timelineAroundArgs(input)),
    timelineReport: (input) => run(timelineReportArgs(input)),
    workstreamsList: (input) => run(workstreamsListArgs(input)),
    workstreamUpdate: (input) => run(workstreamUpdateArgs(input))
  };
}

async function callTraceTool(backend, name, args, toolResult) {
  if (name === "remem_current_state") {
    const result = await backend.currentState(args);
    return toolResult(`Current state ${result.status || "resolved"}.`, result);
  }
  if (name === "remem_commit_lookup") {
    const result = await backend.commitLookup(args);
    return toolResult(`Found ${Array.isArray(result) ? result.length : 0} commit match(es).`, {
      results: result
    });
  }
  if (name === "remem_session_commits") {
    const result = await backend.sessionCommits(args);
    return toolResult(`Found ${Array.isArray(result) ? result.length : 0} linked commit(s).`, {
      results: result
    });
  }
  if (name === "remem_timeline_around") {
    const result = await backend.timelineAround(args);
    return toolResult(`Loaded ${result.count || 0} timeline observation(s).`, result);
  }
  if (name === "remem_timeline_report") {
    const result = await backend.timelineReport(args);
    return toolResult("Timeline report generated.", result);
  }
  if (name === "remem_workstreams_list") {
    const result = await backend.workstreamsList(args);
    return toolResult(`Loaded ${result.count || 0} workstream(s).`, result);
  }
  if (name === "remem_workstream_update") {
    const result = await backend.workstreamUpdate(args);
    return toolResult(`Workstream ${result.id} update ${result.updated ? "applied" : "not applied"}.`, result);
  }
  return null;
}

module.exports = {
  callTraceTool,
  createTraceBackend,
  pushOptional,
  requiredText
};
