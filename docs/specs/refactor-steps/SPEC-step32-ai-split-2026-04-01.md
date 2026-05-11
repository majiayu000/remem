# Step 32 - Split ai module

## Why

`src/ai.rs` 当前同时包含 AI executor 路由、CLI 调用、HTTP 调用、模型配置、价格估算和 usage 记录逻辑，达到 244 行，已经超过项目单文件 200 行限制。这个模块本身职责层次清楚，拆开后可以把配置、定价、记录和执行路径分层，也更方便后续调整 provider 或计费逻辑。

本步只做结构拆分，不改变 `UsageContext` 和 `call_ai()` 的公开接口，也不改变当前 executor 选择、HTTP/CLI fallback、usage 记录和定价默认值语义。

## Scope

- 保持 `pub struct UsageContext` 和 `pub async fn call_ai()` 对外接口不变
- 将 `src/ai.rs` 拆为 `types`、`config`、`pricing`、`usage`、`cli`、`http`、`tests` 子模块
- 保持 `REMEM_EXECUTOR=http|cli|auto` 选择语义不变
- 保持 HTTP 优先、CLI fallback 的 auto 语义不变
- 保持模型短名到 API model id 的映射和默认价格语义不变
- 新增纯单元测试，锁住模型映射和价格计算行为

## Module layout

- `src/ai.rs`
  - 模块声明与 `call_ai()` 入口
- `src/ai/types.rs`
  - `UsageContext`
  - `AiCallResult`
  - `AI_TIMEOUT_SECS`
- `src/ai/config.rs`
  - `get_model_raw`
  - `resolve_model_for_api`
  - `get_claude_path`
- `src/ai/pricing.rs`
  - `estimate_tokens`
  - `pricing_for_model`
  - `estimate_cost_usd`
  - 内部 env 解析 helper
- `src/ai/usage.rs`
  - `record_usage`
- `src/ai/cli.rs`
  - `call_cli`
- `src/ai/http.rs`
  - `call_http`
- `src/ai/tests.rs`
  - 模型映射与价格计算回归测试

## Public interface invariants

- `call_ai()` 继续返回 AI 文本响应，且成功后继续记录 usage
- auto 模式下，若配置了 Anthropic 凭据，继续优先走 HTTP，失败后 fallback 到 CLI
- `resolve_model_for_api()` 继续把 `haiku/sonnet/opus` 映射到固定 Anthropic model id
- `pricing_for_model()` 继续允许全局和按模型环境变量覆盖默认价格
- `record_usage()` 继续在 usage 记录失败时只打 warning，不影响主调用返回

## Validation

定向测试：
- `cargo test resolve_model_for_api_maps_short_names -- --nocapture`
- `cargo test pricing_for_model_uses_model_defaults -- --nocapture`
- `cargo test pricing_for_model_prefers_env_override -- --nocapture`
- `cargo test estimate_cost_usd_combines_input_and_output_prices -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步测试只做纯单元测试，不触发真实 HTTP/CLI 调用。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
