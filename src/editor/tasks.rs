/// Task Runner — resolve and expand run/build/test commands per language.
///
/// Priority: extension tasks → built-in defaults.
/// Commands can contain template variables: {file}, {dir}, {stem}, {workspace}.
use std::path::Path;

use crate::config::Config;
use crate::extension::Extension;

// ── TaskKind ─────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Run,
    Build,
    Test,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Run => "run",
            TaskKind::Build => "build",
            TaskKind::Test => "test",
        }
    }
}

// ── TaskRunner ────────────────────────────────────────────────

pub struct TaskRunner;

impl TaskRunner {
    /// Resolve the command template for a given language + task kind.
    ///
    /// Resolution order:
    ///   1. Installed extensions that declare this language
    ///   2. Built-in defaults table
    pub fn resolve(
        lang: &str,
        kind: TaskKind,
        extensions: &[Extension],
        _config: &Config,
    ) -> Option<String> {
        // 1. Extension tasks — first extension that covers the language wins.
        for ext in extensions {
            if ext.languages.iter().any(|l| l.name == lang) {
                if let Some((_, task)) =
                    ext.tasks.iter().find(|(n, _)| n == kind.as_str())
                {
                    return Some(task.cmd.clone());
                }
            }
        }

        // 2. Built-in defaults.
        builtin_task(lang, kind).map(|s| s.to_string())
    }

    /// Expand template variables in a command string.
    ///
    /// Variables: `{file}`, `{dir}`, `{stem}`, `{workspace}`
    pub fn expand(cmd: &str, file_path: &str, workspace: &str) -> String {
        let path = Path::new(file_path);
        let dir = path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        cmd.replace("{file}", file_path)
            .replace("{dir}", dir)
            .replace("{stem}", stem)
            .replace("{workspace}", workspace)
    }
}

// ── Built-in defaults ─────────────────────────────────────────

fn builtin_task(lang: &str, kind: TaskKind) -> Option<&'static str> {
    match (lang, kind) {
        // Rust
        ("rust", TaskKind::Run) => Some("cargo run"),
        ("rust", TaskKind::Build) => Some("cargo build"),
        ("rust", TaskKind::Test) => Some("cargo test"),
        // Python
        ("python", TaskKind::Run) => Some("python3 {file}"),
        ("python", TaskKind::Test) => Some("pytest {file}"),
        // JavaScript / TypeScript
        ("javascript", TaskKind::Run) => Some("node {file}"),
        ("typescript", TaskKind::Run) => Some("ts-node {file}"),
        // Z ecosystem
        ("zenith", TaskKind::Run) => Some("zenith {file}"),
        ("zymbol", TaskKind::Run) => Some("zymbol {file}"),
        // Shell
        ("shell", TaskKind::Run) => Some("bash {file}"),
        ("bash", TaskKind::Run) => Some("bash {file}"),
        // Go
        ("go", TaskKind::Run) => Some("go run {file}"),
        ("go", TaskKind::Test) => Some("go test ./..."),
        ("go", TaskKind::Build) => Some("go build"),
        // Java
        ("java", TaskKind::Build) => Some("javac {file}"),
        ("java", TaskKind::Run) => Some("java {stem}"),
        // C / C++
        ("c", TaskKind::Build) => Some("gcc -o {stem} {file}"),
        ("c", TaskKind::Run) => Some("./{stem}"),
        ("cpp", TaskKind::Build) => Some("g++ -o {stem} {file}"),
        ("cpp", TaskKind::Run) => Some("./{stem}"),
        // Ruby
        ("ruby", TaskKind::Run) => Some("ruby {file}"),
        // Lua
        ("lua", TaskKind::Run) => Some("lua {file}"),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_file() {
        assert_eq!(
            TaskRunner::expand("python3 {file}", "/home/user/script.py", "/home/user"),
            "python3 /home/user/script.py"
        );
    }

    #[test]
    fn test_expand_stem() {
        assert_eq!(
            TaskRunner::expand("java {stem}", "/home/user/Hello.java", "/workspace"),
            "java Hello"
        );
    }

    #[test]
    fn test_expand_dir() {
        assert_eq!(
            TaskRunner::expand("cd {dir} && make", "/home/user/proj/main.c", "/workspace"),
            "cd /home/user/proj && make"
        );
    }

    #[test]
    fn test_expand_workspace() {
        assert_eq!(
            TaskRunner::expand("cd {workspace} && cargo run", "/a/b/src/main.rs", "/a/b"),
            "cd /a/b && cargo run"
        );
    }

    #[test]
    fn test_expand_no_variables() {
        assert_eq!(
            TaskRunner::expand("cargo test", "/proj/src/lib.rs", "/proj"),
            "cargo test"
        );
    }

    #[test]
    fn test_builtin_rust_run() {
        let cmd = TaskRunner::resolve("rust", TaskKind::Run, &[], &Config::default());
        assert_eq!(cmd.as_deref(), Some("cargo run"));
    }

    #[test]
    fn test_builtin_rust_build() {
        let cmd = TaskRunner::resolve("rust", TaskKind::Build, &[], &Config::default());
        assert_eq!(cmd.as_deref(), Some("cargo build"));
    }

    #[test]
    fn test_builtin_rust_test() {
        let cmd = TaskRunner::resolve("rust", TaskKind::Test, &[], &Config::default());
        assert_eq!(cmd.as_deref(), Some("cargo test"));
    }

    #[test]
    fn test_builtin_python_run() {
        let cmd = TaskRunner::resolve("python", TaskKind::Run, &[], &Config::default());
        assert_eq!(cmd.as_deref(), Some("python3 {file}"));
    }

    #[test]
    fn test_builtin_zenith() {
        let cmd = TaskRunner::resolve("zenith", TaskKind::Run, &[], &Config::default());
        assert_eq!(cmd.as_deref(), Some("zenith {file}"));
    }

    #[test]
    fn test_builtin_unknown_returns_none() {
        let cmd = TaskRunner::resolve("cobol", TaskKind::Run, &[], &Config::default());
        assert!(cmd.is_none());
    }

    #[test]
    fn test_builtin_no_build_for_python() {
        let cmd = TaskRunner::resolve("python", TaskKind::Build, &[], &Config::default());
        assert!(cmd.is_none());
    }

    #[test]
    fn test_extension_task_overrides_builtin() {
        use crate::extension::{Extension, ExtLspConfig, TaskDef};
        use crate::config::LanguageDef;
        use std::path::PathBuf;

        let ext = Extension {
            id: "rust-custom".to_string(),
            name: "Rust Custom".to_string(),
            version: "1.0.0".to_string(),
            dir: PathBuf::from("/fake"),
            languages: vec![LanguageDef {
                name: "rust".to_string(),
                extensions: vec!["rs".to_string()],
                grammar_file: String::new(),
                comment: None,
            }],
            lsp: Some(ExtLspConfig {
                command: "rust-analyzer".to_string(),
                args: vec![],
            }),
            tasks: vec![(
                "run".to_string(),
                TaskDef {
                    cmd: "my-cargo-wrapper run".to_string(),
                    cwd: "{workspace}".to_string(),
                },
            )],
        };

        let cmd = TaskRunner::resolve("rust", TaskKind::Run, &[ext], &Config::default());
        assert_eq!(cmd.as_deref(), Some("my-cargo-wrapper run"));
    }

    #[test]
    fn test_extension_falls_through_to_builtin_for_missing_task() {
        use crate::extension::{Extension, TaskDef};
        use crate::config::LanguageDef;
        use std::path::PathBuf;

        // Extension covers "rust" but only has a "build" task, not "test"
        let ext = Extension {
            id: "rust-ext".to_string(),
            name: "Rust".to_string(),
            version: "1.0.0".to_string(),
            dir: PathBuf::from("/fake"),
            languages: vec![LanguageDef {
                name: "rust".to_string(),
                extensions: vec!["rs".to_string()],
                grammar_file: String::new(),
                comment: None,
            }],
            lsp: None,
            tasks: vec![(
                "build".to_string(),
                TaskDef {
                    cmd: "custom-build".to_string(),
                    cwd: "{workspace}".to_string(),
                },
            )],
        };

        // "test" not in extension → falls through to builtin "cargo test"
        let cmd = TaskRunner::resolve("rust", TaskKind::Test, &[ext], &Config::default());
        assert_eq!(cmd.as_deref(), Some("cargo test"));
    }

    #[test]
    fn test_kind_as_str() {
        assert_eq!(TaskKind::Run.as_str(), "run");
        assert_eq!(TaskKind::Build.as_str(), "build");
        assert_eq!(TaskKind::Test.as_str(), "test");
    }
}
