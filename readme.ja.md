# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**公開状態:** 既存の [Herdr v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) は protocol 3 / observation capability 1 の公開版です。独立 Desktop Pet v0.4.0（protocol 4 / capability 2 / integration API 1）と移行用 bridge v0.3.7 はレビュー済み候補で、公開インストール・実行検証は未完了です。以下は中立製品への切り替え手順であり、文書・タグ・ソース版番号は公開の証明ではありません。

必要な連携だけを導入します。Herdr は薄い consumer です。Tern はネイティブアプリを取得・構築・インストール・自動起動しません。プラグインの更新・削除は共有アプリ、プロファイル、キャラクターを削除しません。通常のアプリ更新で Herdr プラグインを再導入しません。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.ja.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core アプリサイト](https://hanbong5938.github.io/desktop-pet/ja/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 連携サイト](https://hanbong5938.github.io/herdr-desktop-pet/ja/)。旧サイトはアプリ UI・アートを複製しない連携・移行の入口です。

中立リリースと formula が実際に公開された後の公開コマンド:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 以降。ソース構築には Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools が必要ですが、プリビルドには不要です。ad-hoc 署名で、公証されていません。Herdr スナップショットは互換ローカルサーバー（0.9.0+）、返信は別途 `agent.prompt` が必要です。任意の client-attach 自動起動には公式 Herdr 0.9.3 に[連携所有のパッチとホスト手順](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)を適用するか、実際にフック対応のホストが必要です。標準 0.9.0/0.9.3 にはフックがなく、プラグイン導入はホストをパッチしません。

プリビルド・ソース導入は Herdr 連携ではなく **core** チェックアウトで実行します。既定はプリビルドのみで、取得・検証失敗からソースビルドへ黙って切り替えません。`--source` は現在の core だけを構築し、その `dist/DesktopPet.app` を使用します。

## Herdr consumer の契約

このリポジトリは `DesktopPet.app`、ネイティブパッケージ、キャラクター素材を構築・コピーしません。連携専用 manifest・wrapper・adapter はこのチェックアウトに移り、ネイティブソース・アート・ツールは core の所有です。consumer pin の生成と公開導入には実際の公開 core installer/checksum とレビュー済み Release API メタデータが必要です。移転先リンクやローカルソースは公開の証明ではありません。

互換性のある共有アプリを正確に探索する順序は、明示的な絶対実行パス → 検証済みの実アカウントホームの共有 locator → PATH のネイティブ `desktop-pet` → 実際の Brew prefix → 実アカウントホームの `Applications/DesktopPet.app` です。不正な明示パスや locator は別のインストールにフォールバックしません。アプリが存在しない場合だけ固定・digest 検証済みの **core** installer で bootstrap します。非互換なインストールは明示的なネイティブ更新・修復が必要で、ここでは製品ビルドや暗黙の fallback を行いません。

Herdr プロファイルの優先順位は注入された `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 明示的な `--config-dir` / `--state-dir` → Herdr XDG/HOME 既定値です。wrapper は両方の解決済みパスを core に明示し、移動・統合しません。バックエンドの来歴は `--herdr-socket ABS` で伝えます。旧 `--socket` や pet 専用の制御ソケットではありません。`status` は CLI だけでなく実際の実行イメージ・プロファイルを示します。

[Herdr ガイド](integrations/herdr/README.md) は既存の 21 action ID と表示・バブル・ライフサイクルの意味、ホスト構築・パッチ、link、OMP 手順を保持します。`ensure` は冪等で、非表示の pet を表示しません。任意の attach 自動起動には実際にフック対応のホストが必要です。lifecycle ファイルが初めから存在しない場合の Herdr 終了ポリシーは true、既存ファイルでキーがない場合は false です。明示ポリシーと未知のフィールドは書き込み時に保持します。Tern は別の consumer で、共有アプリを自動起動しません。

## ソース / link

まず **desktop-pet core** チェックアウトで `bash scripts/install.sh --source` によりアプリを導入・構築します。互換アプリを選択後、core ではなく**この連携チェックアウト**を link します。`plugin link` はビルドを実行しません。公開/bootstrap 利用には実際の consumer pin 生成が必要です。[独立チェックアウトの手順](integrations/herdr/README.md#build-in-separate-checkouts)を参照してください。プラグイン導入は Herdr をパッチしません。

共通の[使い方](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ja.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[開発](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)、ネイティブ機能・provider 詳細は独立 core が所有します。

## 所有権

このリポジトリは `native-adapter`、`herdr-plugin.toml`、薄い導入・実行 wrapper、`integrations/herdr`、`integrations/omp`、連携検証、凍結された過去リリースを所有します。アプリ・UI・rig・アート・パッケージ・ソース構築は [desktop-pet](https://github.com/hanbong5938/desktop-pet) が所有します。`.herdrchar`、pack v5、`default@0`、rig ABI と [herdr-characters](https://github.com/hanbong5938/herdr-characters) の catalog/gallery 権威は変わりません。過去の [v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) および旧リリース本文・タグ・素材は不変です。連携専用 v0.4.0 にネイティブ archive は含みません。

## アートとライセンス

[ソフトウェアライセンス](LICENSE.txt)とアート／モデルの利用条件は**別物**です。MIT ソフトウェアライセンスによってキャラクターのアートが再ライセンスされることはありません。同梱ルベリアの[ライセンス](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[帰属表示](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[出典記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)、[背景修復記録](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)が正式な情報です。`LICENSE.txt` には元の Coding Cat の告知も残っています。そのアート／ソースは[キャラクターリポジトリ](https://github.com/hanbong5938/herdr-characters)へ移されました。各追加パック独自の条件を必ず保持してください。アート所有者の承認はモデルの権利を与えません。Qwen の研究用ライセンスは制作記録であり、モデルのエンジンや重みは同梱されていません。[キャラクター制作とライセンス（英語）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)を参照してください。
