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
}
