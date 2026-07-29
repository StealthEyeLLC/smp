#!/bin/bash
set -euo pipefail

MODE=${1:-}
DIRECTORY=${2:-}

[[ $# -eq 2 ]] || {
    printf 'usage: module-tree-digest.sh <normalized|legacy> <directory>\n' >&2
    exit 64
}
[[ -d $DIRECTORY ]] || {
    printf 'module-tree directory is unavailable: %s\n' "$DIRECTORY" >&2
    exit 66
}

case "$MODE" in
    normalized)
        (
            cd -- "$DIRECTORY"
            find . -type f -print0 |
                LC_ALL=C sort -z |
                xargs -0 -r sha256sum --zero --
        ) | sha256sum | cut -d' ' -f1
        ;;
    legacy)
        find "$DIRECTORY" -type f -print0 |
            LC_ALL=C sort -z |
            xargs -0 sha256sum |
            sha256sum |
            cut -d' ' -f1
        ;;
    *)
        printf 'unknown module-tree digest mode: %s\n' "$MODE" >&2
        exit 64
        ;;
esac
