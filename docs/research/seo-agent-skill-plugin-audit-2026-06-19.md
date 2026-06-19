---
status: research
date: 2026-06-19
scope: Claude Code / Codex / agent SEO skills, plugins, MCP connectors
method: threads research_spec audit
---

# SEO Agent Skill / Plugin 生态审计与可借鉴设计

## 结论

这轮审计的结论是：原推荐的大方向成立，但不能按营销口径直接安装或复用代码。

- **Claude Code 用户首选 `AgriciDaniel/claude-seo`**：它是当前最完整的 Claude Code SEO plugin/skill suite，GitHub API 显示约 9.2k stars，MIT，覆盖 `/seo audit`、GEO/AEO、Schema、CWV、Local、Backlinks、Google APIs、DataForSEO、Firecrawl 等。
- **Codex 方向只能把 `AgriciDaniel/codex-seo` 当架构参考，不能直接借代码**：它是最接近 Codex 原生形态的候选，有 `.codex-plugin/plugin.json`、skills、hooks、TOML agents、deterministic runners；但根 `LICENSE` 是专有许可，manifest/pyproject 又标 MIT，许可冲突必须先解决。
- **最值得借鉴的不是某个 SEO 规则，而是工程结构**：Skill Contract、orchestrator + specialist skills、deterministic runners、artifact-first reports、`setup_required` 状态、inventory drift checks、reference freshness checks、MCP credential boundaries。
- **MCP 数据层要分级接入**：先用 Google Search Console / PageSpeed / CrUX 这类 owned-site truth，再接 Firecrawl 抓取层，再按预算接 SE Ranking / DataForSEO / Ahrefs。不要一开始把所有本地 `npx` MCP 都接进 agent。
- **第三方 Skill/Plugin 按软件供应链处理**：`SKILL.md` 是 prompt code，installer/hook/MCP 是 executable integration surface。默认 clone + inspect，不默认 `curl | bash`、`npx -y latest`、Docker `:latest`。

本报告没有执行任何第三方安装脚本，没有使用 SEO 付费 API 凭据，也没有实际跑目标网站审计。因此能力判断分为两类：GitHub/API/README/manifest 证据，以及未 runtime 验证的项目声称。

## 审计方法

使用 `threads` 拆成三条 read-only research lane：

| Lane | 范围 | 产出 |
|---|---|---|
| `seo-suite-audit` | `claude-seo` / `codex-seo` / `claude-blog` | 主推套件、Codex readiness、许可冲突、delivery gates |
| `multi-platform-audit` | `Agentic-SEO-Skill` / `seo-geo-claude-skills` / `seo-skill` / `seo-audit-skill` / `agentic-seo` | 多平台 skill 形态、CLI wrapper 形态、AEO checker |
| `mcp-risk-audit` | Firecrawl / DataForSEO / SE Ranking / Ahrefs / Google APIs | 数据源价值、凭据边界、安装与供应链风险 |

主线程同步使用 GitHub API、README、manifest、installer、官方 docs 做交叉验证。验证日期是 2026-06-19。

## 候选项目快照

| 项目 | 当前证据 | 适配面 | 判断 |
|---|---|---|---|
| [`AgriciDaniel/claude-seo`](https://github.com/AgriciDaniel/claude-seo) | GitHub API：9235 stars、1335 forks、MIT、updated 2026-06-19；HEAD `d830cdb2`；`.claude-plugin/plugin.json` v2.2.0 | Claude Code plugin / skill suite | 最完整的 Claude Code SEO 候选，可优先试用，但仍需安装前审计脚本和依赖 |
| [`AgriciDaniel/codex-seo`](https://github.com/AgriciDaniel/codex-seo) | GitHub API：282 stars、44 forks、license `NOASSERTION`；HEAD `97c59bcd`；`.codex-plugin/plugin.json` 标 MIT，根 `LICENSE` 为 proprietary | Codex plugin / skills / TOML agents | Codex 形态最好，但许可冲突阻止代码复用；只借鉴架构 |
| [`AgriciDaniel/claude-blog`](https://github.com/AgriciDaniel/claude-blog) | GitHub API：1101 stars、204 forks、MIT；HEAD `49842ea9` | Claude Code content/blog workflow | 借鉴 5-gate delivery contract、artifact manifest、review/preflight 模式 |
| [`Bhanunamikaze/Agentic-SEO-Skill`](https://github.com/Bhanunamikaze/Agentic-SEO-Skill) | GitHub API：668 stars、108 forks、MIT；HEAD `69199160`；README claims 16 sub-skills、10 agents、89 scripts | Claude / Codex / Cursor / Antigravity / Windsurf / Continue 等多宿主 | 借鉴 host adapter matrix 和 evidence scripts；installer 写入面大，需谨慎 |
| [`aaron-he-zhu/seo-geo-claude-skills`](https://github.com/aaron-he-zhu/seo-geo-claude-skills) | GitHub API：2168 stars、301 forks、Apache-2.0；HEAD `dea9f4d6`；`.claude-plugin/plugin.json` 列 20 skills、5 commands | Claude Code、generic Agent Skills hosts、Codex/Cursor 兼容声称 | 最值得借鉴 Skill Contract / handoff / protocol layer |
| [`aevans-eng/seo-skill`](https://github.com/aevans-eng/seo-skill) | GitHub API：4 stars、1 fork、MIT；单 `SKILL.md` | Claude Code static-site SEO | 可当最小 skill 示例，不适合主力审计 |
| [`seo-skills/seo-audit-skill`](https://github.com/seo-skills/seo-audit-skill) | GitHub API：289 stars、37 forks、MIT；README 说 251 rules / 20 categories；npm CLI + Electron | CLI / Electron + skill wrapper | 借鉴 deterministic CLI、LLM-optimized output；不是纯 agent skill |
| [`addyosmani/agentic-seo`](https://github.com/addyosmani/agentic-seo) | GitHub API：236 stars、35 forks、MIT；npm `agentic-seo` | AEO/docs readiness CLI | 借鉴 agent-ready docs 检查：`llms.txt`、`AGENTS.md`、`skill.md`、token budgets |

## 关键纠偏

### `claude-seo` 是强候选，但不是 Codex 原生

`claude-seo` 的 README 和 plugin manifest 都确认它是 Claude Code plugin/skill suite。它 README 明确把 Codex 用户导向 `codex-seo`。因此如果目标是 Claude Code SEO agent，优先级最高；如果目标是 Codex 原生工作流，不能把它直接当 Codex plugin。

可借鉴点：

- 统一 `/seo` orchestrator。
- 25 个 skill command 面向完整 SEO lifecycle。
- specialist agent fan-out + synthesis。
- audit report artifact，而不是 chat-only 答案。
- Google / Firecrawl / DataForSEO 等 provider extension 只作为增强层。

### `codex-seo` 架构有价值，但许可阻塞

`codex-seo` 的结构最接近我们想要的 Codex-native SEO plugin：

- `.codex-plugin/plugin.json`
- `skills/seo/SKILL.md` orchestrator
- `skills/seo-*/SKILL.md` specialist workflows
- `agents/seo-*.toml`
- `scripts/run_skill_workflow.py`
- `.seo-cache`
- `output/` artifacts
- optional MCP/API setup

但许可有硬风险：根 `LICENSE` 是 proprietary license，禁止复制、再分发、为竞争产品创建衍生作品；同一仓库的 manifest 和 pyproject 标 MIT。处理原则：

- 可以学习目录结构和状态机设计。
- 不复制代码、文本、prompt、runner 实现。
- 如果要 fork 或嵌入，先让上游澄清许可证。

### 多平台项目的价值在 adapter，不在全量安装

`Agentic-SEO-Skill` 的多宿主 installer 支持 Claude、Codex、Cursor、Windsurf、Continue、Antigravity 等路径，这对设计兼容层有价值。但它也说明风险：一个 installer 能写入多个 agent 的技能目录、rule 文件、脚本目录，供应链影响面很大。

可借鉴：

- core skill 内容与 host-specific adapter 分离。
- 每个宿主有明确 install target。
- scripts 作为 evidence collectors，而不是让 LLM 凭空判断。

不要借鉴：

- 默认全宿主安装。
- 未 pin 的 online installer。
- 自动写入多个 agent 配置而无 dry-run/diff。

### `seo-geo-claude-skills` 的 contract 最适合复用

这个项目不是最复杂的，但它的组织方式很值得学习：

- 20 skills 分成 Research / Build / Optimize / Monitor / Cross-cutting。
- 5 个 command entrypoints。
- 每个 skill 遵循同一 activation contract。
- 有 Handoff Summary、Data Sources、Reference Materials、Next Best Skill。
- Cross-cutting 层包含 content quality、domain authority、entity、memory-management。

这比“一个超大 skill 里塞所有规则”更适合长期维护，也更适合 remem 这种需要可追踪、可复用上下文的系统。

## MCP 与数据源顺序

| 层级 | 数据源 | 用途 | 风险 |
|---|---|---|---|
| 1 | Google Search Console API | owned-site clicks、impressions、positions、sitemaps、URL inspection | OAuth/refresh token 管理 |
| 1 | PageSpeed Insights / CrUX | CWV、Lighthouse、field/lab performance | API key quota；PSI field data 变化需跟踪 |
| 2 | Firecrawl MCP | JS-rendered crawl、scrape、map、structured extraction | 本地 `npx` MCP 供应链风险；API credit |
| 3 | SE Ranking remote MCP | keyword、rank、backlink、domain、AI visibility、audit | 付费计划、OAuth/API key、tool scope |
| 4 | DataForSEO MCP/API | backend-grade SERP、keyword、backlink、business、AI optimization data | login/password/API cost，需要预算阈值 |
| 5 | Ahrefs remote MCP | backlink / refdomain / competitive intelligence | paid Lite+、API units；不要用 archived local repo |
| Niche | Google Indexing API | `JobPosting` / `BroadcastEvent` URL update/remove | 不适合普通页面提交 |
| Avoid new core dependency | Google Custom Search JSON API | SERP snippets | 对新客户关闭/生命周期风险 |

推荐顺序：

1. 先接 Google Search Console + PageSpeed / CrUX，获取低风险 owned-site truth。
2. 再接 Firecrawl，用于抓取、渲染和页面证据。
3. 需要完整 SEO/GEO workflow 时先试 SE Ranking remote MCP。
4. 需要可编程规模化数据时接 DataForSEO。
5. 只有已有 Ahrefs 付费计划或明确 backlink 深度需求时再接 Ahrefs remote MCP。

## 可借鉴设计清单

### 1. Skill Contract

每个 skill 都应有固定字段：

- Activation / trigger
- Inputs
- Data sources
- Allowed reads/writes
- Forbidden files
- Output artifact contract
- Confidence labels
- Handoff summary
- Next best skill
- Done-when / verification

这个结构来自 `seo-geo-claude-skills` 的强项，适合迁移到任何自定义 SEO / AEO / discoverability skill。

### 2. Orchestrator + specialist workflows

不要让一个 SEO skill 同时做所有事。推荐：

- `seo-orchestrator`：识别目标、行业、数据可用性、风险、输出计划。
- `technical-seo`：crawlability、indexability、CWV、schema validation。
- `content-quality`：E-E-A-T、claim evidence、readability、thin/duplicate content。
- `geo-aeo`：answer blocks、entity coverage、AI citation readiness。
- `local-seo`：GBP、NAP、reviews、local schema。
- `data-sources`：GSC、PSI/CrUX、Firecrawl、SE Ranking、DataForSEO、Ahrefs。
- `reporter`：合并 evidence，输出 action plan。

### 3. Deterministic runners

LLM 不应该直接声称“这个页面 SEO 很差”。它应先调用 deterministic runner 产出 evidence：

- HTML/meta/schema parser
- robots/sitemap checker
- canonical/indexability matrix
- PageSpeed/CrUX fetcher
- crawl summary
- broken links
- structured data validator
- report generator

输出格式至少包括：

- `SUMMARY.json`
- `FINDINGS.json`
- `FULL-AUDIT-REPORT.md`
- `ACTION-PLAN.md`
- optional HTML/PDF

### 4. `setup_required` contract

缺少 API key 或 MCP server 时，工具必须返回明确状态：

```text
status: setup_required
provider: dataforseo
missing: DATAFORSEO_LOGIN, DATAFORSEO_PASSWORD
fallback_used: local_html_only
data_not_available: live_serp, search_volume, backlinks
```

不要用 warning 后继续输出伪实时数据。这一点和本 repo 的 no silent degradation 规则一致。

### 5. Evidence confidence labels

每条 finding 至少标：

- `confirmed`: deterministic tool 或 first-party API 证明。
- `likely`: 多个弱信号一致，但缺少直接证据。
- `hypothesis`: 需要人工或付费数据验证。

### 6. Inventory drift checks

借鉴 `Agentic-SEO-Skill` 的 inventory/freshness 思路，为 skill pack 加 CI：

- skill count 是否和 README/manifest 一致。
- script count 是否和 docs 一致。
- reference `Updated:` 是否超过 90 天。
- plugin manifest paths 是否存在。
- command namespace 是否和 docs 一致。
- optional provider 状态是否返回 `setup_required` 而不是成功。

### 7. Agent-ready docs / AEO readiness

借鉴 `agentic-seo` 的方向，把“给 AI agent 读懂”当成单独检查：

- `llms.txt`
- `AGENTS.md` / `CLAUDE.md`
- `skill.md` or `SKILL.md`
- agent permissions / access rules
- token budget per page
- headings and content chunks that survive single HTTP fetch
- Copy-for-AI blocks for docs/API references

### 8. Delivery gates

借鉴 `claude-blog` 的 delivery gate 思路，SEO workflow 也应有发布前 gate：

- preflight: dependencies, credentials, target ownership
- evidence gate: no finding without evidence
- quality gate: severity/confidence/actionability present
- security gate: no secret leakage, no unsafe install
- final artifact gate: report/action plan/schema changes generated and verified

## 安装与供应链规则

默认流程：

```bash
git clone <repo>
cd <repo>
rg -n "(curl|wget|npx|npm install|pip install|eval|exec|process.env|API_KEY|TOKEN|SECRET|hooks|mcpServers|rm -rf|chmod|launchctl|crontab)"
find . -maxdepth 3 -type f | sort
```

安装前必须看：

- `SKILL.md`
- plugin manifest：`.codex-plugin/plugin.json` 或 `.claude-plugin/plugin.json`
- `install.sh` / `install.ps1`
- `hooks/`
- `.mcp.json`
- `requirements*.txt`
- `package.json`
- license
- any scripts that run network or browser automation

安全默认值：

- 不默认 `curl | bash`。
- 不默认 `npx -y <package>`，至少 pin version。
- 不用 Docker `:latest` 做长期安装。
- credentials 放用户配置目录或 secret manager，不进 repo。
- MCP tool 先 allowlist，再逐步扩。
- local MCP 优先 sandbox；能用 remote OAuth 的 provider 优先 remote。
- hooks 必须 dry-run/diff 后启用。

OpenAI Codex 官方文档说明 plugin 可以分发 skills、apps、MCP servers 和 hooks；plugin-provided MCP server 可以在 Codex config 中控制 enable/disable、enabled tools、approval mode。因此第三方 SEO plugin 的 MCP server 不应默认全工具自动批准。

Anthropic 对 Claude Skills 的公开安全建议也明确：第三方 skill 可能包含或指示安装第三方软件，应从可信来源安装，并在使用低信任来源时审计 bundled files、dependencies、scripts、resources 和外部网络连接指令。

MCP 官方安全文档把本地 MCP server 列为高风险面：不受信任或限制不足的本地 MCP server 可能带来 arbitrary code execution、data exfiltration、data loss 等风险。

## 对 remem / 自定义 Agent 系统的建议

如果后续要做自己的 SEO / AEO / discoverability skill，不建议直接 fork 一个大 SEO repo。更合适的路线：

1. **先做 contract-first minimal skill pack**
   采用 `seo-geo-claude-skills` 的 Skill Contract 和 handoff 模式，先做 4-6 个核心 skill。

2. **保留 deterministic runner 边界**
   参考 `codex-seo` / SEOmator 的 runner/artifact 模式，但实现自己写，避免许可和供应链问题。

3. **只引入一层 live data**
   第一版只接 GSC + PageSpeed/CrUX。Firecrawl、SE Ranking、DataForSEO、Ahrefs 分阶段接。

4. **把 missing data 做成一等状态**
   `no_data` / `setup_required` / `permission_missing` / `quota_exceeded` 都要进入 report，不要被 LLM 总结吞掉。

5. **把 audit 结果做成可保存记忆**
   对 remem 来说，SEO audit 产物可拆成：
   - project preference: preferred domain/canonical/site type
   - decision: chosen SEO strategy
   - discovery: recurring technical findings
   - workstream: open remediation plan

6. **不要从 SEO skill 直接改高上下文文件**
   `AGENTS.md`、hooks、MCP config、plugin manifests 必须走 explicit plan + diff。

## 推荐采用/暂缓清单

| 动作 | 结论 | 原因 |
|---|---|---|
| 在 Claude Code 里试 `claude-seo` | 可以，先 clone 审计再安装 | 生态最成熟，MIT，但 installer/deps 仍需审查 |
| 在 Codex 里直接装 `codex-seo` | 暂缓 | 许可冲突；未 runtime 验证；不要复制实现 |
| 借鉴 `codex-seo` 目录结构 | 可以 | `.codex-plugin` + skills + TOML agents + runners 是合理架构 |
| 借鉴 `seo-geo-claude-skills` contract | 强烈建议 | 模块化、handoff、protocol layer 清晰 |
| 借鉴 `Agentic-SEO-Skill` installer | 只借鉴 target matrix，不借鉴默认行为 | 写入面太大 |
| 采用 SEOmator CLI 作为 evidence runner | 可以评估 | deterministic output 有价值，但依赖较重 |
| 使用 `agentic-seo` 做 docs/AEO readiness | 可以评估 | 适合 agent-ready docs，不是完整 SEO agent |
| 一次接入所有 MCP | 不建议 | 凭据、成本、供应链、rate limit 风险过高 |
| 把 paid SEO MCP 结果当事实 | 需要 provenance | 必须标 provider、时间、quota、query params |

## Source Links

- `claude-seo`: https://github.com/AgriciDaniel/claude-seo
- `codex-seo`: https://github.com/AgriciDaniel/codex-seo
- `claude-blog`: https://github.com/AgriciDaniel/claude-blog
- `Agentic-SEO-Skill`: https://github.com/Bhanunamikaze/Agentic-SEO-Skill
- `seo-geo-claude-skills`: https://github.com/aaron-he-zhu/seo-geo-claude-skills
- `seo-skill`: https://github.com/aevans-eng/seo-skill
- `seo-audit-skill`: https://github.com/seo-skills/seo-audit-skill
- `agentic-seo`: https://github.com/addyosmani/agentic-seo
- Codex plugins: https://developers.openai.com/codex/plugins
- Codex plugin build docs: https://developers.openai.com/codex/plugins/build
- Codex MCP docs: https://developers.openai.com/codex/mcp
- Claude Skills security guidance: https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills
- Claude Help Center skills note: https://support.claude.com/en/articles/12512180-use-skills-in-claude
- MCP security best practices: https://modelcontextprotocol.io/docs/tutorials/security/security_best_practices
- Firecrawl MCP: https://www.firecrawl.dev/use-cases/ai-mcps
- DataForSEO MCP: https://dataforseo.com/model-context-protocol
- SE Ranking MCP: https://seranking.com/api/integrations/mcp/
- Ahrefs MCP: https://docs.ahrefs.com/en/mcp/docs/introduction
- Google Search Console API limits: https://developers.google.com/webmaster-tools/limits
- PageSpeed Insights API: https://developers.google.com/speed/docs/insights/v5/get-started
- Chrome UX Report API: https://developer.chrome.com/docs/crux/api

## Threads Run Log

```text
mode: research_spec
lanes_total: 3
failure_codes: openai-docs-skill-file-missing
verification_fresh: yes
closure_complete: yes
artifacts:
- docs/research/seo-agent-skill-plugin-audit-2026-06-19.md
```
