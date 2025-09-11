#!/usr/bin/env bash
set -euo pipefail

# Simple local E2E using Codex CLI (codex exec) against the kanban MCP server.
# Prerequisites:
#  - codex CLI installed and configured with an mcp_servers.kanban entry
#  - jq installed
#  - this repo contains .kanban/ (backlog, etc)

BOARD="${BOARD:-$PWD}"
TIMEOUT="${TIMEOUT:-240}"
CODEX_BIN="${CODEX_BIN:-codex}"
JQ_BIN="${JQ_BIN:-jq}"

needcmd() { command -v "$1" >/dev/null 2>&1 || { echo "E: missing command: $1" >&2; exit 127; }; }
needcmd "$CODEX_BIN"; needcmd "$JQ_BIN"

echo "[E2E] BOARD=$BOARD TIMEOUT=${TIMEOUT}s"

cx_exec() {
  local prompt="$1"
  if command -v timeout >/dev/null 2>&1; then
    timeout "${TIMEOUT}s" "$CODEX_BIN" exec --json -C "$BOARD" --skip-git-repo-check --full-auto - <<< "$prompt"
  else
    "$CODEX_BIN" exec --json -C "$BOARD" --skip-git-repo-check --full-auto - <<< "$prompt"
  fi
}

run_text() {
  local name="$1"; shift
  local args="$1"; shift
  local prompt="tools/call name=${name} arguments=${args} を1回だけ実行し、mcp_tool_call_endイベントのcontent[0].textを出力して下さい。"
  cx_exec "$prompt" | "$JQ_BIN" -r 'select(.msg.type=="mcp_tool_call_end") | .msg.result.Ok.content[0].text'
}

fail() { echo "[FAIL] $*" >&2; exit 1; }
pass() { echo "[PASS] $*"; }

# 1) new A/B
A_JSON=$(run_text kanban_new '{"board":"'"$BOARD"'","title":"E2E_A","column":"backlog"}')
echo "$A_JSON" | "$JQ_BIN" . >/dev/null || fail "kanban_new A malformed json"
A_ID=$(echo "$A_JSON" | "$JQ_BIN" -r .cardId)
[ -n "$A_ID" ] || fail "A_ID empty"
pass "new A: $A_ID"

B_JSON=$(run_text kanban_new '{"board":"'"$BOARD"'","title":"E2E_B","column":"backlog"}')
B_ID=$(echo "$B_JSON" | "$JQ_BIN" -r .cardId)
[ -n "$B_ID" ] || fail "B_ID empty"
pass "new B: $B_ID"

# 2) update A title
UPD_JSON=$(run_text kanban_update '{"board":"'"$BOARD"'","cardId":"'"$A_ID"'","patch":{"fm":{"title":"E2E_A_updated"}}}')
[ "$(echo "$UPD_JSON" | "$JQ_BIN" -r .updated)" = "true" ] || fail "update A"
pass "update A"

# 3) move A backlog->doing
MV_JSON=$(run_text kanban_move '{"board":"'"$BOARD"'","cardId":"'"$A_ID"'","toColumn":"doing"}')
[ "$(echo "$MV_JSON" | "$JQ_BIN" -r .to)" = "doing" ] || fail "move A"
pass "move A -> doing"

# 4) list doing includes A
L_DO_JSON=$(run_text kanban_list '{"board":"'"$BOARD"'","columns":["doing"],"limit":200}')
echo "$L_DO_JSON" | "$JQ_BIN" -e --arg ID "$A_ID" '.items | any(.cardId==$ID)' >/dev/null || fail "list doing missing A"
pass "list doing contains A"

# 5) notes append/list
NA_JSON=$(run_text kanban_notes_append '{"board":"'"$BOARD"'","cardId":"'"$A_ID"'","text":"note-1","type":"worklog","tags":["e2e"],"author":"tester"}')
[ "$(echo "$NA_JSON" | "$JQ_BIN" -r .appended)" = "true" ] || fail "notes.append"
NL_JSON=$(run_text kanban_notes_list '{"board":"'"$BOARD"'","cardId":"'"$A_ID"'","limit":1}')
[ "$(echo "$NL_JSON" | "$JQ_BIN" -r '.items | length')" -ge 1 ] || fail "notes.list"
pass "notes append/list"

# 6) parent P and relations.set (A->P)
P_JSON=$(run_text kanban_new '{"board":"'"$BOARD"'","title":"E2E_P","column":"backlog"}')
P_ID=$(echo "$P_JSON" | "$JQ_BIN" -r .cardId)
[ -n "$P_ID" ] || fail "P_ID empty"
REL_JSON=$(run_text kanban_relations_set '{"board":"'"$BOARD"'","add":[{"type":"parent","from":"'"$A_ID"'","to":"'"$P_ID"'"}] }')
[ "$(echo "$REL_JSON" | "$JQ_BIN" -r .updated)" = "true" ] || fail "relations.set parent"
pass "relations.set parent"

# 7) tree P shows A as child
TR_JSON=$(run_text kanban_tree '{"board":"'"$BOARD"'","root":"'"$P_ID"'","depth":3}')
echo "$TR_JSON" | "$JQ_BIN" -e --arg ID "$A_ID" '.tree.children | any(.id|ascii_upcase==$ID)' >/dev/null || fail "tree missing child"
pass "tree includes A"

# 8) done A
DN_JSON=$(run_text kanban_done '{"board":"'"$BOARD"'","cardId":"'"$A_ID"'"}')
[ "$(echo "$DN_JSON" | "$JQ_BIN" -r '.completed_at | length')" -gt 0 ] || fail "done A"
pass "done A"

# 9) watch (smoke)
WC_JSON=$(run_text kanban_watch '{"board":"'"$BOARD"'"}')
STARTED=$(echo "$WC_JSON" | "$JQ_BIN" -r '.started // .alreadyWatching // false')
[ "$STARTED" = "true" ] || fail "watch"
pass "watch"

echo "[E2E] ALL PASS"

