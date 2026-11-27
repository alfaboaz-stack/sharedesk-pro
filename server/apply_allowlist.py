"""Add the ShareDesk Pro approved-masters allowlist to a rustdesk-server checkout.

Usage: python3 apply_allowlist.py <path-to-rustdesk-server>

Every edit is an exact, unique text anchor; the script fails loudly if an
anchor is missing, so an upstream change can never produce a server that
silently skips the check.
"""
import pathlib
import shutil
import sys


def edit(path, anchor, replacement):
    text = path.read_text(encoding="utf-8")
    count = text.count(anchor)
    if count != 1:
        sys.exit(f"ERROR: anchor found {count} times in {path}:\n{anchor}")
    path.write_text(text.replace(anchor, replacement), encoding="utf-8")
    print(f"patched {path.name}: {anchor.strip().splitlines()[0][:70]}")


def main():
    root = pathlib.Path(sys.argv[1])
    src = root / "src"
    shutil.copy(pathlib.Path(__file__).with_name("allowlist.rs"), src / "allowlist.rs")

    # hbbs (library crate) and hbbr (separate binary crate) both compile the module.
    edit(src / "lib.rs", "mod rendezvous_server;\n", "pub mod allowlist;\nmod rendezvous_server;\n")
    edit(src / "hbbr.rs", "mod common;\n", "mod allowlist;\nmod common;\n")

    edit(
        src / "main.rs",
        "    RendezvousServer::start_with_bind(",
        "    hbbs::allowlist::start_hbbs();\n    RendezvousServer::start_with_bind(",
    )
    edit(
        src / "hbbr.rs",
        "        .start()?;\n",
        "        .start()?;\n    allowlist::start_hbbr();\n",
    )

    rs = src / "rendezvous_server.rs"
    # A master's address is remembered only after hbbs has verified the device:
    # a periodic registration from the address it is already known at (with a
    # key on file), or an accepted RegisterPk (uuid/key checks passed). A bare
    # RegisterPeer claiming someone else's ID is not enough.
    edit(
        rs,
        "            if !request_pk {\n"
        "                old.socket_addr = socket_addr;\n",
        "            if !request_pk {\n"
        "                crate::allowlist::note_registration(&id, socket_addr);\n"
        "                old.socket_addr = socket_addr;\n",
    )
    edit(
        rs,
        "                    if changed {\n"
        "                        self.pm.update_pk(id, peer, addr, rk.uuid, rk.pk, ip).await;\n",
        "                    crate::allowlist::note_registration(&id, addr);\n"
        "                    if changed {\n"
        "                        self.pm.update_pk(id, peer, addr, rk.uuid, rk.pk, ip).await;\n",
    )
    edit(
        rs,
        "        let id = ph.id;\n        // punch hole request from A, relay to B,\n",
        "        if !crate::allowlist::is_requester_allowed(addr) {\n"
        '            log::warn!("ShareDesk allowlist: refused connection from {} to {}", addr, ph.id);\n'
        "            let mut msg_out = RendezvousMessage::new();\n"
        "            msg_out.set_punch_hole_response(PunchHoleResponse {\n"
        "                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),\n"
        "                ..Default::default()\n"
        "            });\n"
        "            return Ok((msg_out, None));\n"
        "        }\n"
        "        let id = ph.id;\n        // punch hole request from A, relay to B,\n",
    )
    edit(
        rs,
        "            // record punch hole request (from addr -> peer id/peer_addr)\n",
        "            crate::allowlist::note_target(peer_addr);\n"
        "            // record punch hole request (from addr -> peer id/peer_addr)\n",
    )
    edit(
        rs,
        "                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {\n",
        "                    if !crate::allowlist::is_requester_allowed(addr) {\n"
        '                        log::warn!("ShareDesk allowlist: refused relay from {} to {}", addr, rf.id);\n'
        "                        return true;\n"
        "                    }\n"
        "                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {\n",
    )
    edit(
        rs,
        "                        let peer_addr = peer.read().await.socket_addr;\n",
        "                        let peer_addr = peer.read().await.socket_addr;\n"
        "                        crate::allowlist::note_target(peer_addr);\n",
    )

    edit(
        src / "relay_server.rs",
        "    let key = key.to_owned();\n    let limiter = limiter.clone();\n    tokio::spawn(async move {\n        allow_err!(make_pair(",
        "    if !crate::allowlist::is_relay_allowed(addr) {\n"
        '        log::info!("{} relay refused: no approved device at this address", ip);\n'
        "        return;\n"
        "    }\n"
        "    let key = key.to_owned();\n    let limiter = limiter.clone();\n    tokio::spawn(async move {\n        allow_err!(make_pair(",
    )
    print("ShareDesk allowlist applied")


if __name__ == "__main__":
    main()
