#!/bin/sh
# Runs Heistarr as PUID:PGID with UMASK, so files it creates (library
# imports, config) belong to your NAS user.
set -e
umask "${UMASK:-002}"
mkdir -p "$HEISTARR_DATA"
if [ "$(id -u)" = "0" ]; then
  chown "${PUID}:${PGID}" "$HEISTARR_DATA"
  exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups /app/heistarr "$@"
fi
exec /app/heistarr "$@"
