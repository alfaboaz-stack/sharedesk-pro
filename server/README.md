# ShareDesk Pro server: only approved masters may connect

The ShareDesk Pro installers are public and the server key built into them is
a *public* key: it stops anyone from impersonating the server, but not from
using it. This directory adds an **allowlist of approved masters**
(controlling devices) to the open-source RustDesk server (hbbs/hbbr):

| Device | Result |
|---|---|
| Master whose ID is in `allowed_ids.txt` | Can connect to any host |
| Any host (ShareDesk Pro Host, or ShareDesk Pro run without installing) | Registers freely, can be reached by approved masters only |
| Any other device | Cannot open connections and cannot use the relay |

Enforced on the server, so a modified client cannot bypass it.

## Files
- `allowlist.rs`: the allowlist module (the header comment has the exact rules).
- `apply_allowlist.py`: adds the module and a few one-line hooks to a
  rustdesk-server checkout; fails if an anchor moved.
- `deploy.sh`: installs it on the server (`prepare`, `activate`, `status`, `rollback`).
- Build: Actions ‣ "ShareDesk Pro - server build" → release `sharedesk-server`.

## Approving a master
Edit `allowed_ids.txt` in the private repo `alfaboaz-stack/sharedesk-devices`:

```
434 135 213   # my office PC
```

The server pulls the file every minute and applies it within ~10 seconds.
Delete the line to revoke the master.

## Known limits
- The protocol does not carry the requester's ID, so the server recognises a
  master by the public IP it registered from. Another device behind the same
  public IP as an online approved master could request connections; it still
  needs the host's permanent password and 2FA.
