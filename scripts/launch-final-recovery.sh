#!/bin/bash
set -euo pipefail
umask 077

COMMIT=${1:-}
REPOSITORY=https://github.com/StealthEyeLLC/smp.git
BRANCH=build/smp-firecracker-god-mode-v1
RECOVERY_ROOT=/var/lib/smp/recovery
RESULT_ROOT=/var/lib/smp/results
SOURCE_ROOT="$RECOVERY_ROOT/source-$COMMIT"
LOG=$RESULT_ROOT/final-recovery.log
PID_FILE=$RESULT_ROOT/final-recovery.pid
STATUS_FILE=$RESULT_ROOT/final-recovery.status.json

[[ $(id -u) -eq 0 ]] || { printf 'launch-final-recovery.sh requires root\n' >&2; exit 77; }
[[ $COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf 'expected an immutable 40-character commit SHA\n' >&2; exit 64; }
for tool in git nohup sha256sum; do
    command -v "$tool" >/dev/null || { printf 'missing recovery launcher tool: %s\n' "$tool" >&2; exit 69; }
done

install -d -m 0700 "$RECOVERY_ROOT" "$RESULT_ROOT"
if [[ -r $PID_FILE ]]; then
    OLD_PID="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [[ $OLD_PID =~ ^[0-9]+$ ]] && kill -0 "$OLD_PID" 2>/dev/null; then
        printf 'an SMP final recovery is already running with pid %s\n' "$OLD_PID" >&2
        exit 75
    fi
fi

STAGING="$RECOVERY_ROOT/.source-$COMMIT.new"
rm -rf "$STAGING"
git clone --branch "$BRANCH" --single-branch --no-tags "$REPOSITORY" "$STAGING"
git -C "$STAGING" checkout --detach "$COMMIT"
OBSERVED_COMMIT="$(git -C "$STAGING" rev-parse HEAD)"
OBSERVED_TREE="$(git -C "$STAGING" rev-parse 'HEAD^{tree}')"
[[ $OBSERVED_COMMIT == "$COMMIT" ]] || { printf 'cloned recovery commit mismatch\n' >&2; exit 65; }
[[ -z "$(git -C "$STAGING" status --porcelain)" ]] || { printf 'cloned recovery source is not clean\n' >&2; exit 65; }

if [[ -e $SOURCE_ROOT ]]; then
    mv "$SOURCE_ROOT" "$RECOVERY_ROOT/source-$COMMIT.previous.$(date --utc +%Y%m%dT%H%M%SZ)"
fi
mv "$STAGING" "$SOURCE_ROOT"

ARCHIVE_STAMP="$(date --utc +%Y%m%dT%H%M%SZ)"
if [[ -e $LOG ]]; then
    mv "$LOG" "$RESULT_ROOT/final-recovery.previous.${ARCHIVE_STAMP}.log"
fi
if [[ -e $STATUS_FILE ]]; then
    mv "$STATUS_FILE" "$RESULT_ROOT/final-recovery.previous.${ARCHIVE_STAMP}.status.json"
fi
rm -f "$PID_FILE" "$STATUS_FILE"
{
    printf 'SMP final recovery launch\n'
    printf 'source_commit=%s\n' "$OBSERVED_COMMIT"
    printf 'source_tree=%s\n' "$OBSERVED_TREE"
    printf 'source_path=%s\n' "$SOURCE_ROOT"
    printf 'launched_at=%s\n' "$(date --utc +%Y-%m-%dT%H:%M:%SZ)"
} > "$LOG"
chmod 0600 "$LOG"

nohup bash "$SOURCE_ROOT/scripts/recover-firecracker-acceptance.sh" "$COMMIT" >> "$LOG" 2>&1 </dev/null &
RECOVERY_PID=$!
printf '%s\n' "$RECOVERY_PID" > "$PID_FILE.tmp"
chmod 0600 "$PID_FILE.tmp"
mv -f "$PID_FILE.tmp" "$PID_FILE"

sleep 1
kill -0 "$RECOVERY_PID" 2>/dev/null || {
    printf 'SMP final recovery exited during launch; inspect %s\n' "$LOG" >&2
    exit 70
}
printf 'SMP final recovery launched\n'
printf 'pid=%s\n' "$RECOVERY_PID"
printf 'log=%s\n' "$LOG"
printf 'status=%s\n' "$STATUS_FILE"
printf 'source_commit=%s\n' "$OBSERVED_COMMIT"
printf 'source_tree=%s\n' "$OBSERVED_TREE"
