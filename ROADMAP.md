# pathwx ロードマップ

## v0.1 (現在) — MVP: TCP connectベースの継続観測 + z-score劣化検知

- `init` / `probe` / `watch` / `check` / `report` の5サブコマンド
- TOML設定、JSONLプレーンテキストストレージ (DBなし)
- 平均+標準偏差ベースラインとz-scoreによる健康/劣化判定、コールドスタート対応

## v0.2 候補 — ICMPオプション

- raw socket / ICMP echoによる本来の意味での疎通確認 (要: 昇格権限 or `CAP_NET_RAW`)
- TCP connectが使えない (ポート閉鎖・ファイアウォール) 宛先でも純粋なレイテンシ計測ができるようにする
- 既定は現状どおりTCP connect、`--icmp` フラグ等でオプトイン (権限がない環境でも動き続けることを優先)

## v0.3 候補 — bgp-sentinelとの相関

- pathwxが劣化を検知したタイムスタンプと、bgp-sentinelが検知した経路変化イベントを突き合わせる
- 「このレイテンシ悪化はBGP経路変更と同時刻か?」を自動で注記し、犯人探しの初動を速くする
- netblameへの「深掘り診断」への自動連携 (劣化検知 → netblame一発診断のトリガー) も検討

## v0.4 候補 — ターゲットごとの独立スケジューリング

- 現状 `watch` は全ターゲット中最短intervalで一律ループしているが、ターゲットごとに正確なタイマーで回す
- 高頻度ターゲットと低頻度ターゲットが混在する設定でも無駄なプローブをしない

## v1.0候補 — `serve` / Webダッシュボードモード

- ashealthの `serve` 同様、複数ターゲットの現在状態・トレンドグラフをWebで見られるモード
- `health.pathvector.dev` / ホスティング版構想 (`PATHVECTOR_DEV_VISION.md` 参照) への統合
  - 将来的にはpathwxの継続観測データを ashealth の定期健診・bgp-sentinelのリアルタイム検知と並べて
    1つのダッシュボードで見せる「経路の天気予報」としての位置づけを目指す
- 認証・複数ユーザー・保存データのアクセス制御は、bgp-sentinelの既存基盤 (argon2+JWT) を再利用する想定
