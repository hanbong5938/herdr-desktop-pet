# Herdr 연동 개발

[English](development.md) · **한국어** · [연동 개요](../readme.ko.md)

앱·UI·rig·캐릭터 도구·그림·빌드·패키징·앱 사이트는 독립 [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) 소유입니다. 앱은 별도 core 체크아웃에서 설치·빌드하고 Herdr에는 **이 연동 체크아웃**을 link하세요. `plugin link`는 앱을 빌드하지 않습니다.

[보존된 Herdr 연동 안내](../integrations/herdr/README.md)는 21개 action, adapter 실행 정책, 두 프로필 경로 우선순위, 별도 호스트/core/연동 체크아웃, 공식 Herdr 0.9.3 패치, 선택형 client-attach 훅과 OMP 확장을 설명합니다. 순정 0.9.0/0.9.3은 attach 훅을 제공하지 않으며 플러그인 설치는 호스트를 패치하지 않습니다.

- [공통 앱 개발·설치](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md) · [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#한국어)
- [캐릭터 제작·라이선스](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#캐릭터-제작)
- [Tern Connect·Configure](https://github.com/hanbong5938/tern-desktop-pet/blob/main/README.ko.md) · [이전 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

이 저장소의 `site/`는 네 언어의 연동·이전 입구만 포함하며 앱 화면이나 캐릭터 자산을 복제하지 않습니다. Pages workflow는 정적 파일을 직접 stage하고 main만 배포합니다. 공통 앱 사이트 도구·Wiki 생성/게시 절차는 core의 [Pages 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.ko.md#웹사이트-github-pages)·[Wiki 게시 안내](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/publishing.md)에 있습니다. 공개 상태와 실제 배포 URL은 별도 확인 대상입니다. 실제 Tern 재연결·OMP 모델 전달 성공을 주장하지 않습니다.

[v0.3.6 개발 문서](https://github.com/hanbong5938/herdr-desktop-pet/blob/v0.3.6/docs/development.ko.md)는 동결된 과거 경로이며 현재 설치 지침이 아닙니다.
