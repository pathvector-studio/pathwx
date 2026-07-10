//! 設定ファイル (TOML) の読み書き
//!
//! pathwx は監視対象 (target) のリストを TOML で持つ。各 target は
//! 名前・接続先ホスト・ポート・プローブ間隔・サンプル数・タイムアウトを持つ。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// 1ラウンドあたりの TCP connect 試行回数
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
        Ok(cfg)
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
}
