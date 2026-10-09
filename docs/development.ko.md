# 개발

[English](development.md) · **한국어** · [설치와 개요](../readme.ko.md)

아래 명령은 저장소 루트에서 실행합니다. 이 체크아웃의 대상 버전은 v0.3.4이지만 소스 버전만으로 비초안 Release·완전한 자산·갱신된 설치기/Homebrew 포뮬러를 증명하지 않습니다. 사전 빌드 설치 전 [실제 Release 목록](https://github.com/hanbong5938/herdr-desktop-pet/releases)을 확인하세요. 네이티브 지원 플랫폼은 Apple Silicon macOS 13 이상이며 Intel Mac·Windows·Linux용 네이티브 배포는 없습니다.

## 소스 개발

[Herdr 호스트 연동 안내](../integrations/herdr/README.md#build-in-separate-checkouts)의 별도 체크아웃 절차를 먼저 따르세요. 공식 Herdr 0.9.3 소스 커밋 `7b116c05bfda646af39d2524c54e70c751f57ee8`에 [client-attach 패치](../integrations/herdr/client-attached.patch)를 적용해 호스트를 빌드하고, 빌드된 실행 파일에 `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`를 지정합니다. 해당 안내의 고정 호스트 도구와 격리된 XDG/세션 설정을 **먼저** 준비하세요. 플러그인 링크는 호스트나 앱을 빌드하지 않습니다. 순정 Herdr 0.9.0과 0.9.3에는 `client.attached` 훅이 없고 `min_herdr_version = "0.9.3"`은 패치 기준일 뿐 훅 지원의 증거가 아닙니다. 실제로 훅 지원을 명시한 향후 호스트 릴리스에서만 패치를 대체할 수 있습니다. 연결 경고·패치된 세션의 API 구독/스키마·안내의 클라이언트 attach probe를 확인하세요. 알 수 없는 훅 경고나 연결 성공만으로는 성공이 아닙니다. `PATH`의 다른 `herdr`가 아닌 동일한 패치 바이너리와 세션을 확인해야 합니다.

연결할 개발 체크아웃은 먼저 직접 빌드하세요(관리형 설치와 달리 `plugin link`는 `[[build]]`를 실행하지 않습니다).

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

소스 설치기는 고정된 Bun·npm 의존성을 설치하고 Rust 실행 파일·업데이트 코디네이터를 빌드한 뒤 네이티브 rig 런타임·캐릭터 제작 리소스를 패키징하고 앱을 검증합니다. Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools·`codesign`이 필요하며 별도 호스트 빌드 조건은 연동 안내를 따릅니다. 이미 실행 중인 서버에서 연결·활성화만 해도 캐릭터가 시작되는 것은 아닙니다. `start`를 한 번 실행하거나 `auto_start`가 켜졌다면 이후 정상적인 shell/terminal 클라이언트 attach를 기다리세요. 서버 시작 시에도 `ensure`가 자동으로 실행됩니다. 플러그인 액션에는 실행 중인 활성 호스트가 필요하지만 패키지 네이티브 바이너리의 오프라인 설정 명령에는 필요하지 않습니다. 소스 자동화에는 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`을 사용하세요. 기존 데몬을 종료하거나 `start`·명령·`stop` 모두에 일관된 **설정과 상태** 격리 경로를 사용하세요. 소스 CLI 버전만으로 자산 게시나 구형 실행 데몬의 교체를 증명하지 못하므로 `status`의 실제 실행 파일을 확인하세요.

공통 [설치기](../scripts/install.sh)는 세 모드를 제공합니다. 옵션이 없으면 매니페스트에 고정된 사전 빌드를 시도하고 **없거나 검증 실패한 경우에만** 소스 빌드로 전환합니다. `--prebuilt`는 실패하고 `--source`는 이 체크아웃을 빌드합니다(Bun/npm 의존성은 내려받을 수 있음). 관리형 Herdr 플러그인 설치는 `[[build]]`를 통해 설치기 실행 후 플러그인 디렉터리의 `dist/`에 앱을 둡니다. 매니페스트 대상 `0.3.4` 또는 소스 빌드만으로 공개 사전 빌드가 증명되지 않습니다. 설치기는 기존처럼 `HerdrDesktopPet.app/`만 포함한 압축 파일을 받아 체크섬·네이티브 서명을 검증하고 별도 rig 출처 기록을 설치 입력으로 사용하지 않습니다. client-attach 자동 시작에는 여전히 기능이 있는 패치 호스트가 필요합니다.

저장소의 `herdr-plugin` 토픽으로 [Herdr 마켓](https://herdr.dev/plugins/) 자동 수집 대상에 등록됩니다. 인덱스는 30분마다 갱신되며 **심사받지 않은 커뮤니티 목록**이지 호스트 호환성 검토가 아닙니다. Homebrew [개인 tap](https://github.com/hanbong5938/homebrew-tap) 포뮬러는 `HerdrDesktopPet.app`을 `/Applications`가 아닌 Homebrew prefix에 설치하고 CLI를 `PATH`에 놓습니다. Herdr 플러그인이나 패치 호스트는 설치하지 않으므로 실행 관리 훅에는 플러그인, client-attach 자동 실행에는 호스트 패치가 별도로 필요합니다. 포뮬러 로딩이 거부된 경우에만 `brew trust --formula hanbong5938/tap/herdr-desktop-pet`으로 이 포뮬러만 신뢰하세요. 업그레이드 전 데몬을 종료하세요. `start`는 기존 데몬을 재사용하므로 CLI `--version`과 재시작 후 `status`의 `app_version` 및 실행 파일을 함께 확인하세요. 옛 cask는 `/Applications`에 앱을 복사했습니다. `brew uninstall --cask herdr-desktop-pet` 후 `brew install hanbong5938/tap/herdr-desktop-pet`으로 포뮬러에 이전할 수 있으며 사용자 데이터는 유지됩니다. 안정판·베타 프로필 격리와 이전은 [한영 0.3 이전 안내](releases/0.3.md#한국어--v03-계열)를 참고하세요. 소스 버전만으로 공개 자산이 존재한다고 추정하지 마세요.

## 네이티브 빌드와 서명

소스 설치 후 네이티브 검사는 다음 명령으로 실행합니다.

```sh
bun run check
bun run test:native
```

네이티브 hit-overlay 혼합 계산은 릴리스 러너의 Xcode 16.4 Swift 컴파일러와 호환되도록 타입을 지정한 부분 식을 사용합니다. 오버레이를 고칠 때 이 타입 경계를 유지하세요. 소스 빌드에는 여전히 Xcode Command Line Tools가 필요합니다.

debug·release 네이티브 빌드는 `codesign`을 필요로 하며 [`native/rig/RigDecodeWorker.entitlements.plist`](../native/rig/RigDecodeWorker.entitlements.plist)의 `com.apple.security.cs.allow-jit=true`를 **`rig-decode-worker`에만** 부여해 ad-hoc 서명합니다. Cargo는 plist 변경을 추적해 worker를 다시 빌드·서명합니다. JavaScriptCore가 신뢰된 디코더 번들을 JIT 컴파일할 수 있게 하지만 앱 본체와 네이티브 rig 라이브러리에는 **JIT 권한이 없으며** worker 격리·검증·자원 제한은 유지합니다. `bun run package:native`는 선택한 서명 identity로 worker 전용 권한을 다시 적용하고 외부 앱 서명 후 실제 entitlement 및 worker·앱의 엄격한 서명을 검증합니다. 서명·검증 실패 시 **패키징을 중단**합니다. 기본 ad-hoc 패키지는 로컬 실행용이며 Developer ID 서명·공증이 아닙니다.

Rust/Swift rig 경계는 타입 있는 asset/token을 넘기기 **전에** ABI 버전과 C 구조체 일곱 개의 크기를 확인합니다. 초기 motion은 네이티브 호스트가 복사하는 봉인된 바이트이며 옛 경로 조회 방식으로 돌아가지 않습니다. Metal은 같은 인코더의 색상 그리기 전에 실제 흰자 구멍을 L/R/무방향 stencil로 준비하고 CPU 의미론적 hit·참조 래스터도 같은 소유권 규칙을 적용합니다. 흰자가 없는 rig의 기존 홍채 표시 동작은 유지합니다. `RIG_PROBE_FAULTS` probe의 `--eye-stencil-checks --output ABS`는 실제 픽셀·의미론적 fixture를 저장하지만 과거 취소/연결 해제 상황의 시각 현상 원인을 입증하지 않습니다.

`scripts/build-rig-native.mjs`는 범위 제한된 소스/컴파일러/출력 해시를 `native/target/rig-native/release/rig-native.json`에 기록합니다. 패키징은 소스·생성 출력의 해시를 확인하고 원본 기록을 `Contents/Resources/rig-native-build.json`에 포함하며 독립 업데이트 코디네이터와 protocol/서명 검사도 유지합니다. 앱 서명 **이후** 코디네이터를 포함한 `Contents`의 모든 배포 파일 해시를 외부 `dist/HerdrDesktopPet.app.rig-native.json`에 기록해 서명 번들 안의 자체 해시 순환을 피합니다. 릴리스 워크플로는 기존 앱 전용 압축 파일·체크섬 두 파일 외에 별도 `HerdrDesktopPet-v0.3.4-rig-native.json` 자산을 게시합니다. 설치기/업데이터의 아카이브 입력 형식은 바꾸지 않으며 실제 추출된 서명 후 파일과 사이드카의 해시를 비교해야 배포 바이트 출처를 주장할 수 있습니다.

## 저장소 구조

| 경로 | 역할 |
| --- | --- |
| [`herdr-plugin.toml`](../herdr-plugin.toml) | Herdr 플러그인 실행 관리·액션 정의 |
| [`native/`](../native/) | Rust 앱·네이티브 UI·세션 처리·캐릭터 관리 |
| [`native/rig/`](../native/rig/), [`web/rig/`](../web/rig/) | Rig 렌더링·디코딩·제작 지원 |
| [`assets/`](../assets/) | 기본 루벨리아 팩·메뉴 썸네일만 번들 |
| [`tools/`](../tools/) | 범용 캐릭터 제작·rig 검사 도구 |
| [`scripts/`](../scripts/) | 설치·네이티브 빌드·앱 패키징 |
| [`.github/workflows/release.yml`](../.github/workflows/release.yml) | 릴리스 패키징 워크플로 |

## 캐릭터 제작

팩 제작·검증은 [캐릭터 제작 가이드](../.agents/skills/character-creator/SKILL.md)와 [`tools/character-pack.py`](../tools/character-pack.py)를 참고하세요. 배포 앱에도 `Contents/Resources/creator/SKILL.md`·`Contents/Resources/creator/character-pack.py`가 포함됩니다. 소스 체크아웃의 [로컬 Qwen + See-through 제작 스킬](../.agents/skills/create-pet-character/SKILL.md)은 **승인된 그림에서** 독립된 10포즈를 만듭니다. 외부 엔진·모델 가중치는 번들에 포함하지 않습니다. 루벨리아가 기본 rig 템플릿입니다. PNG 템플릿·Aurora·Coding Cat·선택형 루벨리아 의상과 각각의 그림 생성기는 별도 [herdr-characters](https://github.com/hanbong5938/herdr-characters)에서 관리합니다. 외부 템플릿은 `--templates-root PATH`로 지정해야 하며 번들에 있다고 가정하지 마세요.

가져온 팩의 라이선스·출처·사용 조건 파일을 함께 보관하세요. 기본 루벨리아의 [라이선스](../assets/rubelia-default/LICENSE.txt)·[출처](../assets/rubelia-default/ATTRIBUTION.txt)·[제작 기록](../assets/rubelia-default/source-record.json)·[배경 보정 기록](../assets/rubelia-default/background-repair.json)이 그림 기준입니다. [`LICENSE.txt`](../LICENSE.txt)는 원래 Coding Cat 고지를 유지하지만 Coding Cat과 그 그림 생성 소스는 별도 캐릭터 저장소로 이동했습니다. 이 고지들이 저장소의 모든 구성 요소에 일괄 라이선스를 부여하지는 않습니다. 그림 소유자의 승인은 모델 권리를 부여하지 않으며 소프트웨어 MIT는 그림 라이선스를 바꾸지 않습니다. Qwen 연구 라이선스는 제작 기록으로 보존하고 모델 가중치·엔진은 번들에 넣지 않습니다. 버전별 다시 그리기·얼굴 보정·당시 한계는 [v0.1.6](releases/v0.1.6.md)·[v0.1.9](releases/v0.1.9.md)·[v0.1.10](releases/v0.1.10.md)을 참고하세요.

## 웹사이트 (GitHub Pages)

한국어 기본·영어 홍보 웹사이트는 [`site/`](../site/)에 있습니다. 네이티브 macOS 앱을 설명하는 사이트이며 브라우저에서 앱을 실행하지 않습니다. Pages 주소: [한국어](https://hanbong5938.github.io/herdr-desktop-pet/) · [영어](https://hanbong5938.github.io/herdr-desktop-pet/en/). 제목은 화면 폭에 맞춘 간결한 크기와 좁은 주변 여백을 사용하며 본문·터미널 명령어·버튼 크기는 제목과 독립적으로 유지합니다. 저장소 루트에서 실행하세요.

```sh
bun test tools/pages.test.ts
node tools/pages.mjs check
node tools/pages.mjs serve --port 4173
# http://127.0.0.1:4173/herdr-desktop-pet/ 접속 (영어: /herdr-desktop-pet/en/)
node tools/pages.mjs stage /tmp/herdr-pages
```

사이트 CLI에는 **Node.js 22**를 사용하고 **Bun 1.4.2**는 실제 Node 서버를 실행하는 HTTP 회귀 테스트의 러너로만 사용합니다. CI는 staging 전에 이 회귀 테스트를 실행합니다. `check`는 내부 링크·앵커·언어별 메타데이터·에셋을 확인하며 외부 링크를 네트워크로 검사하지 않습니다. 로컬 미리보기는 공개 파일 중 일반 파일만 제공합니다. 최종 `404.html` 대체 파일이 없거나 심볼릭 링크·일반 파일이 아니라면 파일을 전송하는 대신 `nosniff`가 있는 일반 텍스트 `500`을 반환하며 `HEAD`에는 본문이 없습니다. `stage`는 먼저 검사한 뒤 저장소 **밖의 새 디렉터리 또는 빈 디렉터리**에 공개할 `site/` 파일만 복사하며 배포하지 않습니다. Pages 워크플로는 PR을 검사하고 검토용 아티팩트만 첨부하며 배포하지 않습니다. Pages 배포 소스는 **GitHub Actions**이고, 웹사이트 변경을 `main`에 병합하거나 `main`에서 워크플로를 수동 실행했을 때 조건을 만족하는 `main` 실행만 배포합니다. 이 로컬 명령은 릴리스 바이너리·Wiki·저장소 Pages 설정을 바꾸지 않습니다.
