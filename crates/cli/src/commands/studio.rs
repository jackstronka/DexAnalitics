//! Streaming / studio helpers (MVP).
//!
//! This module intentionally stays **local-first**: it reads JSONL inputs and generates JSONL
//! "segment plans" that can later be spoken by TTS and rendered by OBS/Unreal/etc.

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioLang {
    Pl,
    En,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioSegment {
    /// RFC3339 UTC timestamp when the segment was generated.
    pub ts_utc: String,
    /// A stable ID if present in input (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Suggested silent pause before the narration (viewer reads headline).
    pub pause_secs: u64,
    /// Narration text (to be fed into TTS or read by a human).
    pub narrator_text: String,
    /// Free-form style label for later LLM/TTS routing (e.g. "neutral", "satirical").
    pub style: String,
    pub lang: StudioLang,
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    Ok(())
}

fn extract_string(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("…");
    out
}

fn build_narration(lang: StudioLang, style: &str, title: &str, excerpt: Option<&str>) -> String {
    let excerpt = excerpt
        .map(|e| truncate(e.trim(), 320))
        .filter(|e| !e.is_empty());

    match lang {
        StudioLang::Pl => {
            let mut s = String::new();
            s.push_str("Temat: ");
            s.push_str(title);
            s.push_str(".\n\n");
            if let Some(ex) = excerpt {
                s.push_str("Szybkie streszczenie: ");
                s.push_str(&ex);
                s.push_str("\n\n");
            }
            s.push_str("Komentarz (styl: ");
            s.push_str(style);
            s.push_str("):\n");
            s.push_str("- Co to znaczy i dlaczego to ważne?\n");
            s.push_str("- Jakie są ryzyka / ograniczenia informacji?\n");
            s.push_str("- Jaki jest “następny krok” dla obserwatora?\n");
            s
        }
        StudioLang::En => {
            let mut s = String::new();
            s.push_str("Topic: ");
            s.push_str(title);
            s.push_str(".\n\n");
            if let Some(ex) = excerpt {
                s.push_str("Quick summary: ");
                s.push_str(&ex);
                s.push_str("\n\n");
            }
            s.push_str("Commentary (style: ");
            s.push_str(style);
            s.push_str("):\n");
            s.push_str("- What does this mean and why does it matter?\n");
            s.push_str("- What are the risks / missing context?\n");
            s.push_str("- What is a reasonable next step for a viewer?\n");
            s
        }
    }
}

/// Read a local JSONL "news/events" file and output a JSONL stream plan.
///
/// Input JSONL: any object with at least `title` (or `headline`). Optional: `id`, `url`, `excerpt`.
pub fn run_studio_stream_plan(
    input_jsonl: PathBuf,
    output_jsonl: PathBuf,
    lang: StudioLang,
    style: String,
    pause_secs: u64,
    limit: usize,
) -> Result<()> {
    let content = fs::read_to_string(&input_jsonl)
        .with_context(|| format!("read input {}", input_jsonl.display()))?;

    let mut out_lines: Vec<String> = Vec::new();
    let mut count = 0usize;

    for (idx, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).with_context(|| {
            format!("parse jsonl line {} in {}", idx + 1, input_jsonl.display())
        })?;

        let Some(title) = extract_string(&v, &["title", "headline"]) else {
            continue;
        };
        let source_id = extract_string(&v, &["id", "source_id", "guid"]);
        let url = extract_string(&v, &["url", "link"]);
        let excerpt = extract_string(&v, &["excerpt", "summary", "description"]);

        let narrator_text = build_narration(lang, &style, &title, excerpt.as_deref());

        let seg = StudioSegment {
            ts_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            source_id,
            title,
            url,
            pause_secs,
            narrator_text,
            style: style.clone(),
            lang,
        };

        out_lines.push(serde_json::to_string(&seg)?);
        count += 1;
        if count >= limit {
            break;
        }
    }

    ensure_parent_dir(&output_jsonl)?;
    fs::write(&output_jsonl, out_lines.join("\n") + "\n")
        .with_context(|| format!("write output {}", output_jsonl.display()))?;

    println!(
        "✅ studio-stream-plan: wrote {} segments to {}",
        count,
        output_jsonl.display()
    );

    Ok(())
}
