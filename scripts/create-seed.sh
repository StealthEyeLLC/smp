#!/bin/bash
set -euo pipefail
umask 077

OUTPUT=
HOSTNAME=
AUTHORIZED_KEY_FILE=
ADDRESS=
GATEWAY=
DNS=
MAC=
FILES=
INIT=

while (($#)); do
    case "$1" in
        --output) OUTPUT=$2; shift 2 ;;
        --hostname) HOSTNAME=$2; shift 2 ;;
        --authorized-key-file) AUTHORIZED_KEY_FILE=$2; shift 2 ;;
        --address) ADDRESS=$2; shift 2 ;;
        --gateway) GATEWAY=$2; shift 2 ;;
        --dns) DNS=$2; shift 2 ;;
        --mac) MAC=$2; shift 2 ;;
        --files) FILES=$2; shift 2 ;;
        --init) INIT=$2; shift 2 ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ -n $OUTPUT && -n $HOSTNAME && -n $AUTHORIZED_KEY_FILE && -n $ADDRESS && -n $GATEWAY && -n $DNS && -n $MAC ]] || {
    printf 'missing required seed argument\n' >&2; exit 64;
}
[[ $HOSTNAME =~ ^[a-z][a-z0-9-]{0,62}$ ]] || { printf 'invalid hostname\n' >&2; exit 65; }
[[ -f $AUTHORIZED_KEY_FILE ]] || { printf 'authorized public key file missing\n' >&2; exit 66; }
[[ $ADDRESS =~ ^[0-9.]+/[0-9]+$ ]] || { printf 'invalid address\n' >&2; exit 65; }
[[ $GATEWAY =~ ^[0-9.]+$ ]] || { printf 'invalid gateway\n' >&2; exit 65; }
[[ $DNS =~ ^[0-9.,]+$ ]] || { printf 'invalid DNS list\n' >&2; exit 65; }
[[ $MAC =~ ^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$ ]] || { printf 'invalid MAC address\n' >&2; exit 65; }
[[ ! -e $OUTPUT ]] || { printf 'seed output already exists: %s\n' "$OUTPUT" >&2; exit 73; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
install -d -m 0700 "$WORK/root"
printf '%s\n' "$HOSTNAME" > "$WORK/root/hostname"
install -m 0600 "$AUTHORIZED_KEY_FILE" "$WORK/root/authorized_keys"
cat > "$WORK/root/network.env" <<NETWORK
ADDRESS='$ADDRESS'
GATEWAY='$GATEWAY'
DNS='$DNS'
MAC='$MAC'
NETWORK
if [[ -n $FILES ]]; then
    [[ -d $FILES ]] || { printf 'seed files directory missing\n' >&2; exit 66; }
    tar --create --file "$WORK/root/files.tar" --directory "$FILES" .
fi
if [[ -n $INIT ]]; then
    [[ -f $INIT ]] || { printf 'seed init script missing\n' >&2; exit 66; }
    install -m 0700 "$INIT" "$WORK/root/init.sh"
fi
truncate -s 16M "$OUTPUT"
mkfs.ext4 -q -F -L SMP_SEED -d "$WORK/root" "$OUTPUT"
chmod 0600 "$OUTPUT"
e2fsck -pf "$OUTPUT" || [[ $? -eq 1 ]]
