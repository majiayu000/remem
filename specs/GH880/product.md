# Product Spec：安全本地控制台证据、只读资源与可恢复治理

## Linked Issue

GH-880

## 用户问题

`remem-web` 已能读取候选列表、图谱和用量，但尚不能安全展示候选证据、浏览高敏感度运行时资源，或执行可恢复的记忆治理。现有 candidate review 写端点缺少面向 Web 的详情、provenance、并发与幂等契约；observations、sessions、workstreams、events、tasks 也没有逐项可发现的稳定只读能力。用户因此只能看到解释性 unavailable 页面，或被迫依赖 CLI。

## 目标

- 让用户在审核候选前看到足够、可追溯且已脱敏的证据。
- 为 observations、sessions、workstreams、events、tasks 提供逐项发布的真实只读能力。
- 让 archive/restore 成为可恢复、原子、可重试且可审计的 Web 治理操作。
- 保持永久删除、原始 transcript 和 secret 在 Web 能力范围外。

## 非目标

- 不在 Web 中提供永久 delete。
- 不让前端承担敏感信息脱敏责任。
- 不把未完成的资源包装为空数组、mock 行或成功状态。
- 不更改现有 search、memory browse、graph、stats 或 candidate list 的兼容语义。
- 不放宽 loopback、bearer token、SQL 参数化或 policy suppression 边界。

## Behavior Invariants

1. B-001 候选详情必须返回候选当前内容、状态、稳定版本标识、`can_review`、`blocked_reasons` 和 evidence provenance；列表中的 `evidence_count` 不能替代详情证据。
2. B-002 evidence 缺失、不完整、无法脱敏或 provenance 无法解析时，候选详情仍可读取，但 `can_review` 必须为 false，并返回机器可判定的阻塞原因。
3. B-003 Web 候选审核写入必须携带用户可见理由、预期版本和幂等标识；版本过期、重复请求或候选已离开可审核队列（`pending_review`/`quarantined`）必须返回明确、可重试判定的结果，不能静默覆盖。
4. B-004 approve、reject、edit-and-approve 的成功响应必须包含候选最终状态、关联 memory（如有）、操作与审计标识；事务失败不得创建部分 memory 或提前改变候选状态。
5. B-005 observations、sessions、workstreams、events、tasks 必须作为五个独立 capability 发布；任一资源未完成 endpoint、auth、脱敏、测试或 smoke 时，该 capability 必须保持 false。
6. B-006 每个只读资源必须提供真实 list/detail 状态：loading、真实空集、数据、not found、权限失败和服务端失败。任何失败都不得显示为空列表或 unavailable 成功态。
7. B-007 高变动资源的分页必须使用稳定 cursor；重复读取同一 cursor 不得跳过或重复已确认页面内的记录，非法或过期 cursor 必须返回结构化错误。
8. B-008 observations 和 events 的 Web 响应只能包含服务端生成的脱敏摘要与 bounded preview。原始 transcript、API token、secret、环境变量值和未分类 payload 不得进入响应。
9. B-009 session、workstream 和 task detail 引用 observation/event 时，只能返回安全引用或已脱敏摘要；客户端不得通过关系展开绕过 B-008。
10. B-010 无数据必须返回真实空 `data` 和终止 cursor；未知字段不能自动成为 UI 权限或触发额外读取。
11. B-011 Web v1 只提供 archive 和 restore。永久 delete capability 必须保持 false，且 remem-web 不得展示或调用 delete 操作。
12. B-012 archive 必须要求明确 reason、预期版本和幂等标识，且 Web v1 只允许 archive 当前 active memory；成功后 memory 保留可恢复内容，默认 search 和 remem-web 的 active list 不再返回该 memory，显式 `status=archived` audit 读取仍可定位，原有无 status list 语义保持兼容。
13. B-013 restore 必须只恢复由本 Web archive 契约产生、可审计且仍为 archived 的 memory，并恢复为 active；重复 restore 返回幂等结果，目标已永久缺失、由其它生命周期归档或版本冲突时返回结构化错误。
14. B-014 archive/restore 成功响应必须包含 `operation_id`、`audit_id`、资源 id、before/after 状态和发生时间；失败必须原子回滚并返回同一操作的诊断标识。
15. B-015 所有新 endpoint 继续只允许 loopback bearer-token 访问，使用参数化查询；认证失败、非法 id、非法 cursor、版本冲突和幂等冲突不得泄露敏感内容。
16. B-016 旧版客户端和现有 endpoint 行为保持兼容。新客户端只能依据 `features.<name> === true` 和声明 endpoint 启用功能。
17. B-017 每个能力必须在对应 remem release 发布后才能向 installed-binary 用户声明可用；source-only 实现不能被文档写成已发布能力。

## 验收标准

- [ ] 候选详情能展示脱敏 evidence/provenance，并在证据不足或版本过期时阻止审核。
- [ ] candidate review 的并发、幂等、reason、事务和 audit response 均有端到端回归测试。
- [ ] 五类只读资源分别通过 capability、list/detail、empty/not-found/error 和 cursor 测试。
- [ ] secret/token/transcript/payload 脱敏测试证明敏感原文不会进入 Web 响应。
- [ ] archive/restore 通过成功、重复、版本冲突、失败回滚和审计完整性测试。
- [ ] permanent delete 在 capabilities 中保持 false，remem-web 不展示该操作。
- [ ] native API smoke、current Web API contract、README/release guidance 与实现同步。
- [ ] 现有 native API、CLI 和 memory retrieval 测试无回归。

## 边界情况

- Candidate evidence 指向已清理、已隔离、跨 project 或 policy-suppressed 的来源。
- 同一候选被两个浏览器标签页同时审核，或同一幂等标识携带不同 payload。
- 分页期间新事件写入、旧事件归档、cursor 过期或资源跨 project 移动。
- 仅有敏感内容的 observation/event，脱敏后没有可展示正文。
- memory 已 archived、已 restore、已被 CLI 永久删除或发生版本漂移。
- 后端只发布部分资源 capability，前端直接访问尚未发布的 URL。

## 发布说明

新能力按 capability 独立发布，并在 release note 中记录最低 remem 版本、endpoint、脱敏边界、cursor 兼容期和 remem-web 最低版本。candidate review 与 archive/restore 属于写能力；在 release、smoke 与审计证据完成前保持 disabled。永久 delete 不进入本轮 Web 发布。
