# Herdr 연동 개발

[English](development.md) · **한국어** · [연동 개요](../readme.ko.md)

앱·UI·rig·캐릭터 도구·그림·빌드·패키징·앱 사이트는 독립 [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) 소유입니다. 앱은 별도 core 체크아웃에서 설치·빌드하고 Herdr에는 **이 연동 체크아웃**을 link하세요. `plugin link`는 앱을 빌드하지 않습니다.

[보존된 Herdr 연동 안내](../integrations/herdr/README.md)는 21개 action, adapter 실행 정책, 두 프로필 경로 우선순위, 별도 호스트/core/연동 체크아웃, 공식 Herdr 0.9.3 패치, 선택형 client-attach 훅과 OMP 확장을 설명합니다. 순정 0.9.0/0.9.3은 attach 훅을 제공하지 않으며 플러그인 설치는 호스트를 패치하지 않습니다.

- [공통 앱 개발·설치](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) · [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#한국어)
- [캐릭터 제작·라이선스](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#캐릭터-제작)
- [Tern Connect·Configure](https://github.com/hanbong5938/tern-desktop-pet/blob/main/README.ko.md) · [이전 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

이 저장소의 `site/`는 네 언어의 연동·이전 입구만 포함하며 앱 화면이나 캐릭터 자산을 복제하지 않습니다. Pages workflow는 정적 파일을 직접 stage하고 main만 배포합니다. 공통 앱 사이트 도구·Wiki 생성/게시 절차는 core의 [Pages 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#웹사이트-github-pages)·[Wiki 게시 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/publishing.md)에 있습니다. 공개 상태와 실제 배포 URL은 별도 확인 대상입니다. 실제 Tern 재연결·OMP 모델 전달 성공을 주장하지 않습니다.

## 현재 consumer 기록 — 2026-10-15

현재 consumer 스크립트와 실제 materialize한 metadata/증거8개는 공개 [Core v0.4.7](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.7), Release `413584934`, 정확한 소스 `52514da5115f09aac22de5429c08f7acdd993258`의 중립 protocol4/observation2/API1을 사용합니다. installer SHA-256은 `f64a27593058a68c5a5e112f448be644c6abfbb760be109acf992e2f415f5829`, checksum asset은 `2619293634c27cfc6cbb7db11c9dd8407ac719b3e2c4321d6a6dc5cf21937d5e`, release record는 `0643094bc94adad0ad3a18120a3a69c2688900889ee9f34befe8f94e31186923`입니다. 권위는 실제 태그/소스와 검증된 공개 바이트이며 advisory `target_commitish`가 아닙니다.

진짜 [bridge v0.3.11](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.11), Release `413623057`, metadata 소스 `fa56caa32ea06d86932c5b4cc3eb230010c6dd9c`는 같은 정확한 Core 소스를 legacy3/1/API1로 빌드했습니다. metadata11개·정확한 생성 앱 ignore·실제 logical-`/tmp` native/Git 검증과 두 공개 자산 집합·엄격 서명·68파일 closure가 통과했습니다. stable tap `6693cd9db26e6dba9a257bbe76acaf78cb502c2f`는 Core4.7/legacy3.11이며 beta는 불변입니다. catalog refresh는 앱 upgrade가 아닙니다.

연동 manifest/native adapter는4.1/API1 그대로입니다. 실제 materialize한 install/status는 이전 완료 공개 Standalone4.7 PID2577 / instance `5e4f956f0709497a5f614c24995a8cd7`와 원래 명시 프로필·H1/T0을 설치·시작·native build·Herdr 재설치 없이 재사용했습니다. 실제 Mac native 승인과 main/Pages 관측은 정본 원장에서 구분합니다. Wiki job116329481248은 전용 `WIKI_PUBLISH_TOKEN`이 필요하며 broad fallback·binary 재실행은 없습니다.

실제 공개 Brew3.10→일반3.11 A′는 원래 helper로 정착했고, 독립 동의 B4.7은 readiness/Commit·실제 소유 Brew retirement와 원래 바이트·가져온 캐릭터·대사·이력·무관 프로필 보존을 완료했습니다. 4개 언어 취소와 rollback/Stop/Unknown/확정 실패/native Resume 안전성도 실제 검증했습니다. 동결3.6/beta 경로와 과거 helper/journal/reservation/Unknown은 별도로 보존하며 replay·재분류하지 않습니다. [정본 수락 원장](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.7.md) · [공개·materialization 기록](releases/publishing.md#current-consumer-record--2026-10-15).

## 과거 배포·런타임 증거

아래 v0.4.5/v0.3.10의 “현재”는 당시 스냅샷입니다. 소스·해시·관찰을 현재 버전으로 재지정하지 않습니다.

**현재 실제 권위:** 공개 [Core v0.4.5](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.5), 정확한 `51dcf22415b1ff0faf07651cb83c1c696c980f43`, Release `412667385`, 중립4/2/API1에서 실제 consumer 증거를 materialize했습니다. 의도적인 진짜 [bridge v0.3.10](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.10)는 metadata 소스 `67eb44b53bda6ba96eaf68671b67a94cab7b304d`, Release `412684474`, 같은 공용 Core의 legacy3/1/API1입니다. Git은 metadata11개·정확한 `/dist/HerdrDesktopPet.app/` ignore만 소유하며 실제 native/tmp/Git 보호·두 공개 자산 집합·엄격 서명·68파일 closure를 검증했습니다. Herdr v0.4.1 `6d2ac04`·Tern0.2.0·동결·beta는 그대로입니다.

**실제 런타임·한계:** fresh Brew4.5 설치/read-only 검사는 별도 standalone 앱·프로필·데몬을 만들지 않았으며 명시적 Auto/H0T0 Start와 같은 Hidden default1971을 확인했습니다. 실제4.4→4.5 일반 갱신 a699/Brew9853/exit0/drained와 막힌4.3의 명시적 동일 owner 복구 모두 구 이력 보존·Herdr 재설치 없이 실제4.5 ready·정상 Check로 끝났고 현재 consumer Hide→ensure는10377을 유지했습니다. 새 managed3.6→3.10 A20cca는 원래 서명3.6 helper에서 completed입니다. Terminal Check·별도 Move preview·독립 동의 B6ee850aa5577314c6d9cf6ebc7eae981가 기존4.5 PID3594/2ee0995b8b3dc3abe18e6238a91929d9/정확한 구 config/state/저장 Herdr/H1T0/UI-control ready/Commit으로 완료됐습니다. 진짜 Herdr PID=PGID3648/exit0/quiescent=true가 정확한6d2로 갱신했고 registry/checkout clean·legacy 앱 제거를 확인했습니다. A87개와 네 트리의 모든 기존 파일은 바이트 불변이고 원래 A helper도 completed/default1971도 같은 Hidden instance입니다. 이전3.9/4.4·원래3.8 실패/3.7 Unknown·실제3.9 beta matched-pair는 독립 기록입니다. Pages·실제 SDK 양쪽 순서/prompt/transcript/reconnect는 별도 검증이며 Tern 정상 keychain/OMP 로그인은 없습니다. Core4.5 release는 통과했고 Wiki115840956329는 렌더 후 전용 토큰 부재로 게시 실패입니다. Wiki Home/전용 토큰이 필요하며 broad fallback은 없습니다. [현재 원장](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md).

[v0.3.6 개발 문서](https://github.com/hanbong5938/herdr-desktop-pet/blob/v0.3.6/docs/development.ko.md)는 동결된 과거 경로이며 현재 설치 지침이 아닙니다.
