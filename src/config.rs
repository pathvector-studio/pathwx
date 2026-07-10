//! 設定ファイル (TOML) の読み書き
//!
//! pathwx は監視対象 (target) のリストを TOML で持つ。各 target は
//! 名前・接続先ホスト・ポート・プローブ間隔・サンプル数・タイムアウトを持つ。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// `samples` に許される上限。直列プローブ (probe/mod.rs) 前提のため、
/// 大きすぎる値は `samples × timeout_ms` の合計で1ラウンドが現実的でない
/// 長さになる (例: 100000 × 1s ≈ 28時間) のを防ぐ。
pub const MAX_SAMPLES: u32 = 100;

/// 監視対象1件の設定
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Target {
    /// 表示・ログファイル名に使う識別子
    pub name: String,
    /// 接続先ホスト名 or IP
    pub host: String,
    /// 接続先 TCP ポート
    pub port: u16,
    /// `watch` サブコマンドでのプローブ間隔 (秒)
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    /// 1ラウンドあたりの TCP connect 試行回数 (上限は [`MAX_SAMPLES`]、
    /// `Config::validate` で検証される)
    #[serde(default = "default_samples")]
    pub samples: u32,
    /// 1回の接続試行のタイムアウト (ミリ秒)
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_interval_secs() -> u64 {
    60
}

fn default_samples() -> u32 {
    5
}

fn default_timeout_ms() -> u64 {
    1000
}

/// トップレベル設定
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    /// ログ (JSONL) の保存先ディレクトリ。省略時は `~/.local/share/pathwx`
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    /// ベースライン計算に使う直近ラウンド数の上限。省略時は無制限 (全履歴)
    #[serde(default)]
    pub baseline_window: Option<usize>,
    /// 劣化判定の z-score しきい値 (既定 2.0 = 約95%信頼区間)
    #[serde(default = "default_z_threshold")]
    pub z_threshold: f64,
    /// ベースライン計算に最低限必要なラウンド数 (これ未満は insufficient data)
    #[serde(default = "default_min_baseline_rounds")]
    pub min_baseline_rounds: usize,
    #[serde(default)]
    pub targets: Vec<Target>,
}

fn default_z_threshold() -> f64 {
    2.0
}

fn default_min_baseline_rounds() -> usize {
    5
}

impl Config {
    /// 設定ファイルを読み込む
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("設定ファイルを読み込めません: {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("設定ファイルのパースに失敗しました: {}", path.display()))?;
        cfg.validate()
            .with_context(|| format!("設定ファイルの内容が不正です: {}", path.display()))?;
        Ok(cfg)
    }

    /// TOML としてパース済みの設定内容を検証する (パース自体は成功していても
    /// 意味的に不正な組み合わせを弾く)
    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for t in &self.targets {
            if !seen.insert(t.name.as_str()) {
                bail!(
                    "ターゲット名 '{}' が重複しています。各ターゲットの `name` は一意である必要があります \
                     (同名だと同じログファイル <name>.jsonl に2つのホストのデータが混在してしまいます)",
                    t.name
                );
            }
            if t.samples == 0 {
                bail!(
                    "ターゲット '{}': `samples` は1以上である必要があります (指定値: {})",
                    t.name,
                    t.samples
                );
            }
            if t.samples > MAX_SAMPLES {
                bail!(
                    "ターゲット '{}': `samples` が上限 {} を超えています (指定値: {}). \
                     probe は直列実行のため、大きすぎる値は1ラウンドが非現実的に長くなります",
                    t.name,
                    MAX_SAMPLES,
                    t.samples
                );
            }
        }
        Ok(())
    }

    /// データディレクトリを解決する (設定 > デフォルト `~/.local/share/pathwx`)
    pub fn resolve_data_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = &self.data_dir {
            return Ok(dir.clone());
        }
        let base = dirs::data_dir().context("ホームディレクトリを解決できません")?;
        Ok(base.join("pathwx"))
    }

    /// 名前で target を探す
    pub fn find_target(&self, name: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.name == name)
    }

    /// サンプル設定ファイルの中身を生成する (`pathwx init` 用)
    pub fn example_toml() -> &'static str {
        r#"# pathwx 設定ファイル
# データ保存先 (省略時は ~/.local/share/pathwx)
# data_dir = "/var/lib/pathwx"

# ベースライン計算に使う直近ラウンド数 (省略時は全履歴を使用)
# baseline_window = 200

# 劣化判定の z-score しきい値 (既定 2.0)
z_threshold = 2.0

# ベースライン計算に最低限必要なラウンド数 (これ未満は "insufficient data")
min_baseline_rounds = 5

[[targets]]
name = "cloudflare-dns"
host = "1.1.1.1"
port = 443
interval_secs = 60
samples = 5
timeout_ms = 1000

[[targets]]
name = "local-echo"
host = "127.0.0.1"
port = 17017
interval_secs = 30
samples = 5
timeout_ms = 500
"#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let toml_str = r#"
            [[targets]]
            name = "t1"
            host = "127.0.0.1"
            port = 80
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.targets.len(), 1);
        let t = &cfg.targets[0];
        assert_eq!(t.name, "t1");
        assert_eq!(t.interval_secs, 60);
        assert_eq!(t.samples, 5);
        assert_eq!(t.timeout_ms, 1000);
        assert_eq!(cfg.z_threshold, 2.0);
        assert_eq!(cfg.min_baseline_rounds, 5);
    }

    #[test]
    fn parses_full_config() {
        let toml_str = r#"
            data_dir = "/tmp/pathwx-data"
            baseline_window = 100
            z_threshold = 3.0
            min_baseline_rounds = 10

            [[targets]]
            name = "t1"
            host = "example.com"
            port = 443
            interval_secs = 30
            samples = 10
            timeout_ms = 2000
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.data_dir, Some(PathBuf::from("/tmp/pathwx-data")));
        assert_eq!(cfg.baseline_window, Some(100));
        assert_eq!(cfg.z_threshold, 3.0);
        assert_eq!(cfg.min_baseline_rounds, 10);
        let t = cfg.find_target("t1").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.samples, 10);
    }

    #[test]
    fn example_toml_parses() {
        let cfg: Config = toml::from_str(Config::example_toml()).unwrap();
        assert_eq!(cfg.targets.len(), 2);
    }

    #[test]
    fn empty_config_has_no_targets_and_default_thresholds() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.targets.is_empty());
        assert_eq!(cfg.z_threshold, 2.0);
        assert_eq!(cfg.min_baseline_rounds, 5);
        assert_eq!(cfg.data_dir, None);
        assert_eq!(cfg.baseline_window, None);
    }

    #[test]
    fn missing_required_target_field_fails_to_parse() {
        // `port` has no default, so a target without it must fail parsing
        // rather than silently defaulting to 0.
        let toml_str = r#"
            [[targets]]
            name = "t1"
            host = "127.0.0.1"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn find_target_returns_none_for_unknown_name() {
        let cfg: Config = toml::from_str(Config::example_toml()).unwrap();
        assert!(cfg.find_target("does-not-exist").is_none());
    }

    #[test]
    fn load_missing_file_returns_error() {
        let result = Config::load(Path::new(
            "/nonexistent/pathwx-config-that-does-not-exist.toml",
        ));
        assert!(result.is_err());
    }

    #[test]
    fn load_reads_and_parses_file_from_disk() {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "pathwx-config-test-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, Config::example_toml()).unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.targets.len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolve_data_dir_prefers_configured_value() {
        let cfg = Config {
            data_dir: Some(PathBuf::from("/custom/data/dir")),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolve_data_dir().unwrap(),
            PathBuf::from("/custom/data/dir")
        );
    }

    #[test]
    fn resolve_data_dir_falls_back_to_default_when_unset() {
        let cfg = Config::default();
        let resolved = cfg.resolve_data_dir().unwrap();
        assert!(resolved.ends_with("pathwx"));
    }

    fn target(name: &str) -> Target {
        Target {
            name: name.to_string(),
            host: "127.0.0.1".to_string(),
            port: 80,
            interval_secs: 60,
            samples: 5,
            timeout_ms: 1000,
        }
    }

    #[test]
    fn validate_rejects_duplicate_target_names() {
        let cfg = Config {
            targets: vec![target("dup"), target("dup")],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("dup"),
            "error should name the duplicate target: {err}"
        );
    }

    #[test]
    fn validate_accepts_unique_target_names() {
        let cfg = Config {
            targets: vec![target("a"), target("b")],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn load_rejects_config_with_duplicate_target_names() {
        let toml_str = r#"
            [[targets]]
            name = "dup"
            host = "127.0.0.1"
            port = 80

            [[targets]]
            name = "dup"
            host = "127.0.0.2"
            port = 81
        "#;
        let mut path = std::env::temp_dir();
        path.push(format!(
            "pathwx-config-dup-test-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, toml_str).unwrap();
        let result = Config::load(&path);
        assert!(result.is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn validate_rejects_samples_over_max() {
        let mut t = target("t1");
        t.samples = MAX_SAMPLES + 1;
        let cfg = Config {
            targets: vec![t],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("samples"));
    }

    #[test]
    fn validate_accepts_samples_at_max() {
        let mut t = target("t1");
        t.samples = MAX_SAMPLES;
        let cfg = Config {
            targets: vec![t],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_samples() {
        let mut t = target("t1");
        t.samples = 0;
        let cfg = Config {
            targets: vec![t],
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn example_toml_passes_validation() {
        let cfg: Config = toml::from_str(Config::example_toml()).unwrap();
        assert!(cfg.validate().is_ok());
    }
}
