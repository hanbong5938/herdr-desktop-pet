# 개발

[English](development.md) · **한국어** · [설치와 개요](../readme.ko.md)

아래 명령은 저장소 루트에서 실행합니다. 이 체크아웃의 대상 버전은 v0.3.5이지만 소스 버전만으로 비초안 Release·완전한 자산·갱신된 설치기/Homebrew 포뮬러를 증명하지 않습니다. 사전 빌드 설치 전 [실제 Release 목록](https://github.com/hanbong5938/herdr-desktop-pet/releases)을 확인하세요. 네이티브 지원 플랫폼은 Apple Silicon macOS 13 이상이며 Intel Mac·Windows·Linux용 네이티브 배포는 없습니다.

## 내장 선택형 플러그인 관리자

표준 `HerdrDesktopPet.app`에 기존 의존성을 사용하는 네이티브 관리자 엔진(`native/src/plugin_manager_backend.rs`)과 필요할 때만 여는 컨트롤러(`native/src/plugin_manager.rs`)가 포함됩니다. 별도 crate·도우미·설치·워크플로·릴리스 자산은 없습니다. 설정의 초기 사용 가능 상태는 꺼짐이며 **설치**·**삭제(Remove)**는 사용 가능 플래그와 이를 안전하게 저장하는 데 필요한 admission 메타데이터만 변경합니다. **열기**를 눌러야 플러그인 검색을 시작하며 사용하지 않는 관리자 작업 저장소는 생성하지 않습니다. 이 버튼들은 플러그인을 다운로드·설치·삭제하지 않습니다. 변경에는 호스트 capabilities와 실제 context 양쪽의 protocol 2·`manager_safety_protocol: 1`, capabilities의 `durable_result_protocol: 1`·`admission_fd_protocol: 1`이 필요합니다. 순정·구형 protocol-2 호스트는 관리자 capabilities/context/inventory 메서드가 없어도 지원되는 일반 읽기 전용 목록만 표시하며 다섯 변경 동작을 비활성화하고 다른 호스트나 CLI로 우회하지 않습니다. 고정 호스트 빌드 전에 [`client-attached.patch`](../integrations/herdr/client-attached.patch)를 먼저, [`plugin-manager.patch`](../integrations/herdr/plugin-manager.patch)를 다음에 적용하세요. Pet 패키징은 Herdr를 패치하거나 교체하지 않습니다. 기존 Manager 메서드를 포함한 생성 API 스키마도 패치에 포함되지만 private protocol 22·이름 있는 client endpoint generation 1은 바꾸지 않습니다. [사용법](usage.ko.md)과 [호스트 연동 안내](../integrations/herdr/README.md#build-in-separate-checkouts)를 참고하세요.

Pet은 실행 수명 전체에 걸쳐 설정 경로별 비공개 preferences-owner FD를 보유하며 오프라인 설정 도우미도 다른 상태 경로를 선택했더라도 같은 소유권을 저장 완료까지 확보해야 합니다. 실행 중인 도우미는 읽기·쓰기 전에 정규화된 설정과 상태 경로를 모두 확인합니다. 기존 상대 **설정 경로만** 호출자의 작업 디렉터리를 기준으로 절대 경로로 바꾸고 실제 프로필의 물리적 동일성을 확인합니다. 상태 경로·임의 보호 경로·저장된 경로·요청 경로는 계속 절대 경로여야 하며 기존 상대 `managed_path`를 자동 재해석하거나 이전하지 않습니다. `ProfileOperationStore`를 확보한 뒤에도 기존 leaf 심볼릭 링크·비공개 소유권·inode 검사는 유지합니다. 논리 설정 별칭과 물리 루트는 호스트의 registry·잠금·lease 검증·예상 체크아웃/임시 경로 전반에서 함께 추적하고, 일반 설치 생산자도 정규화된 관리형 메타데이터를 저장합니다. Windows 런타임 루트의 예상 표현도 기존 도우미와 맞추지만 macOS의 소스·플랫폼 매니페스트 회귀 증거가 Windows 컴파일/실행 검증은 아닙니다.

설정 도우미는 초기 연결, 상태 lease 확보 뒤 두 번째 연결, 종료 소유권 해제 대기의 세 위치에서 비공개 typed probe를 사용합니다. 연결 단계의 `NotFound`·`ConnectionRefused`만 소유자 부재이며 연결된 소켓의 timeout·빈 EOF는 부재가 아닙니다. framing·parse·`InvalidData`·`InvalidInput`·권한 오류는 소유자가 나중에 사라져도 영구 실패로 보존합니다. 인증된 정상 owner/profile의 원자적 stopping ping 오류 응답은 유효한 Live 응답으로 처리하고 소유권 해제를 기다립니다. 준비 중·종료 중 대기는 변경 제출 전 실제 전송 바이트를 세는 쓰기까지 하나의 제한된 공통 기한을 사용합니다. 이 제출 전 기한은 기존 제출 후 결과 복구의 별도 대기 창과 구분됩니다. 제출 전 소유자 부재를 증명한 경우에만 독점 오프라인 저장이 가능합니다. `Set` 진입 뒤의 모든 결과는 직접 반환하며 0바이트 전송 실패도 오프라인 저장으로 우회하지 않습니다. 일부 전송 뒤에는 정확한 작업 ID의 `Uncertain`으로 남고 재실행하지 않습니다. 일반 호출자의 기존 bool probe는 그대로 유지합니다. 종료 중 소유자는 안전하게 소유권을 해제하며 정상 설정 창 닫기·Pet 종료는 이미 제출된 호스트 CLI 작업을 취소하지 않습니다.

엄격한 읽기 전용 inventory는 원본 등록을 한 번 읽어 다시 로드한 표시 정보와 짝지으며 확인·변경 시 둘 다 비교합니다. 업데이트는 보존된 이전 전체 체크아웃 백업을 검증하고 확인된 루트·매니페스트 정보만 재매핑합니다. 공유 설치기/백업 이름 문법은 기존 제약 안에서 `@scope`, 공백·Unicode를 허용하고 traversal·NUL·역슬래시·콜론·심볼릭 링크 및 이전 백업 CAS 검사는 유지합니다. 실제 변경 RPC 스트림에서 전송 전에 UID·원래 PID·실행 파일을 인증하며 서버는 lease·파일 접근 전에 원래 PID 또는 정규화된 루트 변경을 거절합니다. 불변 `original_context.config_dir`는 registry 생성/잠금 전과 커밋 직전에 고정·재검증하되 설정 밖의 별도 비보호 stow 대상은 허용합니다. 성공한 비활성화는 일치하는 보기만 지우고 플러그인 창 소유권을 유지합니다. 연결 해제·관리형 제거는 창 소유 기록을 해제하지만 실제 터미널은 닫지 않습니다. registry·잠금·임시 쓰기 대상의 실제 Pet 별칭도 보호합니다. 관리자와 일반 Unix 작업은 수명 FD lease를 공유하며 비-Unix 호스트에서는 관리자 변경을 지원하지 않습니다.

호스트는 기존 설치기의 정확한 관리형 레이아웃 판정을 공유하며 CLI 의존성을 추가하지 않습니다. 정규화된 전체 체크아웃 루트와 기록된 문법 검증 하위 경로를 lexical하게 결합하므로 redirect된 하위 루트를 다시 정규화해 정당화하지 않습니다. 예상 Windows 런타임 루트와 정확한 정규화 매니페스트도 비교합니다. 미리 보기·빌드 전·네트워크 교체 전·최종 교체 후, 서버의 잠금 전/잠금 안, 보존 이전 백업에 같은 판정을 적용합니다. 교체 후 로컬 실패는 기존 rollback으로 이전 체크아웃을 복원하거나, inventory가 불일치하면 `Partial`로 이전 백업을 보존합니다. `Unknown`은 새 체크아웃과 이전 백업을 함께 보존하며 재실행하지 않습니다. 공개 CLI와 사용법은 변경하지 않습니다.

줄바꿈 없는 압축 JSON 파일은 요청 1 MiB·결과 128 KiB의 전체 파일 경계를 포함하며 매 제출마다 active 공개·실행 전에 작업 조상 경로를 동기화합니다. 호스트 registry 저장은 기존·이전에 생성한 조상 체인도 매번 확인하고 임시 파일 full-sync → `renameat` → 같은 고정 대상 부모 FD의 full-sync 순서를 사용하며 전역 영속성 캐시는 없습니다. 커밋 전 실패(`PreCommit`)는 확정 실패지만 rename 뒤 부모 동기화 실패(`CommitUnknown`)는 정확한 API 오류 `plugin_manager_registry_uncertain`을 통해 **변경 작업만** 기존 결과 불명 상태로 남깁니다. 결과 누락도 성공이 아닙니다. 영속 성공만 앱 메모리·창 소유권·보기를 갱신합니다. 결과 불명은 작업 ID·증거를 보존하고 추가 변경을 막으며 정확한 ID의 명시적 확인은 작업을 다시 실행하지 않습니다. 이 수정은 wire 버전·capability·CLI 명령·앱·crate·의존성을 추가하지 않습니다.

## 소스 개발

[Herdr 호스트 연동 안내](../integrations/herdr/README.md#build-in-separate-checkouts)의 별도 체크아웃 절차를 먼저 따르세요. 공식 Herdr 0.9.3 소스 커밋 `7b116c05bfda646af39d2524c54e70c751f57ee8`에 [client-attach 패치](../integrations/herdr/client-attached.patch)를 적용해 호스트를 빌드하고, 빌드된 실행 파일에 `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`를 지정합니다. 해당 안내의 고정 호스트 도구와 격리된 XDG/세션 설정을 **먼저** 준비하세요. 플러그인 링크는 호스트나 앱을 빌드하지 않습니다. 순정 Herdr 0.9.0과 0.9.3에는 `client.attached` 훅이 없고 `min_herdr_version = "0.9.3"`은 패치 기준일 뿐 훅 지원의 증거가 아닙니다. 실제로 훅 지원을 명시한 향후 호스트 릴리스에서만 패치를 대체할 수 있습니다. 연결 경고·패치된 세션의 API 구독/스키마·안내의 클라이언트 attach probe를 확인하세요. 알 수 없는 훅 경고나 연결 성공만으로는 성공이 아닙니다. `PATH`의 다른 `herdr`가 아닌 동일한 패치 바이너리와 세션을 확인해야 합니다.

연결할 개발 체크아웃은 먼저 직접 빌드하세요(관리형 설치와 달리 `plugin link`는 `[[build]]`를 실행하지 않습니다).

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

소스 설치기는 고정된 Bun·npm 의존성을 설치하고 Rust 실행 파일·업데이트 코디네이터를 빌드한 뒤 네이티브 rig 런타임·캐릭터 제작 리소스를 패키징하고 앱을 검증합니다. Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools·`codesign`이 필요하며 별도 호스트 빌드 조건은 연동 안내를 따릅니다. 이미 실행 중인 서버에서 연결·활성화만 해도 캐릭터가 시작되는 것은 아닙니다. `start`를 한 번 실행하거나 `auto_start`가 켜졌다면 이후 정상적인 shell/terminal 클라이언트 attach를 기다리세요. 서버 시작 시에도 `ensure`가 자동으로 실행됩니다. 플러그인 액션에는 실행 중인 활성 호스트가 필요하지만 패키지 네이티브 바이너리의 오프라인 설정 명령에는 필요하지 않습니다. 소스 자동화에는 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`을 사용하세요. 기존 데몬을 종료하거나 `start`·명령·`stop` 모두에 일관된 **설정과 상태** 격리 경로를 사용하세요. 소스 CLI 버전만으로 자산 게시나 구형 실행 데몬의 교체를 증명하지 못하므로 `status`의 실제 실행 파일을 확인하세요.

공통 [설치기](../scripts/install.sh)는 세 모드를 제공합니다. 옵션이 없으면 매니페스트에 고정된 사전 빌드를 시도하고 **없거나 검증 실패한 경우에만** 소스 빌드로 전환합니다. `--prebuilt`는 실패하고 `--source`는 이 체크아웃을 빌드합니다(Bun/npm 의존성은 내려받을 수 있음). 관리형 Herdr 플러그인 설치는 `[[build]]`를 통해 설치기 실행 후 플러그인 디렉터리의 `dist/`에 앱을 둡니다. 매니페스트 대상 `0.3.5` 또는 소스 빌드만으로 공개 사전 빌드가 증명되지 않습니다. 설치기는 기존처럼 `HerdrDesktopPet.app/`만 포함한 압축 파일을 받아 체크섬·네이티브 서명을 검증하고 별도 rig 출처 기록을 설치 입력으로 사용하지 않습니다. client-attach 자동 시작에는 여전히 기능이 있는 패치 호스트가 필요합니다.

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

`scripts/build-rig-native.mjs`는 범위 제한된 소스/컴파일러/출력 해시를 `native/target/rig-native/release/rig-native.json`에 기록합니다. 패키징은 소스·생성 출력의 해시를 확인하고 원본 기록을 `Contents/Resources/rig-native-build.json`에 포함하며 독립 업데이트 코디네이터와 protocol/서명 검사도 유지합니다. 앱 서명 **이후** 코디네이터를 포함한 `Contents`의 모든 배포 파일 해시를 외부 `dist/HerdrDesktopPet.app.rig-native.json`에 기록해 서명 번들 안의 자체 해시 순환을 피합니다. 릴리스 워크플로는 기존 앱 전용 압축 파일·체크섬 두 파일 외에 별도 `HerdrDesktopPet-v0.3.5-rig-native.json` 자산을 게시합니다. 설치기/업데이터의 아카이브 입력 형식은 바꾸지 않으며 실제 추출된 서명 후 파일과 사이드카의 해시를 비교해야 배포 바이트 출처를 주장할 수 있습니다.

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
