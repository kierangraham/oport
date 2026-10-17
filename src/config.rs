//! Settings shared by the bar widget, CLI and TUI.
//!
//! The canonical copy lives on the widget's entry in Omarchy's shell.json,
//! where the shell's settings UI edits it. Command-line flags override it.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const PLUGIN_ID: &str = "kierangraham.oport";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub port_min: u16,
    pub port_max: u16,
    pub memory_limit_mb: u64,
    pub growth_mb: u64,
    pub window_minutes: u64,
    pub idle_hours: f64,
    pub grace_seconds: f64,
    pub protected: Vec<String>,
    pub ignored: Vec<String>,
    /// Include every listening process of this user, not only dev servers.
    pub all: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            port_min: 1024,
            port_max: 32767,
            memory_limit_mb: 2048,
            growth_mb: 500,
            window_minutes: 10,
            idle_hours: 3.0,
            grace_seconds: 5.0,
            protected: list(
                "postgres, redis-server, valkey-server, mongod, mysqld, mariadbd, memcached, dockerd, rootlesskit, containerd",
            ),
            ignored: Vec::new(),
            all: false,
        }
    }
}

fn list(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

pub fn shell_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("omarchy/shell.json"))
}

/// The widget's entry from any bar section, if the widget is placed.
pub fn widget_entry(shell: &Value) -> Option<&Value> {
    let layout = shell.get("bar")?.get("layout")?.as_object()?;
    layout
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(PLUGIN_ID))
}

impl Settings {
    pub fn apply(&mut self, entry: &Value) {
        let number = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
        };
        if let Some(n) = number("portMin") {
            self.port_min = n.min(65535.0) as u16;
        }
        if let Some(n) = number("portMax") {
            self.port_max = n.min(65535.0) as u16;
        }
        if let Some(n) = number("memoryLimitMb") {
            self.memory_limit_mb = n as u64;
        }
        if let Some(n) = number("growthMb") {
            self.growth_mb = n as u64;
        }
        if let Some(n) = number("windowMinutes") {
            self.window_minutes = (n as u64).clamp(1, 120);
        }
        if let Some(n) = number("idleHours") {
            self.idle_hours = n;
        }
        if let Some(n) = number("graceSeconds") {
            self.grace_seconds = n.min(60.0);
        }
        if let Some(raw) = entry.get("protected").and_then(Value::as_str) {
            self.protected = list(raw);
        }
        if let Some(raw) = entry.get("ignored").and_then(Value::as_str) {
            self.ignored = list(raw);
        }
        let flag = |key: &str| entry.get(key).and_then(Value::as_bool);
        if let Some(all) = flag("showAll") {
            self.all = all;
        }
    }

    pub fn load(path: Option<&Path>) -> Settings {
        let mut settings = Settings::default();
        if let Some(entry) = path
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .as_ref()
            .and_then(widget_entry)
        {
            settings.apply(entry);
        }
        settings
    }

    pub fn includes_port(&self, port: u16) -> bool {
        (self.port_min..=self.port_max).contains(&port)
    }

    pub fn is_protected(&self, name: &str) -> bool {
        self.protected.iter().any(|item| name_matches(item, name))
    }

    pub fn is_ignored(&self, name: &str) -> bool {
        self.ignored.iter().any(|item| name_matches(item, name))
    }
}

/// Process names are truncated to 15 bytes by the kernel, so a configured
/// name matches the full name or its truncated form.
fn name_matches(configured: &str, name: &str) -> bool {
    configured.eq_ignore_ascii_case(name)
        || (name.len() == 15
            && configured.len() > 15
            && configured[..15].eq_ignore_ascii_case(name))
}

/// Reloads settings when shell.json changes; flags stay applied on top.
pub struct Watcher {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    overrides: Vec<(String, Value)>,
    pub settings: Settings,
}

impl Watcher {
    pub fn new(path: Option<PathBuf>, overrides: Vec<(String, Value)>) -> Watcher {
        let mut watcher = Watcher {
            path,
            modified: None,
            overrides,
            settings: Settings::default(),
        };
        watcher.reload();
        watcher
    }

    fn reload(&mut self) {
        self.modified = self
            .path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        let mut settings = Settings::load(self.path.as_deref());
        let overrides = Value::Object(self.overrides.iter().cloned().collect());
        settings.apply(&overrides);
        self.settings = settings;
    }

    /// Returns true when the settings changed.
    pub fn refresh(&mut self) -> bool {
        let modified = self
            .path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        if modified == self.modified {
            return false;
        }
        let before = self.settings.clone();
        self.reload();
        before != self.settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_widget_entry_from_any_section() {
        let shell: Value = serde_json::from_str(
            r#"{"bar":{"layout":{"left":[{"id":"omarchy.menu"}],
                "right":[{"id":"kierangraham.oport","memoryLimitMb":4096,"portMax":65535,
                          "protected":"postgres, clickhouse-server","showAll":true}]}}}"#,
        )
        .unwrap();
        let mut settings = Settings::default();
        settings.apply(widget_entry(&shell).unwrap());
        assert_eq!(
            (settings.memory_limit_mb, settings.port_max, settings.all),
            (4096, 65535, true)
        );
        assert!(settings.is_protected("postgres"));
        assert!(settings.is_protected("clickhouse-serv"));
        assert!(!settings.is_protected("redis-server"));
        assert_eq!(settings.growth_mb, 500);
    }

    #[test]
    fn ignores_invalid_values() {
        let mut settings = Settings::default();
        settings
            .apply(&serde_json::json!({"memoryLimitMb": -1, "portMin": "80", "windowMinutes": 0}));
        assert_eq!(settings.memory_limit_mb, 2048);
        assert_eq!(settings.port_min, 1024);
        assert_eq!(settings.window_minutes, 1);
    }
}
