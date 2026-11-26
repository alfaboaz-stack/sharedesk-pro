//! ShareDesk Pro: approved-devices allowlist for hbbs / hbbr.
//!
//! Enabled only when the environment variable `ALLOWLIST_FILE` is set
//! (for example `/data/allowed_ids.txt`). When enabled:
//!
//! * hbbs ignores registrations (RegisterPeer / RegisterPk) of IDs that are
//!   not in the file, so an unapproved device never comes online and cannot
//!   be reached.
//! * hbbs refuses punch-hole and relay requests whose target ID is not
//!   approved, and requests coming from an IP address where no approved
//!   device has registered in the last 10 minutes
//!   (disable with `ALLOWLIST_CHECK_REQUESTER=0`).
//! * hbbs writes the IP addresses of recently seen approved devices to
//!   `.sharedesk_active_ips` next to the allowlist; hbbr refuses relay
//!   connections from other addresses (disable with `ALLOWLIST_RELAY=0`).
//!
//! File format: one ID per line; anything after `#` is a comment.
//! The file is re-read automatically within 10 seconds of a change.
//! A missing or unreadable file means "no device is approved" (fail closed).
//! IDs that were refused are appended once to `pending_ids.txt` next to the
//! allowlist, to make approving a new device easy.
#![allow(dead_code)]

use hbb_common::log;
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
    time::{Duration, Instant, SystemTime},
};

const ACTIVE_IPS_FILE: &str = ".sharedesk_active_ips";
const PENDING_FILE: &str = "pending_ids.txt";
const IP_TTL: Duration = Duration::from_secs(600);
const RELOAD_EVERY: Duration = Duration::from_secs(10);
const RELAY_RELOAD_EVERY: Duration = Duration::from_secs(5);
const DENY_LOG_EVERY: Duration = Duration::from_secs(600);

struct State {
    enabled: bool,
    check_requester: bool,
    check_relay: bool,
    path: PathBuf,
    dir: PathBuf,
    ids: RwLock<HashSet<String>>,
    mtime: Mutex<Option<SystemTime>>,
    peer_ips: Mutex<HashMap<IpAddr, Instant>>,
    relay_ips: RwLock<HashSet<IpAddr>>,
    denied: Mutex<HashMap<String, Instant>>,
    pending_written: Mutex<HashSet<String>>,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v.trim() != "0").unwrap_or(true)
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let path = std::env::var("ALLOWLIST_FILE").unwrap_or_default();
        let path = PathBuf::from(path.trim());
        let enabled = !path.as_os_str().is_empty();
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        let s = State {
            enabled,
            check_requester: env_flag("ALLOWLIST_CHECK_REQUESTER"),
            check_relay: env_flag("ALLOWLIST_RELAY"),
            path,
            dir,
            ids: Default::default(),
            mtime: Mutex::new(None),
            peer_ips: Default::default(),
            relay_ips: Default::default(),
            denied: Default::default(),
            pending_written: Default::default(),
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
                "ShareDesk allowlist loaded from {}: {} approved device(s)",
                s.path.display(),
                ids.len()
            );
            *s.ids.write().unwrap() = ids;
        }
        Err(err) => {
            log::error!(
                "ShareDesk allowlist {} cannot be read ({err}); no device is approved",
                s.path.display()
            );
            s.ids.write().unwrap().clear();
        }
    }
}

fn write_active_ips(s: &State) {
    let ips: Vec<String> = {
        let mut map = s.peer_ips.lock().unwrap();
        map.retain(|_, seen| seen.elapsed() < IP_TTL);
        map.keys().map(|ip| ip.to_string()).collect()
    };
    let target = s.dir.join(ACTIVE_IPS_FILE);
    let tmp = s.dir.join(format!("{ACTIVE_IPS_FILE}.tmp"));
    let res = std::fs::write(&tmp, ips.join("\n")).and_then(|_| std::fs::rename(&tmp, &target));
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
    log::info!(
        "ShareDesk allowlist enabled: file={}, check requester IP={}",
        s.path.display(),
        s.check_requester
    );
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
        let file = s.dir.join(ACTIVE_IPS_FILE);
        let ips: HashSet<IpAddr> = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        *s.relay_ips.write().unwrap() = ips;
        std::thread::sleep(RELAY_RELOAD_EVERY);
    });
}

pub fn is_id_allowed(id: &str) -> bool {
    let s = state();
    !s.enabled || s.ids.read().unwrap().contains(id)
}

/// Like `is_id_allowed`, but also lets through hbbs's own UDP self-test,
/// which registers a fixed ID from loopback and exits hbbs if unanswered.
pub fn is_registration_allowed(id: &str, addr: SocketAddr) -> bool {
    is_id_allowed(id) || (id == "(:test_hbbs:)" && normalize(addr).is_loopback())
}

/// Remember the address of an approved device that just registered.
pub fn note_peer(addr: SocketAddr) {
    let s = state();
    if s.enabled && !normalize(addr).is_loopback() {
        s.peer_ips
            .lock()
            .unwrap()
            .insert(normalize(addr), Instant::now());
    }
}

/// Is the connection request coming from where an approved device lives?
pub fn is_requester_allowed(addr: SocketAddr) -> bool {
    let s = state();
    if !s.enabled || !s.check_requester {
        return true;
    }
    let ip = normalize(addr);
    s.peer_ips
        .lock()
        .unwrap()
        .get(&ip)
        .map_or(false, |seen| seen.elapsed() < IP_TTL)
}

pub fn is_relay_allowed(addr: SocketAddr) -> bool {
    let s = state();
    if !s.enabled || !s.check_relay {
        return true;
    }
    let ip = normalize(addr);
    ip.is_loopback() || s.relay_ips.read().unwrap().contains(&ip)
}

/// Log (rate limited) and record an ID that tried to register without approval.
pub fn report_denied(id: &str, addr: SocketAddr) {
    let s = state();
    {
        let mut denied = s.denied.lock().unwrap();
        if let Some(t) = denied.get(id) {
            if t.elapsed() < DENY_LOG_EVERY {
                return;
            }
        }
        denied.insert(id.to_owned(), Instant::now());
    }
    log::warn!(
        "ShareDesk allowlist: device {id} from {} is not approved",
        normalize(addr)
    );
    let mut written = s.pending_written.lock().unwrap();
    if written.insert(id.to_owned()) {
        let line = format!(
            "{id}  # {} first seen {}\n",
            normalize(addr),
            chrono_like_now()
        );
        let file = s.dir.join(PENDING_FILE);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn chrono_like_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
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
