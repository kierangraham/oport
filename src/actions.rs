//! Stop, restart and open servers.

use crate::{
    procfs,
    scan::{Server, format_bytes},
};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Outcome {
    pub stopped: usize,
    pub killed: usize,
}

fn signal(pid: i32, signal: i32) {
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Ends the server's own process tree: SIGTERM, then SIGKILL for anything
/// still running after `grace` seconds. Each pid is checked against its start
/// time first, so a recycled pid is never signalled.
pub fn stop(server: &Server, force: bool, grace: f64) -> Result<Outcome, String> {
    if server.protected && !force {
        return Err(format!(
            "{} on :{} is protected; pass --force to stop it",
            server.name, server.ports[0]
        ));
    }
    let targets: Vec<(i32, u64)> = server
        .processes
        .iter()
        .filter_map(|p| procfs::read_stat(p.pid).map(|stat| (p.pid, stat.start)))
        .collect();
    if targets.is_empty() {
        return Ok(Outcome {
            stopped: 0,
            killed: 0,
        });
    }
    // Children first, so supervisors do not respawn what was just stopped.
    for (pid, _) in targets.iter().rev() {
        signal(*pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs_f64(grace.max(0.0));
    let running = || {
        targets
            .iter()
            .filter(|(pid, start)| procfs::alive(*pid, *start))
            .count()
    };
    while running() > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    let mut killed = 0;
    for (pid, start) in &targets {
        if procfs::alive(*pid, *start) {
            signal(*pid, libc::SIGKILL);
            killed += 1;
        }
    }
    Ok(Outcome {
        stopped: targets.len(),
        killed,
    })
}

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("oport")
}

fn port_free(port: u16) -> bool {
    !procfs::tcp_sockets().0.iter().any(|l| l.port == port)
}

/// Stops the server and reruns its original command, detached, in the same
/// folder with the same environment. Output goes to a log file.
pub fn restart(server: &Server, force: bool, grace: f64) -> Result<PathBuf, String> {
    if server.cwd_missing {
        return Err("its folder no longer exists".into());
    }
    let mut argv = procfs::cmdline(server.launch_pid);
    // A retitled process (npm's "npm run dev") exposes one space-joined string.
    if argv.len() == 1 && argv[0].contains(' ') {
        argv = argv[0].split_whitespace().map(String::from).collect();
    }
    let env: HashMap<String, String> = procfs::environ(server.launch_pid);
    let cwd = procfs::cwd(server.launch_pid)
        .map(|(path, _)| path)
        .ok_or("its folder is unreadable")?;
    let program = argv.first().ok_or("its command line is unreadable")?;
    // Processes may rename themselves ("next-server (v15)"); only rerun real executables.
    let resolvable = program.contains('/')
        || env.get("PATH").is_some_and(|path| {
            path.split(':')
                .any(|dir| PathBuf::from(dir).join(program).is_file())
        });
    if !resolvable {
        return Err(format!("cannot rerun `{program}`: it is not an executable"));
    }

    stop(server, force, grace)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server.ports.iter().all(|port| port_free(*port)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }

    let logs = state_dir().join("logs");
    fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let slug: String = server
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let log_path = logs.join(format!("{slug}-{}.log", server.ports[0]));
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    unsafe {
        // A new session survives the CLI, TUI or shell that asked for the restart.
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|e| format!("could not start `{program}`: {e}"))?;
    Ok(log_path)
}

pub fn open(server: &Server) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(&server.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            // Reap it, so a long-running TUI does not collect zombies.
            thread::spawn(move || child.wait());
        })
        .map_err(|e| format!("could not run xdg-open: {e}"))
}

pub fn describe(server: &Server) -> String {
    let ports: Vec<String> = server.ports.iter().map(|p| format!(":{p}")).collect();
    format!(
        "{} {} ({})",
        ports.join(" "),
        server.name,
        format_bytes(server.memory)
    )
}
