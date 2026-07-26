use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Config file looked up in the current directory when `--config` is not given.
pub const DEFAULT_CONFIG_FILE: &str = "bqvalid.toml";

/// User configuration deserialized from a TOML file.
///
/// Unknown keys are rejected so typos surface as errors rather than being
/// silently ignored.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Rule IDs whose diagnostics are suppressed.
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Failure while loading a config file: either the file could not be read or
/// its contents could not be parsed as the expected TOML.
#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read config file: {e}"),
            Self::Parse(e) => write!(f, "cannot parse config file: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(e) => Some(e),
            Self::Parse(e) => Some(e),
        }
    }
}

impl Config {
    /// Parse a config from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Read and parse the config file at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
        Self::from_toml(&text).map_err(ConfigError::Parse)
    }
}

/// Resolve which config file to load.
///
/// The explicit `--config` path always wins; otherwise the default file is
/// searched for in `start_dir` and each of its ancestors, using the nearest one
/// found. The search never goes above the git repository root (the directory
/// containing `.git`); outside any git repository only `start_dir` itself is
/// searched. Returns `None` when no config exists within that range.
pub fn discover_config(explicit: Option<PathBuf>, start_dir: &Path) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    // The search ceiling is the git repository root if there is one; otherwise
    // `start_dir` itself, so we never walk up out of an untracked directory.
    let ceiling = start_dir
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(start_dir);
    for dir in start_dir.ancestors() {
        let candidate = dir.join(DEFAULT_CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == ceiling {
            break;
        }
    }
    None
}

/// Resolve the effective ignore list. A non-empty CLI `--ignore` overrides the
/// config's `ignore` wholesale (replacement, not merge); otherwise the config
/// value is used.
pub fn effective_ignore(cli_ignore: Vec<String>, config_ignore: Vec<String>) -> Vec<String> {
    if cli_ignore.is_empty() {
        config_ignore
    } else {
        cli_ignore
    }
}

/// Return the ignore entries that match no known rule id, preserving input
/// order, so the caller can warn about likely typos instead of silently
/// ignoring them.
pub fn unknown_ignore_ids(ignore: &[String], known: &HashSet<&str>) -> Vec<String> {
    ignore
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn from_toml_parses_ignore_list() {
        let cfg = Config::from_toml("ignore = [\"use_current_date\", \"invalid_group_by\"]")
            .expect("valid toml");
        assert_eq!(
            cfg.ignore,
            vec![
                "use_current_date".to_string(),
                "invalid_group_by".to_string()
            ]
        );
    }

    #[test]
    fn from_toml_defaults_ignore_to_empty_when_absent() {
        let cfg = Config::from_toml("").expect("empty is valid");
        assert!(cfg.ignore.is_empty());
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn from_toml_rejects_unknown_keys() {
        // A typo'd key must be an error, not a silently ignored no-op.
        let err = Config::from_toml("ingore = [\"x\"]");
        assert!(err.is_err(), "unknown key should be rejected");
    }

    #[test]
    fn load_reads_and_parses_a_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bqvalid.toml");
        fs::write(&path, "ignore = [\"unnecessary_order_by\"]").unwrap();

        let cfg = Config::load(&path).expect("loads");
        assert_eq!(cfg.ignore, vec!["unnecessary_order_by".to_string()]);
    }

    #[test]
    fn load_missing_file_is_a_read_error() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        match Config::load(&missing) {
            Err(ConfigError::Read(_)) => {}
            other => panic!("expected a read error, got {other:?}"),
        }
    }

    #[test]
    fn load_malformed_toml_is_a_parse_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(&path, "ignore = not-a-list").unwrap();
        match Config::load(&path) {
            Err(ConfigError::Parse(_)) => {}
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn discover_prefers_explicit_path() {
        let dir = tempdir().unwrap();
        let explicit = PathBuf::from("/somewhere/custom.toml");
        assert_eq!(
            discover_config(Some(explicit.clone()), dir.path()),
            Some(explicit)
        );
    }

    #[test]
    fn discover_finds_default_file_in_cwd() {
        let dir = tempdir().unwrap();
        let default = dir.path().join(DEFAULT_CONFIG_FILE);
        fs::write(&default, "").unwrap();
        assert_eq!(discover_config(None, dir.path()), Some(default));
    }

    #[test]
    fn discover_returns_none_when_no_default_exists() {
        let dir = tempdir().unwrap();
        assert_eq!(discover_config(None, dir.path()), None);
    }

    #[test]
    fn discover_walks_up_to_the_git_root() {
        // The config lives at the git repository root; discovery from a nested
        // working directory must find it by walking upwards.
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let root_config = dir.path().join(DEFAULT_CONFIG_FILE);
        fs::write(&root_config, "").unwrap();
        let nested = dir.path().join("sub").join("dir");
        fs::create_dir_all(&nested).unwrap();

        assert_eq!(discover_config(None, &nested), Some(root_config));
    }

    #[test]
    fn discover_does_not_search_above_the_git_root() {
        // A config above the git root must not be picked up: discovery stops at
        // the repository boundary.
        let base = tempdir().unwrap();
        fs::write(base.path().join(DEFAULT_CONFIG_FILE), "").unwrap();
        let repo = base.path().join("repo");
        let nested = repo.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        assert_eq!(discover_config(None, &nested), None);
    }

    #[test]
    fn discover_finds_config_at_the_git_root() {
        let base = tempdir().unwrap();
        let repo = base.path().join("repo");
        let nested = repo.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        let repo_config = repo.join(DEFAULT_CONFIG_FILE);
        fs::write(&repo_config, "").unwrap();

        assert_eq!(discover_config(None, &nested), Some(repo_config));
    }

    #[test]
    fn discover_outside_a_git_repo_does_not_walk_up() {
        // With no git root to anchor to, only the start directory is searched.
        let base = tempdir().unwrap();
        fs::write(base.path().join(DEFAULT_CONFIG_FILE), "").unwrap();
        let nested = base.path().join("sub");
        fs::create_dir_all(&nested).unwrap();

        assert_eq!(discover_config(None, &nested), None);
    }

    #[test]
    fn discover_prefers_the_nearest_ancestor_config() {
        // With a config at both the root and a nested directory, the nearest one
        // (closest to the start directory) wins.
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(DEFAULT_CONFIG_FILE), "").unwrap();
        let nested = dir.path().join("sub");
        fs::create_dir_all(&nested).unwrap();
        let nested_config = nested.join(DEFAULT_CONFIG_FILE);
        fs::write(&nested_config, "").unwrap();

        assert_eq!(discover_config(None, &nested), Some(nested_config));
    }

    #[test]
    fn effective_ignore_prefers_cli_when_present() {
        let cli = vec!["a".to_string()];
        let config = vec!["b".to_string(), "c".to_string()];
        assert_eq!(
            effective_ignore(cli, config),
            vec!["a".to_string()],
            "CLI overrides config wholesale"
        );
    }

    #[test]
    fn effective_ignore_falls_back_to_config_when_cli_empty() {
        let config = vec!["b".to_string()];
        assert_eq!(
            effective_ignore(Vec::new(), config.clone()),
            config,
            "config is used when CLI is empty"
        );
    }

    #[test]
    fn unknown_ignore_ids_flags_only_unrecognised_entries() {
        let known: HashSet<&str> = ["invalid_group_by", "use_current_date"]
            .into_iter()
            .collect();
        let ignore = vec![
            "use_current_date".to_string(),
            "typo_rule".to_string(),
            "another_typo".to_string(),
        ];
        assert_eq!(
            unknown_ignore_ids(&ignore, &known),
            vec!["typo_rule".to_string(), "another_typo".to_string()],
            "known ids are kept, unknown ones are reported in order"
        );
    }
}
