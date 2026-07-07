//! 統計計算: 平均・標準偏差・z-score・損失率、および健康/劣化判定
//!
//! naive なアップタイム監視は「固定しきい値」(例: 100ms超えたらアラート) で
//! 判定するが、それでは回線ごとの正常な変動幅を考慮できず、ノイズが多いネット
//! ワークでは常時誤報になり、逆に鈍いネットワークでは有意な劣化を見逃す。
//! pathwx はヒストリカルなベースライン (平均+標準偏差) からの乖離度 (z-score)
//! で判定することで、そのターゲット固有の「普段の変動幅」を基準にする。

use serde::{Deserialize, Serialize};

/// 平均を計算する (空なら None)
pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// 標本標準偏差 (n-1) を計算する。値が2個未満なら None
pub fn stddev(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let m = mean(values)?;
    let variance =
        values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (values.len() as f64 - 1.0);
    Some(variance.sqrt())
}

/// z-score を計算する。stddev が 0 または None の場合は None を返す
/// (定数値の履歴からは乖離度を定義できないため)
pub fn z_score(value: f64, baseline_mean: f64, baseline_stddev: f64) -> Option<f64> {
    if baseline_stddev <= f64::EPSILON {
        return None;
    }
    Some((value - baseline_mean) / baseline_stddev)
}

/// 損失率 (0.0〜1.0)
pub fn loss_rate(attempts: u32, successes: u32) -> f64 {
    if attempts == 0 {
        return 0.0;
    }
    (attempts - successes) as f64 / attempts as f64
}

/// 1ラウンド分のプローブ結果から統計サマリを作る
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundStats {
    pub attempts: u32,
    pub successes: u32,
    pub loss_rate: f64,
    /// 成功した接続の平均レイテンシ (ms)。成功が0件なら None
    pub mean_latency_ms: Option<f64>,
    /// 成功した接続レイテンシの標準偏差 (ms、jitter)。成功が2件未満なら None
    pub jitter_ms: Option<f64>,
}

impl RoundStats {
    /// 個々の接続試行結果 (成功時は Some(latency_ms)、失敗時は None) から集計する
    pub fn from_samples(samples: &[Option<f64>]) -> RoundStats {
        let attempts = samples.len() as u32;
        let successes_vec: Vec<f64> = samples.iter().filter_map(|s| *s).collect();
        let successes = successes_vec.len() as u32;
        RoundStats {
            attempts,
            successes,
            loss_rate: loss_rate(attempts, successes),
            mean_latency_ms: mean(&successes_vec),
            jitter_ms: stddev(&successes_vec),
        }
    }
}

/// ベースライン (過去ラウンド群から計算した「普段の状態」)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Baseline {
    pub sample_rounds: usize,
    pub mean_latency_ms: f64,
    pub stddev_latency_ms: f64,
    pub mean_loss_rate: f64,
}

impl Baseline {
    /// 過去ラウンドの RoundStats 列からベースラインを計算する。
    /// レイテンシが1件も無ければ None (全滅ラウンドのみの場合など)
    pub fn from_history(history: &[RoundStats]) -> Option<Baseline> {
        if history.is_empty() {
            return None;
        }
        let latencies: Vec<f64> = history.iter().filter_map(|r| r.mean_latency_ms).collect();
        if latencies.is_empty() {
            return None;
        }
        let mean_latency_ms = mean(&latencies)?;
        let stddev_latency_ms = stddev(&latencies).unwrap_or(0.0);
        let mean_loss_rate =
            history.iter().map(|r| r.loss_rate).sum::<f64>() / history.len() as f64;
        Some(Baseline {
            sample_rounds: history.len(),
            mean_latency_ms,
            stddev_latency_ms,
            mean_loss_rate,
        })
    }
}

/// 判定結果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    /// 履歴が min_baseline_rounds 未満でベースラインが確立していない
    InsufficientData,
    /// ベースラインと比較して正常範囲
    Healthy,
    /// ベースラインから有意に乖離 (z-score超過 or 損失率急増)
    Degraded,
    /// 直近ラウンドが全滅 (到達不能)
    Unreachable,
}

/// 判定の詳細 (理由と数値的根拠)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub health: Health,
    pub latency_z: Option<f64>,
    pub reason: String,
}

/// 直近ラウンドをベースラインと比較して健康状態を判定する。
///
/// - `history` が `min_baseline_rounds` 未満 → InsufficientData
/// - 直近ラウンドの成功が0件 (全滅) → Unreachable
/// - レイテンシの z-score が `z_threshold` を超える、または損失率が
///   ベースラインの損失率+しきい値相当を大きく上回る → Degraded
/// - それ以外 → Healthy
pub fn classify(
    latest: &RoundStats,
    history: &[RoundStats],
    z_threshold: f64,
    min_baseline_rounds: usize,
) -> Verdict {
    if latest.successes == 0 && latest.attempts > 0 {
        return Verdict {
            health: Health::Unreachable,
            latency_z: None,
            reason: "直近ラウンドで全ての接続試行が失敗しました (到達不能)".to_string(),
        };
    }

    if history.len() < min_baseline_rounds {
        return Verdict {
            health: Health::InsufficientData,
            latency_z: None,
            reason: format!(
                "ベースライン計算に必要な履歴が不足しています ({}/{} ラウンド)",
                history.len(),
                min_baseline_rounds
            ),
        };
    }

    let baseline = match Baseline::from_history(history) {
        Some(b) => b,
        None => {
            return Verdict {
                health: Health::InsufficientData,
                latency_z: None,
                reason: "有効なレイテンシ履歴がありません".to_string(),
            }
        }
    };

    let latency_z = latest
        .mean_latency_ms
        .and_then(|v| z_score(v, baseline.mean_latency_ms, baseline.stddev_latency_ms));

    // 損失率の急増判定: ベースラインが低損失 (ほぼ0) の場合は絶対値で、
    // そうでない場合は相対比で見る。シンプルな経験則: ベースライン+0.2 を
    // 大きく超えたら劣化とみなす。
    let loss_spike = latest.loss_rate > baseline.mean_loss_rate + 0.2;

    let latency_spike = latency_z.map(|z| z.abs() > z_threshold).unwrap_or(false);

    if loss_spike || latency_spike {
        let mut reasons = Vec::new();
        if latency_spike {
            reasons.push(format!(
                "レイテンシ z-score={:.2} (しきい値 {:.2})",
                latency_z.unwrap(),
                z_threshold
            ));
        }
        if loss_spike {
            reasons.push(format!(
                "損失率 {:.0}% がベースライン {:.0}% から急増",
                latest.loss_rate * 100.0,
                baseline.mean_loss_rate * 100.0
            ));
        }
        return Verdict {
            health: Health::Degraded,
            latency_z,
            reason: reasons.join(", "),
        };
    }

    Verdict {
        health: Health::Healthy,
        latency_z,
        reason: "ベースライン範囲内です".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_and_stddev_basic() {
        let vals = vec![10.0, 20.0, 30.0];
        assert_eq!(mean(&vals), Some(20.0));
        let sd = stddev(&vals).unwrap();
        assert!((sd - 10.0).abs() < 1e-9);
    }

    #[test]
    fn mean_empty_is_none() {
        assert_eq!(mean(&[]), None);
        assert_eq!(stddev(&[]), None);
        assert_eq!(stddev(&[1.0]), None);
    }

    #[test]
    fn z_score_basic() {
        let z = z_score(30.0, 20.0, 5.0).unwrap();
        assert!((z - 2.0).abs() < 1e-9);
    }

    #[test]
    fn z_score_zero_stddev_is_none() {
        assert_eq!(z_score(30.0, 20.0, 0.0), None);
    }

    #[test]
    fn loss_rate_basic() {
        assert_eq!(loss_rate(5, 5), 0.0);
        assert_eq!(loss_rate(5, 0), 1.0);
        assert!((loss_rate(4, 3) - 0.25).abs() < 1e-9);
        assert_eq!(loss_rate(0, 0), 0.0);
    }

    #[test]
    fn round_stats_from_samples() {
        let samples = vec![Some(10.0), Some(20.0), None, Some(30.0)];
        let rs = RoundStats::from_samples(&samples);
        assert_eq!(rs.attempts, 4);
        assert_eq!(rs.successes, 3);
        assert!((rs.loss_rate - 0.25).abs() < 1e-9);
        assert_eq!(rs.mean_latency_ms, Some(20.0));
        assert!(rs.jitter_ms.is_some());
    }

    #[test]
    fn round_stats_all_failed() {
        let samples = vec![None, None, None];
        let rs = RoundStats::from_samples(&samples);
        assert_eq!(rs.successes, 0);
        assert_eq!(rs.loss_rate, 1.0);
        assert_eq!(rs.mean_latency_ms, None);
    }

    fn stable_history(n: usize, latency: f64) -> Vec<RoundStats> {
        (0..n)
            .map(|_| RoundStats {
                attempts: 5,
                successes: 5,
                loss_rate: 0.0,
                mean_latency_ms: Some(latency),
                jitter_ms: Some(1.0),
            })
            .collect()
    }

    #[test]
    fn classify_insufficient_data_cold_start() {
        let history = stable_history(2, 20.0);
        let latest = RoundStats {
            attempts: 5,
            successes: 5,
            loss_rate: 0.0,
            mean_latency_ms: Some(20.0),
            jitter_ms: Some(1.0),
        };
        let verdict = classify(&latest, &history, 2.0, 5);
        assert_eq!(verdict.health, Health::InsufficientData);
    }

    #[test]
    fn classify_healthy_within_baseline() {
        // slight variance in history so stddev isn't 0
        let mut history = stable_history(10, 20.0);
        history[0].mean_latency_ms = Some(19.0);
        history[1].mean_latency_ms = Some(21.0);
        let latest = RoundStats {
            attempts: 5,
            successes: 5,
            loss_rate: 0.0,
            mean_latency_ms: Some(20.5),
            jitter_ms: Some(1.0),
        };
        let verdict = classify(&latest, &history, 2.0, 5);
        assert_eq!(verdict.health, Health::Healthy);
    }

    #[test]
    fn classify_degraded_on_latency_spike() {
        let mut history = stable_history(20, 20.0);
        // introduce small variance so stddev is nonzero but tight
        for (i, h) in history.iter_mut().enumerate() {
            h.mean_latency_ms = Some(20.0 + if i % 2 == 0 { 0.5 } else { -0.5 });
        }
        let latest = RoundStats {
            attempts: 5,
            successes: 5,
            loss_rate: 0.0,
            mean_latency_ms: Some(200.0), // huge spike
            jitter_ms: Some(1.0),
        };
        let verdict = classify(&latest, &history, 2.0, 5);
        assert_eq!(verdict.health, Health::Degraded);
        assert!(verdict.latency_z.unwrap() > 2.0);
    }

    #[test]
    fn classify_degraded_on_loss_spike() {
        let history = stable_history(10, 20.0);
        let latest = RoundStats {
            attempts: 5,
            successes: 1,
            loss_rate: 0.8,
            mean_latency_ms: Some(20.0),
            jitter_ms: None,
        };
        let verdict = classify(&latest, &history, 2.0, 5);
        assert_eq!(verdict.health, Health::Degraded);
    }

    #[test]
    fn classify_unreachable_when_all_fail() {
        let history = stable_history(10, 20.0);
        let latest = RoundStats {
            attempts: 5,
            successes: 0,
            loss_rate: 1.0,
            mean_latency_ms: None,
            jitter_ms: None,
        };
        let verdict = classify(&latest, &history, 2.0, 5);
        assert_eq!(verdict.health, Health::Unreachable);
    }
}
