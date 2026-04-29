use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::config::ResolvedConfig;
use crate::persistence::StateBridge;
use crate::timequake::{ReplayInput, TimequakeCore};

mod config;
mod ocrys;
mod event;
mod profile;
mod aggregate_state;
mod snapshot;
mod app_state;
mod persistence;
mod pipeline;
mod fold;
mod fold_properties;
mod fold_adversarial;
mod timequake;
mod observation;
mod operational_policy;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Parse structured flags first, before AppState creation.
    let flags = Flags::parse(&args);

    // `show-config` runs before AppState — it only needs ResolvedConfig.
    if flags.command.as_deref() == Some("show-config") {
        let cwd = std::env::current_dir()?;
        let cfg = ResolvedConfig::resolve(
            flags.profile.as_deref(),
            flags.lang.as_deref(),
            flags.config_file.as_deref(),
            &cwd,
        )?;
        cfg.print_summary();
        return Ok(());
    }

    // Resolve config (CLI > ENV > FILE > DEFAULT).
    let cwd = std::env::current_dir()?;
    let cfg = ResolvedConfig::resolve(
        flags.profile.as_deref(),
        flags.lang.as_deref(),
        flags.config_file.as_deref(),
        &cwd,
    )?;

    eprintln!("[optimo] profile={} lang={} ({})",
        cfg.profile.value.kind,
        cfg.ocr_lang.value,
        cfg.profile.source,
    );

    // Bootstrap application state.
    let state = AppState::new(
        cfg.profile.value,
        cfg.ocr_lang.value,
    ).await?;

    if flags.command.as_deref() == Some("--replay") {
        run_replay(&state, &flags.positional)?;
        return Ok(());
    }

    if flags.positional.is_empty() {
        eprintln!("Usage: optimo [--profile <name>] [--lang <lang>] [--config <file>] <file1> ...");
        eprintln!("       optimo --replay [document_uuid]");
        eprintln!("       optimo show-config");
        eprintln!();
        eprintln!("Profiles:  tesseract (default)  carbo  legacy_import  strict");
        eprintln!("Env vars:  OPTIMO_PROFILE  OPTIMO_LANG");
        eprintln!("Config:    optimo.yml  (profile: tesseract / lang: ita)");
        return Ok(());
    }

    let docs: Vec<PathBuf> = flags.positional.into_iter().map(PathBuf::from).collect();
    pipeline::process_documents(&state, docs).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Flags — minimal CLI parser (no external dep)
// ---------------------------------------------------------------------------

/// Parsed CLI flags. No positional interpretation inside — that's main's job.
struct Flags {
    /// Value of `--profile`
    profile: Option<String>,
    /// Value of `--lang`
    lang: Option<String>,
    /// Value of `--config`
    config_file: Option<PathBuf>,
    /// First non-flag token that looks like a subcommand: "show-config", "--replay"
    command: Option<String>,
    /// Remaining positional args (file paths, optional document uuid for replay)
    positional: Vec<String>,
}

impl Flags {
    fn parse(args: &[String]) -> Self {
        let mut profile = None;
        let mut lang = None;
        let mut config_file = None;
        let mut command = None;
        let mut positional = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--profile" if i + 1 < args.len() => { profile = Some(args[i+1].clone()); i += 2; }
                "--lang"    if i + 1 < args.len() => { lang    = Some(args[i+1].clone()); i += 2; }
                "--config"  if i + 1 < args.len() => { config_file = Some(PathBuf::from(&args[i+1])); i += 2; }
                "show-config" | "--replay" => {
                    command = Some(args[i].clone());
                    positional.extend_from_slice(&args[i+1..]);
                    break;
                }
                other => { positional.push(other.to_string()); i += 1; }
            }
        }
        Self { profile, lang, config_file, command, positional }
    }
}

fn run_replay(state: &AppState, args: &[String]) -> Result<()> {
    let bridge = StateBridge::new(state);
    let document_id = args
        .first()
        .map(|raw| Uuid::parse_str(raw))
        .transpose()?;

    let checkpoint = bridge.load_latest_snapshot(document_id)?;
    let events = match &checkpoint {
        Some(snapshot) => bridge.load_events_after_snapshot(snapshot)?,
        None => bridge.load_events()?,
    };
    let replay_mode = if checkpoint.is_some() {
        "checkpoint+tail"
    } else {
        "genesis"
    };

    let engine = TimequakeCore::new();
    let output = engine.replay(ReplayInput { checkpoint, events })?;

    println!(
        "replay completed: mode={} applied_ocr_events={} skipped_events={} iterations={} confidence={:.4}",
        replay_mode,
        output.applied_ocr_events,
        output.skipped_events,
        output.state.iterations,
        output.state.global_confidence()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use crate::fold::reduce_documents;
    use crate::ocrys::run_ocr;
    use std::fs;
    use std::process::Command;

    #[test]
    fn pipeline_png_to_reducer_e2e() -> Result<()> {
        let root = std::env::current_dir().context("failed to resolve repo root")?;
        let fixture = root.join("fixtures/trf_title.png");

        if !fixture.exists() {
            eprintln!(
                "skipping pipeline_png_to_reducer_e2e: missing fixture at {:?}",
                fixture
            );
            return Ok(());
        }

        if !tesseract_available() {
            eprintln!("skipping pipeline_png_to_reducer_e2e: tesseract not available");
            return Ok(());
        }

        let tmp = tempfile::tempdir().context("failed to create temp dir")?;
        let run_dir = tmp.path();
        let lang = "ita";
        let variant = "pipeline_e2e";

        let artifact = run_ocr(&fixture, run_dir, lang, variant)
            .context("ocr step failed")?;

        let txt_path = run_dir.join("ocr_pipeline_e2e.txt");
        if !txt_path.exists() {
            anyhow::bail!("expected OCR text artifact not found at {:?}", txt_path);
        }

        let raw_text = fs::read_to_string(&txt_path)
            .with_context(|| format!("failed to read OCR text artifact at {:?}", txt_path))?;

        if raw_text.trim().is_empty() {
            anyhow::bail!("OCR produced empty text artifact at {:?}", txt_path);
        }

        let reduced = reduce_documents(vec![artifact])
            .context("reducer step failed")?;

        if reduced.source != fixture.to_string_lossy() {
            anyhow::bail!(
                "unexpected reducer source: expected {:?}, got {:?}",
                fixture,
                reduced.source
            );
        }

        Ok(())
    }

    fn tesseract_available() -> bool {
        Command::new("tesseract")
            .arg("--version")
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
