# Herdr 連携の使い方

[English](usage.md) · [한국어](usage.ko.md) · [日本語](usage.ja.md) · [简体中文](usage.zh-Hans.md)

現在のネイティブアプリの操作・キャラクター・プロファイル・接続・復旧は独立 Desktop Pet core の所有です。

- [現在のアプリ利用ガイド](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ja.md) · [アプリサイト](https://hanbong5938.github.io/desktop-pet/ja/)
- [Core CLI（英語）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#english) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki)
- [Herdr action・hook・lifecycle・ホスト patch・link・OMP](../integrations/herdr/README.md)
- [既存 A → B 移行](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)
- [Tern Connect / Configure](https://github.com/hanbong5938/tern-desktop-pet)

アプリは core installer で別途導入するか、実際の formula 公開後に `brew install hanbong5938/tap/desktop-pet` を使ってください。任意の連携は `herdr plugin install hanbong5938/herdr-desktop-pet` と `tern plugin install https://github.com/hanbong5938/tern-desktop-pet` です。Herdr は共有アプリを利用し、Tern はアプリを導入・自動起動しません。実際の config/state 両パスを選択し、移動・統合しません。プラグインの削除はアプリ・プロファイル・キャラクターを残します。

[公開状態](../readme.ja.md): リンクやコマンドは実際の Release・formula・Pages・Wiki 公開を証明しません。実際の Tern/OMP 受入を主張しません。凍結された [v0.3.6 文書](https://github.com/hanbong5938/herdr-desktop-pet/tree/v0.3.6/docs)と[過去リリース](releases/README.md)は歴史的記録で、現在の導入手順ではありません。
