//! JSONL ベースのプレーンテキストストレージ
//!
//! DB を使わず、1ラウンド = 1行の JSON として `<data_dir>/<target>.jsonl` に
//! 追記していく。git にコミットできるし、`tail -f` や `jq` でも読める。

use crate::stats::RoundStats;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// ログに1行として保存されるラウンドの記録
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundRecord {
    pub timestamp: DateTime<Utc>,
    pub target: String,
    pub host: String,
    pub port: u16,
    #[serde(flatten)]
    pub stats: RoundStats,
}

/// target名からログファイルパスを決める
pub fn log_path(data_dir: &Path, target_name: &str) -> PathBuf {
    data_dir.join(format!("{target_name}.jsonl"))
}

/// 1ラウンドの記録をログファイルに追記する (ファイル/ディレクトリが無ければ作成)
pub fn append_round(data_dir: &Path, record: &RoundRecord) -> Result<()> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("データディレクトリを作成できません: {}", data_dir.display()))?;
    let path = log_path(data_dir, &record.target);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))?;
    let line = serde_json::to_string(record).context("レコードのシリアライズに失敗しました")?;
    writeln!(file, "{line}")
        .with_context(|| format!("ログファイルへの書き込みに失敗しました: {}", path.display()))?;
    Ok(())
}

/// target のログ全件を読み込む (時系列順、ファイルが無ければ空 Vec)。
///
/// クラッシュ中の追記などで一部の行が壊れている (JSONとしてパースできない)
/// 場合でも、その行だけを警告付きでスキップして残りは読み込む。ファイル
/// 自体が開けない/読み込めない (I/Oエラー) 場合はエラーを返す。
pub fn read_history(data_dir: &Path, target_name: &str) -> Result<Vec<RoundRecord>> {
    let path = log_path(data_dir, target_name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut skipped = 0u32;
    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("{}行目の読み込みに失敗しました", i + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RoundRecord>(&line) {
            Ok(record) => records.push(record),
            Err(e) => {
                skipped += 1;
                eprintln!(
                    "警告: {} の{}行目をスキップしました (JSONパースに失敗: {e})",
                    path.display(),
                    i + 1
                );
            }
        }
    }
    if skipped > 0 {
        eprintln!(
            "警告: {} で破損した行を{}件スキップしました (残り{}件を使用)",
            path.display(),
            skipped,
            records.len()
        );
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::RoundStats;
    use tempfile_shim::TempDir;

    // Minimal self-contained tempdir helper to avoid pulling in the `tempfile`
    // crate as a dependency just for tests.
    mod tempfile_shim {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                let mut dir = std::env::temp_dir();
                let unique = format!(
                    "pathwx-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                dir.push(unique);
                std::fs::create_dir_all(&dir).unwrap();
                TempDir(dir)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn sample_record(target: &str) -> RoundRecord {
        RoundRecord {
            timestamp: Utc::now(),
            target: target.to_string(),
            host: "127.0.0.1".to_string(),
            port: 1234,
            stats: RoundStats {
                attempts: 5,
                successes: 5,
                loss_rate: 0.0,
                mean_latency_ms: Some(12.3),
                jitter_ms: Some(0.5),
            },
        }
    }

    #[test]
    fn roundtrip_write_then_read() {
        let dir = TempDir::new();
        let record = sample_record("mytarget");
        append_round(dir.path(), &record).unwrap();
        let history = read_history(dir.path(), "mytarget").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0], record);
    }

    #[test]
    fn append_multiple_rounds_preserves_order() {
        let dir = TempDir::new();
        for i in 0..5 {
            let mut r = sample_record("t");
            r.stats.mean_latency_ms = Some(10.0 + i as f64);
            append_round(dir.path(), &r).unwrap();
        }
        let history = read_history(dir.path(), "t").unwrap();
        assert_eq!(history.len(), 5);
        for (i, r) in history.iter().enumerate() {
            assert_eq!(r.stats.mean_latency_ms, Some(10.0 + i as f64));
        }
    }

    #[test]
    fn read_history_missing_file_returns_empty() {
        let dir = TempDir::new();
        let history = read_history(dir.path(), "nonexistent").unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn record_serializes_stats_flattened_not_nested() {
        // `stats` is `#[serde(flatten)]`, so the JSON line must have
        // "attempts"/"successes"/... at the top level, not nested under a
        // "stats" key. This is important since the README documents the
        // flat JSONL schema as a stable, `jq`-friendly format.
        let record = sample_record("flat-check");
        let json = serde_json::to_string(&record).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("stats").is_none());
        assert_eq!(value.get("attempts").and_then(|v| v.as_u64()), Some(5));
        assert_eq!(
            value.get("target").and_then(|v| v.as_str()),
            Some("flat-check")
        );
        assert!((value["mean_latency_ms"].as_f64().unwrap() - 12.3).abs() < 1e-9);
    }

    #[test]
    fn log_path_uses_target_name_as_filename() {
        let dir = Path::new("/tmp/somewhere");
        let path = log_path(dir, "cloudflare-dns");
        assert_eq!(path, dir.join("cloudflare-dns.jsonl"));
    }

    #[test]
    fn read_history_skips_blank_lines() {
        let dir = TempDir::new();
        let record = sample_record("blanks");
        append_round(dir.path(), &record).unwrap();
        // Manually append a blank line to the log file, then one more record.
        let path = log_path(dir.path(), "blanks");
        {
            use std::io::Write as _;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f).unwrap();
            let line = serde_json::to_string(&record).unwrap();
            writeln!(f, "{line}").unwrap();
        }
        let history = read_history(dir.path(), "blanks").unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn read_history_skips_single_malformed_json_line() {
        // A single corrupted/truncated line (e.g. left behind by a crash
        // mid-append) must not block reading the rest of the file — it
        // should be skipped with a warning instead of erroring out the
        // whole `check`/`report` command.
        let dir = TempDir::new();
        let path = log_path(dir.path(), "broken");
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(&path, "{not valid json\n").unwrap();
        let result = read_history(dir.path(), "broken");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn read_history_skips_corrupted_line_but_keeps_valid_ones() {
        let dir = TempDir::new();
        let path = log_path(dir.path(), "mixed");
        let good1 = sample_record("mixed");
        let mut good2 = sample_record("mixed");
        good2.stats.mean_latency_ms = Some(99.9);

        let content = format!(
            "{}\n{{this is not json}}\n{}\n",
            serde_json::to_string(&good1).unwrap(),
            serde_json::to_string(&good2).unwrap()
        );
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(&path, content).unwrap();

        let history = read_history(dir.path(), "mixed").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], good1);
        assert_eq!(history[1], good2);
    }

    #[test]
    fn read_history_errors_when_file_is_fully_unreadable() {
        // A directory in place of the expected log file can't be opened as
        // a file at all — that's a genuine I/O error and must still
        // propagate as Err rather than being silently swallowed.
        let dir = TempDir::new();
        let path = log_path(dir.path(), "isdir");
        fs::create_dir_all(&path).unwrap();
        let result = read_history(dir.path(), "isdir");
        assert!(result.is_err());
    }
}
