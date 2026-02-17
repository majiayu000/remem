#!/bin/bash
# remem 替代 claude-mem 完整测试脚本
# 用法: bash test.sh

set -e
REMEM="$(cd "$(dirname "$0")" && pwd)/target/release/remem"
DB="$HOME/.claude-mem/claude-mem.db"
PASS=0
FAIL=0

green() { printf "\033[32m✓ %s\033[0m\n" "$1"; PASS=$((PASS+1)); }
red()   { printf "\033[31m✗ %s\033[0m\n" "$1"; FAIL=$((FAIL+1)); }

echo "=== remem 替代测试 ==="
echo ""

# 0. 前置检查
echo "--- 前置检查 ---"
[ -f "$REMEM" ] && green "binary 存在 ($REMEM)" || red "binary 不存在"
[ -f "$DB" ] && green "数据库存在 ($DB)" || red "数据库不存在"

# 1. Context 生成
echo ""
echo "--- Phase 1: context 生成 ---"
CTX=$("$REMEM" context --cwd "$HOME/Desktop/code/AI/tools/vibeguard" 2>/dev/null)
echo "$CTX" | grep -q "recent context" && green "context 包含 header" || red "context 缺少 header"
echo "$CTX" | grep -q "Legend:" && green "context 包含 legend" || red "context 缺少 legend"
echo "$CTX" | grep -q "Context Economics" && green "context 包含 economics" || red "context 缺少 economics"
echo "$CTX" | grep -q "^###" && green "context 包含日期分组" || red "context 缺少日期分组"
echo "$CTX" | grep -q "| ID |" && green "context 包含表格" || red "context 缺少表格"
echo "$CTX" | grep -qE "202[56]" && green "日期格式正确 (年份)" || red "日期格式错误"
LINES=$(echo "$CTX" | wc -l | tr -d ' ')
[ "$LINES" -gt 20 ] && green "context 有足够内容 (${LINES} 行)" || red "context 内容太少 (${LINES} 行)"

# 2. Context 空项目
echo ""
echo "--- Phase 2: 空项目 context ---"
EMPTY=$("$REMEM" context --cwd /tmp/nonexistent-project-xyz 2>/dev/null)
echo "$EMPTY" | grep -q "No previous sessions" && green "空项目显示正确" || red "空项目显示异常"

# 3. MCP Server
echo ""
echo "--- Phase 3: MCP Server ---"

# 3a. initialize + tools/list
MCP_OUT=$(printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' | (cat; sleep 2) | "$REMEM" mcp 2>/dev/null)

echo "$MCP_OUT" | grep -q '"tools"' && green "MCP tools/list 响应" || red "MCP tools/list 失败"
for TOOL in search timeline get_observations save_memory; do
  echo "$MCP_OUT" | grep -q "\"$TOOL\"" && green "MCP 工具: $TOOL" || red "MCP 缺少工具: $TOOL"
done

# 3b. search
SEARCH_OUT=$(printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":{"query":"guard","project":"vibeguard","limit":3}}}\n' | (cat; sleep 2) | "$REMEM" mcp 2>/dev/null)
echo "$SEARCH_OUT" | grep '"id":3' | grep -q '"text"' && green "MCP search 返回结果" || red "MCP search 失败"

# 3c. get_observations
OBS_OUT=$(printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_observations","arguments":{"ids":[2107]}}}\n' | (cat; sleep 2) | "$REMEM" mcp 2>/dev/null)
echo "$OBS_OUT" | grep '"id":4' | grep -q "narrative" && green "MCP get_observations 返回完整数据" || red "MCP get_observations 失败"

# 3d. timeline
TL_OUT=$(printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"timeline","arguments":{"anchor":2107,"depth_before":2,"depth_after":2}}}\n' | (cat; sleep 2) | "$REMEM" mcp 2>/dev/null)
echo "$TL_OUT" | grep '"id":5' | grep -q '"text"' && green "MCP timeline 返回结果" || red "MCP timeline 失败"

# 4. Session Init
echo ""
echo "--- Phase 4: session-init ---"
echo '{"sessionId":"test-remem-001","cwd":"/tmp/test-project"}' | "$REMEM" session-init 2>/dev/null
SESS=$(sqlite3 "$DB" "SELECT content_session_id FROM sdk_sessions WHERE content_session_id='test-remem-001'" 2>/dev/null)
[ "$SESS" = "test-remem-001" ] && green "session-init 写入 DB" || red "session-init 未写入 DB"

# 清理测试数据
sqlite3 "$DB" "DELETE FROM sdk_sessions WHERE content_session_id='test-remem-001'" 2>/dev/null

# 5. XML 解析测试 (通过 observe 的内部逻辑)
echo ""
echo "--- Phase 5: 进程检查 ---"
PROCS=$(pgrep -f "remem (mcp|context|observe|summarize|session-init)" 2>/dev/null | wc -l | tr -d ' ')
[ "$PROCS" -eq 0 ] && green "零孤儿进程" || red "发现 $PROCS 个残留进程"

# 6. 二进制大小
echo ""
echo "--- Phase 6: 资源占用 ---"
SIZE=$(ls -l "$REMEM" | awk '{print $5}')
SIZE_MB=$((SIZE / 1024 / 1024))
[ "$SIZE_MB" -lt 10 ] && green "二进制大小: ${SIZE_MB}MB (< 10MB)" || red "二进制太大: ${SIZE_MB}MB"

# 总结
echo ""
echo "========================"
echo "通过: $PASS  失败: $FAIL"
[ "$FAIL" -eq 0 ] && echo "🎉 全部通过!" || echo "⚠️  有 $FAIL 项失败"
