#!/bin/bash
set -euo pipefail
umask 077

PURGE_STATE=0
PURGE_ASSETS=0
while (($#)); do
    case "$1" in
        --purge-state) PURGE_STATE=1; shift ;;
        --purge-assets) PURGE_ASSETS=1; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ $(id -u) -eq 0 ]] || { printf 'uninstall.sh requires root\n' >&2; exit 77; }

systemctl disable --now smp-tunnel.service >/dev/null 2>&1 || true
systemctl disable --now smp.service >/dev/null 2>&1 || true
rm -f /etc/systemd/system/smp-tunnel.service /etc/systemd/system/smp.service
systemctl daemon-reload
rm -f /usr/local/bin/smp /usr/local/bin/cloudflared
rm -rf /usr/lib/smp
rm -f /etc/smp/install.json
rm -rf /etc/smp/credentials

if [[ $PURGE_ASSETS -eq 1 ]]; then
    rm -rf /var/lib/smp/assets
fi
if [[ $PURGE_STATE -eq 1 ]]; then
    if find /var/lib/smp/machines -mindepth 1 -maxdepth 2 -name machine.json -type f -print -quit 2>/dev/null | grep -q .; then
        printf 'refusing automatic persistent-state purge; destroy machines explicitly with smp before uninstall\n' >&2
        exit 65
    fi
    rm -rf /var/lib/smp/requests /var/lib/smp/results /var/lib/smp/machines /run/smp
fi

printf 'SMP executable and services removed. Persistent machine disks remain unless they were explicitly destroyed.\n'
