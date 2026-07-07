//! TCP connect ベースのプローブ
//!
//! netblame と同じ制約 (raw socket / ICMP 不使用、生ポートへの TCP connect
//! のみ) の中で、1 ターゲットに対して N 回の接続試行を行い、成功/失敗と
//! 接続レイテンシを記録する。

use crate::config::Target;
use crate::stats::RoundStats;
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// 1回の接続試行結果: 成功なら Some(レイテンシms)、失敗なら None
pub fn connect_once(host: &str, port: u16, timeout: Duration) -> Option<f64> {
    use std::net::ToSocketAddrs;

    let addr = format!("{host}:{port}");
    let sockaddr = match addr.to_socket_addrs() {
        Ok(mut it) => it.next(),
        Err(_) => None,
    }?;

    let start = Instant::now();
    match TcpStream::connect_timeout(&sockaddr, timeout) {
        Ok(_stream) => Some(start.elapsed().as_secs_f64() * 1000.0),
        Err(_) => None,
    }
}

/// target 設定に従って1ラウンド分 (samples 回) プローブし、統計を返す
pub fn run_round(target: &Target) -> RoundStats {
    let timeout = Duration::from_millis(target.timeout_ms);
    let samples: Vec<Option<f64>> = (0..target.samples)
        .map(|_| connect_once(&target.host, target.port, timeout))
        .collect();
    RoundStats::from_samples(&samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;

    /// 127.0.0.1 の自前リスナーに対してプローブし、成功が記録されることを確認する
    #[test]
    fn connect_once_succeeds_against_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 16];
                    let _ = s.read(&mut buf);
                }
            }
        });

        let latency = connect_once("127.0.0.1", port, Duration::from_millis(500));
        assert!(latency.is_some());
        assert!(latency.unwrap() >= 0.0);

        // drain remaining accepts so the thread exits
        let _ = TcpStream::connect(("127.0.0.1", port));
        let _ = TcpStream::connect(("127.0.0.1", port));
        handle.join().ok();
    }

    #[test]
    fn connect_once_fails_when_nothing_listening() {
        // Bind then immediately drop to get a very likely-closed port.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let latency = connect_once("127.0.0.1", port, Duration::from_millis(300));
        assert!(latency.is_none());
    }

    #[test]
    fn run_round_records_full_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let samples = 3u32;
        let handle = thread::spawn(move || {
            for stream in listener.incoming().take(samples as usize) {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 16];
                    let _ = s.read(&mut buf);
                }
            }
        });

        let target = Target {
            name: "local".to_string(),
            host: "127.0.0.1".to_string(),
            port,
            interval_secs: 1,
            samples,
            timeout_ms: 500,
        };
        let stats = run_round(&target);
        assert_eq!(stats.attempts, samples);
        assert_eq!(stats.successes, samples);
        assert_eq!(stats.loss_rate, 0.0);
        assert!(stats.mean_latency_ms.is_some());

        handle.join().ok();
    }
}
