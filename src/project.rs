//! Identifies the project, framework and git branch behind a working directory.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    /// The enclosing repository's name when it differs from the project.
    pub repo: Option<String>,
    pub manifest: Option<&'static str>,
    pub branch: Option<String>,
    pub worktree: bool,
}

const MANIFESTS: &[&str] = &[
    "package.json",
    "deno.json",
    "deno.jsonc",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "manage.py",
    "requirements.txt",
    "Gemfile",
    "mix.exs",
    "composer.json",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "hugo.toml",
    "_config.yml",
];

/// Finds the nearest manifest below `home`, within the enclosing git
/// repository when there is one. Returns None outside a recognisable project.
pub fn detect(cwd: &Path, home: &Path) -> Option<Project> {
    if !cwd.starts_with(home) || cwd == home {
        return None;
    }
    let mut manifest_dir: Option<(PathBuf, &'static str)> = None;
    let mut git_dir: Option<PathBuf> = None;
    for dir in cwd.ancestors().take_while(|dir| *dir != home) {
        if manifest_dir.is_none()
            && let Some(file) = MANIFESTS.iter().find(|file| dir.join(file).is_file())
        {
            manifest_dir = Some((dir.to_path_buf(), *file));
        }
        if dir.join(".git").exists() {
            git_dir = Some(dir.to_path_buf());
            break;
        }
    }
    let (root, manifest) = match (manifest_dir, &git_dir) {
        (Some((dir, file)), _) => (dir, Some(file)),
        (None, Some(repo)) => (repo.clone(), None),
        (None, None) => return None,
    };
    let name = manifest
        .and_then(|file| manifest_name(&root, file))
        .unwrap_or_else(|| super::procfs::basename(&root));
    let repo = git_dir
        .as_ref()
        .map(|dir| super::procfs::basename(dir))
        .filter(|repo| *repo != name && git_dir.as_deref() != Some(root.as_path()));
    let (branch, worktree) = git_dir.as_deref().map(git_head).unwrap_or((None, false));
    Some(Project {
        root,
        name,
        repo,
        manifest,
        branch,
        worktree,
    })
}

fn quoted_value(line: &str, key: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &rest[1..];
    Some(value[..value.find(quote)?].to_string())
}

/// Reads the project name from a manifest without pulling in parsers for
/// every format. Only the common, unambiguous spellings are recognised.
fn manifest_name(root: &Path, file: &str) -> Option<String> {
    let raw = fs::read_to_string(root.join(file)).ok()?;
    let name = match file {
        "package.json" | "composer.json" | "deno.json" | "deno.jsonc" => {
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()?
                .get("name")?
                .as_str()?
                .to_string()
        }
        "Cargo.toml" | "pyproject.toml" => {
            let mut section = "";
            raw.lines().find_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    section = trimmed;
                    return None;
                }
                let wanted = matches!(section, "[package]" | "[project]" | "[tool.poetry]");
                if wanted {
                    quoted_value(trimmed, "name")
                } else {
                    None
                }
            })?
        }
        "go.mod" => raw
            .lines()
            .find_map(|line| line.trim().strip_prefix("module "))?
            .trim()
            .rsplit('/')
            .next()?
            .to_string(),
        _ => return None,
    };
    Some(name).filter(|name| !name.is_empty())
}

/// The checked-out branch (or short commit when detached) and whether the
/// directory is a linked worktree rather than the main checkout.
pub fn git_head(repo: &Path) -> (Option<String>, bool) {
    let dot_git = repo.join(".git");
    let (git_dir, worktree) = if dot_git.is_dir() {
        (dot_git, false)
    } else {
        let Some(target) = fs::read_to_string(&dot_git).ok().and_then(|raw| {
            raw.trim()
                .strip_prefix("gitdir:")
                .map(|t| t.trim().to_string())
        }) else {
            return (None, false);
        };
        let target = PathBuf::from(target);
        let target = if target.is_absolute() {
            target
        } else {
            repo.join(target)
        };
        // Submodules also use a .git file; only worktrees live under "worktrees".
        let worktree = target
            .components()
            .any(|part| part.as_os_str() == "worktrees");
        (target, worktree)
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    let branch = match head.strip_prefix("ref: ") {
        Some(reference) => Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        ),
        None if head.len() >= 7 => Some(head[..7].to_string()),
        None => None,
    };
    (branch, worktree)
}

/// Frameworks recognised from a server's command line, most specific first.
const COMMAND_FRAMEWORKS: &[(&str, &str)] = &[
    ("next-server", "Next.js"),
    ("next dev", "Next.js"),
    ("next start", "Next.js"),
    ("/next/dist", "Next.js"),
    ("nuxi", "Nuxt"),
    ("nuxt", "Nuxt"),
    ("astro", "Astro"),
    ("svelte-kit", "SvelteKit"),
    ("react-router", "React Router"),
    ("remix", "Remix"),
    ("vitepress", "VitePress"),
    ("storybook", "Storybook"),
    ("docusaurus", "Docusaurus"),
    ("gatsby", "Gatsby"),
    ("@angular", "Angular"),
    ("ng serve", "Angular"),
    ("expo", "Expo"),
    ("wrangler", "Wrangler"),
    ("vite", "Vite"),
    ("webpack", "webpack"),
    ("parcel", "Parcel"),
    ("rsbuild", "Rsbuild"),
    ("rspack", "Rspack"),
    ("turbopack", "Turbopack"),
    ("phx.server", "Phoenix"),
    ("manage.py runserver", "Django"),
    ("uvicorn", "Uvicorn"),
    ("fastapi", "FastAPI"),
    ("gunicorn", "Gunicorn"),
    ("flask", "Flask"),
    ("streamlit", "Streamlit"),
    ("jupyter", "Jupyter"),
    ("http.server", "http.server"),
    ("rails", "Rails"),
    ("puma", "Puma"),
    ("artisan serve", "Laravel"),
    ("php -S", "PHP"),
    ("hugo", "Hugo"),
    ("jekyll", "Jekyll"),
    ("zola", "Zola"),
    ("trunk serve", "Trunk"),
    ("dx serve", "Dioxus"),
    ("air", "Air"),
    ("postgres", "PostgreSQL"),
    ("redis-server", "Redis"),
    ("valkey-server", "Valkey"),
    ("mongod", "MongoDB"),
    ("mysqld", "MySQL"),
    ("mariadbd", "MariaDB"),
];

/// Dependencies that imply a framework when the command line is generic.
const DEPENDENCY_FRAMEWORKS: &[(&str, &str)] = &[
    ("next", "Next.js"),
    ("nuxt", "Nuxt"),
    ("astro", "Astro"),
    ("@sveltejs/kit", "SvelteKit"),
    ("@remix-run/dev", "Remix"),
    ("@react-router/dev", "React Router"),
    ("vitepress", "VitePress"),
    ("@angular/core", "Angular"),
    ("expo", "Expo"),
    ("vite", "Vite"),
    ("express", "Express"),
    ("fastify", "Fastify"),
    ("hono", "Hono"),
    ("@nestjs/core", "NestJS"),
    ("koa", "Koa"),
];

fn word_match(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(index, _)| {
        let before = haystack[..index].chars().next_back();
        let after = haystack[index + needle.len()..].chars().next();
        // Only alphanumeric needle edges need a word boundary next to them.
        let edge = |needle_edge: Option<char>, neighbour: Option<char>| {
            !needle_edge.is_some_and(|c| c.is_ascii_alphanumeric())
                || neighbour.is_none_or(|c| !c.is_ascii_alphanumeric())
        };
        edge(needle.chars().next(), before) && edge(needle.chars().next_back(), after)
    })
}

pub fn framework(command: &str, project: Option<&Project>) -> Option<String> {
    if let Some((_, name)) = COMMAND_FRAMEWORKS
        .iter()
        .find(|(needle, _)| word_match(command, needle))
    {
        return Some(name.to_string());
    }
    let project = project?;
    if project.manifest != Some("package.json") {
        return None;
    }
    let raw = fs::read_to_string(project.root.join("package.json")).ok()?;
    let package: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let has = |dep: &str| {
        ["dependencies", "devDependencies"]
            .iter()
            .any(|key| package.get(key).and_then(|deps| deps.get(dep)).is_some())
    };
    DEPENDENCY_FRAMEWORKS
        .iter()
        .find(|(dep, _)| has(dep))
        .map(|(_, name)| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oport-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_monorepo_app_inside_repository() {
        let home = scratch("monorepo");
        let repo = home.join("shop");
        let app = repo.join("apps/web");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/cart\n").unwrap();
        fs::write(
            app.join("package.json"),
            r#"{"name":"@shop/web","devDependencies":{"vite":"7"}}"#,
        )
        .unwrap();

        let project = detect(&app.join("src"), &home).unwrap();
        assert_eq!(project.root, app);
        assert_eq!(project.name, "@shop/web");
        assert_eq!(project.repo.as_deref(), Some("shop"));
        assert_eq!(project.branch.as_deref(), Some("feature/cart"));
        assert!(!project.worktree);
        assert_eq!(
            framework("node server.js", Some(&project)).as_deref(),
            Some("Vite")
        );
        assert_eq!(detect(&home, &home), None);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn recognises_linked_worktrees_and_detached_heads() {
        let home = scratch("worktree");
        let main = home.join("api/.git/worktrees/fix");
        fs::create_dir_all(&main).unwrap();
        fs::write(main.join("HEAD"), "0123456789abcdef\n").unwrap();
        let tree = home.join("api-fix");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join(".git"), format!("gitdir: {}\n", main.display())).unwrap();
        fs::write(
            tree.join("Cargo.toml"),
            "[workspace]\nname = \"no\"\n[package]\nname = \"api\"\n",
        )
        .unwrap();

        let project = detect(&tree, &home).unwrap();
        assert_eq!(
            (project.name.as_str(), project.repo.as_deref()),
            ("api", None)
        );
        assert_eq!(project.branch.as_deref(), Some("0123456"));
        assert!(project.worktree);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn matches_frameworks_on_word_boundaries() {
        assert_eq!(
            framework("node /x/node_modules/.bin/vite --port 5173", None).as_deref(),
            Some("Vite")
        );
        assert_eq!(
            framework("next-server (v15.2.0)", None).as_deref(),
            Some("Next.js")
        );
        assert_eq!(
            framework("node node_modules/next/dist/bin/next", None).as_deref(),
            Some("Next.js")
        );
        assert_eq!(
            framework("python manage.py runserver", None).as_deref(),
            Some("Django")
        );
        assert_eq!(framework("/usr/bin/repair --flag", None), None);
        assert_eq!(framework("node invite-server.js", None), None);
    }
}
