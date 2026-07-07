//! pathwx — ネットワーク気象観測所
//!
//! L3 経路品質 (TCP connect ベースの疎通・レイテンシ・ジッタ) を継続的に
//! 観測し、ヒストリカルなベースラインからの乖離 (z-score) でゆるやかな劣化
//! (「壊れてはいないが、悪くなってきている」) を検出する。
//!
//! 一発診断の netblame、ホストレベル/経路レベルの点検ツール群 (ashealth,
//! driftmap, bgp-sentinel) に対して、pathwx は「時系列でのトレンド」を担当する。

pub mod config;
pub mod probe;
pub mod report;
pub mod stats;
pub mod storage;
