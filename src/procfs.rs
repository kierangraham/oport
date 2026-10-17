//! Thin readers over /proc. Everything here works unprivileged; processes
//! owned by other users simply return less information.

use std::{
    collections::{HashMap, HashSet},
    fs,
    net::{Ipv4Addr, Ipv6Addr},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Listener {
    pub port: u16,
    pub inode: u64,
    pub address: String,
}

#[derive(Clone, Debug)]
pub struct Stat {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub comm: String,
    pub state: char,
    /// utime + stime, in clock ticks.
    pub ticks: u64,
    /// Start time after boot, in clock ticks.
    pub start: u64,
    pub rss_pages: u64,
}

pub fn clock_ticks() -> f64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 { ticks as f64 } else { 100.0 }
}

pub fn page_size() -> u64 {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 { size as u64 } else { 4096 }
}

pub fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn boot_time() -> u64 {
    fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|raw| {
            raw.lines()
                .find_map(|line| line.strip_prefix("btime "))
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(0)
}

/// MemTotal and MemAvailable in bytes.
pub fn meminfo() -> (u64, u64) {
    let raw = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| {
        raw.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kib| kib.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

fn decode_address(hex: &str) -> Option<String> {
    match hex.len() {
        8 => {
            let raw = u32::from_str_radix(hex, 16).ok()?;
            Some(Ipv4Addr::from(raw.to_le_bytes()).to_string())
        }
        32 => {
            let mut bytes = [0u8; 16];
            for word in 0..4 {
                let raw = u32::from_str_radix(&hex[word * 8..word * 8 + 8], 16).ok()?;
                bytes[word * 4..word * 4 + 4].copy_from_slice(&raw.to_le_bytes());
            }
            let address = Ipv6Addr::from(bytes);
            Some(match address.to_ipv4_mapped() {
                Some(v4) => v4.to_string(),
                None => format!("[{address}]"),
            })
        }
        _ => None,
    }
}

/// Parses /proc/net/tcp or tcp6. Returns listening sockets and the number of
/// established connections per local port.
pub fn parse_tcp(raw: &str, listeners: &mut Vec<Listener>, established: &mut HashMap<u16, usize>) {
    for line in raw.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }
        let Some((address, port)) = fields[1].split_once(':') else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(port, 16) else {
            continue;
        };
        match fields[3] {
            "0A" => {
                let (Some(address), Ok(inode)) = (decode_address(address), fields[9].parse())
                else {
                    continue;
                };
                if inode != 0 {
                    listeners.push(Listener {
                        port,
                        inode,
                        address,
                    });
                }
            }
            "01" => *established.entry(port).or_default() += 1,
            _ => {}
        }
    }
}

pub fn tcp_sockets() -> (Vec<Listener>, HashMap<u16, usize>) {
    let mut listeners = Vec::new();
    let mut established = HashMap::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(raw) = fs::read_to_string(path) {
            parse_tcp(&raw, &mut listeners, &mut established);
        }
    }
    (listeners, established)
}

pub fn parse_stat(raw: &str, uid: u32) -> Option<Stat> {
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    let pid = raw[..open].trim().parse().ok()?;
    let comm = raw[open + 1..close].to_string();
    let rest: Vec<&str> = raw.get(close + 2..)?.split_whitespace().collect();
    let number = |index: usize| rest.get(index).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        pid,
        ppid: rest.get(1)?.parse().ok()?,
        uid,
        comm,
        state: rest.first()?.chars().next()?,
        ticks: number(11)? + number(12)?,
        start: number(19)?,
        rss_pages: number(21)?,
    })
}

fn pids() -> impl Iterator<Item = (i32, PathBuf)> {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            Some((pid, entry.path()))
        })
}

pub fn read_stat(pid: i32) -> Option<Stat> {
    let path = PathBuf::from(format!("/proc/{pid}"));
    let uid = fs::metadata(&path).ok()?.uid();
    parse_stat(&fs::read_to_string(path.join("stat")).ok()?, uid)
}

/// Every visible process, keyed by pid.
pub fn process_table() -> HashMap<i32, Stat> {
    pids()
        .filter_map(|(pid, path)| {
            let uid = fs::metadata(&path).ok()?.uid();
            let stat = parse_stat(&fs::read_to_string(path.join("stat")).ok()?, uid)?;
            Some((pid, stat))
        })
        .collect()
}

/// Maps socket inodes to the pids holding them. Only processes owned by `uid`
/// are inspected; other users' descriptors are unreadable anyway.
pub fn socket_owners(inodes: &HashSet<u64>, uid: u32) -> HashMap<u64, Vec<i32>> {
    let mut owners: HashMap<u64, Vec<i32>> = HashMap::new();
    if inodes.is_empty() {
        return owners;
    }
    for (pid, path) in pids() {
        if fs::metadata(&path).map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let Ok(fds) = fs::read_dir(path.join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            let Some(inode) = target
                .to_str()
                .and_then(|t| t.strip_prefix("socket:["))
                .and_then(|t| t.strip_suffix(']'))
                .and_then(|t| t.parse::<u64>().ok())
            else {
                continue;
            };
            if inodes.contains(&inode) {
                let list = owners.entry(inode).or_default();
                if !list.contains(&pid) {
                    list.push(pid);
                }
            }
        }
    }
    owners
}

pub fn cmdline(pid: i32) -> Vec<String> {
    fs::read(format!("/proc/{pid}/cmdline"))
        .map(|raw| {
            raw.split(|b| *b == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Working directory and whether it has been deleted (e.g. a removed worktree).
pub fn cwd(pid: i32) -> Option<(PathBuf, bool)> {
    let link = fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    let text = link.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(path) => Some((PathBuf::from(path), true)),
        None => Some((link.clone(), !link.exists())),
    }
}

pub fn exe(pid: i32) -> Option<PathBuf> {
    let link = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let text = link.to_string_lossy();
    Some(PathBuf::from(
        text.strip_suffix(" (deleted)").unwrap_or(&text),
    ))
}

pub fn environ(pid: i32) -> HashMap<String, String> {
    fs::read(format!("/proc/{pid}/environ"))
        .map(|raw| {
            raw.split(|b| *b == 0)
                .filter_map(|entry| {
                    let entry = String::from_utf8_lossy(entry);
                    let (key, value) = entry.split_once('=')?;
                    Some((key.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Proportional set size, so shared libraries are not counted once per
/// process. Falls back to resident memory when smaps is unreadable.
pub fn memory(stat: &Stat, page_size: u64) -> u64 {
    fs::read_to_string(format!("/proc/{}/smaps_rollup", stat.pid))
        .ok()
        .and_then(|raw| {
            raw.lines()
                .find_map(|line| line.strip_prefix("Pss:"))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|kib| kib.parse::<u64>().ok())
        })
        .map(|kib| kib * 1024)
        .unwrap_or(stat.rss_pages * page_size)
}

pub fn alive(pid: i32, start: u64) -> bool {
    read_stat(pid).is_some_and(|stat| stat.start == start && stat.state != 'Z' && stat.state != 'X')
}

pub fn basename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tcp_tables() {
        let v4 = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:1435 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41234 1 0000000000000000 100 0 0 10 0\n\
   1: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41235 1 0000000000000000 100 0 0 10 0\n\
   2: 0100007F:1435 0100007F:D2F0 01 00000000:00000000 00:00000000 00000000  1000        0 41236 1 0000000000000000 100 0 0 10 0\n";
        let v6 = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 51234 1 0000000000000000 100 0 0 10 0\n\
   1: 0000000000000000FFFF00000100007F:22B8 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 51235 1 0000000000000000 100 0 0 10 0\n";
        let mut listeners = Vec::new();
        let mut established = HashMap::new();
        parse_tcp(v4, &mut listeners, &mut established);
        parse_tcp(v6, &mut listeners, &mut established);
        let summary: Vec<(u16, &str)> = listeners
            .iter()
            .map(|l| (l.port, l.address.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (5173, "127.0.0.1"),
                (3000, "0.0.0.0"),
                (8080, "[::1]"),
                (8888, "127.0.0.1")
            ]
        );
        assert_eq!(established.get(&5173), Some(&1));
    }

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let raw = "4242 (next-server (v15)) S 4200 4242 4200 0 -1 4194304 1 0 0 0 150 50 0 0 20 0 11 0 9000 1000 2560 18446744073709551615";
        let stat = parse_stat(raw, 1000).unwrap();
        assert_eq!(stat.comm, "next-server (v15)");
        assert_eq!(
            (stat.ppid, stat.ticks, stat.start, stat.rss_pages),
            (4200, 200, 9000, 2560)
        );
    }

    #[test]
    fn reads_this_process() {
        let me = std::process::id() as i32;
        let stat = read_stat(me).unwrap();
        assert_eq!(stat.uid, current_uid());
        assert!(alive(me, stat.start));
        assert!(!alive(me, stat.start + 1));
        assert!(cwd(me).is_some_and(|(_, missing)| !missing));
    }
}
