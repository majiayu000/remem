"use strict";

const UI_RESOURCE = "ui://remem/dashboard.html";

function toolDescriptors() {
  return [
    {
      name: "remem_dashboard",
      title: "Remem Dashboard",
      description: "Render Remem runtime, memory health, search, save, governance, and activation state.",
      inputSchema: {
        type: "object",
        properties: {
          project: { type: "string" }
        },
        additionalProperties: false
      },
      outputSchema: {
        type: "object",
        properties: {
          expected_version: { type: "string" },
          plugin_data: { type: "string" }
        },
        additionalProperties: true
      },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { resourceUri: UI_RESOURCE, visibility: ["model", "app"] },
        "openai/outputTemplate": UI_RESOURCE,
        "openai/widgetAccessible": true,
        "openai/toolInvocation/invoking": "Loading Remem",
        "openai/toolInvocation/invoked": "Remem ready"
      }
    },
    {
      name: "remem_search",
      title: "Search Remem",
      description: "Search curated Remem memories and raw archive fallback rows.",
      inputSchema: {
        type: "object",
        properties: {
          query: { type: "string" },
          project: { type: "string" },
          type: { type: "string" },
          limit: { type: "number" },
          offset: { type: "number" },
          include_stale: { type: "boolean" },
          multi_hop: { type: "boolean" }
        },
        required: ["query"],
        additionalProperties: false
      },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { visibility: ["model", "app"] },
        "openai/widgetAccessible": true
      }
    },
    {
      name: "remem_get_memory",
      title: "Get Remem Memory",
      description: "Fetch full details for a selected Remem memory by ID.",
      inputSchema: {
        type: "object",
        properties: { id: { type: "number" } },
        required: ["id"],
        additionalProperties: false
      },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { visibility: ["model", "app"] },
        "openai/widgetAccessible": true
      }
    },
    {
      name: "remem_save_memory",
      title: "Save Remem Memory",
      description: "Explicitly save one durable Remem memory.",
      inputSchema: {
        type: "object",
        properties: {
          text: { type: "string" },
          title: { type: "string" },
          project: { type: "string" },
          memory_type: { type: "string" },
          topic_key: { type: "string" },
          scope: { type: "string" }
        },
        required: ["text"],
        additionalProperties: false
      },
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { visibility: ["model", "app"] },
        "openai/widgetAccessible": true
      }
    },
    {
      name: "remem_activation_plan",
      title: "Remem Activation Plan",
      description: "Preview Codex hook activation without writing config.",
      inputSchema: { type: "object", properties: {}, additionalProperties: false },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { visibility: ["model", "app"] },
        "openai/widgetAccessible": true
      }
    },
    {
      name: "remem_governance_preview",
      title: "Preview Remem Governance",
      description: "Dry-run stale, reject, or delete governance for selected Remem memories.",
      inputSchema: {
        type: "object",
        properties: {
          action: { type: "string", enum: ["stale", "reject", "delete"] },
          ids: { type: "array", items: { type: "number" } },
          project: { type: "string" },
          query: { type: "string" },
          memory_type: { type: "string" },
          status: { type: "string" },
          limit: { type: "number" },
          offset: { type: "number" },
          reason: { type: "string" },
          actor: { type: "string" }
        },
        required: ["action"],
        additionalProperties: false
      },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: {
        ui: { visibility: ["model", "app"] },
        "openai/widgetAccessible": true
      }
    }
  ];
}

module.exports = {
  toolDescriptors,
  UI_RESOURCE
};
