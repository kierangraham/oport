mod actions;
mod agent;
mod config;
mod procfs;
mod project;
mod scan;
mod tui;

use scan::{Scanner, Server, Snapshot, format_bytes, format_duration};
use serde_json::Value;
use std::{
    io::{self, BufRead, IsTerminal, Write},
    process::ExitCode,
    thread,
    time::Duration,
};

const HELP: &str = "oport — dev servers on local ports

Usage: oport [command] [options]

Commands:
  (none)            Interactive view; prints the list when not in a terminal
  list              Print servers and exit
  watch             Stream one JSON snapshot per line (used by the bar widget)
  stop TARGET...    Stop servers and their own process trees
  restart TARGET    Stop a server and rerun its command in the same folder
  open TARGET       Open a server in the browser
  cleanup           Stop servers whose folders were deleted

A TARGET is a port (3000 or :3000), pid:N, or a server id from --json.

Options:
  --json              Machine-readable output for list
  --all               Include every listening process, not only dev servers
  --force             Also stop protected processes such as databases
  --yes               Do not ask for confirmation
  --dry-run           Show what cleanup would stop
  --interval S        Seconds between watch snapshots (default 2)
  --count N           Stop watching after N snapshots
  --ports MIN-MAX     Port range to scan (default 1024-32767)
  --memory-limit MB   Flag servers using more than this (default 2048)
  --growth MB         Flag servers growing this much within the window (default 500)
  --window MIN        Leak window in minutes (default 10)
  --idle-hours H      Hours without CPU or connections before a server is idle (default 3)
  --grace S           Seconds between SIGTERM and SIGKILL (default 5)
  -h, --help          Show this help
  -V, --version       Show the version

Defaults come from the oport widget's settings in ~/.config/omarchy/shell.json.";

#[derive(Default)]
struct Args {
    command: Option<String>,
    targets: Vec<String>,
    json: bool,
    force: bool,
    yes: bool,
    dry_run: bool,
    interval: f64,
    count: Option<u64>,
    overrides: Vec<(String, Value)>,
}

fn parse(raw: Vec<String>) -> Result<Args, String> {
    let mut args = Args {
        interval: 2.0,
        ..Args::default()
    };
    let mut iter = raw.into_iter();
    let number = |flag: &str, value: Option<String>| -> Result<f64, String> {
        value
            .as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| format!("{flag} needs a non-negative number"))
    };
    while let Some(arg) = iter.next() {
        let mut set = |key: &str, value: Value| args.overrides.push((key.to_string(), value));
        match arg.as_str() {
            "-h" | "--help" => args.command = Some("help".into()),
            "-V" | "--version" => args.command = Some("version".into()),
            "--json" => args.json = true,
            "--all" => set("showAll", Value::Bool(true)),
            "--force" => args.force = true,
            "--yes" | "-y" => args.yes = true,
            "--dry-run" => args.dry_run = true,
            "--interval" => args.interval = number(&arg, iter.next())?.clamp(0.25, 60.0),
            "--count" => args.count = Some(number(&arg, iter.next())? as u64),
            "--memory-limit" => set("memoryLimitMb", number(&arg, iter.next())?.into()),
            "--growth" => set("growthMb", number(&arg, iter.next())?.into()),
            "--window" => set("windowMinutes", number(&arg, iter.next())?.into()),
            "--idle-hours" => set("idleHours", number(&arg, iter.next())?.into()),
            "--grace" => set("graceSeconds", number(&arg, iter.next())?.into()),
            "--ports" => {
                let value = iter.next().unwrap_or_default();
                let (min, max) = value
                    .split_once('-')
                    .and_then(|(a, b)| Some((a.parse::<u16>().ok()?, b.parse::<u16>().ok()?)))
                    .filter(|(a, b)| a <= b)
                    .ok_or("--ports needs a range such as 1024-65535")?;
                set("portMin", min.into());
                set("portMax", max.into());
            }
            flag if flag.starts_with('-') && flag.len() > 1 && flag.parse::<i64>().is_err() => {
                return Err(format!("unknown option {flag}"));
            }
            _ if args.command.is_none() => args.command = Some(arg),
            _ => args.targets.push(arg),
        }
    }
    Ok(args)
}

/// Two samples a moment apart, so CPU usage is measured rather than zero.
fn snapshot(settings: &config::Settings) -> Snapshot {
    let mut scanner = Scanner::new(false);
    scanner.scan(settings);
    thread::sleep(Duration::from_millis(300));
    scanner.scan(settings)
}

fn print_table(snapshot: &Snapshot) {
    if snapshot.servers.is_empty() {
        println!("No dev servers are listening.");
    } else {
        let rows: Vec<[String; 8]> = snapshot
            .servers
            .iter()
            .map(|s| {
                [
                    s.ports
                        .iter()
                        .map(|p| format!(":{p}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    s.name.clone(),
                    s.branch.clone().unwrap_or_else(|| "—".into()),
                    s.framework.clone().unwrap_or_else(|| s.runtime.clone()),
                    s.agent
                        .as_ref()
                        .map(|a| a.label.to_string())
                        .unwrap_or_else(|| "—".into()),
                    format_duration(s.uptime),
                    format_bytes(s.memory),
                    format!("{:.1}%", s.cpu),
                ]
            })
            .collect();
        let header = [
            "PORT",
            "PROJECT",
            "BRANCH",
            "FRAMEWORK",
            "AGENT",
            "UP",
            "MEMORY",
            "CPU",
        ];
        let width = |i: usize| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([header[i].len()])
                .max()
                .unwrap_or(0)
        };
        let widths: Vec<usize> = (0..8).map(width).collect();
        let line = |cells: &[String]| {
            cells
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    if i >= 5 {
                        format!("{cell:>w$}", w = widths[i])
                    } else {
                        format!("{cell:<w$}", w = widths[i])
                    }
                })
                .collect::<Vec<_>>()
                .join("  ")
        };
        let bold = io::stdout().is_terminal();
        let header_line = line(&header.map(String::from));
        println!(
            "{}",
            if bold {
                format!("\x1b[1m{header_line}\x1b[0m")
            } else {
                header_line
            }
        );
        for (row, server) in rows.iter().zip(&snapshot.servers) {
            let mut text = line(row);
            if server.cwd_missing {
                text.push_str("  (folder deleted)");
            }
            if server.protected {
                text.push_str("  (protected)");
            }
            println!("{}", text.trim_end());
        }
    }
    let memory = &snapshot.memory;
    println!(
        "\n{} server{} using {} · {} of {} RAM available{}",
        snapshot.servers.len(),
        if snapshot.servers.len() == 1 { "" } else { "s" },
        format_bytes(memory.servers),
        format_bytes(memory.available),
        format_bytes(memory.total),
        if snapshot.hidden > 0 {
            format!(" · {} other listener(s) hidden, see --all", snapshot.hidden)
        } else {
            String::new()
        }
    );
}

fn watch(args: &Args, settings: &mut config::Watcher) -> Result<(), String> {
    let mut scanner = Scanner::new(true);
    let mut stdout = io::stdout().lock();
    let mut sent = 0;
    loop {
        settings.refresh();
        let snapshot = scanner.scan(&settings.settings);
        let line = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
        if writeln!(stdout, "{line}")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            return Ok(()); // The reader went away.
        }
        sent += 1;
        if args.count.is_some_and(|count| sent >= count) {
            return Ok(());
        }
        thread::sleep(Duration::from_secs_f64(args.interval));
    }
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer).is_ok() && matches!(answer.trim(), "y" | "Y" | "yes")
}

fn targets<'a>(servers: &'a [Server], specs: &[String]) -> Result<Vec<&'a Server>, String> {
    if specs.is_empty() {
        return Err("name a server by port, pid:N or id".into());
    }
    let mut found: Vec<&Server> = Vec::new();
    for spec in specs {
        let server =
            scan::find(servers, spec).ok_or_else(|| format!("no dev server matches {spec}"))?;
        if !found.iter().any(|s| s.id == server.id) {
            found.push(server);
        }
    }
    Ok(found)
}

fn stop_all(servers: &[&Server], args: &Args, grace: f64) -> bool {
    // Stop in parallel; each waits out its own grace period.
    let results: Vec<_> = thread::scope(|scope| {
        let handles: Vec<_> = servers
            .iter()
            .map(|server| scope.spawn(move || (server, actions::stop(server, args.force, grace))))
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut ok = true;
    for (server, result) in results {
        match result {
            Ok(outcome) => println!(
                "Stopped {} · {} process{}{}",
                actions::describe(server),
                outcome.stopped,
                if outcome.stopped == 1 { "" } else { "es" },
                if outcome.killed > 0 {
                    format!(" · {} force-quit", outcome.killed)
                } else {
                    String::new()
                }
            ),
            Err(error) => {
                eprintln!("oport: {error}");
                ok = false;
            }
        }
    }
    ok
}

fn run(args: Args) -> Result<bool, String> {
    let mut watcher = config::Watcher::new(config::shell_config_path(), args.overrides.clone());
    let settings = watcher.settings.clone();
    match args.command.as_deref() {
        Some("help") => println!("{HELP}"),
        Some("version") => println!("oport {}", env!("CARGO_PKG_VERSION")),
        None if io::stdout().is_terminal() && io::stdin().is_terminal() => {
            tui::run(watcher, args.interval)?
        }
        None | Some("list") | Some("ls") => {
            let snapshot = snapshot(&settings);
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?
                );
            } else {
                print_table(&snapshot);
            }
        }
        Some("watch") => watch(&args, &mut watcher)?,
        Some("stop") | Some("kill") => {
            let snapshot = snapshot(&settings);
            let servers = targets(&snapshot.servers, &args.targets)?;
            return Ok(stop_all(&servers, &args, settings.grace_seconds));
        }
        Some("restart") => {
            let snapshot = snapshot(&settings);
            for server in targets(&snapshot.servers, &args.targets)? {
                let log = actions::restart(server, args.force, settings.grace_seconds)
                    .map_err(|e| format!("cannot restart {}: {e}", actions::describe(server)))?;
                println!(
                    "Restarted {} · output in {}",
                    actions::describe(server),
                    log.display()
                );
            }
        }
        Some("open") => {
            let snapshot = snapshot(&settings);
            for server in targets(&snapshot.servers, &args.targets)? {
                actions::open(server)?;
            }
        }
        Some("cleanup") => {
            let snapshot = snapshot(&settings);
            let stale: Vec<&Server> = snapshot.servers.iter().filter(|s| s.cleanup).collect();
            if stale.is_empty() {
                println!("Nothing to clean up: every server's folder still exists.");
                return Ok(true);
            }
            for server in &stale {
                println!(
                    "{} · folder deleted: {}",
                    actions::describe(server),
                    server.cwd
                );
            }
            if args.dry_run {
                return Ok(true);
            }
            if !args.yes
                && (!io::stdin().is_terminal()
                    || !confirm(&format!("Stop {} server(s)?", stale.len())))
            {
                println!("Nothing stopped.");
                return Ok(true);
            }
            return Ok(stop_all(&stale, &args, settings.grace_seconds));
        }
        Some(other) => return Err(format!("unknown command {other}; see oport --help")),
    }
    Ok(true)
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1).collect()) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("oport: {error}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("oport: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Result<Args, String> {
        parse(line.split_whitespace().map(String::from).collect())
    }

    #[test]
    fn parses_commands_targets_and_overrides() {
        let args = parse_line("stop :3000 pid:42 --force --grace 1 --ports 3000-4000").unwrap();
        assert_eq!(args.command.as_deref(), Some("stop"));
        assert_eq!(args.targets, [":3000", "pid:42"]);
        assert!(args.force);
        let mut settings = config::Settings::default();
        settings.apply(&Value::Object(args.overrides.into_iter().collect()));
        assert_eq!(
            (settings.port_min, settings.port_max, settings.grace_seconds),
            (3000, 4000, 1.0)
        );
        assert!(parse_line("list --bogus").is_err());
        assert!(parse_line("watch --interval -1").is_err());
        assert!(parse_line("list --ports 9-1").is_err());
    }
}
