# Product Spec

## Linked Issue

GH-664

## 用户问题

remem 已有 repo-local agent instructions 和 `docs/specs/` 生命周期规则，但没有把 SpecRail 工作流资产放进仓库。结果是 agent 只有在本机全局安装了 SpecRail 技能时才可能使用同一套 issue-first/spec-first 流程，换机器、换 checkout 或换 agent 后不可复现。

## 目标

- 让仓库自己声明 SpecRail adoption，而不是依赖某台机器的全局技能目录。
- 让后续 issue、spec、task、PR review、CI diagnosis、PR gate 和 release-note 工作能从 repo-local 文件启动。
- 保留 remem 现有 `docs/specs/` 合同索引和历史规格语义，避免 SpecRail 新 packet 与旧目录混淆。
- 保留所有 human gates：readiness label、spec approval、final PR review、security decision、merge 和 release。

## 非目标

- 不修改 remem runtime、数据库迁移、hook 行为、plugin runtime 或发布产物。
- 不把 repo-local SpecRail 技能安装到 `$HOME`。
- 不迁移或重写现有 `docs/specs/` 历史文件。

## Behavior Invariants

1. P1: 当仓库包含 `skills-lock.json` 和 `skills/specrail-workflow/SKILL.md` 时，agent 必须把 SpecRail 视为 repo workflow contract，并从 `specrail-workflow` 入口启动。
2. P2: route 明确后，agent 只能加载一个匹配 route 的 focused SpecRail skill，不能一次性加载所有 `specrail-*` 技能。
3. P3: SpecRail 的 workflow config、state graph、label groups、templates、checks 和 focused skills 必须能从仓库 checkout 中读取和运行。
4. P4: 新的 SpecRail issue packet 必须位于 `specs/GH<issue-number>/product.md`、`tech.md` 和 `tasks.md`；现有 remem 当前合同和历史实现规格继续由 `docs/specs/` 管理。
5. P5: SpecRail adoption 不能降低 human gates，也不能授权 agent final approval、merge、force-push、security disclosure 或 permission change。
6. P6: 本次 bootstrap 不能改变用户可见的 remem runtime 行为。

## 验收标准

- [ ] 仓库包含 repo-local SpecRail entrypoint、focused skills、`skills-lock.json`、workflow/state/label config、templates 和 checks。
- [ ] `python3 checks/check_workflow.py --repo .` 通过。
- [ ] 至少一个 representative route gate 能在无网络写入的情况下返回 deterministic JSON。
- [ ] `AGENTS.md` 和 `AGENT_USAGE.md` 说明 SpecRail 触发方式、focused skill 路由、artifact 边界和 human gates。
- [ ] `docs/specs/README.md` 说明 `docs/specs/` 与 `specs/GH...` 的边界。

## 边界情况

- 如果没有 linked GitHub issue，`write_spec` route 不能跳过 issue-first 约束，除非人类明确选择非 GitHub workflow。
- 如果本地或远端没有 readiness evidence，route gate 应报告 missing gate，而不是静默继续。
- 如果 future agent 需要安装 skills 到 `$HOME`，必须由人类明确请求，不能由 repo adoption 自动安装。

## 发布说明

这是 contributor workflow 变更，不影响 remem CLI、hooks、MCP、API、数据库或 plugin release behavior。
