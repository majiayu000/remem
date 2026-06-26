document.documentElement.classList.add("js");

const installCommands = {
  quick: "curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh",
  homebrew: "brew install majiayu000/tap/remem\nremem install --target all",
  npm: "npm install -g @remem-ai/remem\nremem install --target all",
  cargo: "cargo install remem-ai --bin remem\nremem install --target all",
};

const copy = {
  en: {
    skip: "Skip to content",
    nav_how: "How it works",
    nav_search: "Search",
    nav_usage: "Usage",
    nav_bench: "Benchmarks",
    nav_install: "Install",
    nav_github: "GitHub",
    hero_eyebrow: "Local-first memory for Claude Code and OpenAI Codex",
    hero_title: "Stop re-explaining your project every new session.",
    hero_lead: "An open-source Rust CLI and MCP memory server that automatically captures, searches, audits, injects, and verifies local project memory across Claude Code, OpenAI Codex, and Codex CLI sessions.",
    hero_primary: "Get started",
    hero_secondary: "View on GitHub",
    trust_binary: "Single Rust binary",
    trust_local: "Localhost only",
    trust_sqlcipher: "SQLCipher at rest",
    trust_mit: "MIT licensed",
    problem_kicker: "The problem",
    problem_title: "Every new session starts from zero.",
    problem_lead: "Context you spent hours building disappears the moment the thread ends.",
    problem_1_title: "Session amnesia",
    problem_1_body: "Each new Claude Code or Codex session starts without the context you already earned.",
    problem_2_title: "Lost rationale",
    problem_2_body: "Design decisions and bug-fix root causes vanish when the session ends.",
    problem_3_title: "Preference fatigue",
    problem_3_body: "The same project preferences get repeated instead of carried forward.",
    problem_4_title: "No continuity",
    problem_4_body: "Long-running work is hard to resume with confidence after context compaction or a fresh session.",
    how_kicker: "How it works",
    how_title: "Hooks into your agents. Nothing to capture by hand.",
    how_lead: "remem records raw session evidence, distills it in the background, then serves compact memory back through hooks and MCP.",
    flow_1: "Inject relevant project memory and preferences.",
    flow_2: "Register sessions and flush stale queues.",
    flow_3: "Queue tool observations without blocking the workflow.",
    flow_4: "Summarize and promote durable memories in the background.",
    search_kicker: "Search architecture",
    search_title: "Four retrieval channels, fused.",
    search_lead: "A query is routed through BM25, entities, temporal parsing, and LIKE fallback, then merged with Reciprocal Rank Fusion.",
    search_feat_1: "Project-scoped search prevents cross-project leakage.",
    search_feat_2: "CJK segmentation and Chinese-English synonym expansion improve recall.",
    search_feat_3: "Title-weighted BM25 and topic-key dedup keep results compact.",
    proof_kicker: "Product proof",
    proof_title: "The memory arrives before the work begins.",
    proof_lead: "Current releases ship through GitHub, crates.io, npm, and Homebrew, with reproducible artifact checks guarding public benchmark and claim wording.",
    proof_caption: "Terminal demo from the remem repository assets.",
    bench_kicker: "Benchmarks",
    bench_title: "Measured, not hand-waved.",
    bench_lead: "Public artifacts now separate memory-system capability evidence from coding-agent outcome claims.",
    metric_artifacts_label: "Public artifacts",
    metric_artifacts: "4 manifests, 4 reports, and 25 run artifacts verified.",
    metric_memory_label: "Memory QA suite",
    metric_memory: "Temporal answers, stale decisions, conflicts, file anchors, and user-context relevance.",
    metric_policy_label: "Policy suite",
    metric_policy: "Secrets, credentials, unsafe claims, stale anchors, and unresolved conflicts.",
    metric_claim_label: "Claim gate",
    metric_claim: "No SOTA or coding-task superiority claim until the public gate passes.",
    usage_kicker: "Usage & cost",
    usage_title: "Measured locally, reported as estimates.",
    usage_lead: "remem records AI usage for its own background extraction, summaries, compression, and promotion calls. The CLI reports token usage and estimated USD cost by day, week, source, project, and executor.",
    usage_feat_1: "Provider/log usage is separated from text-estimated fallback rows.",
    usage_feat_2: "Cost is an estimate, not a provider invoice or subscription bill.",
    usage_feat_3: "Executor and model selection live in ~/.remem/config.toml.",
    usage_feat_4: "The local audit below is a snapshot from 2026-06-06; usage varies by workload.",
    usage_channel_1: "Daily and weekly totals",
    usage_channel_2: "Project-scoped reports",
    usage_channel_3: "Usage source precision",
    usage_channel_4: "Estimated USD cost",
    usage_formula: "provider/log rows stay separate from estimates",
    usage_result: "cost estimate, not invoice",
    usage_audit_project_share: "Project ledger share",
    usage_audit_project_share_body: "This repository's remem ledger was $6.18 against $17,595.10 of total agent spend.",
    usage_audit_global_share: "Full remem ledger share",
    usage_audit_global_share_body: "All remem background memory calls across projects were $437.15 in the same window.",
    usage_audit_project_cost: "Project ledger cost",
    usage_audit_project_cost_body: "Estimated background memory cost for this repository over 8 weeks.",
    usage_audit_total_spend: "Total agent spend",
    usage_audit_total_spend_body: "ccstats all-source spend over the same 2026-04-12 to 2026-06-06 window.",
    install_kicker: "Install",
    install_title: "Use the same commands as the README.",
    install_lead: "Start with Homebrew, npm, the prebuilt GitHub Release installer, or Cargo. Then restart your coding agent.",
    tab_quick: "Quick install",
    copy: "Copy",
    copied: "Copied",
    security_title: "Secure by default",
    security_1: "SQLCipher encryption at rest.",
    security_2: "Data directory 0700 and key file 0600.",
    security_3: "REST API binds to 127.0.0.1 and requires a local token.",
    security_4: "Uninstall removes hooks and MCP config without deleting memory data.",
    cta_title: "Give Claude Code and OpenAI Codex a local memory layer.",
    cta_lead: "Single binary. Local store. Searchable, auditable context for long-running engineering work.",
    cta_primary: "Install remem",
    cta_secondary: "Read the docs",
    footer_tagline: "Local-first memory for Claude Code and OpenAI Codex.",
    footer_product: "Product",
    footer_resources: "Resources",
    footer_community: "Community",
    footer_built: "Built as a static site in the remem repository.",
  },
  zh: {
    skip: "跳到正文",
    nav_how: "工作原理",
    nav_search: "检索",
    nav_usage: "用量",
    nav_bench: "基准测试",
    nav_install: "安装",
    nav_github: "GitHub",
    hero_eyebrow: "Claude Code 和 OpenAI Codex 的本地优先记忆层",
    hero_title: "别再在每个新会话里重新解释你的项目。",
    hero_lead: "一个开源 Rust CLI 和 MCP memory server，自动捕获、搜索、审计、注入并验证 Claude Code、OpenAI Codex 与 Codex CLI 会话间的本地项目记忆。",
    hero_primary: "开始使用",
    hero_secondary: "在 GitHub 查看",
    trust_binary: "单文件 Rust 二进制",
    trust_local: "仅绑定本机",
    trust_sqlcipher: "SQLCipher 静态加密",
    trust_mit: "MIT 许可",
    problem_kicker: "问题所在",
    problem_title: "每个新会话都从零开始。",
    problem_lead: "你花数小时建立的上下文，会在会话结束时消失。",
    problem_1_title: "会话失忆",
    problem_1_body: "每个新的 Claude Code 或 Codex 会话都拿不到你已经积累的项目上下文。",
    problem_2_title: "理由丢失",
    problem_2_body: "设计决策、修复根因和取舍记录，会在会话结束后断掉。",
    problem_3_title: "偏好疲劳",
    problem_3_body: "同样的项目偏好被一遍遍重复，而不是自动延续。",
    problem_4_title: "缺乏连续性",
    problem_4_body: "遇到上下文压缩或新会话时，长期工作很难可靠接上。",
    how_kicker: "工作原理",
    how_title: "接入你的 Agent，无需手动记录。",
    how_lead: "remem 先记录原始会话证据，在后台提炼，再通过 hooks 和 MCP 把紧凑记忆送回会话。",
    flow_1: "注入相关项目记忆与偏好。",
    flow_2: "注册会话并清理过期队列。",
    flow_3: "把工具观察入队，不阻塞工作流。",
    flow_4: "在后台总结并提升为耐久记忆。",
    search_kicker: "检索架构",
    search_title: "四路检索，融合排序。",
    search_lead: "查询会同时进入 BM25、实体、时间解析和 LIKE 兜底，再用倒数排名融合合并。",
    search_feat_1: "项目级检索避免跨项目泄漏。",
    search_feat_2: "CJK 分词和中英文同义词扩展提升召回。",
    search_feat_3: "标题加权 BM25 和 topic_key 去重让结果更紧凑。",
    proof_kicker: "产品证据",
    proof_title: "工作开始前，记忆已经到位。",
    proof_lead: "当前 release 已覆盖 GitHub、crates.io、npm 和 Homebrew，并用可复现 artifact check 约束公开 benchmark 与 claim 文案。",
    proof_caption: "来自 remem 仓库 assets 的终端演示。",
    bench_kicker: "基准测试",
    bench_title: "用数据说话。",
    bench_lead: "公开 artifacts 现在把 memory-system capability evidence 和 coding-agent outcome claim 分开。",
    metric_artifacts_label: "Public artifacts",
    metric_artifacts: "已验证 4 个 manifest、4 个 report 和 25 个 run artifact。",
    metric_memory_label: "Memory QA suite",
    metric_memory: "覆盖时间回答、stale decision、conflict、file anchor 和 user-context relevance。",
    metric_policy_label: "Policy suite",
    metric_policy: "覆盖 secrets、credentials、unsafe claim、stale anchor 和 unresolved conflict。",
    metric_claim_label: "Claim gate",
    metric_claim: "public gate 通过前，不发布最强类或编码任务效果优越类公开声明。",
    usage_kicker: "用量与费用",
    usage_title: "本地计量，按估算报告。",
    usage_lead: "remem 会记录自身后台抽取、总结、压缩和提升记忆时产生的 AI 用量。CLI 可以按天、周、来源、项目和执行器报告 token 与估算美元成本。",
    usage_feat_1: "provider/log 精确用量会和 text_estimate 兜底估算分开显示。",
    usage_feat_2: "费用是估算值，不是供应商账单或订阅账单。",
    usage_feat_3: "执行器和模型选择写在 ~/.remem/config.toml。",
    usage_feat_4: "下面的本地 audit 是 2026-06-06 的快照；实际用量会随工作负载变化。",
    usage_channel_1: "每日与每周汇总",
    usage_channel_2: "按项目查看报告",
    usage_channel_3: "用量来源精度",
    usage_channel_4: "估算美元费用",
    usage_formula: "provider/log 用量会和估算用量分开",
    usage_result: "费用估算，不是账单",
    usage_audit_project_share: "项目账本占比",
    usage_audit_project_share_body: "这个仓库的 remem usage ledger 为 $6.18，对比总 Agent 消费 $17,595.10。",
    usage_audit_global_share: "全局 remem 占比",
    usage_audit_global_share_body: "同一窗口内，所有项目的 remem 后台记忆调用合计 $437.15。",
    usage_audit_project_cost: "项目账本费用",
    usage_audit_project_cost_body: "这个仓库 8 周后台记忆任务的估算成本。",
    usage_audit_total_spend: "总 Agent 消费",
    usage_audit_total_spend_body: "ccstats all-source 在 2026-04-12 到 2026-06-06 同窗口的消费。",
    install_kicker: "安装",
    install_title: "使用和 README 一致的命令。",
    install_lead: "可以从 Homebrew、npm、GitHub Release 快速安装器或 Cargo 开始。安装后重启你的 coding agent。",
    tab_quick: "快速安装",
    copy: "复制",
    copied: "已复制",
    security_title: "默认安全",
    security_1: "SQLCipher 静态加密。",
    security_2: "数据目录 0700，密钥文件 0600。",
    security_3: "REST API 仅绑定 127.0.0.1，并要求本地 token。",
    security_4: "卸载只移除 hooks 和 MCP 配置，不删除记忆数据。",
    cta_title: "给 Claude Code 和 OpenAI Codex 一层本地记忆。",
    cta_lead: "单文件二进制。本地存储。为长期工程工作保留可搜索、可审计的上下文。",
    cta_primary: "安装 remem",
    cta_secondary: "阅读文档",
    footer_tagline: "Claude Code 和 OpenAI Codex 的本地优先记忆层。",
    footer_product: "产品",
    footer_resources: "资源",
    footer_community: "社区",
    footer_built: "作为静态站点维护在 remem 仓库中。",
  },
};

function setLanguage(lang) {
  const selected = copy[lang] ? lang : "en";
  document.documentElement.lang = selected === "zh" ? "zh-CN" : "en";
  localStorage.setItem("remem-site-lang", selected);

  document.querySelectorAll("[data-i18n]").forEach((node) => {
    const key = node.getAttribute("data-i18n");
    if (copy[selected][key]) node.textContent = copy[selected][key];
  });

  document.querySelectorAll("[data-i18n-html]").forEach((node) => {
    const key = node.getAttribute("data-i18n-html");
    if (copy[selected][key]) node.innerHTML = copy[selected][key];
  });

  document.querySelectorAll("[data-lang]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.lang === selected));
  });
}

document.querySelectorAll("[data-lang]").forEach((button) => {
  button.addEventListener("click", () => setLanguage(button.dataset.lang));
});

const installCode = document.getElementById("install-code");
document.querySelectorAll("[data-command]").forEach((button) => {
  button.addEventListener("click", () => {
    const key = button.dataset.command;
    installCode.textContent = installCommands[key] || installCommands.quick;
    document.querySelectorAll("[data-command]").forEach((tab) => {
      tab.setAttribute("aria-selected", String(tab === button));
    });
  });
});

async function writeClipboard(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch (_) {
      // Fall through to the textarea fallback for restricted browser contexts.
    }
  }
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  document.execCommand("copy");
  textarea.remove();
}

document.querySelector("[data-copy-command]").addEventListener("click", async (event) => {
  const button = event.currentTarget;
  const lang = document.documentElement.lang.startsWith("zh") ? "zh" : "en";
  await writeClipboard(installCode.textContent);
  button.querySelector("span").textContent = copy[lang].copied;
  window.setTimeout(() => {
    button.querySelector("span").textContent = copy[lang].copy;
  }, 1400);
});

const observer = new IntersectionObserver(
  (entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      }
    });
  },
  { threshold: 0.12 }
);

document.querySelectorAll(".reveal").forEach((node) => observer.observe(node));

setLanguage(localStorage.getItem("remem-site-lang") || "en");
