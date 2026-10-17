//! Links a server to the coding-agent session that started it.
//!
//! Agents pass identifying variables to every command they run, so the
//! server's own environment is checked first. A process ancestor named after
//! a known agent is the fallback for agents that set no variables.

use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Agent {
    pub kind: &'static str,
    pub label: &'static str,
    pub session: Option<String>,
    /// Shell command that reopens the conversation, run from the server's folder.
    pub resume: Option<String>,
    /// The desktop app hosting the agent, when known.
    pub host: Option<&'static str>,
}

const ANCESTORS: &[(&str, &str, &str)] = &[
    ("claude", "claude", "Claude Code"),
    ("codex", "codex", "Codex"),
    ("opencode", "opencode", "OpenCode"),
    ("gemini", "gemini", "Gemini CLI"),
    ("amp", "amp", "Amp"),
    ("aider", "aider", "Aider"),
    ("goose", "goose", "Goose"),
    ("crush", "crush", "Crush"),
];

fn host(env: &HashMap<String, String>) -> Option<&'static str> {
    let has = |key: &str, needle: &str| env.get(key).is_some_and(|v| v.contains(needle));
    if has("CHROME_DESKTOP", "T3Code") || has("APPIMAGE", "T3-Code") {
        Some("T3 Code")
    } else if env.contains_key("ZED_TERM") {
        Some("Zed")
    } else if has("TERM_PROGRAM", "vscode") {
        Some("VS Code")
    } else {
        None
    }
}

pub fn detect(env: &HashMap<String, String>, ancestors: &[String]) -> Option<Agent> {
    let host = host(env);
    // Session ids end up in a shell command, so accept only plain identifiers.
    let value = |key: &str| {
        env.get(key)
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .filter(|v| {
                v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .cloned()
    };
    if env.contains_key("CLAUDE_CODE_SESSION_ID") || env.get("CLAUDECODE").is_some_and(|v| v == "1")
    {
        let session = value("CLAUDE_CODE_SESSION_ID");
        return Some(Agent {
            kind: "claude",
            label: "Claude Code",
            resume: Some(match &session {
                Some(id) => format!("claude --resume {id}"),
                None => "claude --continue".to_string(),
            }),
            session,
            host,
        });
    }
    if env.contains_key("CODEX_THREAD_ID") || env.contains_key("CODEX_SANDBOX") {
        let session = value("CODEX_THREAD_ID");
        return Some(Agent {
            kind: "codex",
            label: "Codex",
            resume: Some(match &session {
                Some(id) => format!("codex resume {id}"),
                None => "codex resume --last".to_string(),
            }),
            session,
            host,
        });
    }
    ancestors.iter().find_map(|name| {
        let (_, kind, label) = ANCESTORS.iter().find(|(comm, _, _)| name == comm)?;
        Some(Agent {
            kind,
            label,
            session: None,
            resume: None,
            host,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn prefers_session_variables() {
        let claude = detect(
            &env(&[
                ("CLAUDECODE", "1"),
                ("CLAUDE_CODE_SESSION_ID", "abc"),
                ("CHROME_DESKTOP", "com.t3tools.T3Code.desktop"),
            ]),
            &["codex".into()],
        )
        .unwrap();
        assert_eq!(claude.label, "Claude Code");
        assert_eq!(claude.resume.as_deref(), Some("claude --resume abc"));
        assert_eq!(claude.host, Some("T3 Code"));

        let codex = detect(&env(&[("CODEX_THREAD_ID", "t-1")]), &[]).unwrap();
        assert_eq!(codex.resume.as_deref(), Some("codex resume t-1"));

        let odd = detect(&env(&[("CLAUDE_CODE_SESSION_ID", "x; rm -rf ~")]), &[]).unwrap();
        assert_eq!(
            (odd.session, odd.resume.as_deref()),
            (None, Some("claude --continue"))
        );
    }

    #[test]
    fn falls_back_to_ancestors() {
        let agent = detect(&env(&[]), &["bash".into(), "opencode".into()]).unwrap();
        assert_eq!((agent.kind, agent.session), ("opencode", None));
        assert_eq!(
            detect(&env(&[]), &["bash".into(), "tmux: server".into()]),
            None
        );
    }
}
