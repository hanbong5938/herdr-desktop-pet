# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**게시 경계:** 기존 [Herdr v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6)은 protocol 3 / observation capability 1로 출시되었습니다. 독립 Desktop Pet v0.4.0(protocol 4 / capability 2 / integration API 1)과 이전용 bridge v0.3.7은 검토된 후보이며 공개 설치·런타임 검증은 아직 완료되지 않았습니다. 아래 명령은 중립 제품 cutover 안내이며 문서·태그·소스 버전은 게시 증거가 아닙니다.

필요한 연동만 설치하세요. Herdr는 얇은 consumer이며 Tern은 네이티브 앱을 다운로드·빌드·설치·자동 시작하지 않습니다. 플러그인 갱신·제거는 연동만 바꾸고 공용 앱·프로필·캐릭터는 남깁니다. 일반 앱 업데이트는 Herdr 플러그인을 재설치하지 않습니다.

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.ko.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 앱 사이트](https://hanbong5938.github.io/desktop-pet/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 연동 사이트](https://hanbong5938.github.io/herdr-desktop-pet/). 이전 사이트는 앱 UI·그림을 복제하지 않는 연동·이전 입구입니다.

중립 릴리스와 포뮬러가 실제로 게시된 뒤 공개 설치 명령은 다음과 같습니다:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac, macOS 13 이상. 소스 빌드는 Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools가 필요하며 사전 빌드 설치에는 필요 없습니다. 앱은 ad-hoc 서명·미공증입니다. Herdr 스냅샷에는 호환 로컬 서버(0.9.0 이상), 답장에는 별도 `agent.prompt`가 필요합니다. 선택형 client-attach 자동 시작에는 공식 Herdr 0.9.3에 [연동 저장소의 패치와 호스트 빌드 절차](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)를 적용하거나 실제 훅 지원 호스트를 사용해야 합니다. 순정 0.9.0/0.9.3에는 훅이 없고 플러그인 설치는 호스트를 패치하지 않습니다.

사전 빌드·소스 설치는 Herdr 연동이 아니라 **core** 체크아웃에서 실행합니다. 기본 설치는 prebuilt-only이며 다운로드·검증 실패를 소스 빌드로 숨기지 않습니다. `--source`는 현재 core 체크아웃만 빌드하고 그 `dist/DesktopPet.app`을 사용합니다.

## Herdr consumer 계약

이 저장소는 `DesktopPet.app`, 네이티브 패키지나 캐릭터 자산을 빌드·복사하지 않습니다. 연동 전용 manifest·wrapper·adapter는 이제 이 체크아웃에 있고 네이티브 소스·그림·도구는 core 소유입니다. consumer pin 작성과 공개 설치에는 실제 공개 core installer/checksum 및 검토된 릴리스 API 메타데이터가 필요합니다. 목적지 링크나 로컬 소스는 게시 증거가 아닙니다.

정확히 호환되는 공용 앱의 탐색 순서는 명시적 절대 실행 파일 → 검증된 실제 계정 홈의 공용 locator → PATH의 네이티브 `desktop-pet` → 실제 Brew prefix → 실제 계정 홈의 `Applications/DesktopPet.app`입니다. 잘못된 명시 경로나 locator는 다른 설치로 우회하지 않습니다. 앱이 없을 때만 고정·digest 검증된 **core** installer로 bootstrap하며, 비호환 설치는 명시적으로 네이티브 갱신·복구해야 합니다. 여기서 제품 빌드나 조용한 fallback을 하지 않습니다.

Herdr 프로필 우선순위는 주입된 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 명시적 `--config-dir` / `--state-dir` → Herdr XDG/HOME 기본값입니다. wrapper는 두 경로를 core에 명시적으로 전달하고 프로필을 이동·병합하지 않습니다. 백엔드 출처는 `--herdr-socket ABS`로 전달하며 이전 `--socket`이나 pet의 전용 제어 소켓과 다릅니다. `status`는 CLI만이 아닌 실제 실행 중 이미지·프로필을 표시합니다.

[Herdr 안내](integrations/herdr/README.md)는 기존 21개 action ID와 표시·말풍선·수명주기 의미, 호스트 빌드·패치, link 및 OMP 절차를 보존합니다. `ensure`는 멱등적이며 숨겨진 pet을 표시하지 않습니다. 선택형 attach 자동 시작에는 실제 훅 지원 호스트가 필요합니다. lifecycle 파일이 처음부터 없으면 Herdr 종료 정책은 true, 기존 파일에서 키만 없으면 false입니다. 명시적 정책과 알 수 없는 필드는 쓰기 시 보존됩니다. Tern은 별도 consumer이며 공용 앱을 자동 시작하지 않습니다.

## 소스 / link

먼저 **desktop-pet core** 체크아웃에서 `bash scripts/install.sh --source`로 앱을 설치·빌드합니다. 호환 앱을 선택한 뒤 core가 아닌 **이 연동 체크아웃**을 link합니다. `plugin link`는 빌드를 실행하지 않습니다. 공개/bootstrap 사용은 실제 consumer pin 작성 후 가능합니다. [분리된 체크아웃 절차](integrations/herdr/README.md#build-in-separate-checkouts)를 참조하세요. 플러그인 설치는 Herdr를 패치하지 않습니다.

공통 [사용법](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ko.md), [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md), [개발](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) 및 네이티브 기능·provider 상세는 독립 core 소유입니다.

## 소유권

이 저장소는 `native-adapter`, `herdr-plugin.toml`, 얇은 설치·실행 wrapper, `integrations/herdr`, `integrations/omp`, 연동 검증 및 동결된 과거 릴리스를 소유합니다. 앱·UI·rig·그림·패키징·소스 빌드는 [desktop-pet](https://github.com/hanbong5938/desktop-pet) 소유입니다. `.herdrchar`, pack v5, `default@0`, rig ABI 및 [herdr-characters](https://github.com/hanbong5938/herdr-characters) catalog/gallery 권위는 그대로입니다. 과거 [v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) 및 이전 릴리스 본문·태그·자산은 변경하지 않습니다. 연동 전용 v0.4.0에는 네이티브 archive가 없습니다.

## 그림 및 라이선스

소프트웨어 MIT 라이선스와 그림·모델 사용 조건은 서로 다릅니다. 기본 루벨리아의 [라이선스](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)·[출처](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)·[제작 기록](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)·[배경 보정 기록](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json), 원본 Coding Cat 고지가 남은 [LICENSE.txt](LICENSE.txt)를 각각 확인하세요. 그림 소유자 승인은 모델 권리를 부여하지 않으며 소프트웨어 MIT가 그림을 재라이선스하지 않습니다. Qwen 연구 라이선스는 제작 기록에 보존되고 모델 가중치·엔진은 번들에 없습니다. 가져온 팩의 라이선스·출처·사용 조건도 함께 보관하세요. [그림 제작 상세](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#캐릭터-제작).
