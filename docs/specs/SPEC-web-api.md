# SPEC: remem-web local REST API

为 remem-web UI 新增的本地 REST 端点。复用现有 axum router、Bearer 鉴权
（`require_api_token`）与 `db::open_db()` 解密连接。查询端点全部参数化查询。
GET 端点默认不改变 durable memory 内容；详情端点的访问遥测是唯一例外。

## 端点

| 方法 | 路径 | 用途 | 复用 |
|---|---|---|---|
| GET | `/api/v1/memories` | 记忆列表（筛选/分页/搜索） | `MEMORY_COLS`+`map_memory_row` |
| GET | `/api/v1/memories/:id` | 记忆详情（含实体/边），并记录访问遥测 | + `memory_entities` + `memory_edges` + `mark_memories_accessed` |
| GET | `/api/v1/memory?id=` | 兼容详情端点，并记录访问遥测 | `search(..., limit=1)` + `mark_memories_accessed` |
| POST | `/api/v1/memories` | 显式保存 durable memory | `save_memory_with_reference_time` |
| GET | `/api/v1/candidates` | 待审候选 | `memory_candidates` 表 |
| GET | `/api/v1/graph` | 实体图谱（共现） | `entities` + `memory_entities` |
| GET | `/api/v1/stats` | 概览/用量 | `db::query::stats::*` |

## 查询参数约定

列表端点统一：`?project=&type=&scope=&status=&branch=&q=&limit=&offset=`
`project` 为空 = 所有项目。返回 `{ data, meta: { count, total, limit, offset } }`。

## 安全

- 仅 127.0.0.1 + Bearer token（沿用现有 `route_layer`）
- 全部参数化查询（`?N` 占位），无字符串拼接 SQL（SEC-01）
- GET 列表、候选、图谱、状态和搜索端点不写 db
- `GET /api/v1/memories/:id` 与兼容详情端点 `GET /api/v1/memory?id=` 成功返回详情后会更新
  `memories.access_count` 与 `memories.last_accessed_epoch`，作为 usage-aware ranking 的访问遥测；
  失败响应不应更新访问计数
- `POST /api/v1/memories` 是显式写入端点；校验失败返回稳定的 `save_validation_failed`

## 不在范围

- 编辑/删除/批量治理
- 全文检索（已有 `/api/v1/search`）
