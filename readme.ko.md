# Herdr Desktop Pet

[Herdr](https://herdr.dev)용 네이티브 macOS 데스크톱 캐릭터입니다. 루벨리아가 터치에 반응하고 관측한 세션 상태를 말풍선에 표시하며 선택한 로컬 에이전트에게 메시지를 제출할 수 있습니다.

[English](readme.md) · **한국어** · [简体中文](readme.zh-Hans.md) · [日本語](readme.ja.md) · [버전 문서](docs/releases/README.ko.md) · [릴리스](https://github.com/hanbong5938/herdr-desktop-pet/releases)

<img src="assets/rubelia-thumbnail.png" alt="기본 데스크톱 캐릭터 루벨리아" width="220">

[선택 캐릭터·의상 갤러리](https://hanbong5938.github.io/herdr-characters/) (처음에는 영어; **KO** 버튼 선택) · [캐릭터 저장소·출처](https://github.com/hanbong5938/herdr-characters). 기본 번들은 **루벨리아 `default@0` 하나뿐**이며 갤러리의 팩은 별도로 설치합니다.

이 체크아웃의 안정판 **v0.3.5는 소스 및 기본/사전 빌드 대상**입니다. 소스 버전만으로 공개 바이너리·설치기·Homebrew 포뮬러 게시를 주장하지 않습니다. 사전 빌드 전 실제 [Release 자산](https://github.com/hanbong5938/herdr-desktop-pet/releases)과 설치 경로를 확인하세요. [v0.3.5 한영 노트](docs/releases/v0.3.5.md#한국어). 채린은 선택형 팩이며 내장 루벨리아의 대체물이 아닙니다.

## 주요 기능

- 메뉴 막대에서 제어하는 네이티브 캐릭터: 터치·쓰다듬기·이동·크기 변경, 시스템 언어 또는 한국어/영어/중국어 간체/일본어 UI.
- 관측한 로컬·원격 세션의 상태 카드와 검색·정렬; 원격 카드는 읽기 전용.
- 로컬 에이전트 카드의 한 줄 답장과 별도 말풍선 표시·크기·대사 설정.
- 공식 캐릭터 브라우저의 선택형 팩 다운로드·적용 및 로컬 팩 가져오기.
- 실행 관리·관찰 대상 설정과 출처별 명시적 앱 업데이트; 자동 업데이트 확인은 읽기 전용.

## 요구 사항

- **Apple Silicon Mac, macOS 13 이상.** Intel Mac·Windows·Linux 네이티브 배포는 지원하지 않습니다.
- 로컬 세션 스냅샷은 기존 로컬 Herdr 서버(0.9.0 이상)에서 플러그인·호스트 패치 없이 관찰할 수 있습니다. 답장은 해당 서버의 `agent.prompt` 지원이 별도로 필요합니다. **선택형 클라이언트 attach 자동 시작에만** 공식 Herdr 0.9.3 소스의 [제공된 패치](integrations/herdr/client-attached.patch)로 빌드한 호스트(또는 실제 훅 지원을 명시한 향후 릴리스)가 필요합니다. 순정 Herdr 0.9.0/0.9.3에는 `client.attached`가 없으며 매니페스트 최소 버전도 이를 보장하지 않습니다. 관리형 설치에는 [호스트 빌드·확인 안내 (English)](integrations/herdr/README.md#build-in-separate-checkouts)를 따르고 알 수 없는 훅 경고를 성공으로 간주하지 마세요.
- 소스 빌드 및 사전 빌드 실패 후 기본 설치의 소스 전환에는 Rust/Cargo, Bun, Node.js/npm, Xcode Command Line Tools가 필요합니다. 사전 빌드 성공 경로에는 이 빌드 도구가 필요하지 않습니다.

## Homebrew로 설치: 첫 연결

[개인 tap](https://github.com/hanbong5938/homebrew-tap)과 비초안 v0.3.5 arm64 압축 파일·체크섬이 공개되어 있습니다. 설치 전 실제 [릴리스 자산](https://github.com/hanbong5938/herdr-desktop-pet/releases)과 포뮬러를 확인하세요. 캐릭터를 표시할 Mac에서 기존 로컬 Herdr 서버를 실행한 상태로:

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

네이티브 `start`는 수동 시작이며 `herdr plugin install`·`plugin link`·패치 호스트가 필요하지 않습니다. 말풍선을 펼쳐 해당 서버의 로컬 세션 카드(Herdr 0.9.0 이상 스냅샷)를 확인하세요. 실시간 카드에는 실제 에이전트 세션이 있어야 합니다. 답장은 해당 서버의 `agent.prompt`도 필요하고 제출 ACK는 완료가 아닙니다. 카드가 없으면 실제 Herdr CLI·실행 중인 서버와 `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`를 확인하세요. 백엔드 소켓이 앱의 환경/기본값과 다르면 서버가 실제로 출력한 **확인된 소켓 경로**를 사용하고 추측하지 마세요:

```sh
herdr-desktop-pet --socket /actual/path/to/herdr-api.sock start
herdr-desktop-pet status
```

`/actual/path/to/herdr-api.sock`은 실제 Herdr **백엔드** 소켓 경로로 바꾸세요. `--socket`은 앱의 비공개 제어 소켓이 아닙니다. 실행 중 데몬을 `start`가 재사용하므로 백엔드 변경 전 종료하고, `--version`뿐 아니라 `status`로 실제 데몬 버전·실행 파일을 확인하세요. [연결 문제 해결](docs/usage.ko.md#설정과-문제-해결).

기존 데몬은 업그레이드 **전에 종료**해야 합니다. `start`가 이전 데몬을 재사용할 수 있으므로 CLI 버전 외에 재시작 후 `status`의 `app_version`과 실제 실행 파일도 확인하세요.

```sh
herdr-desktop-pet stop
brew update && brew upgrade herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

포뮬러는 앱을 Homebrew prefix에, CLI를 `PATH`에 설치하며 `/Applications`·Herdr 플러그인·패치 호스트는 설치하지 않습니다. 앱은 ad-hoc 서명·미공증입니다. 포뮬러 로딩이 거부된 경우에만 `brew trust --formula hanbong5938/tap/herdr-desktop-pet`으로 해당 포뮬러만 신뢰하세요. 옛 cask에서 이전하려면 [안정판 Homebrew 설치·업그레이드](docs/migrations/v0.1.11-to-v0.2.0.md#안정판-homebrew-설치업그레이드)를, 현재 0.3 계열로 이전하려면 [0.3 업그레이드 안내](docs/releases/0.3.md#upgrade-from-v021-or-beta)를 참고하세요.

### Herdr 없이 캐릭터만 실행

정상 로컬 Herdr 서버 없이 계속 표시하려면 새 프로필의 기본 30초 단절 종료를 **시작 전에** 끕니다:

```sh
herdr-desktop-pet settings set exit_with_herdr off
herdr-desktop-pet settings get
herdr-desktop-pet start
herdr-desktop-pet status
```

Herdr 설치·플러그인·패치는 필요하지 않습니다. 시작 후 `herdr-desktop-pet hide_bubble`로 빈 말풍선을 선택적으로 숨기고 캐릭터는 유지할 수 있습니다. 약 30초 후 캐릭터가 사라지면 `settings get`과 `status`를 확인하세요. `exit_with_herdr`가 켜졌고 정상 서버가 없다면 유예 후 종료됩니다. 같은 실제 설정/상태 프로필에서 끈 뒤 다시 시작하세요. 이 스위치로 등록된 모든 로컬 플러그인 endpoint의 확인된 비활성화/연결 해제 종료를 막을 수는 없습니다. [실행 관리 상세](docs/usage.ko.md#실행-관리와-플러그인-액션).

### 선택형 고급 경로: Herdr 관리형 플러그인·attach 자동 시작

해당 호환 Herdr 호스트에서 [플러그인 저장소](https://github.com/hanbong5938/herdr-desktop-pet)를 설치한 뒤 실행 중인 호스트에서 명시적으로 시작합니다:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

관리형 설치는 플러그인을 클론해 `scripts/install.sh`를 실행합니다. 기본 모드는 매니페스트에 고정된 v0.3.5 앱 전용 압축 파일을 시도하고 자산 부재·검증 실패 시 안내 후 소스를 빌드합니다. `--prebuilt`는 실패해도 소스로 전환하지 않고 `--source`는 현재 체크아웃을 빌드합니다. **기본 fallback은 공개 사전 빌드의 증거가 아닙니다.** 실제 압축 파일·체크섬·설치기 일치를 확인하세요. 별도 출처 사이드카는 설치기 압축 파일에 넣지 않습니다. 사전 빌드는 SHA-256·아카이브·arm64·서명을 검사하지만 앱은 **ad-hoc 서명·미공증**입니다. attach 자동 시작을 원하면 [패치 호스트 안내 (English)](integrations/herdr/README.md#build-in-separate-checkouts)를 따르세요. 위 네이티브 수동 관찰에는 둘 다 필요 없습니다. [설치기·서명 상세](docs/development.ko.md#소스-개발).

## 이 체크아웃 소스 빌드: 관리형 연동

**선택형 관리형 attach 경로에서는** 저장소 루트에서 먼저 [호스트 빌드·격리 프로필 안내 (English)](integrations/herdr/README.md#build-in-separate-checkouts)에 따라 공식 Herdr 0.9.3 소스를 패치·빌드하고 `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`와 해당 호스트 세션을 준비하세요. 아래의 `plugin link`는 호스트도 앱도 빌드하지 않습니다. 앱만 소스에서 빌드하는 데 패치 호스트가 필요하지는 않습니다.

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

연결·활성화만으로 이미 실행 중인 호스트에서 캐릭터가 즉시 시작되지는 않습니다. 한 번 `start`를 실행하거나 자동 시작이 켜졌다면 이후 정상적인 shell/terminal 클라이언트 attach를 기다리세요. 플러그인 액션에는 실행 중인 호스트가 필요합니다. 소스 CLI는 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`이며 소스 CLI 버전만으로 게시 자산이나 이전 데몬 교체를 입증하지 않습니다. [소스 개발 상세](docs/development.ko.md#소스-개발).

이 소스의 매니페스트 수명주기·액션은 `scripts/run-native.sh`로 실행되며, 매니페스트의 안정판 또는 `-beta.N` 버전에 맞는 `HerdrDesktopPet.app`/`HerdrDesktopPetBeta.app`을 선택합니다. 해당 번들이 없으면 다른 채널을 대신 실행하지 않습니다. 직접 자동화할 때는 안정판 소스의 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`을 사용하고, 기존 데몬을 종료하거나 설정·상태 경로 **둘 다** 격리한 뒤 `status`로 실제 실행 파일과 버전을 확인하세요.

### 앱 내 업데이트 (이 체크아웃의 프로토콜 3 소스)

**설정 → 앱 업데이트**에는 독립적인 **업데이트 자동 확인**(기본 켜짐)·**베타 업데이트 받기**(기본 꺼짐), **지금 확인**, 실행 버전·설치 출처, 정책별 마지막 성공 확인 시각과 명시적인 실행 버튼이 있습니다. 안정판만 받으면 공식 안정판을, 베타를 선택하면 안정판·베타 중 SemVer상 가장 높은 적격 릴리스를 확인하며 같은 기본 버전의 정식판은 베타보다 우선합니다. 베타를 꺼도 설치된 베타를 다운그레이드하지 않습니다. 앱이 열린 동안 자동 확인은 안정판 전용·베타 포함 각각의 성공 시각에 따라 최대 하루에 한 번 실행됩니다. 베타 선택 변경은 이전 후보를 무효화하고 자동 확인이 켜져 있을 때만 새 정책으로 확인하며, 꺼져 있으면 **지금 확인**을 기다립니다. 확인은 읽기 전용입니다. 두 스위치와 확인만으로 설치·재시작·Brew 메타데이터 갱신·Git 변경은 일어나지 않으며 출처·채널·대상 변경에는 정확한 제안에 대한 별도 네이티브 동의가 필요합니다.

카드는 **현재 확인·인계 진행 상태**와 보존된 **이전 작업 결과**를 분리해 표시합니다. 기본 상태·도우미 안내는 선택한 UI 언어를 따르며 접힌 **진단 정보**에는 전체 경로·해시·ID·원본 오류가 표시됩니다. 디스크에 설치된 후보가 곧 적용·실행된 것은 아니며 출처 기준 최신/고정 ref도 공개 최신판이라는 뜻은 아닙니다. Herdr 관리는 동일한 실행 중인 호환 호스트에서 공식 태그를 변경 불가 커밋으로 고정하고 사용자 체크아웃을 Git reset하지 않습니다. Homebrew는 원본 manager·prefix, 정확한 대상 포뮬러·버전과 양쪽 물리 설치 루트를 검증하며 동의받은 작업에서만 메타데이터를 재검증합니다. 원본 포뮬러를 몰래 제거하지 않습니다. 로컬 체크아웃은 디스크의 파일을 명시적으로 재빌드하며 Git pull/reset하지 않습니다. 검증된 프로토콜 3 호환 기설치 패키지만 installer 없이 **설치된 업데이트 적용**을 제안할 수 있습니다.

네이티브 동의·Prepare는 미저장 초안·IME 조합·진행 조작을 재검사한 뒤 변경 조작을 동결하고 원본을 계속 실행합니다. 소유권 ACK는 설치·Stop 완료가 아닙니다. 수락된 Prepare 뒤 일치하는 최종 visible/prepared ACK만 동의를 이전하며, 정확히 예약된 Stop의 수락 응답이 완전히 전달된 경우에만 updater 종료를 확정합니다. staging은 worker에서 진행되어 도우미 창은 반응하며 네이티브 인계의 **30초 초과는 취소가 아니라 대기**입니다. 새 인스턴스의 실제 준비를 확인하고 설치·적용 결과를 구분합니다. 후보 시작은 현재 디스크에서 추정한 값이나 원본 source token이 아니라 저널·예약에 검증되어 고정된 **이미지와 설치 출처** 쌍을 사용합니다. 불명확하거나 구형 프로토콜 예약은 근거를 보존하며 manager 자동 재실행·journal 임의 삭제·원본 자동 재시작을 하지 않습니다. 일반 사용자 Stop은 재시작을 금지하지만 안전하게 중지된 작업임이 입증되면 조정과 자기 예약 해제를 마칠 수 있습니다. 불확실한 작업의 재시도 전 [복구 안내](docs/usage.ko.md#앱-내-업데이트)와 [CLI 상태·복구](docs/cli.md#앱-업데이트-상태와-복구)를 확인하세요.

공개 안정판 `v0.3.3`과 대응 베타 `0.3.3-beta.1`의 압축 파일은 변경 불가 프로토콜 2 앱·도우미 쌍이며, 확인만으로 이 체크아웃의 프로토콜 3 베타 설정을 얻지 않습니다. 이 체크아웃의 소스 버전·네이티브 CLI 버전만으로 프로토콜 3 릴리스 게시를 입증할 수 없습니다. 직접 `--source`로 빌드하거나 별도로 검증된 공개 프로토콜 3 패키지로 명시적으로 채택해야 합니다. 서로 다른 프로토콜의 앱·도우미를 섞지 않으며 구형 프로토콜 1/2 기록은 보존된 해당 버전 도우미로 확인·복구하세요. 게시 여부는 실제 [Release 자산](https://github.com/hanbong5938/herdr-desktop-pet/releases)에서 확인하세요.

## 처음 사용하기

| 조작 | 동작 |
| --- | --- |
| 메뉴 막대 아이콘 좌클릭 / 우클릭·Control-click | 설정 열기 / 설정·종료·필요 시 **캐릭터 표시·조작 복구** 메뉴. |
| 머리·몸통 탭, 머리 위 쓰다듬기; 캐릭터 드래그·오른쪽 아래 조절점 | 반응; 이동·크기 변경. |
| 말풍선 변·모서리 드래그; 카드 선택 | 말풍선 크기 변경; 로컬 에이전트 카드 아래 답장칸 열기. |
| 답장칸 Enter·Command+Enter·보내기 / Escape | IME 조합 중에는 제출하지 않고 Escape는 먼저 조합을 취소합니다. 조합 중이 아닐 때 Enter·Command+Enter·보내기는 메시지를 제출하고 Escape는 답장칸을 접습니다. 검색창에 포커스가 있으면 Enter·Command+Enter는 답장을 제출하지 않습니다. |

캐릭터와 말풍선은 각각 숨길 수 있습니다. 둘 다 숨기거나 **전체 창 클릭 통과**를 켰다면 메뉴 막대 아이콘을 우클릭·Control-click하고 **캐릭터 표시·조작 복구**를 선택하세요. **투명 영역 클릭 통과**는 캐릭터의 투명 부분에만 적용됩니다. [조작·복구 상세](docs/usage.ko.md#조작) · [독립 표시](docs/usage.ko.md#독립-표시와-메뉴-막대-복구).

## 선택 캐릭터와 의상

기본 루벨리아 외 팩은 [캐릭터 갤러리](https://hanbong5938.github.io/herdr-characters/)에서 살펴보세요(처음 영어로 열리면 **KO** 버튼). **캐릭터 → 캐릭터 브라우저 열기…**에서 지원되는 공식 팩의 **다운로드 및 적용**을 명시적으로 선택합니다. 로컬 폴더나 `.herdrchar`를 **캐릭터 추가…**로 가져오기만 해서는 캐릭터가 바뀌지 않습니다. 설치된 카드를 **선택**한 뒤 **캐릭터 적용**을 누르세요. 작업 결과가 불확실하면 작업 ID로 상태를 확인하고 무작정 반복하지 마세요. [팩 사용법](docs/usage.ko.md#캐릭터-팩) · [팩 CLI](docs/cli.md#팩-명령-및-기존-경계) · [제작](docs/development.ko.md#캐릭터-제작).

## 안전 및 제한

- 답장은 선택한 **로컬** Herdr 출처의 `agent.prompt`로만 전송됩니다. 원격 관찰은 읽기 전용이고 터미널의 승인·질문은 터미널에서 직접 처리하세요. 제출 **ACK는 에이전트 완료가 아닙니다**. 전달 여부가 불명확하면 확인 없이 다시 보내지 마세요. 현재 API에는 예상 세션을 원자적으로 고정하는 보호가 없어 제출 순간 패널의 에이전트·세션 변경 경합이 가능합니다. [전달 상세](docs/usage.ko.md#인라인-답장과-메시지-전달).
- **워크트리 삭제…**는 확인한 카드 하나만이 아닌 **작업 공간 전체**의 탭·패널과 실행 중 프로세스를 닫고, 연결 체크아웃의 **무시된 파일까지** 되돌릴 수 없이 삭제합니다. 확인 창의 **정확한 체크아웃 경로**와 전체 영향을 먼저 검토하세요. 불확실한 결과를 반복 요청하지 마세요. [삭제 조건·확인](docs/usage.ko.md#워크트리-삭제).
- 앱 업데이트의 자동 확인은 읽기 전용이며 설치·적용에는 별도 동의가 필요합니다. 도우미 소유권 ACK도 완료가 아닙니다. 불명확한 결과는 [복구 절차](docs/usage.ko.md#앱-내-업데이트)와 [CLI 계약](docs/cli.md#앱-업데이트-상태와-복구)에 따라 확인하세요.

## 설정 및 문제 해결

`HERDR_PLUGIN_CONFIG_DIR`에는 `preferences.json`, `lifecycle.json`, 관리 `characters/`·`menu-bar-icons/`가, `HERDR_PLUGIN_STATE_DIR`에는 잠금·제어 소켓·`desktop-pet.log`가 있습니다. 경로가 주입되지 않으면 Herdr의 XDG 플러그인 경로를 사용합니다. 네이티브 `--config-dir`·`--state-dir`는 격리 경로이며 주입 환경변수가 우선합니다. 안정판/베타 전환 시 실제 **설정과 상태를 모두 백업**하고 시작·명령·종료 경로를 일치시키세요.

서버 없이 시작한 뒤 약 30초 후 캐릭터가 사라지면 `settings get`으로 `exit_with_herdr`를 확인하고 독립 실행용으로 **재시작 전에** 끄세요. 등록된 모든 로컬 endpoint의 확인된 비활성화/연결 해제 종료는 예외입니다. 숨긴 창이나 클릭 통과만으로 시작 실패를 판단하지 말고 메뉴 막대 복구를 사용하세요. 관리형 시작 문제라면 해당 호스트의 `client.attached` 경고, 수동 `start`, `status`, 데몬 로그, 실제 Herdr CLI·소켓·서버를 확인하세요. 원격 세션은 로컬 Mac에서 앱을 실행해 관찰하며 원격 서버에 앱을 설치하지 않습니다. [문제 해결·설정](docs/usage.ko.md#설정과-문제-해결) · [호스트 안내 (English)](integrations/herdr/README.md).

## 문서 및 웹사이트

[사용 안내](docs/usage.ko.md) · [개발 안내](docs/development.ko.md) · [v0.3.5 한영 노트](docs/releases/v0.3.5.md) · [CLI 계약](docs/cli.md#한국어) · [호스트 연동 (English)](integrations/herdr/README.md) · [릴리스 기록](docs/releases/README.ko.md) · [0.3 업그레이드/대응 베타](docs/releases/0.3.md#한국어--v03-계열) · [과거 베타·프리뷰](docs/migrations/unreleased.ko.md).

[한국어 웹사이트](https://hanbong5938.github.io/herdr-desktop-pet/) · [영어 웹사이트](https://hanbong5938.github.io/herdr-desktop-pet/en/) · [중국어 간체 웹사이트](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/) · [일본어 웹사이트](https://hanbong5938.github.io/herdr-desktop-pet/ja/): 네이티브 앱 소개 페이지이며 브라우저에서 앱을 실행하지 않습니다. 별도 캐릭터 갤러리는 영어·한국어만 제공합니다. 웹사이트 개발·배포는 [개발 안내](docs/development.ko.md#웹사이트-github-pages)에 있습니다.

## 그림 및 라이선스

소프트웨어 MIT 라이선스와 그림·모델 사용 조건은 서로 다릅니다. 기본 루벨리아의 [라이선스](assets/rubelia-default/LICENSE.txt)·[출처](assets/rubelia-default/ATTRIBUTION.txt)·[제작 기록](assets/rubelia-default/source-record.json)·[배경 보정 기록](assets/rubelia-default/background-repair.json), 원본 Coding Cat 고지가 남은 [LICENSE.txt](LICENSE.txt)를 각각 확인하세요. 그림 소유자 승인은 모델 권리를 부여하지 않으며 소프트웨어 MIT가 그림을 재라이선스하지 않습니다. Qwen 연구 라이선스는 제작 기록에 보존되고 모델 가중치·엔진은 번들에 없습니다. 가져온 팩의 라이선스·출처·사용 조건도 함께 보관하세요. [그림 제작 상세](docs/development.ko.md#캐릭터-제작).
