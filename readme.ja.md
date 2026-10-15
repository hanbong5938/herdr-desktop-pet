# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**現在の公開配布:** [Core v0.4.7](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.7)、Release `413584934`、`2026-10-15T08:39:17Z`、共通ソース `52514da5115f09aac22de5429c08f7acdd993258`、neutral protocol4/observation2/API1。本物の [bridge v0.3.11](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.11)、Release `413623057`、`2026-10-15T09:25:08Z`、metadata-only ingress `fa56caa32ea06d86932c5b4cc3eb230010c6dd9c`、annotated tag object `a907424e89c10d484f8646f41d5f020fa93ba8bc` は同じ Core から genuine legacy3/1/API1 を構築します。bridge の11-file source closure・匿名公開5資産・厳密署名・配備68ファイルと Core の6資産/closure を検証済みです。公開 tap `6693cd9db26e6dba9a257bbe76acaf78cb502c2f` は stable Core4.7/legacy3.11、beta は不変。Herdr **v0.4.1** `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8` / assetless Release `412252863` は Herdr-managed source 専用の凍結 retirement authority。Tern0.2.0/互換 pin も不変で、新連携タグやこの consumer 内の native build はありません。

## 歴史的な実行証拠 — Core4.7/bridge3.11 の受け入れではない

以下の4.5/3.10以前の結果は元の版・identity・証拠を保持します。以下の「現在」「新しい」は当時の実行だけを指し、現在の公開版や実ユーザー Mac の状態ではありません。

**現在の Core 実行証拠:** fresh public Brew v0.4.5 導入と read-only 検査は daemon、standalone app、中立 profile を作りませんでした。明示 Start 後の実 GUI は Desktop Pet / Auto / Herdr Waiting0 / Tern Waiting0。PID `1971` / instance `27752246b4da5c6f3964a74dd22881a2` は明示 Hide 後も同一のまま実行中です。独立した既存 profile の通常 v0.4.4→v0.4.5 更新 `a6991e69233377c32b150186c62888fd` は本物の Brew PID=PGID `9853`、exit0/drained true で完了しました。共有 owner 更新前に他の所有済み既定 profile だけを明示 Stop し、暗黙再起動や merge はしません。新 PID `10377` / instance `37953e017fc9208bfcc785a4ff138494` は正確な旧 profile、Herdr1/Tern0、UI/control ready、次の実 Check も緑色でした。旧証拠/profile は daemon log 以外不変で、Herdr 再導入なし。停止していた旧 v0.4.3 checker ではなく、署名済み原本 backup・明示 Stop・実固定 installer・明示 Start による同一 owner 修復後の v0.4.5 Check も通過しました。以前の公開 v0.3.9/v0.4.4 A/B と87証拠/無関係既定 profile 保持は[当時の台帳](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.4.md)に残し、新組合せの証明にはしません。

**新しい公開 managed A→B は完了:** 実 native 凍結3.6→公開3.10 A `20cca91548584437af112bb04cc260b0` の terminal Check・別 Move preview 後、独立 protocol4 同意 B `6ee850aa5577314c6d9cf6ebc7eae981` が既存 Brew4.5 PID3594 / instance2ee0995b8b3dc3abe18e6238a91929d9 で完了。正確な旧 config/state、保存 Herdr/H1T0、実 UI/control ready/Commit。本物の Herdr PID=PGID3648/exit0/quiescent=true が正確な6d2へ更新し、Git clean・legacy app 不在を確認しました。元の A87ファイルと四つの profile/evidence tree の全既存ファイルは byte 不変、元の署名3.6 helper も completed。無関係な default1971 は Stop/merge なしで同一 instance の Hidden 実行を保持。[現在の台帳](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)。

**過去の失敗は保持:** 実 Brew v0.3.6 → v0.3.7 → Core v0.4.2 A→B 成功と managed v0.3.7 A の論理 `/tmp` 不備は別です。後者は **Unknown** のまま保持し、復旧・再実行・reset しません。元の公開 v0.3.8 A は成功しましたが、B preflight は生成 app の untracked ファイル正確に68個で B 同意・Stop 前に拒否しました。`.gitignore` のない元の10 metadata タグを成功に変えません。公開 v0.3.9 は正確な `/dist/HerdrDesktopPet.app/` 所有宣言だけを追加します。実 CI native `/tmp`・生成 app Git gate は通過し、tracked script 編集・無関係な root file・`dist/user-owned-note.txt` は dirty のままです。手動 ignore・user file clean/reset・dirty 保護緩和は移行手順ではありません。

**独立検証と限界:** 実公開3.9 beta 経路は live 置換と protocol2 直接 bridge を拒否し、元の凍結3.6 matched pair を自動起動せず導入しました。同じ beta config/state の明示 Start/実 readiness、署名 helper と user source/profile 保持を検証後、所有3.6だけを Stop。新3.10 beta 実行の主張ではありません。f7edf5e の確定 retirement 失敗・新同意 same-plan resume は過去の別証拠です。Pages と実 SDK 両順序/prompt/transcript/reconnect は独立検証。通常 Tern keychain と OMP provider SignIn/NoModel が未解決で、偽 account/ACK/transcript は使いません。

## 現在の経路と受け入れ境界

公開 Brew3.10 はまず通常の **A′で genuine3.11** に更新し、元の helper の terminal/quiescent settlement を確認します。その後に別の Move preview・Source Review・独立した最終 native 同意 **Bで Core4.7**、readiness/Commit、所有 manager の retirement です。Brew は承認済み stable legacy formula だけを削除し、凍結連携4.1は Herdr-managed 専用。凍結3.6互換経路と beta→元の凍結3.6 matched pair は別です。Unknown の replay/reset や helper/journal の置換は禁止です。

[Core4.7 正本 ledger](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.7.md) は実公開 Brew3.10→通常3.11 A′・元 helper 確定→独立同意の公開 B4.7→readiness/Commit・実所有 Brew retirement と、元バイト・import 済みキャラクター/台詞/履歴・無関係な profile の保持を記録します。現 consumer 再利用・最終 native Check と Cancel/rollback/Stop/Unknown/確定失敗/native Resume も実検証済みです。利用者 Mac native 承認は別途で、H1/T0 は本物 Tern-account/両順序の証明ではありません。

必要な連携だけを導入します。Herdr は薄い consumer です。Tern はネイティブアプリを取得・構築・インストール・自動起動しません。プラグインの更新・削除は共有アプリ、プロファイル、キャラクターを削除しません。通常のアプリ更新で Herdr プラグインを再導入しません。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.ja.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core アプリサイト](https://hanbong5938.github.io/desktop-pet/ja/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 連携サイト](https://hanbong5938.github.io/herdr-desktop-pet/ja/)。旧サイトはアプリ UI・アートを複製しない連携・移行の入口です。

Core CI38706583599・genuine bridge CI38707461727 と実 logical-`/tmp` native・生成 app Git admission が通過しました。Wiki job116329481248 は専用 `WIKI_PUBLISH_TOKEN`・初期 Home が必要で、広範 credential 代替・binary 再実行はしません。実 default branch・4言語 Pages 配信観測は正本 ledger を参照してください。

公開 formula は Core4.7 を選びます ([tap6693cd9](https://github.com/hanbong5938/homebrew-tap/commit/6693cd9db26e6dba9a257bbe76acaf78cb502c2f))。共有 app と必要な連携のみ、review 済み default branch から導入してください:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 以降。ソース構築には Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools が必要ですが、プリビルドには不要です。ad-hoc 署名で、公証されていません。Herdr スナップショットは互換ローカルサーバー（0.9.0+）、返信は別途 `agent.prompt` が必要です。任意の client-attach 自動起動には公式 Herdr 0.9.3 に[連携所有のパッチとホスト手順](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)を適用するか、実際にフック対応のホストが必要です。標準 0.9.0/0.9.3 にはフックがなく、プラグイン導入はホストをパッチしません。

プリビルド・ソース導入は Herdr 連携ではなく **core** チェックアウトで実行します。既定はプリビルドのみで、取得・検証失敗からソースビルドへ黙って切り替えません。`--source` は現在の core だけを構築し、その `dist/DesktopPet.app` を使用します。

## Herdr consumer の契約

この repository は `DesktopPet.app`・native package・character asset を構築/コピーしない薄い consumer です。現行 metadata/evidence8ファイルは匿名公開 Core4.7 Release `413584934`、source `52514da5115f09aac22de5429c08f7acdd993258` から実 materialize 済み。installer SHA-256 `f64a27593058a68c5a5e112f448be644c6abfbb760be109acf992e2f415f5829`、checksum asset `2619293634c27cfc6cbb7db11c9dd8407ac719b3e2c4321d6a6dc5cf21937d5e`、release record `0643094bc94adad0ad3a18120a3a69c2688900889ee9f34befe8f94e31186923`。adapter104テスト成功、実 `install.sh` / `run-native.sh status` は同じ Standalone4.7 PID1446 / instance `d10dcb4df0f131506f0b38a0a9f7aedd`・明示 profile・Herdr1/Tern0を install/start/Herdr再導入なしで再利用しました。[現在の台帳](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.7.md)は凍結連携4.1と歴史的 Core4.2最小 pin を変更/再公開しません。

互換性のある共有アプリを正確に探索する順序は、明示的な絶対実行パス → 検証済みの実アカウントホームの共有 locator → PATH のネイティブ `desktop-pet` → 実際の Brew prefix → 実アカウントホームの `Applications/DesktopPet.app` です。不正な明示パスや locator は別のインストールにフォールバックしません。アプリが存在しない場合だけ固定・digest 検証済みの **core** installer で bootstrap します。非互換なインストールは明示的なネイティブ更新・修復が必要で、ここでは製品ビルドや暗黙の fallback を行いません。

Herdr プロファイルの優先順位は注入された `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 明示的な `--config-dir` / `--state-dir` → Herdr XDG/HOME 既定値です。wrapper は両方の解決済みパスを core に明示し、移動・統合しません。バックエンドの来歴は `--herdr-socket ABS` で伝えます。旧 `--socket` や pet 専用の制御ソケットではありません。`status` は CLI だけでなく実際の実行イメージ・プロファイルを示します。

[Herdr ガイド](integrations/herdr/README.md) は既存の 21 action ID と表示・バブル・ライフサイクルの意味、ホスト構築・パッチ、link、OMP 手順を保持します。`ensure` は冪等で、非表示の pet を表示しません。任意の attach 自動起動には実際にフック対応のホストが必要です。lifecycle ファイルが初めから存在しない場合の Herdr 終了ポリシーは true、既存ファイルでキーがない場合は false です。明示ポリシーと未知のフィールドは書き込み時に保持します。Tern は別の consumer で、共有アプリを自動起動しません。

## ソース / link

先に **desktop-pet Core** checkout で `bash scripts/install.sh --source` により app を導入/構築し、**この連携 checkout** を link します。`plugin link` は構築しません。現行 local consumer は実 Core4.7 と同一 instance 再利用を記録し、新連携タグ/native build はありません。歴史的連携4.1/Core4.2同時 bootstrap/wrapper と Core4.5 Hide→ensure 証拠は当時の範囲です。[独立 checkout 手順](integrations/herdr/README.md#build-in-separate-checkouts)を参照。plugin 導入は Herdr を patch しません。

共通の[使い方](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ja.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[開発](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)、ネイティブ機能・provider 詳細は独立 core が所有します。

## 所有権

このリポジトリは `native-adapter`、`herdr-plugin.toml`、薄い導入・実行 wrapper、`integrations/herdr`、`integrations/omp`、連携検証、凍結された過去リリースを所有します。アプリ・UI・rig・アート・パッケージ・ソース構築は [desktop-pet](https://github.com/hanbong5938/desktop-pet) が所有します。`.herdrchar`、pack v5、`default@0`、rig ABI と [herdr-characters](https://github.com/hanbong5938/herdr-characters) の catalog/gallery 権威は変わりません。すべての公開本文・タグ・素材は不変です。修正版 v0.4.1 は作業ツリー全体・Git index・保存された Git tree の所有権ゲートを通過し、ネイティブ bundle・archive・rig 製品を含みません。

## アートとライセンス

[ソフトウェアライセンス](LICENSE.txt)とアート／モデルの利用条件は**別物**です。MIT ソフトウェアライセンスによってキャラクターのアートが再ライセンスされることはありません。同梱ルベリアの[ライセンス](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[帰属表示](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[出典記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)、[背景修復記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)が正式な情報です。`LICENSE.txt` には元の Coding Cat の告知も残っています。そのアート／ソースは[キャラクターリポジトリ](https://github.com/hanbong5938/herdr-characters)へ移されました。各追加パック独自の条件を必ず保持してください。アート所有者の承認はモデルの権利を与えません。Qwen の研究用ライセンスは制作記録であり、モデルのエンジンや重みは同梱されていません。[キャラクター制作とライセンス（英語）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)を参照してください。
