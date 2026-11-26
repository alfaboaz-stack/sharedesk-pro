"""Bake the ShareDesk Pro locked settings into the client at build time.

RustDesk only accepts a custom-client config (custom.txt) signed with the
RustDesk team's key. This script splits read_custom_client() so the JSON part
can be applied without a signature, and makes load_custom_client() always
apply our settings compiled into the binary (they cannot be removed by
deleting a file next to the exe).

Env: SERVER, KEY. Run from the repository root.
"""
import json
import os
import pathlib
import sys

VARIANT = os.environ.get("SHAREDESK_VARIANT", "master").strip()
if VARIANT not in ("master", "host"):
    sys.exit(f"unknown SHAREDESK_VARIANT {VARIANT!r}")
SERVER = os.environ["SERVER"].strip()
KEY = os.environ["KEY"].strip()
if not SERVER or not KEY:
    sys.exit("SERVER and KEY are required")

settings = {
    "default-settings": {
        "lang": "he",
    },
    "override-settings": {
        # Our server; also makes the UI stop treating us as the public server.
        "custom-rendezvous-server": SERVER,
        "key": KEY,
        # Only the permanent password (plus 2FA) opens a connection.
        "verification-method": "use-permanent-password",
        "approve-mode": "password",
        "allow-numeric-one-time-password": "N",
        # Settings a user must not change.
        "hide-server-settings": "Y",
        "disable-change-id": "Y",
        "enable-lan-discovery": "N",
        "direct-server": "N",
        "hide-powered-by-me": "Y",
    },
}
if VARIANT == "host":
    # Host-only build: no "control remote desktop" side at all.
    settings["conn-type"] = "incoming"
data = json.dumps(settings, separators=(",", ":"))


def edit(path, anchor, replacement):
    text = path.read_text(encoding="utf-8")
    if text.count(anchor) != 1:
        sys.exit(f"ERROR: anchor found {text.count(anchor)} times in {path}:\n{anchor}")
    path.write_text(text.replace(anchor, replacement), encoding="utf-8")


# Master build run without installing (portable): host only, so controlling
# always goes through an installed, server-approved master.
PORTABLE_HOOK = (
    "    #[cfg(windows)]\n"
    "    if !crate::platform::is_installed() {\n"
    "        config::HARD_SETTINGS\n"
    "            .write()\n"
    "            .unwrap()\n"
    "            .insert(\"conn-type\".to_owned(), \"incoming\".to_owned());\n"
    "    }\n"
) if VARIANT == "master" else ""

common = pathlib.Path("src/common.rs")
edit(
    common,
    "    let Ok(mut data) =\n"
    "        serde_json::from_slice::<std::collections::HashMap<String, serde_json::Value>>(&data)\n",
    "    apply_custom_client_json(&data);\n"
    "}\n"
    "\n"
    "pub fn apply_custom_client_json(data: &[u8]) {\n"
    "    let Ok(mut data) =\n"
    "        serde_json::from_slice::<std::collections::HashMap<String, serde_json::Value>>(data)\n",
)
edit(
    common,
    "pub fn load_custom_client() {\n",
    "const SHAREDESK_SETTINGS: &str = " + json.dumps(data) + ";\n"
    "\n"
    "pub fn load_custom_client() {\n"
    "    apply_custom_client_json(SHAREDESK_SETTINGS.as_bytes());\n"
    + PORTABLE_HOOK,
)
print(f"ShareDesk Pro ({VARIANT}) settings embedded:", ", ".join(sorted(settings["override-settings"])))
