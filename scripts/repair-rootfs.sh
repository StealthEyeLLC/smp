#!/bin/bash
set -euo pipefail
umask 077

ASSETS_ROOT=/var/lib/smp/assets
SOURCE_ROOT=/usr/lib/smp/assets
BUILD_COMMIT=unknown

while (($#)); do
    case "$1" in
        --assets-root) ASSETS_ROOT=$2; shift 2 ;;
        --source-root) SOURCE_ROOT=$2; shift 2 ;;
        --build-commit) BUILD_COMMIT=$2; shift 2 ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ $(id -u) -eq 0 ]] || { printf 'repair-rootfs.sh requires root\n' >&2; exit 77; }
for tool in cp debugfs e2fsck install jq losetup mount mv sha256sum sync umount; do
    command -v "$tool" >/dev/null || { printf 'missing rootfs repair tool: %s\n' "$tool" >&2; exit 69; }
done

MANIFEST="$ASSETS_ROOT/manifest.json"
DESIRED_INIT="$SOURCE_ROOT/guest/smp-seed-init.sh"
DESIRED_SERVICE="$SOURCE_ROOT/guest/smp-seed-init.service"
PROVENANCE="$ASSETS_ROOT/provenance/rootfs-guest-revision.json"
[[ -r $MANIFEST ]] || { printf 'asset manifest is unavailable: %s\n' "$MANIFEST" >&2; exit 66; }
[[ -f $DESIRED_INIT && -f $DESIRED_SERVICE ]] || { printf 'canonical guest initializer sources are unavailable\n' >&2; exit 66; }

ROOTFS="$(jq -er '.rootfs.path | select(type == "string" and length > 0)' "$MANIFEST")"
EXPECTED_ROOTFS_SHA="$(jq -er '.rootfs.sha256 | select(test("^[0-9a-f]{64}$"))' "$MANIFEST")"
[[ -f $ROOTFS ]] || { printf 'rootfs asset is unavailable: %s\n' "$ROOTFS" >&2; exit 66; }
PREVIOUS="${ROOTFS}.previous"

if [[ -f $PREVIOUS ]]; then
    CURRENT_SHA=
    PREVIOUS_SHA="$(sha256sum "$PREVIOUS" | cut -d' ' -f1)"
    if [[ -f $ROOTFS ]]; then
        CURRENT_SHA="$(sha256sum "$ROOTFS" | cut -d' ' -f1)"
    fi
    if [[ $CURRENT_SHA == "$EXPECTED_ROOTFS_SHA" ]]; then
        rm -f "$PREVIOUS"
    elif [[ $PREVIOUS_SHA == "$EXPECTED_ROOTFS_SHA" ]]; then
        rm -f "$ROOTFS"
        mv "$PREVIOUS" "$ROOTFS"
    else
        printf 'cannot recover interrupted rootfs repair: neither candidate matches the manifest\n' >&2
        exit 65
    fi
fi

OBSERVED_ROOTFS_SHA="$(sha256sum "$ROOTFS" | cut -d' ' -f1)"
[[ $OBSERVED_ROOTFS_SHA == "$EXPECTED_ROOTFS_SHA" ]] || {
    printf 'rootfs digest mismatch before repair: expected %s, observed %s\n' "$EXPECTED_ROOTFS_SHA" "$OBSERVED_ROOTFS_SHA" >&2
    exit 65
}
if losetup -j "$ROOTFS" | grep -q .; then
    printf 'refusing to repair a rootfs currently attached to a loop device: %s\n' "$ROOTFS" >&2
    exit 73
fi

DESIRED_INIT_SHA="$(sha256sum "$DESIRED_INIT" | cut -d' ' -f1)"
DESIRED_SERVICE_SHA="$(sha256sum "$DESIRED_SERVICE" | cut -d' ' -f1)"
WORK="$(mktemp -d "$ASSETS_ROOT/.rootfs-repair.XXXXXX")"
MOUNT_POINT="$WORK/mount"
CANDIDATE="$WORK/rootfs.ext4"
EXISTING_INIT="$WORK/existing-init"
EXISTING_SERVICE="$WORK/existing-service"
MOUNTED=0
ROOT_REPLACED=0
COMMITTED=0
cleanup() {
    if [[ $MOUNTED -eq 1 ]]; then
        umount "$MOUNT_POINT" >/dev/null 2>&1 || true
    fi
    if [[ $ROOT_REPLACED -eq 1 && $COMMITTED -eq 0 && -f $PREVIOUS ]]; then
        rm -f "$ROOTFS"
        mv "$PREVIOUS" "$ROOTFS"
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

EXISTING_INIT_SHA=missing
EXISTING_SERVICE_SHA=missing
if debugfs -R "dump -p /usr/local/libexec/smp-seed-init $EXISTING_INIT" "$ROOTFS" >/dev/null 2>&1; then
    EXISTING_INIT_SHA="$(sha256sum "$EXISTING_INIT" | cut -d' ' -f1)"
fi
if debugfs -R "dump -p /etc/systemd/system/smp-seed-init.service $EXISTING_SERVICE" "$ROOTFS" >/dev/null 2>&1; then
    EXISTING_SERVICE_SHA="$(sha256sum "$EXISTING_SERVICE" | cut -d' ' -f1)"
fi

write_provenance() {
    local prior_sha=$1 final_sha=$2 mutation=$3
    jq -n \
      --arg schemaVersion "1" \
      --arg priorRootfsSha256 "$prior_sha" \
      --arg rootfsSha256 "$final_sha" \
      --arg guestInitSha256 "$DESIRED_INIT_SHA" \
      --arg guestServiceSha256 "$DESIRED_SERVICE_SHA" \
      --arg buildCommit "$BUILD_COMMIT" \
      --arg repairedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
      --argjson mutationApplied "$mutation" \
      '{schemaVersion:($schemaVersion|tonumber),priorRootfsSha256:$priorRootfsSha256,rootfsSha256:$rootfsSha256,guestInitSha256:$guestInitSha256,guestServiceSha256:$guestServiceSha256,buildCommit:$buildCommit,repairedAt:$repairedAt,mutationApplied:$mutationApplied}' \
      > "$PROVENANCE.new"
    chmod 0600 "$PROVENANCE.new"
}

if [[ $EXISTING_INIT_SHA == "$DESIRED_INIT_SHA" && $EXISTING_SERVICE_SHA == "$DESIRED_SERVICE_SHA" ]]; then
    write_provenance "$OBSERVED_ROOTFS_SHA" "$OBSERVED_ROOTFS_SHA" false
    jq --arg provenance "$PROVENANCE" '.rootfs.provenancePath = $provenance' "$MANIFEST" > "$MANIFEST.new"
    chmod 0600 "$MANIFEST.new"
    mv "$PROVENANCE.new" "$PROVENANCE"
    mv "$MANIFEST.new" "$MANIFEST"
    printf 'Canonical guest initializer already present in rootfs\n'
    exit 0
fi

printf 'Repairing existing rootfs guest initializer without rebuilding Debian\n'
cp --reflink=auto --sparse=always --preserve=mode,timestamps -- "$ROOTFS" "$CANDIDATE"
chmod 0600 "$CANDIDATE"
mkdir -p "$MOUNT_POINT"
mount -o loop,rw "$CANDIDATE" "$MOUNT_POINT"
MOUNTED=1
install -m 0755 "$DESIRED_INIT" "$MOUNT_POINT/usr/local/libexec/smp-seed-init"
install -m 0644 "$DESIRED_SERVICE" "$MOUNT_POINT/etc/systemd/system/smp-seed-init.service"
mkdir -p "$MOUNT_POINT/etc/systemd/system/multi-user.target.wants" "$MOUNT_POINT/var/lib/smp-seed"
ln -sfn ../smp-seed-init.service "$MOUNT_POINT/etc/systemd/system/multi-user.target.wants/smp-seed-init.service"
rm -f "$MOUNT_POINT/var/lib/smp-seed/complete" "$MOUNT_POINT/var/lib/smp-seed/status"
sync
umount "$MOUNT_POINT"
MOUNTED=0
set +e
e2fsck -pf "$CANDIDATE"
FSCK_STATUS=$?
set -e
if (( FSCK_STATUS > 1 )); then
    printf 'repaired rootfs failed e2fsck with status %s\n' "$FSCK_STATUS" >&2
    exit 65
fi
FINAL_ROOTFS_SHA="$(sha256sum "$CANDIDATE" | cut -d' ' -f1)"
write_provenance "$OBSERVED_ROOTFS_SHA" "$FINAL_ROOTFS_SHA" true
jq \
  --arg sha "$FINAL_ROOTFS_SHA" \
  --arg provenance "$PROVENANCE" \
  --arg builtAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  '.rootfs.sha256 = $sha | .rootfs.provenancePath = $provenance | .builtAt = $builtAt' \
  "$MANIFEST" > "$MANIFEST.new"
chmod 0600 "$MANIFEST.new"
chmod 0444 "$CANDIDATE"
rm -f "$PREVIOUS"
mv "$ROOTFS" "$PREVIOUS"
mv "$CANDIDATE" "$ROOTFS"
ROOT_REPLACED=1
mv "$PROVENANCE.new" "$PROVENANCE"
mv "$MANIFEST.new" "$MANIFEST"
COMMITTED=1
rm -f "$PREVIOUS"
printf 'Rootfs guest initializer repaired and manifest rebound to %s\n' "$FINAL_ROOTFS_SHA"
