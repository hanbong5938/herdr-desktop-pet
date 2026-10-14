# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**현재 공개 배포:** [Core v0.4.5](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.5), Release `412667385`, `2026-10-14T12:01:32Z`, 정확한 공용 소스 `51dcf22415b1ff0faf07651cb83c1c696c980f43`, protocol4/observation2/API1입니다. 현재 consumer는 실제 공개 installer 바이트를 인증합니다. 의도적인 legacy ingress는 진짜 [bridge v0.3.10](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.10), Release `412684474`, `2026-10-14T12:20:45Z`, metadata 소스 `67eb44b53bda6ba96eaf68671b67a94cab7b304d`를 선택합니다. 같은 Core 소스로 빌드한 실제 legacy3/1/API1이며 Git에는 regular UTF-8 metadata 11개와 정확한 생성 앱 ignore만 있습니다. 두 공개 자산 집합·엄격 서명·68파일 closure를 검증했습니다. 구 stable formula `7bf1fd7f1b89bd77e68179fcee95cacf044927dc`는 v0.3.10입니다. Herdr **v0.4.1**의 불변 `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8`·assetless Release `412252863` 및 Tern v0.2.0·호환 Core pin은 그대로입니다. 새 연동 태그·Tern 네이티브 설치/자동 시작·이전 태그/본문/자산/beta 교체는 없습니다.

**현재 Core의 실제 런타임:** fresh public Brew v0.4.5 설치·read-only 검사는 별도 standalone 앱·데몬·중립 프로필을 만들지 않았습니다. 명시적 Start 뒤 실제 GUI는 Desktop Pet / Auto / Herdr Waiting0 / Tern Waiting0이었고 PID `1971` / instance `27752246b4da5c6f3964a74dd22881a2`는 명시적 Hide 후에도 같은 인스턴스로 실행 중입니다. 별도 기존 프로필의 실제 v0.4.4→v0.4.5 일반 갱신 `a6991e69233377c32b150186c62888fd`는 진짜 Brew PID=PGID `9853`, exit0/drained true로 완료됐습니다. 기존 기본 프로필은 공유 owner 교체 전에 명시적으로 Stop했으며 몰래 재시작·병합하지 않았습니다. 새 PID `10377` / instance `37953e017fc9208bfcc785a4ff138494`는 같은 구 프로필·Herdr1/Tern0·UI/control ready였고 다음 실제 Check도 초록색 정상 결과였습니다. 기존 증거와 프로필은 daemon log 외에 불변이며 Herdr 재설치는 없었습니다. 막힌 v0.4.3은 서명된 원본 백업·명시적 Stop·실제 고정 installer·명시적 Start로 같은 owner만 복구했고 실제 v0.4.5 Check도 정상 통과했습니다. 구 checker의 자동 갱신은 아닙니다. 이전 공개 v0.3.9/v0.4.4 A/B·87개 증거·무관한 기본 프로필 보존은 [당시 원장](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.4.md)에 별도 유지하며 새 조합의 성공으로 바꾸지 않습니다.

**새 공개 managed A→B 완료:** 실제 native 동결3.6→공개3.10 A `20cca91548584437af112bb04cc260b0`의 terminal Check·별도 Move preview 뒤 독립 protocol4 동의 B `6ee850aa5577314c6d9cf6ebc7eae981`가 기존 Brew4.5 PID3594 / instance2ee0995b8b3dc3abe18e6238a91929d9로 완료됐습니다. 정확한 구 config/state·저장 Herdr/H1T0·실제 UI/control ready/Commit을 확인했습니다. 진짜 Herdr PID=PGID3648/exit0/quiescent=true가 정확한6d2로 갱신했고 Git clean·legacy 앱 제거를 확인했습니다. 원래 A87개 파일과 네 프로필/증거 트리의 모든 기존 파일은 바이트 불변이며 원래 서명3.6 helper도 여전히 completed입니다. 무관한 default1971은 Stop·병합 없이 같은 instance로 Hidden 실행 중입니다. [현재 원장](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md).

**과거 실패는 그대로 보존:** 실제 Brew v0.3.6 → v0.3.7 → Core v0.4.2 A→B 성공과 managed v0.3.7 A의 논리 `/tmp` 오류는 별개입니다. 후자는 **Unknown**으로 보존하며 복구·재실행·reset하지 않습니다. 원래 공개 v0.3.8 A는 성공했지만 B preflight는 생성 앱의 untracked 파일 정확히 68개로 B 동의·Stop 전에 거절됐습니다. `.gitignore` 없는 원래 metadata 10개 태그를 성공으로 바꾸지 않습니다. 공개 v0.3.9는 정확한 `/dist/HerdrDesktopPet.app/` 소유 선언만 추가합니다. 실제 CI의 native `/tmp`·생성 앱 Git gate는 통과했고 tracked script 수정·무관한 root 파일·`dist/user-owned-note.txt`는 계속 dirty입니다. 수동 ignore·사용자 파일 clean/reset·dirty 보호 완화는 이전 절차가 아닙니다.

**별도 검증과 한계:** 실제 public3.9 beta 경로는 live 교체·protocol2 직접 bridge를 거절하고 원래 동결3.6 matched pair를 자동 시작 없이 설치했습니다. 같은 beta config/state의 명시적 Start·실제 ready와 원래 서명 helper·사용자 소스/프로필 보존을 검증한 뒤 소유3.6만 Stop했습니다. 새3.10 beta 실행 주장은 아닙니다. f7edf5e의 확정 retirement 실패·새 동의 동일 plan resume은 과거 별도 증거입니다. Pages와 실제 SDK 양쪽 순서/prompt/transcript/reconnect는 독립 검증이며 Tern 정상 keychain과 OMP provider SignIn/NoModel이 없어 완료를 주장하지 않습니다. 계정·ACK·transcript를 꾸미지 않습니다.

필요한 연동만 설치하세요. Herdr는 얇은 consumer이며 Tern은 네이티브 앱을 다운로드·빌드·설치·자동 시작하지 않습니다. 플러그인 갱신·제거는 연동만 바꾸고 공용 앱·프로필·캐릭터는 남깁니다. 일반 앱 업데이트는 Herdr 플러그인을 재설치하지 않습니다.

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.ko.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 앱 사이트](https://hanbong5938.github.io/desktop-pet/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 연동 사이트](https://hanbong5938.github.io/herdr-desktop-pet/). 이전 사이트는 앱 UI·그림을 복제하지 않는 연동·이전 입구입니다.

Core v0.4.5 CI `38560075408`의 release job `115838673595`와 진짜 bridge CI `38561115955` job `115842136732`는 통과했습니다. 실제 native 논리 `/tmp`·생성 앱 Git 보호도 통과했습니다. 뒤따른 Wiki job `115840956329`는 렌더 후 전용 `WIKI_PUBLISH_TOKEN` 부재로 게시에 실패했습니다. 소유자 Home 초기화·전용 자격 증명이 필요하며 broad `gh`/`GITHUB_TOKEN` fallback은 없습니다. 현재 Core/Herdr Pages는 소스·링크만으로 배포 완료를 주장하지 않습니다.

공개 중립 formula는 Core v0.4.5를 선택합니다 ([tap `d66411383141644f01f1ca839086b6d505a997ac`](https://github.com/hanbong5938/homebrew-tap/commit/d66411383141644f01f1ca839086b6d505a997ac)). 공용 앱과 필요한 main 연동만 설치하세요:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac, macOS 13 이상. 소스 빌드는 Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools가 필요하며 사전 빌드 설치에는 필요 없습니다. 앱은 ad-hoc 서명·미공증입니다. Herdr 스냅샷에는 호환 로컬 서버(0.9.0 이상), 답장에는 별도 `agent.prompt`가 필요합니다. 선택형 client-attach 자동 시작에는 공식 Herdr 0.9.3에 [연동 저장소의 패치와 호스트 빌드 절차](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)를 적용하거나 실제 훅 지원 호스트를 사용해야 합니다. 순정 0.9.0/0.9.3에는 훅이 없고 플러그인 설치는 호스트를 패치하지 않습니다.

사전 빌드·소스 설치는 Herdr 연동이 아니라 **core** 체크아웃에서 실행합니다. 기본 설치는 prebuilt-only이며 다운로드·검증 실패를 소스 빌드로 숨기지 않습니다. `--source`는 현재 core 체크아웃만 빌드하고 그 `dist/DesktopPet.app`을 사용합니다.

## Herdr consumer 계약

이 저장소는 `DesktopPet.app`, 네이티브 패키지나 캐릭터 자산을 빌드·복사하지 않습니다. 연동 manifest·wrapper·adapter는 이 체크아웃, 네이티브 소스·그림·도구는 Core 소유입니다. 현재 `core-release.json` / `release-evidence/`는 실제 공개 Core v0.4.5 Release `412667385`, 정확한 `51dcf22415b1ff0faf07651cb83c1c696c980f43`에서 materialize했습니다. installer SHA-256은 `fdcd9d80f8672b54ceca967bfea6f72223d2e5caed97f845140a68b9e7414a16`, checksum 자산 SHA-256은 `8c9c8ab020bc4561c4a2c17f4c6d20b3e8a18672b84f508426e68ed259598c5e`입니다. [실제 권위와 런타임 증거](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)는 불변 v0.4.1 연동 태그의 당시 Core v0.4.2 최소 pin을 바꾸거나 재게시하지 않습니다.

정확히 호환되는 공용 앱의 탐색 순서는 명시적 절대 실행 파일 → 검증된 실제 계정 홈의 공용 locator → PATH의 네이티브 `desktop-pet` → 실제 Brew prefix → 실제 계정 홈의 `Applications/DesktopPet.app`입니다. 잘못된 명시 경로나 locator는 다른 설치로 우회하지 않습니다. 앱이 없을 때만 고정·digest 검증된 **core** installer로 bootstrap하며, 비호환 설치는 명시적으로 네이티브 갱신·복구해야 합니다. 여기서 제품 빌드나 조용한 fallback을 하지 않습니다.

Herdr 프로필 우선순위는 주입된 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 명시적 `--config-dir` / `--state-dir` → Herdr XDG/HOME 기본값입니다. wrapper는 두 경로를 core에 명시적으로 전달하고 프로필을 이동·병합하지 않습니다. 백엔드 출처는 `--herdr-socket ABS`로 전달하며 이전 `--socket`이나 pet의 전용 제어 소켓과 다릅니다. `status`는 CLI만이 아닌 실제 실행 중 이미지·프로필을 표시합니다.

[Herdr 안내](integrations/herdr/README.md)는 기존 21개 action ID와 표시·말풍선·수명주기 의미, 호스트 빌드·패치, link 및 OMP 절차를 보존합니다. `ensure`는 멱등적이며 숨겨진 pet을 표시하지 않습니다. 선택형 attach 자동 시작에는 실제 훅 지원 호스트가 필요합니다. lifecycle 파일이 처음부터 없으면 Herdr 종료 정책은 true, 기존 파일에서 키만 없으면 false입니다. 명시적 정책과 알 수 없는 필드는 쓰기 시 보존됩니다. Tern은 별도 consumer이며 공용 앱을 자동 시작하지 않습니다.

## 소스 / link

먼저 **desktop-pet Core** 체크아웃에서 `bash scripts/install.sh --source`로 앱을 설치·빌드한 다음 Core가 아닌 **이 연동 체크아웃**을 link합니다. `plugin link`는 빌드하지 않습니다. 현재 consumer는 실제 Core v0.4.5를 고정하고 실제 같은 4.5 인스턴스의 Hide→ensure 보존을 검증했습니다. 과거 태그 v0.4.1 / Core v0.4.2의 동시 fresh 공개 bootstrap·동시 wrapper 증거는 별도 기록이며 새 연동 태그를 만들지 않습니다. [분리된 체크아웃](integrations/herdr/README.md#build-in-separate-checkouts)을 참조하세요. 플러그인 설치는 Herdr를 패치하지 않습니다.

공통 [사용법](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ko.md), [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md), [개발](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) 및 네이티브 기능·provider 상세는 독립 core 소유입니다.

## 소유권

이 저장소는 adapter·manifest·얇은 wrapper·Herdr/OMP 연동·연동 검증·불변 과거 기록을 소유합니다. 앱·UI·rig·그림·패키징·소스 빌드는 [desktop-pet](https://github.com/hanbong5938/desktop-pet) 소유입니다. `.herdrchar`, pack v5, `default@0`, rig ABI 및 [herdr-characters](https://github.com/hanbong5938/herdr-characters) 권위는 그대로입니다. 공개 본문·태그·자산은 불변입니다. 수정 v0.4.1은 전체 filesystem/index/archived-tree 소유권 검사를 통과했고 네이티브 bundle/archive/rig 제품을 포함하지 않습니다.

## 그림 및 라이선스

소프트웨어 MIT 라이선스와 그림·모델 사용 조건은 서로 다릅니다. 기본 루벨리아의 [라이선스](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)·[출처](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)·[제작 기록](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)·[배경 보정 기록](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json), 원본 Coding Cat 고지가 남은 [LICENSE.txt](LICENSE.txt)를 각각 확인하세요. 그림 소유자 승인은 모델 권리를 부여하지 않으며 소프트웨어 MIT가 그림을 재라이선스하지 않습니다. Qwen 연구 라이선스는 제작 기록에 보존되고 모델 가중치·엔진은 번들에 없습니다. 가져온 팩의 라이선스·출처·사용 조건도 함께 보관하세요. [그림 제작 상세](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#캐릭터-제작).
