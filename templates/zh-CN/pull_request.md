# Summary

用 1-3 句话描述本次变更。

## Linked Work

- Issue:
- Spec packet:

## 可选 SpecRail 上下文

以下项目仅提供建议性上下文，不是实施、审查或合并的前置条件。

- [ ] 关联 issue 已有 `ready_to_implement`，或这是一个已说明的小型 bug fix。
- [ ] 有帮助时已链接 product/tech spec。
- [ ] 可选的 `route_gate` 诊断结果：

## 审查与安全要求

- [ ] 已完成 agent first-pass review，或明确说明跳过原因。
- [ ] 已完成人工最终代码审查。
- [ ] 需要 ownership approval 时，已明确 owner。
- [ ] 已记录适用的安全决策和所需批准。

## 合并要求

- [ ] 已记录 PR head SHA。
- [ ] CI/check rollup 已完成且通过。
- [ ] 已检查 review threads，未解决的 actionable threads 已处理。
- [ ] merge state 为 clean。
- [ ] merge 前已记录 human merge authorization。

以下 SpecRail 诊断为可选项；结果仅供参考，不会授予批准，也不会阻止仓库工作：

- [ ] `python3 checks/github_pr_evidence.py --github-repo OWNER/REPO --pr <pr-number> --json > pr-evidence.json` 结果：
- [ ] `python3 checks/pr_gate.py --repo . --evidence <evidence.json>` 结果：

## Verification

- [ ] Tests:
- [ ] Manual proof:
- [ ] 用户可见变更附 screenshots 或 logs:

## Release Notes

- [ ] 需要 changelog 或 release note。
- [ ] 非用户可见。
- [ ] release 前已记录 human release authorization。

## Agent Disclosure

- [ ] No agent was used.
- [ ] Agent assisted; human author reviewed the full diff.
