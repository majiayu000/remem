# SPEC: remem-web 只读 API

为 remem-web UI 新增的只读 REST 端点。复用现有 axum router、Bearer 鉴权
（`require_api_token`）与 `db::open_db()` 解密连接。全部 GET，全部参数化查询。

## 端点

| 方法 | 路径 | 用途 | 复用 |
|---|---|---|---|
| GET | `/api/v1/memories` | 记忆列表（筛选/分页/搜索） | `MEMORY_COLS`+`map_memory_row` |
| GET | `/api/v1/memories/:id` | 记忆详情（含实体/边） | + `memory_entities` + `memory_edges` |
| GET | `/api/v1/candidates` | 待审候选 | `memory_candidates` 表 |
| GET | `/api/v1/graph` | 实体图谱（共现） | `entities` + `memory_entities` |
| GET | `/api/v1/stats` | 概览/用量 | `db::query::stats::*` |

## 查询参数约定

列表端点统一：`?project=&type=&scope=&status=&branch=&q=&limit=&offset=`
`project` 为空 = 所有项目。返回 `{ data, meta: { count, total, limit, offset } }`。

## 安全

- 仅 127.0.0.1 + Bearer token（沿用现有 `route_layer`）
- 全部参数化查询（`?N` 占位），无字符串拼接 SQL（SEC-01）
- 只读，不写 db

## 不在范围

- 写入/编辑/删除（UI 暂只读浏览）
- 全文检索（已有 `/api/v1/search`）
