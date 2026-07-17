# Product Spec：安全本地控制台证据、只读资源与可恢复治理

## Linked Issue

GH-880

## 用户问题

`remem-web` 已能读取候选列表、图谱和用量，但尚不能安全展示候选证据、浏览高敏感度运行时资源，或执行可恢复的记忆治理。现有 candidate review 写端点缺少面向 Web 的详情、provenance、并发与幂等契约；observations、sessions、workstreams、events、tasks 也没有逐项可发现的稳定只读能力。用户因此只能看到解释性 unavailable 页面，或被迫依赖 CLI。

## 目标

- 让用户在审核候选前看到足够、可追溯且已脱敏的证据。
- 为 observations、sessions、workstreams、events、tasks 提供逐项发布的真实只读能力。
- 让 archive/restore 成为可恢复、原子、可重试且可审计的 Web 治理操作。
- 保持永久删除和 secret 在 Web 能力范围外；本轮新增 Web capability 不新增 raw transcript 暴露，legacy `/api/v1/search.raw_hits` 例外维持现状。

## 非目标

- 不在 Web 中提供永久 delete。
- 不让前端承担敏感信息脱敏责任。
- 不把未完成的资源包装为空数组、mock 行或成功状态。
- 不更改现有 search、memory browse、graph、stats 或 candidate list 的兼容语义。
- 不在本轮移除 legacy `/api/v1/search.raw_hits[].preview`；该字段仍是显式的 raw archive 兼容面，GH-880 的 raw-transcript 禁止仅约束本轮新增 endpoint 及其关系展开。
- 不放宽 loopback、bearer token、SQL 参数化或 policy suppression 边界。

## Behavior Invariants

1. B-001 候选详情必须返回候选当前内容、状态、稳定版本标识、`can_review`、`blocked_reasons` 和 evidence provenance；列表中的 `evidence_count` 不能替代详情证据，evidence preview 也不得复制原始 captured event 内容。
2. B-002 evidence 缺失、不完整、无法脱敏或 provenance 无法解析时，候选详情仍可读取，但 `can_review` 必须为 false，并返回机器可判定的阻塞原因。
3. B-003 Web 候选审核写入必须携带用户可见理由、预期版本和幂等标识；幂等标识必须经过统一的有界字符集校验，并且只以不可逆摘要进入持久化、审计和日志，原始值不得存储或返回；版本过期、重复请求或候选已离开可审核队列（`pending_review`/`quarantined`）必须返回明确、可重试判定的结果，不能静默覆盖。
4. B-004 approve、reject、edit-and-approve 的成功响应必须包含候选最终状态、关联 memory（如有）、操作与审计标识；事务失败不得创建部分 memory 或提前改变候选状态。
5. B-005 observations、sessions、workstreams、events、tasks 必须作为五个独立 capability 发布；任一资源未完成 endpoint、auth、脱敏、测试或 smoke 时，该 capability 必须保持 false。每个 `<resource>` capability 只能与 `<resource>_list` 和 `<resource>_detail` 两个 endpoint key 原子发布；false 时两 key 全 absent，true 时两 key 全 present，不允许 partial pair。
6. B-006 每个只读资源必须提供真实 list/detail 状态：loading、真实空集、数据、not found、权限失败和服务端失败。五类新增资源默认应用 active policy suppression：list 省略被 suppression 命中的行，detail 对被命中的行返回 404，policy 查询或评估失败返回结构化 5xx；任何失败都不得显示为空列表或 unavailable 成功态。
7. B-007 高变动资源的分页必须使用基于永不复用、单调递增 source id 的稳定 cursor；五类 list 的 `page_size` 缺省为 50，并由服务端限制到 `1..=100`，任何响应不得超过 100 行；重复读取同一 cursor 不得跳过或重复已确认页面内的记录，清理/重插入或 suppression 过滤不得让 cursor 循环或混入复用 id，非法或过期 cursor 必须返回结构化错误。
8. B-008 candidate evidence、observations 和 events 的 GH-880 新增 Web 响应只能包含服务端从 allowlisted metadata 或已批准派生数据生成的安全摘要与 bounded preview。通用脱敏不能把原始 captured event 内容变成可公开投影；原始 transcript、API token、secret、环境变量值、`captured_events.content_text` 和未分类 payload 不得进入这些新 endpoint 的任何响应字段或关系展开。既有 `/api/v1/search.raw_hits` 兼容面不由本轮重定义。
9. B-009 session、workstream 和 task detail 引用 observation/event/memory/topic/entity 时，只能返回通过 active suppression 检查的安全引用或已脱敏摘要；客户端不得通过关系展开绕过 B-008 或 policy suppression。
10. B-010 eligible scan 已耗尽时必须返回真实空 `data` 和终止 cursor；若一批原始行全部被 suppression 过滤但扫描预算尚未覆盖后续行，可以返回空 `data` 加非空 continuation cursor，并且 cursor 必须严格前进。未知字段不能自动成为 UI 权限或触发额外读取。
11. B-011 Web v1 只提供 archive 和 restore。`memory_archive` 和 `memory_restore` 只能分别与同名 endpoint key 原子发布；永久 `memory_delete` capability 必须保持 false，endpoint map 不得存在 delete key，且 remem-web 不得展示或调用 delete 操作。
12. B-012 archive 必须要求明确 reason、预期版本和幂等标识，且 Web v1 只允许 archive 当前 active memory；用于治理的 active memory list/detail 必须公开当前整数 `version` 供客户端提交 `expected_version`；成功后 memory 保留可恢复内容，默认 search 和 remem-web 的 active list 不再返回该 memory，显式 `status=archived` audit 读取仍可定位，原有无 status list 语义保持兼容。
13. B-013 restore 必须只恢复由本 Web archive 契约产生、可审计且仍为 archived 的 memory，并恢复为 active；archived list/detail 必须公开当前整数 `version` 供客户端提交 `expected_version`；当前 archived 状态必须由最近一次状态转换中的成功 Web `memory_archive` 产生，曾经存在 Web archive、但其后发生 restore 或任一非 Web status transition，不构成当前可恢复 provenance；重复 restore 返回幂等结果，目标已永久缺失、由其它生命周期归档或版本冲突时返回结构化错误。
14. B-014 archive/restore 成功响应必须包含 `operation_id`、`audit_id`、资源 id、before/after 状态、最终 `version` 和发生时间；幂等标识通过校验并建立 operation 后的失败必须原子回滚并返回同一 `operation_id`，`idempotency_key_invalid` 属于 operation 建立前的 400 validation error，只返回独立 request/trace id。
15. B-015 所有新 endpoint 继续只允许 loopback bearer-token 访问，使用参数化查询；认证失败、非法 id、非法 cursor、非法幂等标识、版本冲突和幂等冲突不得泄露敏感内容。
16. B-016 旧版客户端和现有 endpoint 行为保持兼容。新客户端只能依据 `features.<name> === true` 和声明 endpoint 启用功能；`candidate_detail` 与 `candidate_evidence` 可以显式映射到同一个 candidate detail endpoint，但两项声明均不可省略。`candidate_review_safe=true` 只能与 `candidate_review_safe_approve`、`candidate_review_safe_reject`、`candidate_review_safe_edit` 三个 endpoint key 原子发布，客户端不得推导或硬编码任一写入路径。
17. B-017 每个能力必须在对应 remem release 发布后才能向 installed-binary 用户声明可用；source-only 实现不能被文档写成已发布能力。

## 验收标准

- [ ] 候选详情能展示安全派生的 evidence/provenance，raw-only evidence 不返回原始内容且阻止审核，证据不足或版本过期时同样 fail closed。
- [ ] candidate review 的并发、幂等、reason、事务和 audit response 均有端到端回归测试；首次成功改变版本/状态后，原 stale request 的 same-key replay 仍返回首次结果且不重复 mutation/audit；非法幂等标识 fail closed，原始幂等值不进入 DB、audit、日志或响应。
- [ ] 五类只读资源分别通过 capability、list/detail、empty/not-found/error、cursor、purge/reinsert、active suppression 和 `page_size` default/clamp/invalid 测试；capabilities contract 精确断言 `observations|sessions|workstreams|events|tasks` 的 `_list`/`_detail` 十个 key/path，并在缺任一 pair 成员时 fail closed。
- [ ] secret/token/transcript/payload 脱敏测试证明敏感原文和非敏感 raw transcript sentinel 都不会进入 GH-880 新增 endpoint 或其关系展开；legacy search contract regression 证明本规格没有静默改变 `/api/v1/search.raw_hits`。
- [ ] archive/restore 通过从 active/archived list/detail 读取 `version`、提交该值、成功后取得最终 `version`、状态/marker 已变化后的 same-key replay、非法幂等标识、原始幂等值无泄漏、版本冲突、失败回滚、审计完整性及 Web restore 后被非 Web writer 再归档不可恢复的混合序列测试。
- [ ] `memory_archive`/`memory_restore` capability 与同名 endpoint key 各自全有或全无；permanent `memory_delete` 在 capabilities 中保持 false、delete endpoint key 不存在，remem-web 不展示该操作。
- [ ] native API smoke、current Web API contract、README/release guidance 与实现同步，并从 advertised endpoint map 验证全部新 route：candidate detail/evidence 两 key、safe review 三 action key、五类资源十个 list/detail key 及 archive/restore 两 key 均精确映射，且各 capability bundle 全有或全无。
- [ ] 现有 native API、CLI 和 memory retrieval 测试无回归。

## 边界情况

- Candidate evidence 指向已清理、已隔离、跨 project 或 policy-suppressed 的来源。
- 同一候选被两个浏览器标签页同时审核，或同一幂等标识携带不同 payload。
- 幂等标识为空白、超过长度上限、包含 Unicode、控制字符或字符集外空格。
- 分页期间新事件写入、旧事件归档、cursor 过期或资源跨 project 移动。
- 仅有敏感内容的 observation/event，脱敏后没有可展示正文。
- active pattern suppression 命中任一安全投影字段，或 memory/topic/entity relation 指向 suppressed 目标。
- page1/page2 之间清理当前最大 observation/task 后插入新行。
- memory 已 archived、已 restore、已被 CLI 永久删除或发生版本漂移。
- 后端只发布部分资源 capability，前端直接访问尚未发布的 URL。

## 发布说明

新能力按 capability 独立发布，并在 release note 中记录最低 remem 版本、endpoint、脱敏边界、cursor 兼容期和 remem-web 最低版本。candidate review 与 archive/restore 属于写能力；在 release、smoke 与审计证据完成前保持 disabled。永久 delete 不进入本轮 Web 发布。
