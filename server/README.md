# ShareDesk Pro server: approved devices only

The ShareDesk Pro client has the server address and key built in, and the
installer is public. The key is a *public* key: it stops anyone from
impersonating the server, but it does not stop strangers from using it.
This directory adds an **allowlist of approved device IDs** to the
open-source RustDesk server (hbbs/hbbr):

| Who | Result |
|---|---|
| Device whose ID is in `allowed_ids.txt` | Registers, can be reached, can connect |
| Any other device | Never comes online ("not ready"), cannot be reached, cannot connect to approved devices, cannot use the relay |

Enforced on the server, so a modified client cannot bypass it.

## Files
- `allowlist.rs`: the allowlist module (see the header comment for the exact rules).
- `apply_allowlist.py`: adds the module and five one-line hooks to a rustdesk-server checkout; fails if an anchor moved.
- `deploy.sh`: installs it on the server (`prepare`, `activate`, `status`, `rollback`).
- Build: Actions ‣ "ShareDesk Pro - server build" → release `sharedesk-server`.

## Approving a device
Edit `allowed_ids.txt` in the private repo `alfaboaz-stack/sharedesk-devices`:

```
123456789   # Office PC
```

The server pulls the file every minute and applies it within ~10 seconds.
Remove a line to revoke a device.

## Known limits
- The requester check is by public IP: a device without approval that sits
  behind the same public IP as an approved, online device can *request*
  connections. It still needs the target's permanent password and 2FA, and
  it can never be reached itself.
- `docker logs rustdesk-server` and `/opt/rustdesk/data/pending_ids.txt`
  show refused IDs.
