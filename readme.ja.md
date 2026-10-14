# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**現在の公開配布:** [Core v0.4.5](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.5)、Release `412667385`、`2026-10-14T12:01:32Z`、正確な共通ソース `51dcf22415b1ff0faf07651cb83c1c696c980f43`、protocol4/observation2/API1 です。現行 consumer は実公開 installer bytes を認証します。意図的な legacy ingress は本物の [bridge v0.3.10](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.10)、Release `412684474`、`2026-10-14T12:20:45Z`、metadata ソース `67eb44b53bda6ba96eaf68671b67a94cab7b304d` を選びます。同じ Core から実 legacy3/1/API1 を構築し、Git は11 regular UTF-8 metadata と限定 generated-app ignore のみです。両公開資産集合、厳密署名、68-file closure を検証済みです。旧 stable formula `7bf1fd7f1b89bd77e68179fcee95cacf044927dc` は v0.3.10。Herdr **v0.4.1** の不変 `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8` / assetless Release `412252863`、Tern v0.2.0 と互換 Core pin は変更しません。新連携タグ、Tern native 導入/自動起動、以前のタグ/本文/資産/beta の置換はありません。

**現在の Core 実行証拠:** fresh public Brew v0.4.5 導入と read-only 検査は daemon、standalone app、中立 profile を作りませんでした。明示 Start 後の実 GUI は Desktop Pet / Auto / Herdr Waiting0 / Tern Waiting0。PID `1971` / instance `27752246b4da5c6f3964a74dd22881a2` は明示 Hide 後も同一のまま実行中です。独立した既存 profile の通常 v0.4.4→v0.4.5 更新 `a6991e69233377c32b150186c62888fd` は本物の Brew PID=PGID `9853`、exit0/drained true で完了しました。共有 owner 更新前に他の所有済み既定 profile だけを明示 Stop し、暗黙再起動や merge はしません。新 PID `10377` / instance `37953e017fc9208bfcc785a4ff138494` は正確な旧 profile、Herdr1/Tern0、UI/control ready、次の実 Check も緑色でした。旧証拠/profile は daemon log 以外不変で、Herdr 再導入なし。停止していた旧 v0.4.3 checker ではなく、署名済み原本 backup・明示 Stop・実固定 installer・明示 Start による同一 owner 修復後の v0.4.5 Check も通過しました。以前の公開 v0.3.9/v0.4.4 A/B と87証拠/無関係既定 profile 保持は[当時の台帳](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.4.md)に残し、新組合せの証明にはしません。

**新しい公開 managed A→B は完了:** 実 native 凍結3.6→公開3.10 A `20cca91548584437af112bb04cc260b0` の terminal Check・別 Move preview 後、独立 protocol4 同意 B `6ee850aa5577314c6d9cf6ebc7eae981` が既存 Brew4.5 PID3594 / instance2ee0995b8b3dc3abe18e6238a91929d9 で完了。正確な旧 config/state、保存 Herdr/H1T0、実 UI/control ready/Commit。本物の Herdr PID=PGID3648/exit0/quiescent=true が正確な6d2へ更新し、Git clean・legacy app 不在を確認しました。元の A87ファイルと四つの profile/evidence tree の全既存ファイルは byte 不変、元の署名3.6 helper も completed。無関係な default1971 は Stop/merge なしで同一 instance の Hidden 実行を保持。[現在の台帳](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)。

**過去の失敗は保持:** 実 Brew v0.3.6 → v0.3.7 → Core v0.4.2 A→B 成功と managed v0.3.7 A の論理 `/tmp` 不備は別です。後者は **Unknown** のまま保持し、復旧・再実行・reset しません。元の公開 v0.3.8 A は成功しましたが、B preflight は生成 app の untracked ファイル正確に68個で B 同意・Stop 前に拒否しました。`.gitignore` のない元の10 metadata タグを成功に変えません。公開 v0.3.9 は正確な `/dist/HerdrDesktopPet.app/` 所有宣言だけを追加します。実 CI native `/tmp`・生成 app Git gate は通過し、tracked script 編集・無関係な root file・`dist/user-owned-note.txt` は dirty のままです。手動 ignore・user file clean/reset・dirty 保護緩和は移行手順ではありません。

**独立検証と限界:** 実公開3.9 beta 経路は live 置換と protocol2 直接 bridge を拒否し、元の凍結3.6 matched pair を自動起動せず導入しました。同じ beta config/state の明示 Start/実 readiness、署名 helper と user source/profile 保持を検証後、所有3.6だけを Stop。新3.10 beta 実行の主張ではありません。f7edf5e の確定 retirement 失敗・新同意 same-plan resume は過去の別証拠です。Pages と実 SDK 両順序/prompt/transcript/reconnect は独立検証。通常 Tern keychain と OMP provider SignIn/NoModel が未解決で、偽 account/ACK/transcript は使いません。

必要な連携だけを導入します。Herdr は薄い consumer です。Tern はネイティブアプリを取得・構築・インストール・自動起動しません。プラグインの更新・削除は共有アプリ、プロファイル、キャラクターを削除しません。通常のアプリ更新で Herdr プラグインを再導入しません。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.ja.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core アプリサイト](https://hanbong5938.github.io/desktop-pet/ja/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 連携サイト](https://hanbong5938.github.io/herdr-desktop-pet/ja/)。旧サイトはアプリ UI・アートを複製しない連携・移行の入口です。

Core v0.4.5 CI `38560075408` release job `115838673595` と本物の bridge CI `38561115955` job `115842136732` は成功し、実 native 論理 `/tmp` と生成 app Git 保護も通過しました。後続 Wiki job `115840956329` は render 後、専用 `WIKI_PUBLISH_TOKEN` 不在で公開に失敗。所有者 Home 初期化と専用資格情報が必要で、広範な `gh`/`GITHUB_TOKEN` fallback は使いません。現在 Core/Herdr Pages の source/link だけで配備成功とはしません。

公開中立 formula は Core v0.4.5 を選びます ([tap `d66411383141644f01f1ca839086b6d505a997ac`](https://github.com/hanbong5938/homebrew-tap/commit/d66411383141644f01f1ca839086b6d505a997ac))。共有 app と必要な main 連携だけを導入します:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 以降。ソース構築には Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools が必要ですが、プリビルドには不要です。ad-hoc 署名で、公証されていません。Herdr スナップショットは互換ローカルサーバー（0.9.0+）、返信は別途 `agent.prompt` が必要です。任意の client-attach 自動起動には公式 Herdr 0.9.3 に[連携所有のパッチとホスト手順](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)を適用するか、実際にフック対応のホストが必要です。標準 0.9.0/0.9.3 にはフックがなく、プラグイン導入はホストをパッチしません。

プリビルド・ソース導入は Herdr 連携ではなく **core** チェックアウトで実行します。既定はプリビルドのみで、取得・検証失敗からソースビルドへ黙って切り替えません。`--source` は現在の core だけを構築し、その `dist/DesktopPet.app` を使用します。

## Herdr consumer の契約

この repository は `DesktopPet.app`、native package、character asset を構築・コピーしません。連携 manifest/wrapper/adapter はここ、native source/art/tool は Core 所有です。現行 `core-release.json` / `release-evidence/` は実公開 Core v0.4.5 Release `412667385`、正確な `51dcf22415b1ff0faf07651cb83c1c696c980f43` から materialize 済みです。installer SHA-256 は `fdcd9d80f8672b54ceca967bfea6f72223d2e5caed97f845140a68b9e7414a16`、checksum asset は `8c9c8ab020bc4561c4a2c17f4c6d20b3e8a18672b84f508426e68ed259598c5e`。[実権威と runtime 証拠](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)は不変 v0.4.1 タグの歴史的 Core v0.4.2 最小 pin を変更・再公開しません。

互換性のある共有アプリを正確に探索する順序は、明示的な絶対実行パス → 検証済みの実アカウントホームの共有 locator → PATH のネイティブ `desktop-pet` → 実際の Brew prefix → 実アカウントホームの `Applications/DesktopPet.app` です。不正な明示パスや locator は別のインストールにフォールバックしません。アプリが存在しない場合だけ固定・digest 検証済みの **core** installer で bootstrap します。非互換なインストールは明示的なネイティブ更新・修復が必要で、ここでは製品ビルドや暗黙の fallback を行いません。

Herdr プロファイルの優先順位は注入された `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 明示的な `--config-dir` / `--state-dir` → Herdr XDG/HOME 既定値です。wrapper は両方の解決済みパスを core に明示し、移動・統合しません。バックエンドの来歴は `--herdr-socket ABS` で伝えます。旧 `--socket` や pet 専用の制御ソケットではありません。`status` は CLI だけでなく実際の実行イメージ・プロファイルを示します。

[Herdr ガイド](integrations/herdr/README.md) は既存の 21 action ID と表示・バブル・ライフサイクルの意味、ホスト構築・パッチ、link、OMP 手順を保持します。`ensure` は冪等で、非表示の pet を表示しません。任意の attach 自動起動には実際にフック対応のホストが必要です。lifecycle ファイルが初めから存在しない場合の Herdr 終了ポリシーは true、既存ファイルでキーがない場合は false です。明示ポリシーと未知のフィールドは書き込み時に保持します。Tern は別の consumer で、共有アプリを自動起動しません。

## ソース / link

先に **desktop-pet Core** checkout で `bash scripts/install.sh --source` により app を導入・構築し、Core ではなく **この連携 checkout** を link します。`plugin link` は構築しません。現行 consumer は実 Core v0.4.5 を固定し、実同一4.5 instance の Hide→ensure 保持を検証済みです。歴史的 v0.4.1 / Core v0.4.2 の同時 bootstrap/wrapper 証拠は別記録で、新連携タグは作りません。[独立 checkout 手順](integrations/herdr/README.md#build-in-separate-checkouts)を参照。plugin 導入は Herdr を patch しません。

共通の[使い方](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ja.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[開発](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)、ネイティブ機能・provider 詳細は独立 core が所有します。

## 所有権

このリポジトリは `native-adapter`、`herdr-plugin.toml`、薄い導入・実行 wrapper、`integrations/herdr`、`integrations/omp`、連携検証、凍結された過去リリースを所有します。アプリ・UI・rig・アート・パッケージ・ソース構築は [desktop-pet](https://github.com/hanbong5938/desktop-pet) が所有します。`.herdrchar`、pack v5、`default@0`、rig ABI と [herdr-characters](https://github.com/hanbong5938/herdr-characters) の catalog/gallery 権威は変わりません。すべての公開本文・タグ・素材は不変です。修正版 v0.4.1 は作業ツリー全体・Git index・保存された Git tree の所有権ゲートを通過し、ネイティブ bundle・archive・rig 製品を含みません。

## アートとライセンス

[ソフトウェアライセンス](LICENSE.txt)とアート／モデルの利用条件は**別物**です。MIT ソフトウェアライセンスによってキャラクターのアートが再ライセンスされることはありません。同梱ルベリアの[ライセンス](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[帰属表示](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[出典記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)、[背景修復記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)が正式な情報です。`LICENSE.txt` には元の Coding Cat の告知も残っています。そのアート／ソースは[キャラクターリポジトリ](https://github.com/hanbong5938/herdr-characters)へ移されました。各追加パック独自の条件を必ず保持してください。アート所有者の承認はモデルの権利を与えません。Qwen の研究用ライセンスは制作記録であり、モデルのエンジンや重みは同梱されていません。[キャラクター制作とライセンス（英語）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)を参照してください。
