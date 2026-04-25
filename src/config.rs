/// Runtime configuration resolution.
///
/// Precedence (highest → lowest):
///   1. CLI flags     (`--profile`, `--lang`)
///   2. ENV vars      (`OPTIMO_PROFILE`, `OPTIMO_LANG`)
///   3. Config file   (`optimo.yml` in cwd or path from `--config`)
///   4. Defaults      (hardcoded safe values)
///
/// Config is resolved ONCE at startup. No lookup happens at runtime.
/// See `ResolvedConfig::resolve()`.
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{IngestionProfile, ProfileKind};

// ---------------------------------------------------------------------------
// ConfigSource — where did this value come from?
// ---------------------------------------------------------------------------

/// Tracks the origin of each resolved config value.
/// Printed by `show-config` so the operator knows exactly what is active and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    Cli,
    Env(String),           // env var name
    File(PathBuf),         // path to the config file
    Default,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli => write!(f, "cli"),
            Self::Env(var) => write!(f, "env:{}", var),
            Self::File(p) => write!(f, "file:{}", p.display()),
            Self::Default => write!(f, "default"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigFile — the optimo.yml schema
// ---------------------------------------------------------------------------

/// Schema for `optimo.yml`.
///
/// Intentionally minimal: only preset names, no individual field overrides.
/// Individual flags live in code (factory methods on IngestionProfile).
/// This avoids 50-option sprawl.
///
/// Example:
/// ```yaml
/// profile: tesseract
/// lang: ita
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigFile {
    /// Profile preset name. Valid: tesseract, carbo, legacy_import, strict.
    pub profile: Option<String>,
    /// OCR language passed to tesseract. Default: "ita".
    pub lang: Option<String>,
}

impl ConfigFile {
    /// Load and parse from a YAML file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {:?}", path))?;
        serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse config file {:?}", path))
    }

    /// Search for `optimo.yml` in `cwd`, returning `None` if not found.
    pub fn find_default(cwd: &Path) -> Option<PathBuf> {
        let candidate = cwd.join("optimo.yml");
        if candidate.exists() { Some(candidate) } else { None }
    }
}

// ---------------------------------------------------------------------------
// ResolvedValue<T> — a value + its source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolvedValue<T> {
    pub value: T,
    pub source: ConfigSource,
}

// ---------------------------------------------------------------------------
// ResolvedConfig — the single source of truth for runtime config
// ---------------------------------------------------------------------------

/// Fully resolved runtime configuration.
///
/// Each field carries its value AND where it came from.
/// Produced once by `resolve()` at startup. Immutable afterwards.
#[derive(Debug)]
pub struct ResolvedConfig {
    pub profile: ResolvedValue<IngestionProfile>,
    pub ocr_lang: ResolvedValue<String>,
    /// Path to config file that was loaded, if any.
    pub config_file: Option<PathBuf>,
}

impl ResolvedConfig {
    /// Resolve config following the precedence chain: CLI > ENV > FILE > DEFAULT.
    ///
    /// # Arguments
    /// - `cli_profile`  — value of `--profile` flag, if provided
    /// - `cli_lang`     — value of `--lang` flag, if provided
    /// - `cli_config`   — value of `--config` flag (explicit config file path), if provided
    /// - `cwd`          — working directory to search for `optimo.yml`
    pub fn resolve(
        cli_profile: Option<&str>,
        cli_lang: Option<&str>,
        cli_config: Option<&Path>,
        cwd: &Path,
    ) -> Result<Self> {
        // -- Config file (loaded once, used as fallback layer) --
        let config_file_path = cli_config
            .map(|p| p.to_path_buf())
            .or_else(|| ConfigFile::find_default(cwd));

        let file_config = config_file_path
            .as_deref()
            .map(ConfigFile::load)
            .transpose()?
            .unwrap_or_default();

        // -- profile --
        let profile = resolve_profile(cli_profile, &file_config, config_file_path.as_deref())?;

        // -- ocr_lang --
        let ocr_lang = resolve_string(
            cli_lang,
            "OPTIMO_LANG",
            file_config.lang.as_deref(),
            "ita",
            config_file_path.as_deref(),
        );

        Ok(Self {
            profile,
            ocr_lang,
            config_file: config_file_path,
        })
    }

    /// Print a human-readable summary of the resolved configuration.
    /// Used by the `show-config` command.
    pub fn print_summary(&self) {
        println!("Optimo resolved configuration");
        println!("{}", "─".repeat(65));

        if let Some(path) = &self.config_file {
            println!("  {:<30} {}", "config file", path.display());
        } else {
            println!("  {:<30} (none found)", "config file");
        }

        println!();
        println!("  {:<30} {:<20} [{}]",
            "profile",
            self.profile.value.kind.to_string(),
            self.profile.source,
        );
        println!("  {:<30} {:<20} [{}]",
            "lang",
            self.ocr_lang.value,
            self.ocr_lang.source,
        );
        println!();
        println!("Active profile fields (kind: {}):", self.profile.value.kind);
        println!("{}", "─".repeat(65));
        let p = &self.profile.value;
        println!("  {:<30} {}", "allow_duplicate_positions", p.allow_duplicate_positions);
        println!("  {:<30} {}", "normalize_whitespace",      p.normalize_whitespace);
        println!("  {:<30} {}", "normalize_case",            p.normalize_case);
        println!("  {:<30} {}", "unicode_normalize",         p.unicode_normalize);
        println!("  {:<30} {:.2}", "min_confidence",         p.min_confidence);
        println!("{}", "─".repeat(65));
        println!("Precedence: CLI > ENV > FILE > DEFAULT");
    }
}

// ---------------------------------------------------------------------------
// Internal resolvers
// ---------------------------------------------------------------------------

fn resolve_profile(
    cli: Option<&str>,
    file: &ConfigFile,
    file_path: Option<&Path>,
) -> Result<ResolvedValue<IngestionProfile>> {
    // 1. CLI
    if let Some(name) = cli {
        let kind = ProfileKind::from_str(name)
            .with_context(|| format!("unknown profile {:?} (valid: tesseract, carbo, legacy_import, strict)", name))?;
        return Ok(ResolvedValue {
            value: IngestionProfile::for_kind(kind),
            source: ConfigSource::Cli,
        });
    }

    // 2. ENV
    if let Ok(name) = std::env::var("OPTIMO_PROFILE") {
        let kind = ProfileKind::from_str(&name)
            .with_context(|| format!("unknown profile in OPTIMO_PROFILE={:?}", name))?;
        return Ok(ResolvedValue {
            value: IngestionProfile::for_kind(kind),
            source: ConfigSource::Env("OPTIMO_PROFILE".to_string()),
        });
    }

    // 3. FILE
    if let Some(name) = &file.profile {
        let kind = ProfileKind::from_str(name)
            .with_context(|| format!("unknown profile {:?} in config file", name))?;
        return Ok(ResolvedValue {
            value: IngestionProfile::for_kind(kind),
            source: ConfigSource::File(file_path.unwrap().to_path_buf()),
        });
    }

    // 4. DEFAULT
    Ok(ResolvedValue {
        value: IngestionProfile::tesseract(),
        source: ConfigSource::Default,
    })
}

fn resolve_string(
    cli: Option<&str>,
    env_var: &str,
    file_value: Option<&str>,
    default: &str,
    file_path: Option<&Path>,
) -> ResolvedValue<String> {
    if let Some(v) = cli {
        return ResolvedValue { value: v.to_string(), source: ConfigSource::Cli };
    }
    if let Ok(v) = std::env::var(env_var) {
        return ResolvedValue { value: v, source: ConfigSource::Env(env_var.to_string()) };
    }
    if let Some(v) = file_value {
        return ResolvedValue {
            value: v.to_string(),
            source: ConfigSource::File(file_path.unwrap().to_path_buf()),
        };
    }
    ResolvedValue { value: default.to_string(), source: ConfigSource::Default }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn resolve(cli_profile: Option<&str>) -> ResolvedConfig {
        let dir = tempdir().unwrap();
        ResolvedConfig::resolve(cli_profile, None, None, dir.path()).unwrap()
    }

    #[test]
    fn default_resolves_to_tesseract() {
        let cfg = resolve(None);
        assert_eq!(cfg.profile.value.kind, ProfileKind::Tesseract);
        assert_eq!(cfg.profile.source, ConfigSource::Default);
    }

    #[test]
    fn cli_overrides_default() {
        let cfg = resolve(Some("strict"));
        assert_eq!(cfg.profile.value.kind, ProfileKind::Strict);
        assert_eq!(cfg.profile.source, ConfigSource::Cli);
    }

    #[test]
    fn unknown_cli_profile_returns_error() {
        let dir = tempdir().unwrap();
        let result = ResolvedConfig::resolve(Some("zzz"), None, None, dir.path());
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("unknown profile"));
    }

    #[test]
    fn config_file_sets_profile() {
        let dir = tempdir().unwrap();
        let yml = dir.path().join("optimo.yml");
        std::fs::write(&yml, "profile: carbo\n").unwrap();

        let cfg = ResolvedConfig::resolve(None, None, None, dir.path()).unwrap();
        assert_eq!(cfg.profile.value.kind, ProfileKind::Carbo);
        match &cfg.profile.source {
            ConfigSource::File(p) => assert_eq!(p, &yml),
            other => panic!("expected File source, got {:?}", other),
        }
        assert_eq!(cfg.config_file, Some(yml));
    }

    #[test]
    fn cli_overrides_config_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("optimo.yml"), "profile: carbo\n").unwrap();

        let cfg = ResolvedConfig::resolve(Some("strict"), None, None, dir.path()).unwrap();
        assert_eq!(cfg.profile.value.kind, ProfileKind::Strict);
        assert_eq!(cfg.profile.source, ConfigSource::Cli);
    }

    #[test]
    fn config_file_sets_lang() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("optimo.yml"), "lang: eng\n").unwrap();

        let cfg = ResolvedConfig::resolve(None, None, None, dir.path()).unwrap();
        assert_eq!(cfg.ocr_lang.value, "eng");
        assert!(matches!(cfg.ocr_lang.source, ConfigSource::File(_)));
    }

    #[test]
    fn default_lang_is_ita() {
        let cfg = resolve(None);
        assert_eq!(cfg.ocr_lang.value, "ita");
        assert_eq!(cfg.ocr_lang.source, ConfigSource::Default);
    }

    #[test]
    fn explicit_config_path_overrides_default_search() {
        let dir = tempdir().unwrap();
        // No optimo.yml in dir, but an explicit file elsewhere
        let other_dir = tempdir().unwrap();
        let explicit = other_dir.path().join("custom.yml");
        std::fs::write(&explicit, "profile: legacy_import\n").unwrap();

        let cfg = ResolvedConfig::resolve(None, None, Some(&explicit), dir.path()).unwrap();
        assert_eq!(cfg.profile.value.kind, ProfileKind::LegacyImport);
    }
}
