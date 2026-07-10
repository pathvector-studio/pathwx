//! pathwx — ネットワーク気象観測所
//! L3経路品質 (TCP connect ベース) を継続観測し、ベースラインからの乖離
//! (z-score) でゆるやかな劣化を検知するCLI。

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use pathwx::config::Config;
use pathwx::probe;
use pathwx::report::sparkline;
use pathwx::stats::{classify, Baseline, Health, RoundStats};
use pathwx::storage::{append_round, read_history, RoundRecord};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// ネットワーク気象観測所 — L3経路品質の継続観測とトレンド劣化検知
#[derive(Parser, Debug)]
#[command(name = "pathwx", version, about, arg_required_else_help = true)]
struct Args {
    /// 設定ファイルのパス
    #[arg(long, global = true, default_value = "pathwx.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// サンプル設定ファイルを生成する
    Init {
        /// 出力先パス (既定: pathwx.toml)
        #[arg(long, default_value = "pathwx.toml")]
        out: PathBuf,
        /// 既存ファイルを上書きする
        #[arg(long)]
        force: bool,
    },
    /// 全ターゲットに1ラウンドのプローブを実行し、結果をログに追記する
    Probe,
    /// 各ターゲットの interval_secs ごとにプローブし続ける (Ctrl-Cで終了)
    Watch {
        /// 最大ラウンド数 (省略時は無限ループ)
        #[arg(long)]
        rounds: Option<u64>,
    },
    /// 1ラウンド実行し、ベースラインと比較して健康/劣化を判定する
    Check {
        /// 対象ターゲット名 (省略時は全ターゲット)
        target: Option<String>,
    },
    /// 過去ログからベースライン統計とトレンドを表示する
    Report {
        /// 対象ターゲット名
        #[arg(long)]
        target: String,
        /// 表示する直近ラウンド数
        #[arg(long, default_value_t = 20)]
        window: usize,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<ExitCode> {
    match args.command {
        Command::Init { out, force } => cmd_init(&out, force),
        Command::Probe => cmd_probe(&args.config),
        Command::Watch { rounds } => cmd_watch(&args.config, rounds),
        Command::Check { target } => cmd_check(&args.config, target.as_deref()),
        Command::Report { target, window } => cmd_report(&args.config, &target, window),
    }
}

fn cmd_init(out: &PathBuf, force: bool) -> Result<ExitCode> {
    if out.exists() && !force {
        bail!(
            "{} は既に存在します。上書きするには --force を指定してください",
            out.display()
        );
    }
    std::fs::write(out, Config::example_toml())
        .with_context(|| format!("設定ファイルを書き込めません: {}", out.display()))?;
    println!("サンプル設定ファイルを書き出しました: {}", out.display());
    println!(
        "編集後、`pathwx probe --config {}` を実行してください",
        out.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// 1ターゲットに対して1ラウンド実行し、ログに追記して RoundRecord を返す
fn probe_and_record(cfg: &Config, target: &pathwx::config::Target) -> Result<RoundRecord> {
    let data_dir = cfg.resolve_data_dir()?;
    let stats: RoundStats = probe::run_round(target);
    let record = RoundRecord {
        timestamp: Utc::now(),
        target: target.name.clone(),
        host: target.host.clone(),
        port: target.port,
        stats,
    };
    append_round(&data_dir, &record)?;
    Ok(record)
}

fn cmd_probe(config_path: &Path) -> Result<ExitCode> {
    let cfg = Config::load(config_path)?;
    if cfg.targets.is_empty() {
        bail!("設定ファイルにターゲットがありません");
    }

    println!(
        "{:<20} {:>8} {:>8} {:>10} {:>10}",
        "TARGET", "SENT", "OK", "LOSS%", "AVG(ms)"
    );
    for target in &cfg.targets {
        let record = probe_and_record(&cfg, target)?;
        println!(
            "{:<20} {:>8} {:>8} {:>9.0}% {:>10}",
            target.name,
            record.stats.attempts,
            record.stats.successes,
            record.stats.loss_rate * 100.0,
            record
                .stats
                .mean_latency_ms
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_watch(config_path: &Path, max_rounds: Option<u64>) -> Result<ExitCode> {
    let cfg = Config::load(config_path)?;
    if cfg.targets.is_empty() {
        bail!("設定ファイルにターゲットがありません");
    }

    println!("pathwx watch: Ctrl-C で終了します");
    let mut round: u64 = 0;
    loop {
        round += 1;
        let now = Utc::now();
        println!("--- round {round} ({now}) ---");
        for target in &cfg.targets {
            let record = probe_and_record(&cfg, target)?;
            println!(
                "  {:<20} loss={:>5.1}% avg={}",
                target.name,
                record.stats.loss_rate * 100.0,
                record
                    .stats
                    .mean_latency_ms
                    .map(|v| format!("{v:.1}ms"))
                    .unwrap_or_else(|| "-".to_string())
            );
        }

        if let Some(max) = max_rounds {
            if round >= max {
                break;
            }
        }

        // シンプルに全ターゲットの最小 interval だけ待つ (per-target scheduling
        // ではなく1つのループで揃えて回す、MVPとしての単純化)
        let min_interval = cfg
            .targets
            .iter()
            .map(|t| t.interval_secs)
            .min()
            .unwrap_or(60);
        std::thread::sleep(Duration::from_secs(min_interval));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_check(config_path: &Path, target_name: Option<&str>) -> Result<ExitCode> {
    let cfg = Config::load(config_path)?;
    let data_dir = cfg.resolve_data_dir()?;

    let targets: Vec<&pathwx::config::Target> = match target_name {
        Some(name) => {
            let t = cfg
                .find_target(name)
                .with_context(|| format!("ターゲット '{name}' が設定に見つかりません"))?;
            vec![t]
        }
        None => cfg.targets.iter().collect(),
    };
    if targets.is_empty() {
        bail!("設定ファイルにターゲットがありません");
    }

    let mut worst_code = 0u8;
    for target in &targets {
        // 判定前の履歴を読む (直近ラウンドを含めないため、プローブ前に読む)
        let history_before = read_history(&data_dir, &target.name)?;
        let history_stats: Vec<RoundStats> =
            history_before.iter().map(|r| r.stats.clone()).collect();

        let record = probe_and_record(&cfg, target)?;
        let verdict = classify(
            &record.stats,
            &history_stats,
            cfg.z_threshold,
            cfg.min_baseline_rounds,
        );

        let (label, code) = match verdict.health {
            Health::Healthy => ("HEALTHY", 0u8),
            Health::InsufficientData => ("INSUFFICIENT DATA", 0u8),
            Health::Degraded => ("DEGRADED", 1u8),
            Health::Unreachable => ("UNREACHABLE", 2u8),
        };
        worst_code = worst_code.max(code);

        println!(
            "{:<20} [{}] loss={:.0}% avg={} — {}",
            target.name,
            label,
            record.stats.loss_rate * 100.0,
            record
                .stats
                .mean_latency_ms
                .map(|v| format!("{v:.1}ms"))
                .unwrap_or_else(|| "-".to_string()),
            verdict.reason
        );
    }

    Ok(ExitCode::from(worst_code))
}

fn cmd_report(config_path: &Path, target_name: &str, window: usize) -> Result<ExitCode> {
    let cfg = Config::load(config_path)?;
    let data_dir = cfg.resolve_data_dir()?;

    let history = read_history(&data_dir, target_name)?;
    if history.is_empty() {
        println!("ターゲット '{target_name}' の履歴がまだありません。`pathwx probe` を実行してください。");
        return Ok(ExitCode::SUCCESS);
    }

    let history_stats: Vec<RoundStats> = history.iter().map(|r| r.stats.clone()).collect();
    let baseline = Baseline::from_history(&history_stats);

    println!("=== pathwx report: {target_name} ===");
    println!("総ラウンド数: {}", history.len());

    match &baseline {
        Some(b) => {
            println!(
                "ベースライン (全{}ラウンド): 平均レイテンシ={:.2}ms 標準偏差={:.2}ms 平均損失率={:.1}%",
                b.sample_rounds,
                b.mean_latency_ms,
                b.stddev_latency_ms,
                b.mean_loss_rate * 100.0
            );
        }
        None => println!("ベースライン: 有効なレイテンシ履歴がありません"),
    }

    let recent: Vec<&RoundRecord> = history.iter().rev().take(window).rev().collect();
    let recent_latencies: Vec<f64> = recent
        .iter()
        .filter_map(|r| r.stats.mean_latency_ms)
        .collect();

    println!(
        "直近{}ラウンドのレイテンシ推移: {}",
        recent.len(),
        sparkline(&recent_latencies)
    );
    if let (Some(first), Some(last)) = (recent_latencies.first(), recent_latencies.last()) {
        println!("  先頭={first:.1}ms 末尾={last:.1}ms");
    }

    // 直近ラウンドの判定 (残りの履歴をベースラインとして使う)
    if let Some((last, rest)) = history_stats.split_last() {
        let verdict = classify(last, rest, cfg.z_threshold, cfg.min_baseline_rounds);
        println!(
            "直近ラウンドの判定: {:?} — {}",
            verdict.health, verdict.reason
        );
        if let Some(z) = verdict.latency_z {
            println!("  レイテンシ z-score: {z:.2}");
        }
    }

    Ok(ExitCode::SUCCESS)
}
