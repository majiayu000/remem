document.documentElement.classList.add("js");

const installCommands = {
  quick: "curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh",
  homebrew: "brew install majiayu000/tap/remem\nremem install --target all",
  cargo: "cargo install remem-ai --bin remem\nremem install --target all",
};

const copy = {
  en: {
    skip: "Skip to content",
    nav_how: "How it works",
    nav_search: "Search",
    nav_usage: "Usage & cost",
    nav_bench: "Benchmarks",
    nav_install: "Install",
    nav_github: "GitHub",
    hero_eyebrow: "Persistent memory for Claude Code and Codex",
    hero_title: "Stop re-explaining your project every new session.",
    hero_lead: "A single Rust binary that automatically captures, distills, and injects project context across sessions: decisions, patterns, preferences, and learnings.",
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
    proof_lead: "The site ships with the repository's existing terminal demo asset, so the first release uses real project media instead of stock visuals.",
    proof_caption: "Terminal demo from the remem repository assets.",
    usage_kicker: "Usage & cost",
    usage_title: "Background memory AI usage stays visible.",
    usage_lead: "remem records usage for its own extraction, summary, compression, and promotion calls. The report is designed for operational visibility, not billing.",
    usage_feat_1: "Memory AI execution is configured in ~/.remem/config.toml.",
    usage_feat_2: "Claude Code host-side memory, system context, and cache usage can remain outside remem's ledger.",
    usage_command_label: "Report command",
    usage_source_anthropic: "Provider-reported usage from the Anthropic Messages API.",
    usage_source_codex: "Exact token counts parsed from codex exec JSON usage events.",
    usage_source_estimate: "Fallback estimates from prompt and response text length.",
    usage_note: "Cost is an estimate, not an invoice. Historical rows may be text estimates or repriced from older rows that did not store the exact model.",
    bench_kicker: "Benchmarks",
    bench_title: "Measured, not hand-waved.",
    bench_lead: "Published README snapshots cover LoCoMo, internal retrieval, and local QA evaluation.",
    metric_locomo: "v2 optimized, session summary ingest, gpt-5.4.",
    metric_mrr: "Internal eval over 1,777 real memories.",
    metric_hit: "Internal retrieval hit rate.",
    metric_leak: "Project-scoped search validation.",
    install_kicker: "Install",
    install_title: "Use the same commands as the README.",
    install_lead: "Start with Homebrew, the prebuilt GitHub Release installer, or Cargo. Then restart your coding agent.",
    tab_quick: "Quick install",
    copy: "Copy",
    copied: "Copied",
    security_title: "Secure by default",
    security_1: "SQLCipher encryption at rest.",
    security_2: "Data directory 0700 and key file 0600.",
    security_3: "REST API binds to 127.0.0.1 and requires a local token.",
    security_4: "Uninstall removes hooks and MCP config without deleting memory data.",
    cta_title: "Give Claude Code and Codex a memory.",
    cta_lead: "Single binary. Local store. Durable context for long-running engineering work.",
    cta_primary: "Install remem",
    cta_secondary: "Read the docs",
    footer_tagline: "Persistent memory for Claude Code and Codex.",
    footer_product: "Product",
    footer_resources: "Resources",
    footer_community: "Community",
    footer_built: "Built as a static site in the remem repository.",
  },
  zh: {
    skip: "跳到正文",
    nav_how: "工作原理",
    nav_search: "检索",
    nav_usage: "用量与成本",
    nav_bench: "基准测试",
    nav_install: "安装",
    nav_github: "GitHub",
    hero_eyebrow: "为 Claude Code 和 Codex 提供持久记忆",
    hero_title: "别再在每个新会话里重新解释你的项目。",
    hero_lead: "一个单文件 Rust 二进制程序，自动捕获、提炼并在会话之间注入项目上下文：决策、模式、偏好与经验。",
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
    proof_lead: "首版官网复用仓库已有终端演示资产，使用真实项目媒体而不是图库视觉。",
    proof_caption: "来自 remem 仓库 assets 的终端演示。",
    usage_kicker: "用量与成本",
    usage_title: "后台记忆 AI 用量保持可见。",
    usage_lead: "remem 会记录自身提取、总结、压缩和提升记忆调用的用量。报告用于运维可见性，不是账单。",
    usage_feat_1: "记忆 AI 执行配置位于 ~/.remem/config.toml。",
    usage_feat_2: "Claude Code 宿主侧记忆、系统上下文和缓存用量可能不在 remem 账本内。",
    usage_command_label: "报告命令",
    usage_source_anthropic: "来自 Anthropic Messages API 的 provider-reported 用量。",
    usage_source_codex: "从 codex exec JSON usage 事件解析出的精确 token 计数。",
    usage_source_estimate: "基于 prompt 和 response 文本长度的兜底估算。",
    usage_note: "成本是估算，不是账单。历史行可能是文本估算，也可能由没有精确模型信息的旧行重新计价而来。",
    bench_kicker: "基准测试",
    bench_title: "用数据说话。",
    bench_lead: "README 已发布 LoCoMo、内部检索和本地 QA 评测快照。",
    metric_locomo: "v2 优化版，会话总结写入，gpt-5.4。",
    metric_mrr: "基于 1,777 条真实记忆的内部评测。",
    metric_hit: "内部检索命中率。",
    metric_leak: "项目级隔离检索验证。",
    install_kicker: "安装",
    install_title: "使用和 README 一致的命令。",
    install_lead: "可以从 Homebrew、GitHub Release 快速安装器或 Cargo 开始。安装后重启你的 coding agent。",
    tab_quick: "快速安装",
    copy: "复制",
    copied: "已复制",
    security_title: "默认安全",
    security_1: "SQLCipher 静态加密。",
    security_2: "数据目录 0700，密钥文件 0600。",
    security_3: "REST API 仅绑定 127.0.0.1，并要求本地 token。",
    security_4: "卸载只移除 hooks 和 MCP 配置，不删除记忆数据。",
    cta_title: "给 Claude Code 和 Codex 一份记忆。",
    cta_lead: "单文件二进制。本地存储。为长期工程工作保留耐久上下文。",
    cta_primary: "安装 remem",
    cta_secondary: "阅读文档",
    footer_tagline: "为 Claude Code 和 Codex 提供持久记忆。",
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
