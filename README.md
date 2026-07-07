# pathwx — ネットワーク気象観測所

**「壊れているか/いないか」ではなく、「悪くなってきているか」を見るツールです。**

pathwx (path + 航空気象の "wx") は、指定した宛先への TCP 接続を一定間隔で繰り返し観測し、
損失率・レイテンシ・ジッタの時系列をプレーンテキスト (JSONL) に蓄積、
そのターゲット固有の「普段の変動幅」(ベースライン)からの統計的な乖離 (z-score) を見て、
ゆるやかな劣化を検知する常駐監視 CLI です。

## なぜこのツールが要るのか

pathvector-studio の他のツールは、それぞれ別の瞬間・別のレイヤーを見ています。

| ツール | 見ているもの |
|---|---|
| [netblame](../netblame) | いま・この1回の接続不良の犯人探し (一発診断) |
| [ashealth](../ashealth) | AS運用のBGP/RPKI/IRR/PeeringDB整合性 (定期健診) |
| [driftmap](../driftmap) | LLDPスナップショット間のトポロジ差分 (構成ドリフト) |
| [bgp-sentinel](../bgp-sentinel) | リアルタイムのBGPハイジャック検知 |

どれも「ある瞬間の状態」か「構成の差分」を見ていて、
**「経路品質が先週より緩やかに悪化している」という時系列トレンド**を継続的に見るツールがありません。
ping監視ツールは大抵、固定しきい値 (例: 100ms超えたらアラート) でしか判定できず、
ノイズの多い回線では常時誤報になり、逆に安定した回線のわずかな異常は検知できません。

pathwx はヒストリカルなベースライン (平均+標準偏差) からの **z-score** で判定することで、
「そのターゲットにとって普段どおりか」を基準にします。障害になる前の「予兆」を捉えるのが狙いです。

## 5分で試す

```console
$ cargo build --release
$ ./target/release/pathwx init
サンプル設定ファイルを書き出しました: pathwx.toml

$ cat pathwx.toml
# ...
[[targets]]
name = "cloudflare-dns"
host = "1.1.1.1"
port = 443
interval_secs = 60
samples = 5
timeout_ms = 1000

# 1ラウンド実行 (ログに追記される)
$ ./target/release/pathwx probe
TARGET                   SENT       OK      LOSS%    AVG(ms)
cloudflare-dns              5        5         0%       12.4

# 何度か probe を繰り返して履歴を貯めたあと、ベースラインと比較して判定
$ ./target/release/pathwx check cloudflare-dns
cloudflare-dns       [HEALTHY] loss=0% avg=13.1ms — ベースライン範囲内です

# 過去ログからベースライン統計とスパークラインを表示
$ ./target/release/pathwx report --target cloudflare-dns --window 20
=== pathwx report: cloudflare-dns ===
総ラウンド数: 12
ベースライン (全12ラウンド): 平均レイテンシ=13.20ms 標準偏差=2.10ms 平均損失率=0.0%
直近12ラウンドのレイテンシ推移: ▂▃▁▄▂▃▂▁▃▂▄▂
直近ラウンドの判定: Healthy — ベースライン範囲内です

# 常駐観測 (Ctrl-Cで終了。各ターゲットのinterval_secsに従って回る)
$ ./target/release/pathwx watch
```

CI・cron から使う場合は `check` の終了コードで分岐できます:

```console
$ pathwx check cloudflare-dns; echo $?
0   # healthy または insufficient data (履歴不足はエラー扱いにしない)
1   # degraded (ベースラインからの乖離)
2   # unreachable (直近ラウンドが全滅) / 実行時エラー
```

## サブコマンド

| コマンド | 内容 |
|---|---|
| `pathwx init [--out FILE] [--force]` | サンプル設定ファイルを生成 |
| `pathwx probe` | 全ターゲットに1ラウンド実行し、サマリ表示 + ログ追記 |
| `pathwx watch [--rounds N]` | 各ターゲットのintervalに従い無限ループ観測 (フォアグラウンド常駐) |
| `pathwx check [target]` | 1ラウンド実行しベースラインと比較、判定を表示し終了コードを返す |
| `pathwx report --target NAME [--window N]` | 過去ログからベースラインと直近トレンド(スパークライン)を表示 |

## 設定ファイル (TOML)

```toml
# データ保存先 (省略時は ~/.local/share/pathwx)
# data_dir = "/var/lib/pathwx"

# ベースライン計算に使う直近ラウンド数 (省略時は全履歴)
# baseline_window = 200

# 劣化判定のz-scoreしきい値 (既定 2.0)
z_threshold = 2.0

# ベースライン確立に必要な最低ラウンド数 (これ未満は insufficient data)
min_baseline_rounds = 5

[[targets]]
name = "cloudflare-dns"
host = "1.1.1.1"
port = 443
interval_secs = 60   # watchでのプローブ間隔(秒)
samples = 5           # 1ラウンドあたりのTCP connect試行回数
timeout_ms = 1000     # 1回の接続試行のタイムアウト(ミリ秒)
```

## データ形式

各ターゲットのログは `<data_dir>/<target名>.jsonl` に1ラウンド1行で追記されます。
DBを使わないプレーンテキストなので、**git にコミットしてレビューできますし、`jq` や `tail -f` でもそのまま読めます**。

```json
{"timestamp":"2026-07-07T11:00:00Z","target":"cloudflare-dns","host":"1.1.1.1","port":443,"attempts":5,"successes":5,"loss_rate":0.0,"mean_latency_ms":12.4,"jitter_ms":1.1}
```

## 劣化判定のロジック

1. 直近ラウンドの成功が0件 → **UNREACHABLE**
2. 履歴ラウンド数が `min_baseline_rounds` 未満 → **INSUFFICIENT DATA** (誤報防止のコールドスタート対応)
3. 履歴 (直近ラウンドを除く全ラウンド、または `baseline_window` 分) から平均・標準偏差のベースラインを計算
4. 直近ラウンドの平均レイテンシの z-score が `z_threshold` を超える、または損失率がベースラインから急増 → **DEGRADED**
5. それ以外 → **HEALTHY**

固定しきい値ではなく、そのターゲット自身の過去の振る舞いを基準にするのがポイントです。

## 制約・既知の限界

- **TCP connectベースのみ。ICMP/raw socketは不使用** — netblame と同じ制約で、特権なしで動かせる代わりに「ポートが開いていない宛先」の純粋な疎通性は測れません(将来的なICMPオプションは ROADMAP.md 参照)
- **`watch` はシングルプロセスのフォアグラウンドループのみ** — systemdなどでデーモン化する運用は利用者側で行ってください。デタッチやsystemdユニットの同梱は今のところありません
- ターゲットごとに独立したスケジューリングではなく、`watch` は全ターゲット中最短の `interval_secs` でループを回します(MVPとしての単純化。ターゲットごとの正確なタイマーは今後の課題)
- ベースラインは「平均+標準偏差」のシンプルな統計モデルです。周期的な変動 (時間帯・曜日パターンなど) は考慮しません
- 1プロセス1マシンでの利用を想定。分散観測・複数拠点からの相関は対象外

## ライセンス

MIT License. 詳細は [LICENSE](LICENSE) を参照してください。

## 関連プロジェクト

- [netblame](../netblame) — 一発診断
- [ashealth](../ashealth) — AS運用の健康診断
- [driftmap](../driftmap) — 腐らないネットワーク図
- [bgp-sentinel](../bgp-sentinel) — リアルタイムBGPハイジャック検知

今後の展望は [ROADMAP.md](ROADMAP.md) を参照してください。
