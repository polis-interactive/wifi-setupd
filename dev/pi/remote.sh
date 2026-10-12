#!/bin/sh
# Sent over SSH by pi.py; never installed in the product image.
set -eu
action=$1 serial=$2 ethernet=$3 app=$4 service=$5 log=$6
shift 6
actual=$(tr -d '\000' </proc/device-tree/serial-number)
[ "$actual" = "$serial" ] || { echo 'Pi serial mismatch' >&2; exit 1; }
[ "$(uname -m)" = aarch64 ] || { echo 'Expected AArch64 Pi' >&2; exit 1; }
# Keep every development command on Ethernet, including after Wi-Fi connects.
address=$(printf '%s\n' "$SSH_CONNECTION" | awk '{print $3}')
ip -o -4 addr show dev "$ethernet" | awk '{split($4,a,"/"); print a[1]}' |
    grep -Fx "$address" >/dev/null || { echo 'SSH is not using Pi Ethernet' >&2; exit 1; }

case "$action" in
    status)
        printf 'serial=%s\nethernet_address=%s\n' "$actual" "$address"
        printf 'boot_id='; cat /proc/sys/kernel/random/boot_id
        if [ -f "$app" ]; then sha256sum "$app"; else echo 'app=not-installed'; fi
        if [ -f "$app.previous" ]; then sha256sum "$app.previous"; fi
        ;;
    check) exec polis-dev-check ;;
    exec) exec "$@" ;;
    logs) [ ! -f "$log" ] || tail -n 100 "$log" ;;
    deploy|rollback|start|stop)
        mkdir -p "$(dirname "$app")"
        exec 9>"$app.update.lock"
        /bin/busybox flock -n 9 || { echo 'Another update is running' >&2; exit 1; }
        case "$action" in
            deploy)
                expected=$1 size=$2
                stage="$app.upload.$$"
                trap 'rm -f "$stage"' EXIT
                trap 'exit 1' HUP INT TERM
                umask 077
                cat >"$stage"
                [ "$(wc -c <"$stage")" -eq "$size" ] || { echo 'Incomplete upload; active app unchanged' >&2; exit 1; }
                actual_hash=$(sha256sum "$stage"); actual_hash=${actual_hash%% *}
                [ "$actual_hash" = "$expected" ] || { echo 'Checksum mismatch; active app unchanged' >&2; exit 1; }
                chmod 755 "$stage"
                # Upload and verification complete before the active process is touched.
                "$service" stop 9>&-
                if [ -f "$app" ]; then
                    cp -p "$app" "$app.previous.new"
                    mv -f "$app.previous.new" "$app.previous"
                fi
                sync
                mv -f "$stage" "$app"
                sync
                sha256sum "$app"
                echo 'Activated; use start or run to execute it.'
                ;;
            rollback)
                [ -f "$app.previous" ] || { echo 'No previous artifact' >&2; exit 1; }
                "$service" stop 9>&-
                cp -p "$app.previous" "$app.rollback.new"
                mv -f "$app.rollback.new" "$app"
                sync
                sha256sum "$app"
                echo 'Restored previous artifact; use start or run to execute it.'
                ;;
            start|stop) "$service" "$action" 9>&- ;;
        esac
        ;;
    *) echo "Unknown action: $action" >&2; exit 2 ;;
esac
