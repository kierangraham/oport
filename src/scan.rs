//! Turns listening sockets into dev servers: one entry per process tree,
//! with its ports, project, agent session and resource use.

use crate::{
    agent::{self, Agent},
    config::Settings,
    procfs::{self, Stat},
    project,
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub const SCHEMA_VERSION: u32 = 1;
const MIB: u64 = 1024 * 1024;
/// History kept for charts, whatever the leak window.
const HISTORY_SECONDS: f64 = 600.0;
const HISTORY_BUCKETS: usize = 60;
/// CPU at or above this, or any open connection, counts as activity.
const ACTIVE_CPU: f64 = 2.0;

#[derive(Clone, Debug, Serialize)]
pub struct Process {
    pub pid: i32,
    pub name: String,
    pub command: String,
    pub memory: u64,
    pub cpu: f64,
    pub listening: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct History {
    pub memory: Vec<u64>,
    pub cpu: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Server {
    /// Stable for the life of the root process: `<pid>-<start ticks>`.
    pub id: String,
    pub pid: i32,
    /// The process whose command line describes the server, e.g. `npm run dev`
    /// rather than the `sh -c` an agent wrapped it in. Restart reruns this.
    pub launch_pid: i32,
    pub ports: Vec<u16>,
    pub addresses: Vec<String>,
    pub url: String,
    pub name: String,
    pub repo: Option<String>,
    pub framework: Option<String>,
    pub runtime: String,
    pub command: String,
    pub cwd: String,
    pub project_root: Option<String>,
    pub cwd_missing: bool,
    pub branch: Option<String>,
    pub worktree: bool,
    pub agent: Option<Agent>,
    pub started_at: u64,
    pub uptime: u64,
    /// Proportional memory of the whole process tree, in bytes.
    pub memory: u64,
    /// Sum over the tree; one logical core is 100%.
    pub cpu: f64,
    pub connections: usize,
    pub protected: bool,
    pub processes: Vec<Process>,
    // Populated only while watching, from history.
    pub leak: Option<String>,
    pub growth: Option<i64>,
    pub idle: bool,
    pub idle_seconds: Option<u64>,
    pub cleanup: bool,
    pub history: History,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemorySummary {
    pub total: u64,
    pub available: u64,
    pub servers: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub memory_limit: u64,
    pub growth: u64,
    pub window_minutes: u64,
    pub idle_hours: f64,
    pub port_min: u16,
    pub port_max: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub timestamp: f64,
    pub tracked: bool,
    pub servers: Vec<Server>,
    pub memory: MemorySummary,
    /// Listening processes left out because they do not look like dev servers.
    pub hidden: usize,
    pub limits: Limits,
}

/// Ports, addresses and listening pids gathered under one root process.
#[derive(Default)]
struct Group {
    ports: BTreeSet<u16>,
    addresses: BTreeSet<String>,
    listening: BTreeSet<i32>,
}

struct Track {
    samples: VecDeque<(f64, u64, f64)>,
    last_active: f64,
}

pub struct Scanner {
    clock_ticks: f64,
    page_size: u64,
    uid: u32,
    boot: u64,
    home: PathBuf,
    track: bool,
    previous: HashMap<i32, (u64, u64)>,
    previous_at: Option<Instant>,
    tracks: HashMap<String, Track>,
}

const DEV_RUNTIMES: &[&str] = &[
    "node",
    "bun",
    "deno",
    "ruby",
    "java",
    "beam.smp",
    "erl",
    "dotnet",
    "uvicorn",
    "gunicorn",
    "hugo",
    "zola",
    "caddy",
    "air",
    "wrangler",
    "workerd",
    "esbuild",
    "next-server",
];
const DEV_RUNTIME_PREFIXES: &[&str] = &["python", "php", "ruby", "node-"];

/// Processes that only launch or supervise a server. Climbing through them
/// lets Stop and Restart act on `npm run dev` rather than the bare child.
const WRAPPERS: &[&str] = &[
    "npm",
    "npx",
    "pnpm",
    "pnpx",
    "yarn",
    "bun",
    "bunx",
    "deno",
    "cargo",
    "cargo-watch",
    "uv",
    "uvx",
    "poetry",
    "pipenv",
    "pdm",
    "hatch",
    "rye",
    "bundle",
    "make",
    "just",
    "turbo",
    "nx",
    "concurrently",
    "nodemon",
    "tsx",
    "ts-node-dev",
    "mise",
    "watchexec",
    "entr",
    "go",
    "corepack",
    "dotenv",
    "foreman",
    "overmind",
    "honcho",
    "env",
];
const WRAPPER_SCRIPTS: &[&str] = &[
    "npm-cli.js",
    "npx-cli.js",
    "pnpm.cjs",
    "pnpm.js",
    "pnpm",
    "yarn.js",
    "yarn.cjs",
    "yarn",
    "npm",
    "npx",
    "nodemon",
    "concurrently",
    "turbo",
    "tsx",
    "nx",
    "ts-node-dev",
];
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "fish"];
const AGENTS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "gemini",
    "amp",
    "aider",
    "goose",
    "crush",
    "cursor-agent",
];

/// Program name without its directory. Tools that set their process title
/// (npm becomes "npm run dev") are reduced to the first word.
fn base(arg: &str) -> &str {
    let name = arg.split_whitespace().next().unwrap_or(arg);
    name.rsplit('/').next().unwrap_or(name)
}

fn is_dev_runtime(name: &str) -> bool {
    DEV_RUNTIMES.contains(&name)
        || DEV_RUNTIME_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn is_agent(stat: &Stat, argv: &[String]) -> bool {
    AGENTS.contains(&stat.comm.as_str())
        || argv.iter().take(2).any(|arg| AGENTS.contains(&base(arg)))
}

/// Whether `parent` merely wraps `child` and belongs to the same server.
fn wraps(parent: &Stat, parent_argv: &[String], child_argv: &[String]) -> bool {
    if is_agent(parent, parent_argv) {
        return false;
    }
    let comm = base(&parent.comm);
    let first = parent_argv.first().map(|a| base(a)).unwrap_or(comm);
    if SHELLS.contains(&comm) || SHELLS.contains(&first) {
        // Interactive shells own a terminal, not a server; `sh -c cmd` is a wrapper.
        return parent_argv.iter().any(|arg| arg == "-c");
    }
    if WRAPPERS.contains(&comm) || WRAPPERS.contains(&first) {
        return true;
    }
    // Reloaders re-run their own command line in a child (Django, Flask).
    if !child_argv.is_empty() && parent_argv == child_argv {
        return true;
    }
    if parent_argv
        .iter()
        .any(|arg| arg == "--reload" || arg == "--watch" || arg.starts_with("--watch="))
    {
        return true;
    }
    let script = parent_argv.get(1).map(|a| base(a)).unwrap_or("");
    if is_dev_runtime(comm) && WRAPPER_SCRIPTS.contains(&script) {
        return true;
    }
    // `next dev` supervises the next-server child that actually listens.
    script == "next"
        && parent_argv
            .iter()
            .skip(2)
            .any(|arg| arg == "dev" || arg == "start")
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn descendants(root: i32, children: &HashMap<i32, Vec<i32>>) -> Vec<i32> {
    let mut found = vec![root];
    let mut index = 0;
    while index < found.len() {
        if let Some(kids) = children.get(&found[index]) {
            found.extend(kids);
        }
        index += 1;
    }
    found
}

fn human_bytes(value: u64) -> String {
    let mut n = value as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit = 0;
    while n >= 1024.0 && unit < units.len() - 1 {
        n /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{n:.1} {}", units[unit])
    }
}

impl Scanner {
    pub fn new(track: bool) -> Scanner {
        Scanner {
            clock_ticks: procfs::clock_ticks(),
            page_size: procfs::page_size(),
            uid: procfs::current_uid(),
            boot: procfs::boot_time(),
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
            track,
            previous: HashMap::new(),
            previous_at: None,
            tracks: HashMap::new(),
        }
    }

    pub fn scan(&mut self, settings: &Settings) -> Snapshot {
        let started = Instant::now();
        let timestamp = now_unix();
        let (listeners, established) = procfs::tcp_sockets();
        let listeners: Vec<_> = listeners
            .into_iter()
            .filter(|l| settings.includes_port(l.port))
            .collect();
        let inodes: HashSet<u64> = listeners.iter().map(|l| l.inode).collect();
        let owners = procfs::socket_owners(&inodes, self.uid);
        let table = procfs::process_table();

        let elapsed = self
            .previous_at
            .map(|at| started.duration_since(at).as_secs_f64())
            .unwrap_or(0.0);
        let cpu = |stat: &Stat| -> f64 {
            match self.previous.get(&stat.pid) {
                Some((start, ticks)) if *start == stat.start && elapsed > 0.0 => {
                    stat.ticks.saturating_sub(*ticks) as f64 / self.clock_ticks / elapsed * 100.0
                }
                _ => 0.0,
            }
        };

        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        for stat in table.values() {
            children.entry(stat.ppid).or_default().push(stat.pid);
        }
        for list in children.values_mut() {
            list.sort_unstable();
        }

        // A forked server shares its socket with its workers: keep the topmost holder.
        let mut sockets: Vec<(i32, u16, String)> = Vec::new();
        for listener in &listeners {
            let Some(pids) = owners.get(&listener.inode) else {
                continue;
            };
            let top = pids
                .iter()
                .copied()
                .filter(|pid| table.get(pid).is_none_or(|s| !pids.contains(&s.ppid)))
                .min()
                .unwrap_or(pids[0]);
            sockets.push((
                top,
                listener.port,
                format!("{}:{}", listener.address, listener.port),
            ));
        }
        let listener_pids: HashSet<i32> = sockets.iter().map(|(pid, _, _)| *pid).collect();

        let mut argv_cache: HashMap<i32, Vec<String>> = HashMap::new();
        let mut argv = |pid: i32| {
            argv_cache
                .entry(pid)
                .or_insert_with(|| procfs::cmdline(pid))
                .clone()
        };
        let listeners_under = |pid: i32| {
            descendants(pid, &children)
                .iter()
                .filter(|p| listener_pids.contains(p))
                .count()
        };

        let mut groups: HashMap<i32, Group> = HashMap::new();
        for (pid, port, address) in &sockets {
            let mut root = *pid;
            while let Some(stat) = table.get(&root) {
                let Some(parent) = table.get(&stat.ppid) else {
                    break;
                };
                if parent.pid <= 1 || parent.uid != self.uid {
                    break;
                }
                // Keep sibling servers separate from a shared task runner.
                if listeners_under(parent.pid) != listeners_under(root) {
                    break;
                }
                if !wraps(parent, &argv(parent.pid), &argv(root)) {
                    break;
                }
                root = parent.pid;
            }
            let group = groups.entry(root).or_default();
            group.ports.insert(*port);
            group.addresses.insert(address.clone());
            group.listening.insert(*pid);
        }

        let mut servers = Vec::new();
        let mut hidden = 0;
        for (
            root,
            Group {
                ports,
                addresses,
                listening,
            },
        ) in groups
        {
            let Some(root_stat) = table.get(&root) else {
                continue;
            };
            let main = *listening.iter().next().unwrap_or(&root);
            let main_stat = table.get(&main).unwrap_or(root_stat);
            let runtime = procfs::exe(main)
                .map(|p| procfs::basename(&p))
                .unwrap_or_else(|| main_stat.comm.clone());
            let (cwd, cwd_missing) = procfs::cwd(main).unwrap_or_default();
            let root_missing = procfs::cwd(root).is_some_and(|(_, missing)| missing);
            let project = project::detect(&cwd, &self.home);

            let tree: Vec<&Stat> = descendants(root, &children)
                .iter()
                .filter_map(|pid| table.get(pid))
                .filter(|stat| stat.state != 'Z')
                .collect();
            let names: Vec<&str> = tree
                .iter()
                .map(|s| s.comm.as_str())
                .chain([runtime.as_str()])
                .collect();
            let protected = names.iter().any(|name| settings.is_protected(name));
            let name = project
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| main_stat.comm.clone());
            if settings.is_ignored(&name) || names.iter().any(|n| settings.is_ignored(n)) {
                hidden += 1;
                continue;
            }
            let looks_like_dev = project.is_some()
                || protected
                || is_dev_runtime(&runtime)
                || is_dev_runtime(&main_stat.comm);
            if !settings.all && !looks_like_dev {
                hidden += 1;
                continue;
            }

            let mut launch = main;
            let mut cursor = main;
            while cursor != root {
                let Some(parent) = table.get(&cursor).map(|s| s.ppid) else {
                    break;
                };
                cursor = parent;
                if table
                    .get(&cursor)
                    .is_some_and(|s| !SHELLS.contains(&s.comm.as_str()))
                {
                    launch = cursor;
                }
            }
            let launch_argv = argv(launch);
            let main_argv = argv(main);
            let launch_comm = table
                .get(&launch)
                .map(|s| s.comm.clone())
                .unwrap_or_default();
            let command = if launch_argv.is_empty() {
                launch_comm
            } else {
                launch_argv.join(" ")
            };
            let framework_hint = format!("{} {}", main_argv.join(" "), command);
            let framework = project::framework(&framework_hint, project.as_ref());

            let mut ancestors = Vec::new();
            let mut cursor = table.get(&root_stat.ppid);
            while let Some(stat) = cursor.filter(|s| s.pid > 1) {
                ancestors.push(stat.comm.clone());
                cursor = table.get(&stat.ppid);
            }
            let agent = agent::detect(&procfs::environ(main), &ancestors);

            let processes: Vec<Process> = tree
                .iter()
                .map(|stat| {
                    let args = argv(stat.pid);
                    Process {
                        pid: stat.pid,
                        name: stat.comm.clone(),
                        command: if args.is_empty() {
                            stat.comm.clone()
                        } else {
                            args.join(" ")
                        },
                        memory: procfs::memory(stat, self.page_size),
                        cpu: cpu(stat),
                        listening: listening.contains(&stat.pid),
                    }
                })
                .collect();
            let started_at = self.boot + (root_stat.start as f64 / self.clock_ticks) as u64;
            let ports: Vec<u16> = ports.into_iter().collect();
            servers.push(Server {
                id: format!("{root}-{}", root_stat.start),
                pid: root,
                launch_pid: launch,
                url: format!("http://localhost:{}", ports[0]),
                addresses: addresses.into_iter().collect(),
                connections: ports
                    .iter()
                    .map(|p| established.get(p).copied().unwrap_or(0))
                    .sum(),
                ports,
                repo: project.as_ref().and_then(|p| p.repo.clone()),
                branch: project.as_ref().and_then(|p| p.branch.clone()),
                worktree: project.as_ref().is_some_and(|p| p.worktree),
                project_root: project.as_ref().map(|p| p.root.display().to_string()),
                name,
                framework,
                runtime,
                command,
                cwd: cwd.display().to_string(),
                cwd_missing: cwd_missing || root_missing,
                agent,
                started_at,
                uptime: (timestamp as u64).saturating_sub(started_at),
                memory: processes.iter().map(|p| p.memory).sum(),
                cpu: processes.iter().map(|p| p.cpu).sum(),
                protected,
                processes,
                leak: None,
                growth: None,
                idle: false,
                idle_seconds: None,
                cleanup: false,
                history: History::default(),
            });
        }
        servers.sort_by_key(|server| server.ports[0]);

        self.previous = table
            .values()
            .map(|s| (s.pid, (s.start, s.ticks)))
            .collect();
        self.previous_at = Some(started);
        if self.track {
            self.update_tracks(&mut servers, settings, timestamp);
        } else {
            for server in &mut servers {
                server.cleanup = server.cwd_missing && !server.protected;
            }
        }

        let (total, available) = procfs::meminfo();
        Snapshot {
            schema_version: SCHEMA_VERSION,
            timestamp,
            tracked: self.track,
            memory: MemorySummary {
                total,
                available,
                servers: servers.iter().map(|s| s.memory).sum(),
            },
            servers,
            hidden,
            limits: Limits {
                memory_limit: settings.memory_limit_mb * MIB,
                growth: settings.growth_mb * MIB,
                window_minutes: settings.window_minutes,
                idle_hours: settings.idle_hours,
                port_min: settings.port_min,
                port_max: settings.port_max,
            },
        }
    }

    fn update_tracks(&mut self, servers: &mut [Server], settings: &Settings, now: f64) {
        let window = settings.window_minutes as f64 * 60.0;
        let keep = window.max(HISTORY_SECONDS);
        self.tracks
            .retain(|id, _| servers.iter().any(|s| &s.id == id));
        for server in servers.iter_mut() {
            let track = self
                .tracks
                .entry(server.id.clone())
                .or_insert_with(|| Track {
                    samples: VecDeque::new(),
                    last_active: now,
                });
            track.samples.push_back((now, server.memory, server.cpu));
            while track
                .samples
                .front()
                .is_some_and(|(t, _, _)| now - t > keep)
            {
                track.samples.pop_front();
            }
            if server.cpu >= ACTIVE_CPU || server.connections > 0 {
                track.last_active = now;
            }
            let idle_seconds = (now - track.last_active).max(0.0);
            server.idle_seconds = Some(idle_seconds as u64);
            server.idle = settings.idle_hours > 0.0 && idle_seconds >= settings.idle_hours * 3600.0;

            let recent: Vec<&(f64, u64, f64)> = track
                .samples
                .iter()
                .filter(|(t, _, _)| now - t <= window)
                .collect();
            let low = recent
                .iter()
                .map(|(_, m, _)| *m)
                .min()
                .unwrap_or(server.memory);
            let observed = recent.first().map(|(t, _, _)| now - t).unwrap_or(0.0);
            let growth = server.memory as i64 - low as i64;
            // Startup allocation is not a leak; require two minutes of history.
            server.growth = (observed >= 120.0).then_some(growth);
            let limit = settings.memory_limit_mb * MIB;
            let growth_limit = settings.growth_mb * MIB;
            server.leak = if limit > 0 && server.memory >= limit {
                Some(format!(
                    "Using {}, over the {} limit",
                    human_bytes(server.memory),
                    human_bytes(limit)
                ))
            } else if growth_limit > 0 && observed >= 120.0 && growth >= growth_limit as i64 {
                Some(format!(
                    "Grew {} in {} min",
                    human_bytes(growth as u64),
                    (observed / 60.0).round().max(1.0)
                ))
            } else {
                None
            };
            server.cleanup =
                !server.protected && server.leak.is_none() && (server.cwd_missing || server.idle);

            let span = HISTORY_SECONDS / HISTORY_BUCKETS as f64;
            let mut buckets: Vec<Option<(u64, f64)>> = vec![None; HISTORY_BUCKETS];
            for (t, memory, cpu) in &track.samples {
                let age = now - t;
                if age >= HISTORY_SECONDS {
                    continue;
                }
                let index = HISTORY_BUCKETS - 1 - (age / span) as usize;
                let bucket = buckets[index].get_or_insert((0, 0.0));
                bucket.0 = bucket.0.max(*memory);
                bucket.1 = bucket.1.max(*cpu);
            }
            let filled: Vec<(u64, f64)> = buckets.into_iter().flatten().collect();
            server.history = History {
                memory: filled.iter().map(|(m, _)| *m).collect(),
                cpu: filled
                    .iter()
                    .map(|(_, c)| (c * 10.0).round() / 10.0)
                    .collect(),
            };
        }
    }
}

/// Finds a server by `:port`, `port`, `pid:N` or `id`.
pub fn find<'a>(servers: &'a [Server], spec: &str) -> Option<&'a Server> {
    let spec = spec.trim();
    if let Some(pid) = spec
        .strip_prefix("pid:")
        .and_then(|p| p.parse::<i32>().ok())
    {
        return servers
            .iter()
            .find(|s| s.pid == pid || s.processes.iter().any(|p| p.pid == pid));
    }
    if let Some(server) = servers.iter().find(|s| s.id == spec) {
        return Some(server);
    }
    let port = spec.trim_start_matches(':').parse::<u16>().ok()?;
    servers.iter().find(|s| s.ports.contains(&port))
}

pub fn format_bytes(value: u64) -> String {
    human_bytes(value)
}

pub fn format_duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86400, seconds % 86400 / 3600, seconds % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

pub fn home_relative(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && Path::new(path).starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(pid: i32, comm: &str) -> Stat {
        Stat {
            pid,
            ppid: 1,
            uid: 1000,
            comm: comm.into(),
            state: 'S',
            ticks: 0,
            start: 0,
            rss_pages: 0,
        }
    }
    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(String::from).collect()
    }

    #[test]
    fn climbs_only_through_wrappers() {
        let child = args("node /p/node_modules/.bin/vite");
        assert!(wraps(
            &stat(1, "npm run dev"),
            &["npm run dev".to_string()],
            &child
        ));
        assert!(wraps(
            &stat(1, "node"),
            &args("node /usr/lib/node_modules/npm/bin/npm-cli.js run dev"),
            &child
        ));
        assert!(wraps(&stat(1, "sh"), &args("sh -c vite"), &child));
        assert!(wraps(
            &stat(1, "node"),
            &args("node /p/node_modules/.bin/next dev"),
            &args("next-server (v15)")
        ));
        assert!(wraps(
            &stat(1, "python3"),
            &args("python3 manage.py runserver"),
            &args("python3 manage.py runserver")
        ));
        assert!(!wraps(&stat(1, "bash"), &args("-bash"), &child));
        assert!(!wraps(&stat(1, "zsh"), &args("zsh"), &child));
        assert!(!wraps(&stat(1, "claude"), &args("claude"), &child));
        assert!(!wraps(
            &stat(1, "node"),
            &args("node /home/u/.bin/gemini --watch"),
            &child
        ));
        assert!(!wraps(&stat(1, "tmux: server"), &args("tmux"), &child));
    }

    fn server(id: &str, memory: u64) -> Server {
        Server {
            id: id.into(),
            pid: 10,
            launch_pid: 10,
            ports: vec![3000, 9229],
            addresses: vec![],
            url: String::new(),
            name: "web".into(),
            repo: None,
            framework: None,
            runtime: "node".into(),
            command: String::new(),
            cwd: String::new(),
            project_root: None,
            cwd_missing: false,
            branch: None,
            worktree: false,
            agent: None,
            started_at: 0,
            uptime: 0,
            memory,
            cpu: 0.0,
            connections: 0,
            protected: false,
            processes: vec![Process {
                pid: 11,
                name: "node".into(),
                command: String::new(),
                memory: 0,
                cpu: 0.0,
                listening: true,
            }],
            leak: None,
            growth: None,
            idle: false,
            idle_seconds: None,
            cleanup: false,
            history: History::default(),
        }
    }

    #[test]
    fn finds_servers_by_port_pid_or_id() {
        let server = server("10-5", 0);
        let servers = [server];
        assert!(find(&servers, ":9229").is_some());
        assert!(find(&servers, "3000").is_some());
        assert!(find(&servers, "pid:11").is_some());
        assert!(find(&servers, "10-5").is_some());
        assert!(find(&servers, "4000").is_none());
    }

    #[test]
    fn flags_leaks_idle_servers_and_cleanup_candidates() {
        let mib = 1024 * 1024;
        let settings = Settings {
            idle_hours: 1.0,
            ..Settings::default()
        };
        let mut scanner = Scanner::new(true);
        // Steady growth from 400 MiB: 100 MiB a minute for six minutes.
        for minute in 0..=6u64 {
            let mut servers = [
                server("1-1", (400 + minute * 100) * mib),
                server("2-2", 300 * mib),
            ];
            scanner.update_tracks(&mut servers, &settings, minute as f64 * 60.0);
            let [growing, steady] = servers;
            if minute < 2 {
                assert_eq!(
                    growing.leak, None,
                    "no verdict before two minutes of history"
                );
            }
            if minute == 5 {
                assert_eq!(growing.leak.as_deref(), Some("Grew 500.0 MiB in 5 min"));
                assert_eq!(growing.growth, Some(500 * mib as i64));
                assert!(!growing.cleanup, "leaking servers are never preselected");
            }
            assert_eq!(steady.leak, None);
        }
        let mut servers = [server("3-3", 2048 * mib)];
        scanner.update_tracks(&mut servers, &settings, 400.0);
        assert!(
            servers[0]
                .leak
                .as_deref()
                .unwrap()
                .starts_with("Using 2.0 GiB")
        );

        // Idle: no CPU and no connections for an hour; a deleted folder is stale at once.
        let mut quiet = [server("4-4", mib)];
        scanner.update_tracks(&mut quiet, &settings, 1000.0);
        scanner.update_tracks(&mut quiet, &settings, 1000.0 + 3599.0);
        assert!(!quiet[0].idle);
        scanner.update_tracks(&mut quiet, &settings, 1000.0 + 3600.0);
        assert!(quiet[0].idle && quiet[0].cleanup);
        quiet[0].connections = 1;
        scanner.update_tracks(&mut quiet, &settings, 1000.0 + 3700.0);
        assert!(!quiet[0].idle);
        let mut gone = [server("5-5", mib)];
        gone[0].cwd_missing = true;
        scanner.update_tracks(&mut gone, &settings, 5000.0);
        assert!(gone[0].cleanup);
        gone[0].protected = true;
        scanner.update_tracks(&mut gone, &settings, 5002.0);
        assert!(!gone[0].cleanup);
        assert!(!gone[0].history.memory.is_empty());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(42), "42s");
        assert_eq!(format_duration(3720), "1h 2m");
        assert_eq!(format_duration(90000), "1d 1h");
    }
}
