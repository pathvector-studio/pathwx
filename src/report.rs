//! レポート表示ヘルパー: ASCII スパークライン等

/// 8段階の棒グラフ文字 (低い順)
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// レイテンシ (ms) を人間向けに整形する。`{:.1}` 固定だとサブミリ秒
/// (例: localhostの ~0.02ms) が "0.0" に潰れてしまうため、1ms未満は
/// 小数点以下3桁、それ以上は1桁で表示する。
pub fn format_latency_ms(value: f64) -> String {
    if value.abs() < 1.0 {
        format!("{value:.3}")
    } else {
        format!("{value:.1}")
    }
}

/// 値の列から ASCII スパークラインを生成する。
/// 全て同じ値、または空の場合は平坦な線を返す。
pub fn sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    values
        .iter()
        .map(|&v| {
            if range <= f64::EPSILON {
                BLOCKS[0]
            } else {
                let idx = (((v - min) / range) * (BLOCKS.len() as f64 - 1.0)).round() as usize;
                BLOCKS[idx.min(BLOCKS.len() - 1)]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_empty() {
        assert_eq!(sparkline(&[]), "");
    }

    #[test]
    fn sparkline_flat_values() {
        let s = sparkline(&[5.0, 5.0, 5.0]);
        assert_eq!(s.chars().count(), 3);
        // all identical -> all same lowest block
        assert!(s.chars().all(|c| c == BLOCKS[0]));
    }

    #[test]
    fn sparkline_increasing_values() {
        let s = sparkline(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars.len(), 5);
        assert_eq!(chars[0], BLOCKS[0]);
        assert_eq!(chars[4], BLOCKS[BLOCKS.len() - 1]);
    }

    #[test]
    fn format_latency_sub_millisecond_uses_three_decimals() {
        // A localhost-style ~0.02ms round-trip must not be rounded away to
        // "0.0" — that's indistinguishable from a hung/zero measurement.
        assert_eq!(format_latency_ms(0.02), "0.020");
        assert_eq!(format_latency_ms(0.999), "0.999");
    }

    #[test]
    fn format_latency_millisecond_and_above_uses_one_decimal() {
        assert_eq!(format_latency_ms(1.0), "1.0");
        assert_eq!(format_latency_ms(12.44), "12.4");
        assert_eq!(format_latency_ms(200.0), "200.0");
    }
}
