//! ShareDesk Pro: only approved controlling devices (masters) may open connections.
//!
//! Enabled only when the environment variable `ALLOWLIST_FILE` is set
//! (for example `/data/allowed_ids.txt`). The file lists the IDs of the
//! approved masters. When enabled:
//!
//! * Any device may register, so host-only installs need no approval.
//! * hbbs refuses punch-hole and relay requests unless they come from an IP
//!   address where an approved master has registered in the last 10 minutes
//!   (the protocol does not carry the requester's ID, only its address).
//! * hbbs writes the addresses of approved masters, plus hosts they asked to
//!   reach in the last 10 minutes, to `.sharedesk_active_ips` next to the
//!   allowlist; hbbr refuses relay connections from other addresses
//!   (disable with `ALLOWLIST_RELAY=0`).
//!
//! File format: one ID per line (spaces inside the ID are ignored); anything
//! after `#` is a comment. Changes are picked up within 10 seconds.
//! A missing or unreadable file means "no master is approved" (fail closed).
#![allow(dead_code)]

use hbb_common::log;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
    time::{Duration, Instant, SystemTime},
};

const ACTIVE_IPS_FILE: &str = ".sharedesk_active_ips";
const IP_TTL: Duration = Duration::from_secs(600);
const RELOAD_EVERY: Duration = Duration::from_secs(10);
const RELAY_RELOAD_EVERY: Duration = Duration::from_secs(5);

struct State {
    enabled: bool,
    check_relay: bool,
    path: PathBuf,
    dir: PathBuf,
    ids: RwLock<HashSet<String>>,
    mtime: Mutex<Option<SystemTime>>,
    master_ips: Mutex<HashMap<IpAddr, Instant>>,
    target_ips: Mutex<HashMap<IpAddr, Instant>>,
    relay_ips: RwLock<HashSet<IpAddr>>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let path = PathBuf::from(std::env::var("ALLOWLIST_FILE").unwrap_or_default().trim());
        let enabled = !path.as_os_str().is_empty();
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        let s = State {
            enabled,
            check_relay: std::env::var("ALLOWLIST_RELAY").map_or(true, |v| v.trim() != "0"),
            path,
            dir,
            ids: Default::default(),
            mtime: Mutex::new(None),
            master_ips: Default::default(),
            target_ips: Default::default(),
            relay_ips: Default::default(),
        };
        if enabled {
            reload(&s, true);
        }
        s
    })
}

fn normalize(addr: SocketAddr) -> IpAddr {
    hbb_common::try_into_v4(addr).ip()
}

fn parse(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let id: String = line
                .split('#')
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .collect();
            (!id.is_empty()).then_some(id)
        })
        .collect()
}

fn reload(s: &State, force: bool) {
    let mtime = std::fs::metadata(&s.path).and_then(|m| m.modified()).ok();
    {
        let mut last = s.mtime.lock().unwrap();
        if !force && *last == mtime {
            return;
        }
        *last = mtime;
    }
    match std::fs::read_to_string(&s.path) {
        Ok(text) => {
            let ids = parse(&text);
            log::info!(
                "ShareDesk allowlist loaded from {}: {} approved master(s)",
                s.path.display(),
                ids.len()
            );
            *s.ids.write().unwrap() = ids;
        }
        Err(err) => {
            log::error!(
                "ShareDesk allowlist {} cannot be read ({err}); no master is approved",
                s.path.display()
            );
            s.ids.write().unwrap().clear();
        }
    }
}

fn fresh(map: &Mutex<HashMap<IpAddr, Instant>>) -> Vec<IpAddr> {
    let mut map = map.lock().unwrap();
    map.retain(|_, seen| seen.elapsed() < IP_TTL);
    map.keys().copied().collect()
}

fn write_active_ips(s: &State) {
    let mut ips = fresh(&s.master_ips);
    ips.extend(fresh(&s.target_ips));
    let text: Vec<String> = ips.iter().map(|ip| ip.to_string()).collect();
    let target = s.dir.join(ACTIVE_IPS_FILE);
    let tmp = s.dir.join(format!("{ACTIVE_IPS_FILE}.tmp"));
    let res = std::fs::write(&tmp, text.join("\n")).and_then(|_| std::fs::rename(&tmp, &target));
    if let Err(err) = res {
        log::warn!("ShareDesk allowlist: cannot write {}: {err}", target.display());
    }
}

/// Called once by hbbs at startup.
pub fn start_hbbs() {
    let s = state();
    if !s.enabled {
        log::info!("ShareDesk allowlist disabled (ALLOWLIST_FILE not set)");
        return;
    }
    log::info!("ShareDesk allowlist enabled: file={}", s.path.display());
    std::thread::spawn(|| loop {
        let s = state();
        reload(s, false);
        write_active_ips(s);
        std::thread::sleep(RELOAD_EVERY);
    });
}

/// Called once by hbbr at startup.
pub fn start_hbbr() {
    let s = state();
    if !s.enabled || !s.check_relay {
        log::info!("ShareDesk relay allowlist disabled");
        return;
    }
    log::info!("ShareDesk relay allowlist enabled");
    std::thread::spawn(|| loop {
        let s = state();
        let ips: HashSet<IpAddr> = std::fs::read_to_string(s.dir.join(ACTIVE_IPS_FILE))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        *s.relay_ips.write().unwrap() = ips;
        std::thread::sleep(RELAY_RELOAD_EVERY);
    });
}

/// A device registered: if it is an approved master, remember where it lives.
pub fn note_registration(id: &str, addr: SocketAddr) {
    let s = state();
    if s.enabled && s.ids.read().unwrap().contains(id) {
        s.master_ips
            .lock()
            .unwrap()
            .insert(normalize(addr), Instant::now());
    }
}

/// May a connection be opened from this address?
pub fn is_requester_allowed(addr: SocketAddr) -> bool {
    let s = state();
    if !s.enabled {
        return true;
    }
    let ip = normalize(addr);
    s.master_ips
        .lock()
        .unwrap()
        .get(&ip)
        .map_or(false, |seen| seen.elapsed() < IP_TTL)
}

/// An approved master asked to reach this host: let the host use the relay.
pub fn note_target(addr: SocketAddr) {
    let s = state();
    if s.enabled {
        s.target_ips
            .lock()
            .unwrap()
            .insert(normalize(addr), Instant::now());
    }
}

pub fn is_relay_allowed(addr: SocketAddr) -> bool {
    let s = state();
    if !s.enabled || !s.check_relay {
        return true;
    }
    let ip = normalize(addr);
    ip.is_loopback() || s.relay_ips.read().unwrap().contains(&ip)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_ids_and_comments() {
        let ids = parse("# header\n123456789  # office\n\n  987654321\n1 121 828 947 # spaced\n");
        assert!(ids.contains("123456789"));
        assert!(ids.contains("987654321"));
        assert!(ids.contains("1121828947"));
        assert_eq!(ids.len(), 3);
    }
}
