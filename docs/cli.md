# Herdr integration CLI entrypoint

## English

The current native `desktop-pet` CLI contract is owned by [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#english), not this integration. Install core separately; the normal app formula is `brew install hanbong5938/tap/desktop-pet` after actual publication. Herdr's optional consumer installation is `herdr plugin install hanbong5938/herdr-desktop-pet`.

[Herdr integration commands](../integrations/herdr/README.md) retain all 21 action IDs, wrapper invocation, exact executable/config/state selection, adapter-owned `--herdr-socket ABS`, lifecycle, hooks, patch, link and OMP setup. Invoke actions with `herdr plugin action invoke ACTION --plugin desktop-pet`. Core generic commands use the actual selected executable and both profile paths. Legacy `--socket` is not a neutral alias or the pet's private control socket.

[Migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md) · [Tern Connect / Configure](https://github.com/hanbong5938/tern-desktop-pet) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki). Historical [v0.3.6 CLI](https://github.com/hanbong5938/herdr-desktop-pet/blob/v0.3.6/docs/cli.md) remains frozen; it does not describe the neutral app. [Publication boundary](../readme.md) applies; command text does not establish live release or integration acceptance.

## 한국어

현재 네이티브 `desktop-pet` CLI 계약은 [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#한국어) 소유입니다. 실제 게시 후 앱은 `brew install hanbong5938/tap/desktop-pet`, 선택형 Herdr 연동은 `herdr plugin install hanbong5938/herdr-desktop-pet`으로 각각 설치합니다. [Herdr 안내](../integrations/herdr/README.md)는 기존 21개 action·wrapper·실행 파일/두 프로필 경로·`--herdr-socket ABS`·실행 정책·훅·패치·link·OMP 절차를 보존합니다. 실제 Tern/OMP 수락을 주장하지 않습니다.

## 日本語

現在のネイティブ CLI は [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#english)、Herdr の action・hook・patch・link・OMP は[連携ガイド](../integrations/herdr/README.md)を参照してください。[アプリ利用ガイド](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ja.md)と[移行](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)は core の所有です。

## 简体中文

当前原生 CLI 由 [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#english) 维护；Herdr action、hook、patch、link 与 OMP 见[集成指南](../integrations/herdr/README.md)。[应用使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md)与[迁移](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)属于 core。
