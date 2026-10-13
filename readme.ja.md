# Herdr Desktop Pet

[Herdr](https://herdr.dev) 向けのネイティブ macOS デスクトップコンパニオンです。ルベリアはタッチに反応し、観測したエージェントセッションをデスクトップに表示して、選択したローカルセッションへの返信を可能にします。

[English](readme.md) · [한국어](readme.ko.md) · [简体中文](readme.zh-Hans.md) · **日本語** · [バージョン資料（英語）](docs/releases/README.md) · [リリース](https://github.com/hanbong5938/herdr-desktop-pet/releases)

<img src="assets/rubelia-thumbnail.png" alt="標準デスクトップコンパニオンのルベリア" width="220">

[追加キャラクター・衣装ギャラリー](https://hanbong5938.github.io/herdr-characters/)（最初は英語表示、韓国語は KO を選択）· [キャラクターパックのソースとライセンス](https://github.com/hanbong5938/herdr-characters)。**同梱されるのはルベリア（`default@0`）だけです。**

[v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) は公開済みの最新安定版で、公開 arm64 資産・チェックサムと更新済み安定版 Homebrew formula は検証済みです。対応するアプリ／ヘルパーの能力は `protocol: 3` と `observation_mode: 1` です。過去の v0.3.5 とベータ資産は変更しません。

## 機能

- 移動・サイズ変更可能なデスクトップキャラクター。タッチや撫でる操作への反応、独立した状態バブル、メニューバー操作に対応。UI はシステム言語、韓国語、英語、簡体字中国語、日本語から選択できます。
- 観測した Herdr セッションのカード。検索・絞り込み・並べ替えと、ローカル／リモートの明確な区別。リモート観測は任意で読み取り専用です。
- 選択した**ローカル**エージェントセッションへの一行返信。IME の変換中入力に配慮し、ネイティブの状態フィードバックを表示します。
- キャラクター、バブル、セリフ、表示状態の設定。検証済み追加パックと公式カタログブラウザーに対応します。
- Herdr のライフサイクル操作、保護付きの linked worktree 削除、インストール元を区別した明示的なアプリ更新。

## 動作要件

- **Apple Silicon 搭載 Mac、macOS 13 以降。** Intel Mac、Windows、Linux はネイティブ版の対応対象外です。
- ローカル観測には既存のローカル Herdr サーバーを使用します（スナップショットには 0.9.0 以降）。プラグインやホストへのパッチは不要です。返信には別途そのサーバーの `agent.prompt` が必要です。**任意のクライアント接続時の自動起動だけ**は、提供される [`client.attached` パッチ](integrations/herdr/client-attached.patch)を適用した公式 Herdr 0.9.3 ソースから構築したホスト、または将来そのフックへの対応を実際に明示するリリースが必要です。無改変の Herdr 0.9.0／0.9.3 にそのフックはなく、マニフェストの最低バージョン指定だけでは使えません。管理下で運用する場合は[ホスト連携ガイド（英語）](integrations/herdr/README.md)に従い、未知のフックについての警告を見逃さないでください。
- ソースからのビルドには Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools が必要です。ビルド済み版にはソース用ツールチェーンは不要です。このアプリは **ad-hoc 署名であり、Developer ID 署名も公証もありません**。macOS Gatekeeper で明示的なローカル承認が必要になる場合があります。署名検証は公証を意味しません。

## Homebrew でのインストール：最初の接続

以下の安定版 formula で公開済みビルドを導入します。Rust/Cargo、Bun、Node.js、ソースチェックアウトは不要です。既存の Brew 利用者は下記の更新コマンドを使います。既存のプロファイルと設定は保持されます。

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

ネイティブの `start` は手動起動です。`herdr plugin install`、`plugin link`、パッチ適用済みホストは必要ありません。バブルを開くとそのサーバーのローカルセッションカードを確認できます（スナップショットには Herdr 0.9.0 以降が必要）。リアルタイムのカードには稼働中のエージェントセッションが必要です。返信にはさらにサーバーの `agent.prompt` が必要で、送信 ACK は処理完了を意味しません。カードが表示されない場合は実際に動いている Herdr CLI／サーバーと `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` を調べてください。バックエンドソケットがキャラクター側の環境／既定値と異なるなら、推測したパスではなく**サーバーが実際に表示したソケットパス**を使用します。

```sh
herdr-desktop-pet --socket /actual/path/to/herdr-api.sock start
herdr-desktop-pet status
```

例のパスを確認済みの Herdr **バックエンド**ソケットに置き換えてください。`--socket` はキャラクター専用の制御ソケットではありません。`start` は実行中のデーモンを再利用するため、バックエンドの変更前には停止してください。`--version` だけでなく `status` で実際のデーモンのバージョンと実行ファイルを確認します。[接続トラブルシューティング](docs/usage.ja.md#設定とトラブルシューティング)を参照してください。

既存インストールの更新前には**古いデーモンを停止**してください。`start` は稼働中のデーモンを再利用します。

```sh
herdr-desktop-pet stop
brew update && brew upgrade herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

CLI が `0.3.6` を報告し、再起動後の `status` に `app_version: 0.3.6` **と想定した実行中の実行ファイル**が表示されることを確認します。formula はアプリを Homebrew のプレフィックス以下に置き、**`/Applications` には置きません**。CLI は `PATH` 上に配置されます。Herdr プラグインやパッチ済みホストは導入されません。古いデーモンは先に停止してください。プロファイルの分離と旧 cask からの移行については [0.3 更新ガイド（英語）](docs/releases/0.3.md#upgrade-from-v021-or-beta)を参照してください。

### Herdr を使わない単独のキャラクター

正常なローカル Herdr サーバーがなくてもキャラクターを表示し続けるには、新規プロファイルで標準で有効な「切断から 30 秒後に終了」を**起動前に**無効にします。

```sh
herdr-desktop-pet settings set exit_with_herdr off
herdr-desktop-pet settings get
herdr-desktop-pet start
herdr-desktop-pet status
```

Herdr 本体、プラグイン、パッチは不要です。起動後、必要なら `herdr-desktop-pet hide_bubble` で空のバブルだけを隠し、キャラクターを残せます。約 30 秒で消えた場合は `settings get` と `status` を確認します。`exit_with_herdr` がオンで正常なサーバーがなければ猶予後に終了します。同じ実際の設定／状態プロファイルで、再起動前にオフにしてください。この設定は登録済みの全ローカルプラグイン接続先が確認済みの無効化／リンク解除となった場合の終了を**無効化しません**。[ライフサイクルの詳細](docs/usage.ja.md#ライフサイクルとプラグイン操作)。

### 任意の上級設定：管理下の Herdr プラグインと接続時の自動起動

対象となる互換性のある Herdr ホストで[プラグインのリポジトリ](https://github.com/hanbong5938/herdr-desktop-pet)をインストールし、明示的にキャラクターを起動します。

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

管理下でのインストールは `bash scripts/install.sh` を実行します。まずマニフェスト指定の v0.3.6 の `.app` 専用アーカイブを試し、チェックサム、アーカイブのエントリー、arm64 署名、同梱キャラクターを検証します。**公開済み v0.3.6 Release の完全なアーカイブ／チェックサムと対応インストーラー**を使用します。別途アップロードされる来歴サイドカーはインストーラーのアーカイブに含まれません。ビルド済み資産が存在しないか無効なら、標準のインストーラーはその旨を報告し、このチェックアウトをソースからビルドします。明示的な `--prebuilt` はフォールバックせず失敗し、`--source` はこのチェックアウトをビルドします。プラグインの導入は Herdr へのパッチ適用も Herdr のビルドも行いません。接続時の自動起動には[こちら（英語）](integrations/herdr/README.md#build-in-separate-checkouts)のパッチ済みホストを用意してください。前述の手動によるネイティブ観測には不要です。

## 管理下の連携用にこのチェックアウトをソースからインストール

**任意の管理下での接続ワークフローの場合**、リポジトリのルートから最初に[ホストのビルド手順（英語）](integrations/herdr/README.md#build-in-separate-checkouts)に従って別のチェックアウトにある公式 Herdr 0.9.3 にパッチを当ててビルドし、分離したサーバーを実行します。`PATCHED_HERDR` は**ビルドしたホストの実行ファイル**に設定します（例：`PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`）。以下のコマンドでは同じ対象ホスト／プロファイル／ソケットを使ってください。`PATH` にある無改変のバイナリは接続時の自動起動には代用できません。キャラクターをソースからビルドするだけならパッチ済みホストは不要です。

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

`plugin link` はマニフェストのビルドを**実行しません**。先にキャラクターをビルドしてください。既に稼働中のホストでリンク／有効化しても、それだけではキャラクターは起動しません。一度 `start` を呼び出すか、`auto_start` を有効にした上で対象となる次の shell／ターミナルクライアントの接続を待ちます。リンク時の警告を確認してください。未知の `client.attached` フックは必要なパッチがホストにないことを示します。Herdr の操作には稼働中で有効なホストが必要です。サーバー起動時にも自動的に `ensure` が実行されます。
マニフェストのライフサイクル／操作コマンドは `scripts/run-native.sh` を使用し、安定版のマニフェストバージョンなら `HerdrDesktopPet.app`、`-beta.N` なら `HerdrDesktopPetBeta.app` を選びます。対応するバンドルがなければ失敗し、別チャネルのバンドルは実行しません。直接ソースから自動化する場合は `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet` を使用します。古いデーモンを停止するか**設定と状態の両方**のパスを分離し、`status` で実際の実行ファイル／バージョンを確認してください。[ソース開発（英語）](docs/development.md#source-development)。


## Tern の導入と独立プロファイル

まずネイティブアプリ、次に独立した公開プラグインを明示的に導入します。プラグインはアプリをダウンロード、ビルド、起動しません。

```sh
brew install hanbong5938/tap/herdr-desktop-pet
tern plugin install https://github.com/hanbong5938/tern-desktop-pet
PET="$(command -v herdr-desktop-pet)"
PET_CONFIG="$HOME/.config/herdr/plugins/config/desktop-pet-tern"
PET_STATE="$HOME/.local/state/herdr/plugins/desktop-pet-tern"
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" stop
"$PET" --version
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" start
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" status
```

既定のホストは Herdr のままです。`--host tern` に Herdr サーバー、ソケット、プラグインは不要です。実際の Tern ウィンドウで **Configure desktop pet connection** に絶対実行ファイル `PET` と上記の設定・状態の両方のパスを指定し、**Connect to desktop pet** を実行します。変数名ではなく実際のパスを入力してください。ウィンドウの接続と同じプロファイルのネイティブ `status` を確認します。プラグインの検出だけでは接続の証明になりません。バイナリやモードを変える前に同じプロファイルを停止します。ソース開発は任意です。現在のチェックアウトで `bash scripts/install.sh --source` を実行し、`PET="$PWD/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"` を指定できます。特別なブランチは不要です。アプリ／ヘルパーは対応する protocol 3 のペアで、追加 JSON 能力は `protocol: 3` と `observation_mode: 1` です。英語・韓国語・簡体字中国語・日本語を維持します。**ベータ更新を受け取る**は既定でオフで明示的な選択が必要です。確認だけではインストールや再起動をしません。

## 初回利用

### アプリ内更新（このチェックアウトではプロトコル 3）

**設定 → アプリの更新**では、独立した**自動確認**（既定でオン）と**ベータ版の更新を受け取る**（既定でオフ）、**今すぐ確認**、実行中のバージョン／由来、ポリシー別の最終確認成功時刻、明示的な操作を表示します。安定版のみの確認では公式安定版を選びます。ベータ版を許可すると、対象となる安定版とベータ版からセマンティックバージョンが最高のものを選びます（同じ基本バージョンなら正式版がベータ版より優先）。ベータ版の許可を取り消しても、インストール済みベータ版をダウングレードしません。アプリ起動中の自動確認は最大でも一日一回で、安定版のみとベータ版を含む場合の時刻を別々に記録します。ベータ版ポリシーを切り替えると以前の候補は無効となり、自動確認がオンの場合だけ新ポリシーで確認します。オフの場合は**今すぐ確認**を使います。確認は読み取り専用です。設定の切り替えや確認だけでインストール、再起動、Brew メタデータの更新、Git の変更はしません。リリース／チャネル／ソースと更新先をまたぐ移動には、正確な提案に対するネイティブ画面での個別の同意が必要です。

カードは進行中の確認／引き継ぎと、保持された**前回の操作**の結果を区別します。**診断を表示**すると選択可能なパス、ハッシュ、ID、生のエラーを展開します。短い状態表示とヘルパーの案内は選択中の UI 言語に従います。候補がディスクにインストール済みでも適用済み・実行中とは限りません。ソースが現行または固定されているという結果は、公開版で最新であることを意味しません。Herdr 管理下での移行では、チェックアウトをリセットせず、同じ互換性のある稼働中のホスト上で公式リリースタグを不変のコミットに固定します。Homebrew は取得済みのマネージャー／プレフィックスを維持し、対象の formula／バージョンを明示します。確認された操作はメタデータを再検証し、両方のインストールルートを予約します。元の formula を黙ってアンインストールしません。ローカルのチェックアウトは Git pull/reset をせずにディスク上のファイルを再ビルドします。検証済みで互換性のあるインストール済みパッケージに限り、インストーラーを再実行せず**インストール済み更新を適用**できる場合があります。不明な由来を暗黙に管理下のインストールへ変えることはありません。

ネイティブ画面での同意と Prepare は、変更可能な操作を凍結する前に、保留中の編集、IME 変換、操作を再確認します。元のアプリは引き継ぎ中も実行を続けます。所有権 ACK はインストールの完了でも Stop の許可でもありません。受理された Prepare の後で、一致する最終の表示済み／準備済み ACK だけが同意を移します。完全に送達され受理された予約済み Stop だけが更新用の終了を確定します。ステージングはワーカーで行われ、ヘルパーのウィンドウは応答を続けます。ネイティブ側の 30 秒の引き継ぎ期限を越えた場合は**待機であり、キャンセルではありません**。ヘルパーは置き換え先の準備状況を確認し、インストール済みと適用済みを別々に記録します。候補の起動には、現在のディスクや元のソーストークンから推測した由来ではなく、ジャーナル／予約内の正確に検証された**イメージとインストール元**の組を使用します。不確実な予約や旧プロトコルの予約は証拠を維持したまま処理を阻止します。復旧時にインストーラーを再実行したり、ジャーナルを密かに削除したり、元のアプリを自動再起動したりしません。利用者による Stop は復活を防ぎますが、安全に停止したと証明済みのトランザクションは整合処理を完了し、自身の予約を解除できます。不確実な操作をやり直す前に[更新動作と復旧](docs/usage.ja.md#アプリ内更新)および [CLI の状態と復旧（英語）](docs/cli.md#app-update-status-and-recovery)を参照してください。

公開済みの安定版 `v0.3.3` と対になる `0.3.3-beta.1` のアーカイブは、不変のアプリ／ヘルパーのプロトコル 2 対応パッケージです。更新を確認しただけで、このチェックアウトのプロトコル 3 のベータ設定が**加わることはありません**。このチェックアウトのソースバージョンやネイティブ CLI のバージョンだけでは、公開済みのプロトコル 3 リリースを証明できません。導入にはこのプロトコル 3 チェックアウトからの明示的なソースビルドか、別途検証済みの公開プロトコル 3 パッケージが必要です。アプリとヘルパーで異なるプロトコルの組み合わせは使えません。従来のプロトコル 1／2 記録には、保存された対応するヘルパーが必要であり、新しい更新機能による変換はできません。公開版は実際の[リリース資産](https://github.com/hanbong5938/herdr-desktop-pet/releases)を確認してください。

## 操作

| 操作 | 結果 |
| --- | --- |
| 頭／体をタップ、頭の上をなぞる | ルベリアが反応し、撫でられます。 |
| キャラクターをドラッグ、右下のグリップを使う | 移動、サイズ変更します。 |
| バブルの背景または辺／角をドラッグ | 付属キャラクターまたは独立したバブルを移動し、バブルを個別にサイズ変更します。 |
| メニューバーのアイコンを左クリック | キャラクター、観測、バブル、ライフサイクルの設定を開きます。 |
| バブルを開き、ローカルエージェントカードを選択 | 一行返信を開きます。Enter、Command+Enter、送信ボタンで送信します。リモートカードは読み取り専用です。 |

IME 変換中は**決して送信されません**。Escape はまず変換を取り消します。検索欄で Enter／Command+Enter を押しても返信は送信されません。キャラクターとバブルの表示は独立しており、両方を隠すかウィンドウ全体をクリック透過にすると、デスクトップ上のクリックで操作できなくなる場合があります。その際はメニューバーのアイコンから**キャラクターを表示して有効化**する復旧操作を使用します（標準モードでは常にアイコンを表示し、復旧時のみのモードでは必要なときに表示）。すべての操作、検索、保存済みマシン経由のリモート観測、表示動作は[利用ガイド](docs/usage.ja.md#操作)を参照してください。

## 追加キャラクターと衣装

[キャラクターギャラリー](https://hanbong5938.github.io/herdr-characters/)では追加パックをプレビューでき、[別リポジトリ](https://github.com/hanbong5938/herdr-characters)にソースと利用条件があります。設定の**キャラクター → キャラクターを探す…**を選んでください。肖像画を見るだけではアーカイブはダウンロードされません。対象となる公開済み公式パックには明示的で検証済みの**ダウンロードして適用**操作があります。ローカルフォルダーや `.herdrchar` の場合、**読み込み**だけでは有効化されません。インストール済みカード／リビジョンを選び、**適用**をクリックしてください。同梱・標準キャラクターはルベリアだけです。[キャラクターパック](docs/usage.ja.md#キャラクターパック)と[制作方法（英語）](docs/development.md#character-authoring)も参照してください。

## 安全上の注意と制限

- 返信に使うのは選択した**ローカル** Herdr ソースの `agent.prompt` のみであり、SSH、shell、ペインへの生入力は使いません。リモートカードからは送信できません。Herdr の ACK は送信を意味し、**エージェントの処理完了ではありません**。承認や質問は実際のターミナルで扱ってください。手動で再送信する前に、不確実な送達状況を確認します。送信前後のセッション／ペイン置換には現在の利用者についての競合があり、セッションの原子的な保証はありません。[送達の詳細](docs/usage.ja.md#インライン返信とメッセージ配送)。
- **Worktree を削除…**は元に戻せません。続行前に**正確なチェックアウトパス**と**ワークスペース全体**のタブ／ペインを確認してください。そこにあるプロセス／エージェントを終了し、**無視されているファイルも含めて**リンク済みチェックアウトを削除します。ゴミ箱への移動や取り消しはできません。メインのリポジトリとブランチは削除しません。[削除時の安全策](docs/usage.ja.md#ワークツリーの削除)。
- リモート観測には、選択済みの保存されたマシン設定と互換性のあるリモート Herdr サーバーが必要ですが、**リモート側へのキャラクターのインストールは不要**です。ローカルアプリはリモートの状態をポーリングするため、見逃した短い状態遷移を復元したり、リモートのプロンプトを送ったりはできません。[観測ガイド](docs/usage.ja.md#観測元)。
- アプリ更新の自動確認は読み取り専用です。更新のインストール／適用には明示的な同意が必要です。更新機能の所有権 ACK は**完了を意味しません**。結果が不明な場合は再試行前に[アプリ内更新の復旧](docs/usage.ja.md#アプリ内更新)と [CLI 更新状態と復旧（英語）](docs/cli.md#app-update-status-and-recovery)を参照してください。

## 設定とトラブルシューティング

`HERDR_PLUGIN_CONFIG_DIR` には `preferences.json`、`lifecycle.json`、管理対象の `characters/` と `menu-bar-icons/`、`HERDR_PLUGIN_STATE_DIR` にはロック、制御ソケット、`desktop-pet.log` が保存されます。パスを注入していない場合は Herdr の XDG プラグイン用パスが使用されます。ネイティブの `--config-dir` / `--state-dir` でプロファイルを分離できますが、環境変数で注入したパスが優先します。更新やチャネル切り替え前に、実際の**両方のディレクトリ**をバックアップしてください。`lifecycle.json` は他の設定とは別に `auto_start` と `exit_with_herdr` を制御します。

サーバーなしで起動して約 30 秒後にキャラクターが消えた場合、`settings get` で `exit_with_herdr` を確認し、単独使用するなら**再起動前に**オフにしてください。ただし登録済みのすべてのローカル接続先が確認済みの無効化／リンク解除となれば今回の実行は終了します。隠れているだけなら再インストールせず、前述のメニューバー操作で復旧してください。管理下のホストで表示されない場合は `client.attached` の警告、手動での `start`／`status`、実際の実行ファイル／バージョン、デーモンログを確認します。自動起動を無効にしていれば意図的に起動されない場合があります。接続エラーでは `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` とバックエンドソケットを実際に所有するサーバーを、リモートエラーでは保存済み設定とローカル CLI を確認します。キャラクターは**表示させたい Mac**上で起動してください。プロバイダー連携は上流の Herdr に属し、そのフックをインストールした後は既存のプロバイダーセッションを再読み込みします。[トラブルシューティング](docs/usage.ja.md#設定とトラブルシューティング)と[ライフサイクル](docs/usage.ja.md#ライフサイクルとプラグイン操作)を参照してください。

## 資料とウェブサイト

- [使い方と復旧](docs/usage.ja.md) · [開発とインストール（英語）](docs/development.md) · [ネイティブ自動化 CLI 契約（英語）](docs/cli.md#english) · [パッチ済みホスト連携（英語）](integrations/herdr/README.md)。
- [リリース履歴（英語）](docs/releases/README.md) · [v0.3.6 の変更点（英語／韓国語）](docs/releases/v0.3.6.md) · [v0.3 更新ガイド（英語）](docs/releases/0.3.md#upgrade-from-v021-or-beta) · [過去のベータ／プレビュー案内（英語）](docs/migrations/unreleased.md) · [v0.1.11 → v0.2.0 移行（英語）](docs/migrations/v0.1.11-to-v0.2.0.md)。
- 宣伝用の[日本語](https://hanbong5938.github.io/herdr-desktop-pet/ja/)・[韓国語](https://hanbong5938.github.io/herdr-desktop-pet/)・[英語](https://hanbong5938.github.io/herdr-desktop-pet/en/)・[簡体字中国語](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)ウェブサイトはこの**ネイティブ**アプリを紹介しています。ブラウザー内でアプリを動かすものではありません。ウェブサイトの開発／公開は [GitHub Pages ガイド（英語）](docs/development.md#website-github-pages)を参照してください。別の[キャラクターギャラリー](https://hanbong5938.github.io/herdr-characters/)では追加パックを紹介しており、対応言語は英語と韓国語のみです。

## アートとライセンス

[ソフトウェアライセンス](LICENSE.txt)とアート／モデルの利用条件は**別物**です。MIT ソフトウェアライセンスによってキャラクターのアートが再ライセンスされることはありません。同梱ルベリアの[ライセンス](assets/rubelia-default/LICENSE.txt)、[帰属表示](assets/rubelia-default/ATTRIBUTION.txt)、[出典記録](assets/rubelia-default/source-record.json)、[背景修復記録](assets/rubelia-default/background-repair.json)が正式な情報です。`LICENSE.txt` には元の Coding Cat の告知も残っています。そのアート／ソースは[キャラクターリポジトリ](https://github.com/hanbong5938/herdr-characters)へ移されました。各追加パック独自の条件を必ず保持してください。アート所有者の承認はモデルの権利を与えません。Qwen の研究用ライセンスは制作記録であり、モデルのエンジンや重みは同梱されていません。[キャラクター制作とライセンス（英語）](docs/development.md#character-authoring)を参照してください。
