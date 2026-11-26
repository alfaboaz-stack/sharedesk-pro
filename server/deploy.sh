#!/bin/bash
# ShareDesk Pro server: only approved masters (controlling devices) may open connections.
#
#   sudo bash deploy.sh prepare    install binaries + sync job, print the deploy key (no restart)
#   sudo bash deploy.sh activate   fetch the approved list, switch the server to it, verify
#   sudo bash deploy.sh status     show what is running and the approved/pending devices
#   sudo bash deploy.sh rollback   go back to the original server (no allowlist)
#
# Approved master IDs live in the private GitHub repo $DEVICES_REPO (file allowed_ids.txt);
# the server pulls it every minute with a read-only deploy key.
set -euo pipefail

RELEASE_URL="https://github.com/alfaboaz-stack/sharedesk-pro/releases/download/sharedesk-server"
DEVICES_REPO="git@github.com:alfaboaz-stack/sharedesk-devices.git"
BASE=/opt/rustdesk
DATA=$BASE/data
BIN=$BASE/bin
KEYFILE=$BASE/devices-key
KNOWN=$BASE/github_known_hosts
CLONE=$BASE/devices
SYNC=$BASE/sync-allowlist.sh
CRON=/etc/cron.d/sharedesk-allowlist

[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 1; }

download() {
  local tmp; tmp=$(mktemp -d)
  for f in hbbs hbbr SHA256SUMS; do
    curl -fsSL --retry 3 -o "$tmp/$f" "$RELEASE_URL/$f"
  done
  (cd "$tmp" && grep -E ' (hbbs|hbbr)$' SHA256SUMS | sha256sum -c -)
  install -d -m 755 "$BIN"
  install -m 755 "$tmp/hbbs" "$BIN/hbbs"
  install -m 755 "$tmp/hbbr" "$BIN/hbbr"
  rm -rf "$tmp"
  echo "binaries installed in $BIN"
}

write_sync() {
  cat > "$SYNC" <<EOF
#!/bin/bash
# Pull the approved device list from GitHub (read-only deploy key).
set -euo pipefail
exec 9>/run/sharedesk-allowlist.lock
flock -n 9 || exit 0
export GIT_SSH_COMMAND="ssh -i $KEYFILE -o IdentitiesOnly=yes -o UserKnownHostsFile=$KNOWN -o StrictHostKeyChecking=yes -o BatchMode=yes"
if [ ! -d $CLONE/.git ]; then
  rm -rf $CLONE
  git clone -q --depth 1 $DEVICES_REPO $CLONE
else
  git -C $CLONE fetch -q --depth 1 origin
  git -C $CLONE reset -q --hard FETCH_HEAD
fi
if [ -f $CLONE/allowed_ids.txt ] && ! cmp -s $CLONE/allowed_ids.txt $DATA/allowed_ids.txt; then
  install -m 600 $CLONE/allowed_ids.txt $DATA/allowed_ids.txt.new
  mv $DATA/allowed_ids.txt.new $DATA/allowed_ids.txt
  logger -t sharedesk "allowed_ids.txt updated"
fi
EOF
  chmod 700 "$SYNC"
}

prepare() {
  apt-get install -y -qq git curl python3 >/dev/null
  download
  if [ ! -f "$KEYFILE" ]; then
    ssh-keygen -q -t ed25519 -N "" -C "sharedesk-server-readonly" -f "$KEYFILE"
  fi
  chmod 600 "$KEYFILE"
  curl -fsSL https://api.github.com/meta \
    | python3 -c 'import json,sys; print("\n".join("github.com "+k for k in json.load(sys.stdin)["ssh_keys"]))' > "$KNOWN"
  write_sync
  [ -f "$BASE/run.sh.before-allowlist" ] || cp -p "$BASE/run.sh" "$BASE/run.sh.before-allowlist"
  echo
  echo "=== Deploy key (add to github.com/alfaboaz-stack/sharedesk-devices > Settings > Deploy keys, read-only) ==="
  cat "$KEYFILE.pub"
}

activate() {
  [ -x "$BIN/hbbs" ] || { echo "run prepare first"; exit 1; }
  "$SYNC"
  local n
  n=$(sed 's/#.*//' "$DATA/allowed_ids.txt" | awk 'NF' | wc -l)
  [ "$n" -ge 1 ] || { echo "allowed_ids.txt has no device; no approved master: refusing to lock everyone out"; exit 1; }
  echo "approved masters: $n"
  echo '* * * * * root '"$SYNC"' >/dev/null 2>>/var/log/sharedesk-allowlist.log' > "$CRON"

  local relay
  relay=$(grep -o 'RELAY=[^ ]*' "$BASE/run.sh.before-allowlist" | head -1 | cut -d= -f2)
  [ -n "$relay" ] || { echo "cannot find RELAY in run.sh.before-allowlist"; exit 1; }
  cat > "$BASE/run.sh" <<EOF
#!/bin/bash
# ShareDesk Pro: RustDesk server; only approved masters may open connections.
# Original version: run.sh.before-allowlist
docker pull rustdesk/rustdesk-server-s6:latest
docker rm -f rustdesk-server 2>/dev/null
docker run -d --name rustdesk-server --net=host --restart unless-stopped \\
  -e RELAY=$relay -e ENCRYPTED_ONLY=1 -e ALLOWLIST_FILE=/data/allowed_ids.txt \\
  -v $DATA:/data \\
  -v $BIN/hbbs:/usr/bin/hbbs:ro -v $BIN/hbbr:/usr/bin/hbbr:ro \\
  rustdesk/rustdesk-server-s6:latest
docker image prune -f
EOF
  chmod 700 "$BASE/run.sh"
  "$BASE/run.sh"
  sleep 15
  status
  docker logs rustdesk-server 2>&1 | grep "ShareDesk allowlist enabled" >/dev/null \
    || { echo "allowlist NOT active - check the log above (rollback: sudo bash deploy.sh rollback)"; exit 1; }
  echo "OK: allowlist active"
}

status() {
  docker ps --filter name=rustdesk-server --format '{{.Names}}  {{.Status}}'
  docker logs --tail 200 rustdesk-server 2>&1 | grep -E "ShareDesk|ERROR|panicked" | tail -15 || true
  ss -lntup | grep -E ':2111[5-7]' || true
  echo "--- approved masters:"; cat "$DATA/allowed_ids.txt" 2>/dev/null || echo "(none)"
  echo "--- refused connection attempts (last 200 log lines):"
  docker logs --tail 200 rustdesk-server 2>&1 | grep "ShareDesk allowlist: refused" | tail -5 || true
}

rollback() {
  [ -f "$BASE/run.sh.before-allowlist" ] || { echo "no backup found"; exit 1; }
  cp -p "$BASE/run.sh.before-allowlist" "$BASE/run.sh"
  rm -f "$CRON"
  "$BASE/run.sh"
  echo "rolled back to the original server"
}

case "${1:-}" in
  prepare) prepare ;;
  activate) activate ;;
  status) status ;;
  rollback) rollback ;;
  *) sed -n '2,10p' "$0"; exit 1 ;;
esac
