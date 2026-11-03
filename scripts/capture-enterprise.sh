#!/usr/bin/env bash
set -euo pipefail
umask 077

out="$HOME/gigawatts-enterprise-capture"
stamp="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
run="$out/runs/$stamp"
mkdir -p "$run"
chmod 700 "$out" "$out/runs"

failed=0
fail() {
  echo "$1" >&2
  failed=1
}

transcript_folders() {
  { grep -rhoE --include='*.jsonl' '"cwd":"[^"]*"' "$HOME/.claude/projects" || true; } \
    | LC_ALL=C sort -u | sed -E 's/^"cwd":"(.*)"$/\1/'
}

repo_map() {
  transcript_folders | while IFS= read -r cwd; do
    [ -d "$cwd" ] || continue
    common="$(git -C "$cwd" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    remote="$(git -C "$cwd" remote get-url origin 2>/dev/null | sed -E 's#^([a-z+]+://)[^/@]*@#\1#' || true)"
    branch="$(git -C "$cwd" branch --show-current 2>/dev/null || true)"
    jq -cn --arg cwd "$cwd" --arg common "$common" --arg remote "$remote" --arg branch "$branch" \
      '{cwd: $cwd, git_common_dir: $common, remote: $remote, branch: $branch}'
  done
}

date -u +%Y-%m-%dT%H:%M:%SZ > "$run/captured-at.txt"

# No --delete, so transcripts Claude Code later removes stay in the mirror.
mkdir -p "$out/projects"
rsync -a --include='*/' --include='*.jsonl' --exclude='*' --prune-empty-dirs "$HOME/.claude/projects/" "$out/projects/" || fail "rsync exited $?; the transcript mirror may be incomplete"

git --version > /dev/null 2>&1 || fail "git is unavailable; repos.jsonl will resolve no repos"
repo_map > "$run/repos.jsonl" || fail "mapping transcript folders to repos failed"
if [ -s "$run/repos.jsonl" ] && ! jq -es 'any(.git_common_dir != "")' "$run/repos.jsonl" > /dev/null; then
  fail "no transcript folder in repos.jsonl resolved to a git repo"
fi
if [ -d "$HOME/.claude/spend" ]; then
  cp -R "$HOME/.claude/spend" "$run/spend" || fail "copying ~/.claude/spend failed"
fi
if [ -f "$HOME/.claude/settings.json" ]; then
  jq '{cleanupPeriodDays}' "$HOME/.claude/settings.json" > "$run/settings-retention.json" || fail "reading settings.json failed"
fi
jq '.cachedUsageUtilization' "$HOME/.claude.json" > "$run/cached-usage-before.json" || fail "reading cachedUsageUtilization failed"
jq '.oauthAccount | {accountUuid, organizationUuid}' "$HOME/.claude.json" > "$run/account-id.json" || fail "reading account IDs failed"
jq '.oauthAccount | {billingType, organizationType, organizationRateLimitTier, userRateLimitTier, seatTier, hasExtraUsageEnabled}' \
  "$HOME/.claude.json" > "$run/plan-type.json" || fail "reading plan type failed"

claude --version > "$run/claude-version.txt" || fail "claude --version failed"
# get_usage makes no model call; without skip_behaviors it also returns what /usage shows.
if ! printf '%s\n' '{"type":"control_request","request_id":"u1","request":{"subtype":"get_usage"}}' \
  | (cd "$out" && claude -p --input-format stream-json --output-format stream-json --verbose) \
  > "$run/raw-stream.jsonl"; then
  fail "claude -p get_usage failed; raw output kept in $run/raw-stream.jsonl"
elif ! jq 'select(.response.request_id? == "u1") | .response.response' "$run/raw-stream.jsonl" > "$run/get-usage.json" \
  || ! jq -e '.rate_limits != null' "$run/get-usage.json" > /dev/null 2>&1; then
  rm -f "$run/get-usage.json"
  fail "get_usage returned no rate_limits; raw output kept in $run/raw-stream.jsonl"
else
  rm "$run/raw-stream.jsonl"
  echo "Shape:"
  jq -c '{subscription_type, top: keys, limits: [.rate_limits.limits[]? | {kind, scope}], non_null: [.rate_limits | to_entries[] | select(.value != null) | .key]}' "$run/get-usage.json" \
    || fail "shape summary failed"
  jq -c . "$run/plan-type.json" || fail "printing plan type failed"
fi
jq '.cachedUsageUtilization' "$HOME/.claude.json" > "$run/cached-usage.json" || fail "reading refreshed cachedUsageUtilization failed"

echo "Run directory: $run"
echo "Transcript mirror: $(du -sh "$out/projects" | cut -f1) in $out/projects"
if [ "$failed" -ne 0 ]; then
  echo "Some steps failed; see the messages above." >&2
fi
exit "$failed"
