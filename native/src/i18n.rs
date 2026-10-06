use crate::dialogue::DialogueSlot;
use crate::session_view::{DisplayStatus, SessionStatusSummary};
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::fmt::Write as _;

/// The persisted language choice. `System` resolves to the first supported
/// primary language reported by the operating system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LanguagePreference {
    #[default]
    System,
    Ko,
    En,
}

impl LanguagePreference {
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Ko => "ko",
            Self::En => "en",
        }
    }
}

impl Serialize for LanguagePreference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.token())
    }
}

impl<'de> Deserialize<'de> for LanguagePreference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LanguagePreferenceVisitor;

        impl<'de> Visitor<'de> for LanguagePreferenceVisitor {
            type Value = LanguagePreference;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("the language token system, ko, or en")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(match value {
                    "ko" => LanguagePreference::Ko,
                    "en" => LanguagePreference::En,
                    "system" => LanguagePreference::System,
                    _ => LanguagePreference::System,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_bytes<E>(self, _value: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LanguagePreference::System)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while sequence.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LanguagePreference::System)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LanguagePreference::System)
            }
        }

        deserializer.deserialize_any(LanguagePreferenceVisitor)
    }
}

/// A concrete UI language. The system preference is intentionally not a
/// locale: it is resolved to one of these two values before rendering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiLocale {
    Ko,
    En,
}

impl UiLocale {
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::Ko => "ko",
            Self::En => "en",
        }
    }
}

/// Resolve an explicit preference or the first supported primary language in
/// an ordered system language list. Both hyphenated BCP-47 tags and the
/// underscore form returned by some platform APIs are accepted.
pub(crate) fn resolve_language(
    preference: LanguagePreference,
    preferred_tags: &[&str],
) -> UiLocale {
    match preference {
        LanguagePreference::Ko => UiLocale::Ko,
        LanguagePreference::En => UiLocale::En,
        LanguagePreference::System => preferred_tags
            .iter()
            .find_map(|tag| primary_locale(tag))
            .unwrap_or(UiLocale::En),
    }
}

fn primary_locale(tag: &str) -> Option<UiLocale> {
    let tag = tag.trim();
    let end = tag
        .find(|character| character == '-' || character == '_')
        .unwrap_or(tag.len());
    let primary = &tag[..end];
    if primary.eq_ignore_ascii_case("ko") {
        Some(UiLocale::Ko)
    } else if primary.eq_ignore_ascii_case("en") {
        Some(UiLocale::En)
    } else {
        None
    }
}

/// Every fixed host-owned string belongs to this catalog. Data supplied by a
/// character pack, terminal, source, operation, or daemon is never put in
/// this enum and is passed through the typed formatters below instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Message {
    ResetPosition,
    ScaleUp,
    ScaleDown,
    FollowSystemSettings,
    LanguageSystem,
    KoreanLanguage,
    EnglishLanguage,
    Characters,
    More,
    MenuPanelTitle,
    MenuCharacterTab,
    MenuBubbleTab,
    MenuSettingsTab,
    MenuCharacterVisible,
    MenuBubbleVisible,
    MenuStatusIndicators,
    MenuBubblePlacement,
    BubbleTheme,
    ThemeWarmIvory,
    ThemeDustyRose,
    ThemeMoonlitInk,
    ThemeCustom,
    CustomizeBubbleColors,
    BubbleSurfaceColor,
    BubbleTextColor,
    BubbleMutedColor,
    BubbleBorderColor,
    BubbleAccentColor,
    ApplyBubbleColors,
    ResetBubbleColors,
    InvalidBubbleColor,
    BubbleAppearanceSaveFailure,
    StatusIndicatorsSaveFailure,
    MenuClickBehavior,
    MenuFullPassthrough,
    MenuAlphaPassthrough,
    MenuFullPassthroughHelp,
    MenuAlphaPassthroughHelp,
    MenuScale,
    MenuAppearance,
    MenuLanguage,
    ContextSettings,
    ContextShowCharacter,
    ContextHideCharacter,
    ContextShowBubble,
    ContextHideBubble,
    RecoveryMenuTitle,
    RecoverCharacterInteraction,
    MenuQuit,
    LifecycleWindowTitle,
    LifecycleHeading,
    LifecycleAutoStart,
    LifecycleAutoStartHelp,
    LifecycleExit,
    LifecycleExitHelp,
    LifecycleQuitHelp,
    LifecycleSaveFailure,
    ObservationTitle,
    ObservationLocal,
    ObservationLocalHelp,
    ObservationRemote,
    ObservationMachines,
    ObservationSession,
    ObservationHelpTitle,
    ObservationHelpDetails,
    ObservationDetails,
    ObservationHideDetails,
    ObservationNone,
    ObservationLoading,
    ObservationEmpty,
    ObservationSelect,
    ObservationPaused,
    ObservationMachinePaused,
    ObservationRegistration,
    ObservationError,
    ObservationPreviousResults,
    ObservationConnecting,
    ObservationOnline,
    ObservationOffline,
    ObservationDisabled,
    ObservationNotSelected,
    ObservationSaveFailed,
    ObservationCopyReconnect,
    ObservationCommandCopied,
    ObservationCopyFailed,
    ObservationMachineObserve,
    MenuManageCharacter,
    CharacterCandidate,
    CharacterSelectionPrompt,
    CharacterApply,
    CharacterCancelSelection,
    CharacterSelectionStale,
    CharacterOperationQueued,
    CharacterOperationPreparing,
    CharacterOperationApplying,
    CharacterOperationCompleted,
    CharacterOperationFailed,
    CharacterOperationCanceled,
    CharacterOperationPendingApply,
    CharacterOperationUnknown,
    CharacterOperationNotSubmitted,
    CharacterOperationUnavailable,
    CharacterEditingOpen,
    DialogueWindowTitle,
    DialogueTarget,
    DialogueDefaultValue,
    DialogueCustomValue,
    DialogueNoChanges,
    DialogueShowOriginal,
    DialogueHideOriginal,
    DialogueLoading,
    DialogueTargetUnavailable,
    DialogueResetMenu,
    DialogueResetDraftConfirm,
    DialogueResetDraftConfirmHelp,
    DialogueDraftSessionHelp,
    DialogueLanguage,
    DialogueSlot,
    DialogueText,
    DialogueBytes,
    DialogueTooLong,
    DialogueStatusReference,
    DialogueUnsaved,
    DialogueSave,
    DialogueResetEntry,
    DialogueResetCharacter,
    DialogueResetConfirm,
    DialogueResetConfirmHelp,
    DialogueSaveFailed,
    DialogueHeadTap,
    DialogueBodyTap,
    DialoguePet,
    DialogueCompletion,
    DialogueIdle,
    DialogueRunning,
    DialogueWaiting,
    DialogueUnknown,
    MenuPlacementAuto,
    MenuPlacementAbove,
    MenuPlacementBelow,
    MenuPlacementLeft,
    MenuPlacementRight,
    Collapse,
    ExpandSpeechBubble,
    CollapseSpeechBubble,
    CloseBubbleWindow,
    FullSpeechBubbleMessage,
    AllSessions,
    ComposerSelectSession,
    ComposerInput,
    ComposerSend,
    ComposerSending,
    ComposerSent,
    ComposerEmpty,
    ComposerTooLarge,
    ComposerBusy,
    ComposerOffline,
    ComposerStaleTarget,
    ComposerReadOnly,
    ComposerBlocked,
    ComposerNotReady,
    ComposerUnsupported,
    ComposerUnknownDelivery,
    ComposerFailed,
    ComposerShortcut,
    ComposerComposingShortcut,
    Idle,
    Working,
    Waiting,
    Completed,
    Unknown,
    Offline,
    NoSessions,
    NoMatches,
    UnnamedSession,
    UnnamedTab,
    Workspace,
    Tab,
    Cwd,
    Source,
    Terminal,
    Pane,
    LastKnown,
    DisconnectedSource,
    NoConnectedTasks,
    InProgress,
    NeedsAttention,
    Complete,
    BeingChecked,
    AddCharacter,
    AddCharacterTitle,
    UpdateCharacterTitle,
    Active,
    ActiveOverride,
    ActivePending,
    Operation,
    RubeliaBuiltIn,
    Update,
    RestoreRevision,
    Revision,
    Remove,
    ChooseCharacterSource,
    Choose,
    RemoveCharacterPack,
    Cancel,
    LanguageSaveFailed,
    LanguageSaveFailedAccessibility,
    Accepted,
    Preparing,
    Applying,
    CommittedPendingApply,
    DurabilityUnknown,
    Failed,
    Canceled,
    InspectPack,
    InspectPackTitle,
    DiagnoseCharacter,
    BuiltInTag,
    ManagedTag,
    CharacterDiagnostics,
    SelectedMismatchWarning,
    OverrideActiveWarning,
    Close,
}

/// The shared event label for a dialogue slot in menus and the editor sidebar.
pub(crate) const fn dialogue_slot_message(slot: DialogueSlot) -> Message {
    match slot {
        DialogueSlot::Idle => Message::DialogueIdle,
        DialogueSlot::Running => Message::DialogueRunning,
        DialogueSlot::Waiting => Message::DialogueWaiting,
        DialogueSlot::Unknown => Message::DialogueUnknown,
        DialogueSlot::HeadTap => Message::DialogueHeadTap,
        DialogueSlot::BodyTap => Message::DialogueBodyTap,
        DialogueSlot::Pet => Message::DialoguePet,
        DialogueSlot::Completion => Message::DialogueCompletion,
    }
}

/// Look up one fixed catalog entry without allocating.
pub(crate) const fn text(locale: UiLocale, message: Message) -> &'static str {
    match locale {
        UiLocale::Ko => match message {
            Message::ResetPosition => "위치 초기화",
            Message::ScaleUp => "확대",
            Message::ScaleDown => "축소",
            Message::FollowSystemSettings => "시스템 설정 따르기",
            Message::LanguageSystem => "시스템",
            Message::KoreanLanguage => "한국어",
            Message::EnglishLanguage => "English",
            Message::Characters => "캐릭터",
            Message::More => "더 보기",
            Message::MenuPanelTitle => "Herdr Desktop Pet 설정",
            Message::MenuCharacterTab => "캐릭터",
            Message::MenuBubbleTab => "말풍선",
            Message::MenuSettingsTab => "설정",
            Message::MenuCharacterVisible => "캐릭터 표시 여부",
            Message::MenuBubbleVisible => "말풍선 표시 여부",
            Message::MenuStatusIndicators => "상태 아이콘·색상 표시",
            Message::MenuBubblePlacement => "말풍선 위치",
            Message::BubbleTheme => "말풍선 테마",
            Message::ThemeWarmIvory => "따뜻한 아이보리",
            Message::ThemeDustyRose => "더스티 로즈",
            Message::ThemeMoonlitInk => "달빛 잉크",
            Message::ThemeCustom => "사용자 지정",
            Message::CustomizeBubbleColors => "말풍선 색상 사용자 지정",
            Message::BubbleSurfaceColor => "배경 색상",
            Message::BubbleTextColor => "글자 색상",
            Message::BubbleMutedColor => "보조 글자 색상",
            Message::BubbleBorderColor => "테두리 색상",
            Message::BubbleAccentColor => "강조 색상",
            Message::ApplyBubbleColors => "색상 적용",
            Message::ResetBubbleColors => "색상 초기화",
            Message::InvalidBubbleColor => "색상은 #RRGGBB 형식이어야 합니다",
            Message::BubbleAppearanceSaveFailure => "말풍선 모양 설정 저장 실패",
            Message::StatusIndicatorsSaveFailure => "상태 아이콘·색상 설정 저장 실패",
            Message::MenuClickBehavior => "클릭 동작",
            Message::MenuFullPassthrough => "펫 전체 클릭 통과",
            Message::MenuAlphaPassthrough => "투명 영역만 클릭 통과",
            Message::MenuFullPassthroughHelp => "펫 전체에서 클릭이 통과합니다.",
            Message::MenuAlphaPassthroughHelp => "투명한 영역에서만 클릭이 통과합니다.",
            Message::MenuScale => "크기",
            Message::MenuAppearance => "외관",
            Message::MenuLanguage => "언어",
            Message::ObservationTitle => "관찰 대상",
            Message::ObservationLocal => "이 Mac",
            Message::ObservationLocalHelp => "이 Mac의 Herdr 세션을 관찰합니다.",
            Message::ObservationRemote => "원격 머신",
            Message::ObservationMachines => "관찰할 머신",
            Message::ObservationSession => "세션",
            Message::ObservationHelpTitle => "원격 머신 관찰 설정",
            Message::ObservationHelpDetails => {
                "대화형 터미널에서 Herdr CLI로 SSH 호스트를 등록하세요. 예: herdr machine add <SSH-host> (실제 SSH 호스트로 바꿔 입력).\n등록 후 원격 머신 관찰을 켜고 관찰할 머신 프로필을 선택하세요. 선택한 프로필의 저장된 세션만 5초마다 확인합니다.\n원격 관찰은 읽기 전용이며 메시지를 보내지 않습니다. SSH 인증과 재연결은 터미널에서 직접 진행하세요."
            }
            Message::ObservationDetails => "오류 자세히 보기",
            Message::ObservationHideDetails => "오류 내용 숨기기",
            Message::ObservationNone => "관찰 대상이 꺼져 있습니다. 이 Mac 또는 원격 머신을 켜세요.",
            Message::ObservationLoading => "저장된 머신을 읽는 중…",
            Message::ObservationEmpty => "저장된 머신이 없습니다. 터미널에서 머신을 등록한 뒤 선택하세요.",
            Message::ObservationSelect => "관찰할 머신을 선택하세요.",
            Message::ObservationPaused => "원격 관찰이 꺼져 있습니다. 선택한 머신을 관찰하려면 켜세요.",
            Message::ObservationMachinePaused => "관찰 꺼짐",
            Message::ObservationRegistration => "등록 방법",
            Message::ObservationError => "머신 목록을 읽지 못했습니다. 터미널에서 Herdr CLI 상태를 확인하세요.",
            Message::ObservationPreviousResults => "이전에 읽은 머신 목록을 표시합니다. 현재 목록과 다를 수 있습니다.",
            Message::ObservationConnecting => "상태 확인 중…",
            Message::ObservationOnline => "관찰 중",
            Message::ObservationOffline => "상태를 읽지 못함",
            Message::ObservationDisabled => "Herdr에서 비활성화됨",
            Message::ObservationNotSelected => "선택되지 않음",
            Message::ObservationSaveFailed => "관찰 설정 저장 실패",
            Message::ObservationCopyReconnect => "재연결 명령 복사",
            Message::ObservationCommandCopied => "재연결 명령을 복사했습니다.",
            Message::ObservationCopyFailed => "재연결 명령을 복사하지 못했습니다.",
            Message::ObservationMachineObserve => "관찰할 머신",
            Message::ContextSettings => "설정…",
            Message::ContextShowCharacter => "캐릭터 표시",
            Message::ContextHideCharacter => "캐릭터 숨기기",
            Message::ContextShowBubble => "대화상자 표시",
            Message::ContextHideBubble => "대화상자 숨기기",
            Message::RecoveryMenuTitle => "Desktop Pet 복구",
            Message::RecoverCharacterInteraction => "캐릭터 표시·조작 복구",
            Message::MenuQuit => "종료",
            Message::LifecycleWindowTitle => "Herdr 실행 관리",
            Message::LifecycleHeading => "Herdr 실행 관리",
            Message::LifecycleAutoStart => "Herdr 실행 시 자동 시작",
            Message::LifecycleAutoStartHelp => "서버 시작 또는 클라이언트 연결 시 펫이 꺼져 있으면 시작합니다. 숨긴 펫을 다시 표시하지 않습니다.",
            Message::LifecycleExit => "모든 Herdr 서버 연결이 끊기면 종료",
            Message::LifecycleExitHelp => "서버 연결이 모두 끊기면 30초간 재접속을 기다린 뒤 펫을 종료합니다. 클라이언트 화면 분리는 서버 종료가 아닙니다.",
            Message::LifecycleQuitHelp => "종료는 현재 실행만 멈춥니다. 자동 시작 설정은 바뀌지 않습니다.",
            Message::LifecycleSaveFailure => "실행 관리 설정 저장 실패",
            Message::MenuManageCharacter => "캐릭터 관리",
            Message::CharacterCandidate => "적용할 캐릭터",
            Message::CharacterSelectionPrompt => "캐릭터를 선택하세요.",
            Message::CharacterApply => "캐릭터 적용",
            Message::CharacterCancelSelection => "선택 취소",
            Message::CharacterSelectionStale => "선택한 캐릭터가 변경되었습니다. 다시 선택하세요.",
            Message::CharacterOperationQueued => "캐릭터 작업 대기 중",
            Message::CharacterOperationPreparing => "캐릭터 작업 준비 중",
            Message::CharacterOperationApplying => "캐릭터 적용 중",
            Message::CharacterOperationCompleted => "캐릭터 작업 완료",
            Message::CharacterOperationFailed => "캐릭터 작업 실패",
            Message::CharacterOperationCanceled => "캐릭터 작업 취소됨",
            Message::CharacterOperationPendingApply => "저장됨 · 화면에 적용 대기 중",
            Message::CharacterOperationUnknown => "캐릭터 작업 결과를 확인할 수 없음",
            Message::CharacterOperationNotSubmitted => "미제출/접수 거절 · 서비스에서 실행되지 않았습니다",
            Message::CharacterOperationUnavailable => "현재 캐릭터 작업을 시작할 수 없음",
            Message::CharacterEditingOpen => "대사 편집…",
            Message::DialogueWindowTitle => "대사 편집기",
            Message::DialogueTarget => "편집할 캐릭터",
            Message::DialogueDefaultValue => "팩 기본 대사",
            Message::DialogueCustomValue => "사용자 지정 대사",
            Message::DialogueNoChanges => "변경 사항 없음",
            Message::DialogueShowOriginal => "팩 원본 보기",
            Message::DialogueHideOriginal => "팩 원본 숨기기",
            Message::DialogueLoading => "대사 불러오는 중",
            Message::DialogueTargetUnavailable => "선택한 캐릭터를 사용할 수 없습니다. 다시 선택하세요.",
            Message::DialogueResetMenu => "대사 초기화",
            Message::DialogueResetDraftConfirm => "초안을 버리고 이 대사를 초기화할까요?",
            Message::DialogueResetDraftConfirmHelp => {
                "현재 초안과 이 항목에 저장된 사용자 대사를 삭제합니다. 팩 원본은 변경되지 않습니다."
            }
            Message::DialogueDraftSessionHelp => {
                "초안은 앱 실행 중 캐릭터·언어·대사 종류별로 유지됩니다. 저장해야 다음 실행에도 남습니다."
            }
            Message::DialogueLanguage => "편집 언어",
            Message::DialogueSlot => "대사 종류",
            Message::DialogueText => "대사 입력 (여러 줄 가능)",
            Message::DialogueBytes => "UTF-8 바이트",
            Message::DialogueTooLong => "대사는 2048바이트를 넘을 수 없습니다.",
            Message::DialogueStatusReference => {
                "원본 대사가 없습니다. 시스템 상태 메시지가 표시됩니다."
            }
            Message::DialogueUnsaved => "저장하지 않음",
            Message::DialogueSave => "대사 저장",
            Message::DialogueResetEntry => "이 대사 초기화",
            Message::DialogueResetCharacter => "캐릭터 대사 전체 초기화…",
            Message::DialogueResetConfirm => "이 캐릭터의 모든 대사를 초기화할까요?",
            Message::DialogueResetConfirmHelp => {
                "한국어와 영어의 모든 사용자 대사가 삭제됩니다. 팩 원본은 변경되지 않습니다."
            }
            Message::DialogueSaveFailed => "대사 저장 실패",
            Message::DialogueHeadTap => "머리 터치",
            Message::DialogueBodyTap => "몸 터치",
            Message::DialoguePet => "쓰다듬기",
            Message::DialogueCompletion => "작업 완료",
            Message::DialogueIdle => "대기",
            Message::DialogueRunning => "작업 중",
            Message::DialogueWaiting => "확인 대기",
            Message::DialogueUnknown => "알 수 없음",
            Message::MenuPlacementAuto => "자동",
            Message::MenuPlacementAbove => "위",
            Message::MenuPlacementBelow => "아래",
            Message::MenuPlacementLeft => "왼쪽",
            Message::MenuPlacementRight => "오른쪽",
            Message::Collapse => "접기",
            Message::ExpandSpeechBubble => "말풍선 펼치기",
            Message::CollapseSpeechBubble => "말풍선 접기",
            Message::CloseBubbleWindow => "대화창 닫기",
            Message::FullSpeechBubbleMessage => "전체 말풍선 메시지",
            Message::AllSessions => "전체",
            Message::ComposerSelectSession => "답장할 로컬 세션 카드를 선택하세요",
            Message::ComposerInput => "선택한 세션에 보낼 한 줄 답장",
            Message::ComposerSend => "보내기",
            Message::ComposerSending => "보내는 중…",
            Message::ComposerSent => "메시지 제출됨 (처리 완료는 아님)",
            Message::ComposerEmpty => "메시지를 입력하세요",
            Message::ComposerTooLarge => "메시지가 너무 깁니다",
            Message::ComposerBusy => "이전 메시지를 보내는 중입니다",
            Message::ComposerOffline => "선택한 세션의 소스가 오프라인입니다",
            Message::ComposerReadOnly => {
                "원격 소스는 읽기 전용 관찰 대상이므로 메시지를 보낼 수 없습니다"
            }
            Message::ComposerStaleTarget => "선택한 세션이 더 이상 현재 세션이 아닙니다",
            Message::ComposerBlocked => "승인 또는 질문 응답은 실제 터미널에서 처리하세요",
            Message::ComposerNotReady => "선택한 세션에 지금은 메시지를 보낼 수 없습니다",
            Message::ComposerUnsupported => "Herdr 서버가 agent.prompt를 지원하지 않습니다",
            Message::ComposerUnknownDelivery => {
                "전달 여부를 확인할 수 없습니다. 확인 없이 다시 보내지 마세요"
            }
            Message::ComposerFailed => "메시지를 보내지 못했습니다",
            Message::ComposerShortcut => "Enter 보내기 · Esc 접기",
            Message::ComposerComposingShortcut => "조합 중에는 전송하지 않음 · Esc 조합 취소",
            Message::Idle => "대기",
            Message::Working => "작업 중",
            Message::Waiting => "확인 대기",
            Message::Completed => "완료",
            Message::Unknown => "알 수 없음",
            Message::Offline => "오프라인",
            Message::NoSessions => "세션 없음",
            Message::NoMatches => "검색 결과 없음",
            Message::UnnamedSession => "이름 없는 세션",
            Message::UnnamedTab => "이름 없는 탭",
            Message::Workspace => "작업 공간",
            Message::Tab => "탭",
            Message::Cwd => "작업 디렉터리",
            Message::Source => "소스",
            Message::Terminal => "터미널",
            Message::Pane => "패널",
            Message::LastKnown => "마지막 상태",
            Message::DisconnectedSource => "연결 끊긴 소스",
            Message::NoConnectedTasks => "연결된 작업 없음",
            Message::InProgress => "진행 중",
            Message::NeedsAttention => "확인이 필요",
            Message::Complete => "완료",
            Message::BeingChecked => "확인 중",
            Message::AddCharacter => "캐릭터 추가…",
            Message::AddCharacterTitle => "캐릭터 추가",
            Message::UpdateCharacterTitle => "캐릭터 업데이트",
            Message::Active => "활성",
            Message::ActiveOverride => "활성 재정의",
            Message::ActivePending => "활성: 대기 중",
            Message::Operation => "작업",
            Message::RubeliaBuiltIn => "루벨리아 (내장)",
            Message::Update => "업데이트…",
            Message::RestoreRevision => "리비전 복원",
            Message::Revision => "리비전",
            Message::Remove => "삭제",
            Message::ChooseCharacterSource => "캐릭터 폴더 또는 .herdrchar 압축 파일을 선택하세요.",
            Message::Choose => "선택",
            Message::RemoveCharacterPack => "캐릭터 팩을 삭제할까요?",
            Message::Cancel => "취소",
            Message::LanguageSaveFailed => "언어 설정 저장 실패",
            Message::LanguageSaveFailedAccessibility => "언어 설정 저장 실패 안내",
            Message::Accepted => "수락됨",
            Message::Preparing => "준비 중",
            Message::Applying => "적용 중",
            Message::CommittedPendingApply => "커밋됨, 적용 대기 중",
            Message::DurabilityUnknown => "내구성 알 수 없음",
            Message::Failed => "실패",
            Message::Canceled => "취소됨",
            Message::InspectPack => "팩 상세 정보…",
            Message::InspectPackTitle => "캐릭터 팩 상세",
            Message::DiagnoseCharacter => "캐릭터 진단 정보",
            Message::BuiltInTag => "내장",
            Message::ManagedTag => "관리 팩",
            Message::CharacterDiagnostics => "진단 정보",
            Message::SelectedMismatchWarning => "선택된 캐릭터와 활성 캐릭터가 일치하지 않습니다.",
            Message::OverrideActiveWarning => "임시 캐릭터 재정의가 활성화되어 있습니다.",
            Message::Close => "닫기",
        },
        UiLocale::En => match message {
            Message::ResetPosition => "Reset Position",
            Message::ScaleUp => "Scale Up",
            Message::ScaleDown => "Scale Down",
            Message::FollowSystemSettings => "Follow System Settings",
            Message::LanguageSystem => "System",
            Message::KoreanLanguage => "한국어",
            Message::EnglishLanguage => "English",
            Message::Characters => "Characters",
            Message::More => "More",
            Message::MenuPanelTitle => "Herdr Desktop Pet Settings",
            Message::MenuCharacterTab => "Character",
            Message::MenuBubbleTab => "Bubble",
            Message::MenuSettingsTab => "Settings",
            Message::MenuCharacterVisible => "Character visible",
            Message::MenuBubbleVisible => "Bubble visible",
            Message::MenuStatusIndicators => "Status icons and colors",
            Message::MenuBubblePlacement => "Bubble placement",
            Message::BubbleTheme => "Bubble theme",
            Message::ThemeWarmIvory => "Warm Ivory",
            Message::ThemeDustyRose => "Dusty Rose",
            Message::ThemeMoonlitInk => "Moonlit Ink",
            Message::ThemeCustom => "Custom",
            Message::CustomizeBubbleColors => "Customize bubble colors",
            Message::BubbleSurfaceColor => "Surface color",
            Message::BubbleTextColor => "Text color",
            Message::BubbleMutedColor => "Secondary text color",
            Message::BubbleBorderColor => "Border color",
            Message::BubbleAccentColor => "Accent color",
            Message::ApplyBubbleColors => "Apply colors",
            Message::ResetBubbleColors => "Reset colors",
            Message::InvalidBubbleColor => "Colors must use #RRGGBB",
            Message::BubbleAppearanceSaveFailure => "Bubble appearance could not be saved",
            Message::StatusIndicatorsSaveFailure => {
                "Status icon and color setting could not be saved"
            }
            Message::MenuClickBehavior => "Click behavior",
            Message::MenuFullPassthrough => "Whole pet click-through",
            Message::MenuAlphaPassthrough => "Transparent-only click-through",
            Message::MenuFullPassthroughHelp => "Clicks pass through the entire pet.",
            Message::MenuAlphaPassthroughHelp => "Clicks pass through transparent areas only.",
            Message::MenuScale => "Scale",
            Message::MenuAppearance => "Appearance",
            Message::MenuLanguage => "Language",
            Message::ContextSettings => "Settings…",
            Message::ContextShowCharacter => "Show Character",
            Message::ContextHideCharacter => "Hide Character",
            Message::ContextShowBubble => "Show Bubble Window",
            Message::ContextHideBubble => "Hide Bubble Window",
            Message::RecoveryMenuTitle => "Desktop Pet Recovery",
            Message::RecoverCharacterInteraction => "Show and Enable Character",
            Message::MenuQuit => "Quit",
            Message::LifecycleWindowTitle => "Herdr Launch Management",
            Message::LifecycleHeading => "Herdr Launch Management",
            Message::LifecycleAutoStart => "Start automatically with Herdr",
            Message::LifecycleAutoStartHelp => "Start the pet when a server starts or a client attaches. An already hidden pet stays hidden.",
            Message::LifecycleExit => "Quit when all Herdr servers disconnect",
            Message::LifecycleExitHelp => "Wait 30 seconds for a server to reconnect, then quit. Detaching a client screen does not stop the server.",
            Message::LifecycleQuitHelp => "Quit stops this run only; it does not change automatic start.",
            Message::LifecycleSaveFailure => "Launch management settings could not be saved",
            Message::ObservationTitle => "Observation sources",
            Message::ObservationLocal => "This Mac",
            Message::ObservationLocalHelp => "Observe Herdr sessions on this Mac.",
            Message::ObservationRemote => "Remote machines",
            Message::ObservationMachines => "Machines to observe",
            Message::ObservationSession => "Session",
            Message::ObservationHelpTitle => "Set up remote observation",
            Message::ObservationHelpDetails => {
                "Register an SSH host with the Herdr CLI in an interactive terminal. Example: herdr machine add <SSH-host> (replace with your actual SSH host).\nThen enable remote observation and select the machine profile to observe. Only saved sessions on selected profiles are checked every 5 seconds.\nRemote observation is read-only and does not send messages. Perform SSH authentication and reconnection manually in the terminal."
            }
            Message::ObservationDetails => "Show error details",
            Message::ObservationHideDetails => "Hide error details",
            Message::ObservationNone => "All observation sources are off. Turn on This Mac or Remote machines.",
            Message::ObservationLoading => "Loading saved machines…",
            Message::ObservationEmpty => "No saved machines. Register one in the terminal, then select it.",
            Message::ObservationSelect => "Select a machine to observe.",
            Message::ObservationPaused => "Remote observation is off. Turn it on to observe selected machines.",
            Message::ObservationMachinePaused => "Observation off",
            Message::ObservationRegistration => "How to register",
            Message::ObservationError => "Could not load the machine list. Check the Herdr CLI in the terminal.",
            Message::ObservationPreviousResults => "Showing a previously loaded machine list; the current list may differ.",
            Message::ObservationConnecting => "Checking status…",
            Message::ObservationOnline => "Observing",
            Message::ObservationOffline => "Status unavailable",
            Message::ObservationDisabled => "Disabled in Herdr",
            Message::ObservationNotSelected => "Not selected",
            Message::ObservationSaveFailed => "Could not save observation settings",
            Message::ObservationCopyReconnect => "Copy reconnect command",
            Message::ObservationCommandCopied => "Reconnect command copied.",
            Message::ObservationCopyFailed => "Could not copy reconnect command.",
            Message::ObservationMachineObserve => "Observe machine",
            Message::MenuManageCharacter => "Manage character",
            Message::CharacterCandidate => "Character to apply",
            Message::CharacterSelectionPrompt => "Select a character to apply.",
            Message::CharacterApply => "Apply character",
            Message::CharacterCancelSelection => "Cancel selection",
            Message::CharacterSelectionStale => "The selected character has changed. Select it again.",
            Message::CharacterOperationQueued => "Character operation queued",
            Message::CharacterOperationPreparing => "Preparing character operation",
            Message::CharacterOperationApplying => "Applying character",
            Message::CharacterOperationCompleted => "Character operation completed",
            Message::CharacterOperationFailed => "Character operation failed",
            Message::CharacterOperationCanceled => "Character operation canceled",
            Message::CharacterOperationPendingApply => "Saved · waiting to appear on screen",
            Message::CharacterOperationUnknown => "Character operation outcome is unknown",
            Message::CharacterOperationNotSubmitted => "Not submitted / rejected · not executed by the service",
            Message::CharacterOperationUnavailable => "Character operation is currently unavailable",
            Message::CharacterEditingOpen => "Edit dialogue…",
            Message::DialogueWindowTitle => "Dialogue editor",
            Message::DialogueTarget => "Character to edit",
            Message::DialogueDefaultValue => "Pack default dialogue",
            Message::DialogueCustomValue => "Custom dialogue",
            Message::DialogueNoChanges => "No changes",
            Message::DialogueShowOriginal => "Show pack original",
            Message::DialogueHideOriginal => "Hide pack original",
            Message::DialogueLoading => "Loading dialogue",
            Message::DialogueTargetUnavailable => "The selected character is unavailable. Select another.",
            Message::DialogueResetMenu => "Reset dialogue",
            Message::DialogueResetDraftConfirm => "Discard the draft and reset this entry?",
            Message::DialogueResetDraftConfirmHelp => {
                "This discards the current draft and removes the saved custom dialogue for this entry. The pack original is unchanged."
            }
            Message::DialogueDraftSessionHelp => {
                "Drafts are kept by character, language, and event during this app session. Save to keep them after restarting."
            }
            Message::DialogueLanguage => "Editing language",
            Message::DialogueSlot => "Dialogue event",
            Message::DialogueText => "Dialogue text (multiple lines)",
            Message::DialogueBytes => "UTF-8 bytes",
            Message::DialogueTooLong => "Dialogue cannot exceed 2048 bytes.",
            Message::DialogueStatusReference => {
                "No original dialogue. The system status message appears instead."
            }
            Message::DialogueUnsaved => "Unsaved",
            Message::DialogueSave => "Save dialogue",
            Message::DialogueResetEntry => "Reset this entry",
            Message::DialogueResetCharacter => "Reset all character dialogue…",
            Message::DialogueResetConfirm => "Reset all dialogue for this character?",
            Message::DialogueResetConfirmHelp => {
                "Removes all personal dialogue in Korean and English. The pack stays unchanged."
            }
            Message::DialogueSaveFailed => "Could not save dialogue",
            Message::DialogueHeadTap => "Head tap",
            Message::DialogueBodyTap => "Body tap",
            Message::DialoguePet => "Pet",
            Message::DialogueCompletion => "Task completion",
            Message::DialogueIdle => "Idle",
            Message::DialogueRunning => "Running",
            Message::DialogueWaiting => "Waiting",
            Message::DialogueUnknown => "Unknown",
            Message::MenuPlacementAuto => "Auto",
            Message::MenuPlacementAbove => "Above",
            Message::MenuPlacementBelow => "Below",
            Message::MenuPlacementLeft => "Left",
            Message::MenuPlacementRight => "Right",
            Message::Collapse => "Collapse",
            Message::ExpandSpeechBubble => "Expand speech bubble",
            Message::CollapseSpeechBubble => "Collapse speech bubble",
            Message::CloseBubbleWindow => "Close Bubble Window",
            Message::FullSpeechBubbleMessage => "Full speech bubble message",
            Message::AllSessions => "All",
            Message::ComposerSelectSession => "Select a local session card to reply",
            Message::ComposerInput => "One-line reply to selected session",
            Message::ComposerSend => "Send",
            Message::ComposerSending => "Sending…",
            Message::ComposerSent => "Message submitted (not yet processed)",
            Message::ComposerEmpty => "Enter a message",
            Message::ComposerTooLarge => "Message is too long",
            Message::ComposerBusy => "A message is already being sent",
            Message::ComposerOffline => "The selected session's source is offline",
            Message::ComposerReadOnly => {
                "Remote sources are observation-only; prompts cannot be sent"
            }
            Message::ComposerStaleTarget => "The selected session is no longer current",
            Message::ComposerBlocked => "Handle approvals or questions in the actual terminal",
            Message::ComposerNotReady => "Cannot message the selected session right now",
            Message::ComposerUnsupported => "This Herdr server does not support agent.prompt",
            Message::ComposerUnknownDelivery => "Delivery is uncertain. Check before sending again",
            Message::ComposerFailed => "Could not send message",
            Message::ComposerShortcut => "Enter to send · Esc to close",
            Message::ComposerComposingShortcut => "No sending while composing · Esc cancels composition",
            Message::Idle => "Idle",
            Message::Working => "Working",
            Message::Waiting => "Waiting",
            Message::Completed => "Completed",
            Message::Unknown => "Unknown",
            Message::Offline => "Offline",
            Message::NoSessions => "No sessions",
            Message::NoMatches => "No matches",
            Message::UnnamedSession => "Unnamed session",
            Message::UnnamedTab => "Unnamed tab",
            Message::Workspace => "Workspace",
            Message::Tab => "Tab",
            Message::Cwd => "Working directory",
            Message::Source => "Source",
            Message::Terminal => "Terminal",
            Message::Pane => "Pane",
            Message::LastKnown => "last known",
            Message::DisconnectedSource => "disconnected source",
            Message::NoConnectedTasks => "No connected tasks",
            Message::InProgress => "in progress",
            Message::NeedsAttention => "need attention",
            Message::Complete => "complete",
            Message::BeingChecked => "being checked",
            Message::AddCharacter => "Add Character…",
            Message::AddCharacterTitle => "Add Character",
            Message::UpdateCharacterTitle => "Update Character",
            Message::Active => "Active",
            Message::ActiveOverride => "Active override",
            Message::ActivePending => "Active: pending",
            Message::Operation => "Operation",
            Message::RubeliaBuiltIn => "Rubelia (built-in)",
            Message::Update => "Update…",
            Message::RestoreRevision => "Restore revision",
            Message::Revision => "Revision",
            Message::Remove => "Remove",
            Message::ChooseCharacterSource => "Choose a character folder or .herdrchar archive.",
            Message::Choose => "Choose",
            Message::RemoveCharacterPack => "Remove character pack?",
            Message::Cancel => "Cancel",
            Message::LanguageSaveFailed => "Language setting could not be saved",
            Message::LanguageSaveFailedAccessibility => "Language setting save failure",
            Message::Accepted => "Accepted",
            Message::Preparing => "Preparing",
            Message::Applying => "Applying",
            Message::CommittedPendingApply => "Committed; pending apply",
            Message::DurabilityUnknown => "Durability unknown",
            Message::Failed => "Failed",
            Message::Canceled => "Canceled",
            Message::InspectPack => "Pack Details…",
            Message::InspectPackTitle => "Character Pack Details",
            Message::DiagnoseCharacter => "Character Diagnostics",
            Message::BuiltInTag => "Built-in",
            Message::ManagedTag => "Managed",
            Message::CharacterDiagnostics => "Diagnostics",
            Message::SelectedMismatchWarning => "Selected character differs from active character.",
            Message::OverrideActiveWarning => "A temporary character override is active.",
            Message::Close => "Close",
        },
    }
}

pub(crate) const fn session_local_source(locale: UiLocale) -> &'static str {
    text(locale, Message::ObservationLocal)
}

/// Status used by the status bubble. `count` is the phase count, while
/// `total` is the session count used when there is no phase-specific count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskStatus {
    NoConnectedTasks,
    InProgress,
    NeedsAttention,
    Complete,
    BeingChecked,
}

pub(crate) fn task_status(
    locale: UiLocale,
    status: TaskStatus,
    count: usize,
    total: usize,
) -> String {
    if matches!(status, TaskStatus::NoConnectedTasks) {
        return text(locale, Message::NoConnectedTasks).to_owned();
    }
    let count = if count > 0 { count } else { total };
    let label = match status {
        TaskStatus::NoConnectedTasks => unreachable!(),
        TaskStatus::InProgress => Message::InProgress,
        TaskStatus::NeedsAttention => Message::NeedsAttention,
        TaskStatus::Complete => Message::Complete,
        TaskStatus::BeingChecked => Message::BeingChecked,
    };
    match locale {
        UiLocale::Ko => format!("작업 {count}개 {}", text(locale, label)),
        UiLocale::En if count == 1 && matches!(status, TaskStatus::NeedsAttention) => {
            "1 task needs attention".to_owned()
        }
        UiLocale::En if count == 1 => format!("1 task {}", text(locale, label)),
        UiLocale::En => format!("{count} tasks {}", text(locale, label)),
    }
}

/// New indicator labels intentionally distinguish observed completion from confirmed success.
pub(crate) fn display_status_label(locale: UiLocale, status: DisplayStatus) -> &'static str {
    match (locale, status) {
        (UiLocale::Ko, DisplayStatus::NoSessions) => text(locale, Message::NoConnectedTasks),
        (UiLocale::Ko, DisplayStatus::Idle) => "작업 대기 중",
        (UiLocale::Ko, DisplayStatus::Running) => "작업 중",
        (UiLocale::Ko, DisplayStatus::Waiting) => "확인 대기 중",
        (UiLocale::Ko, DisplayStatus::Succeeded) => "성공 확인됨",
        (UiLocale::Ko, DisplayStatus::Failed) => "실패",
        (UiLocale::Ko, DisplayStatus::Cancelled) => "취소됨",
        (UiLocale::Ko, DisplayStatus::Completed) => "완료됨 (성공 미확인)",
        (UiLocale::Ko, DisplayStatus::Unknown) => "상태 알 수 없음",
        (UiLocale::Ko, DisplayStatus::Offline) => "오프라인",
        (UiLocale::En, DisplayStatus::NoSessions) => text(locale, Message::NoConnectedTasks),
        (UiLocale::En, DisplayStatus::Idle) => "Idle · waiting for work",
        (UiLocale::En, DisplayStatus::Running) => "Running",
        (UiLocale::En, DisplayStatus::Waiting) => "Waiting for attention",
        (UiLocale::En, DisplayStatus::Succeeded) => "Succeeded",
        (UiLocale::En, DisplayStatus::Failed) => "Failed",
        (UiLocale::En, DisplayStatus::Cancelled) => "Cancelled",
        (UiLocale::En, DisplayStatus::Completed) => "Completed (success unverified)",
        (UiLocale::En, DisplayStatus::Unknown) => "Unknown",
        (UiLocale::En, DisplayStatus::Offline) => "Offline",
    }
}

pub(crate) fn status_indicator_summary(locale: UiLocale, summary: SessionStatusSummary) -> String {
    if summary.count == 0 {
        return display_status_label(locale, summary.status).to_owned();
    }
    match locale {
        UiLocale::Ko => {
            format!(
                "작업 {}개 · {}",
                summary.count,
                display_status_label(locale, summary.status)
            )
        }
        UiLocale::En => {
            let noun = if summary.count == 1 { "task" } else { "tasks" };
            let state = match summary.status {
                DisplayStatus::NoSessions => "not connected",
                DisplayStatus::Idle => "idle · waiting for work",
                DisplayStatus::Running => "running",
                DisplayStatus::Waiting => "waiting for attention",
                DisplayStatus::Succeeded => "succeeded",
                DisplayStatus::Failed => "failed",
                DisplayStatus::Cancelled => "cancelled",
                DisplayStatus::Completed => "completed (success unverified)",
                DisplayStatus::Unknown => "unknown",
                DisplayStatus::Offline => "offline",
            };
            format!("{} {noun} {state}", summary.count)
        }
    }
}

pub(crate) fn task_disclosure(locale: UiLocale, tasks: usize) -> String {
    match locale {
        UiLocale::Ko => format!("작업 {tasks}개 보기"),
        UiLocale::En if tasks == 1 => "View 1 task".to_owned(),
        UiLocale::En => format!("View {tasks} tasks"),
    }
}

pub(crate) fn disconnected_sources(locale: UiLocale, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    match locale {
        UiLocale::Ko => format!("{} {count}개", text(locale, Message::DisconnectedSource)),
        UiLocale::En if count == 1 => {
            format!("1 {}", text(locale, Message::DisconnectedSource))
        }
        UiLocale::En => format!("{count} disconnected sources"),
    }
}

pub(crate) fn session_summary(
    locale: UiLocale,
    total: usize,
    matched: usize,
    omitted: usize,
) -> String {
    let mut summary = match locale {
        UiLocale::Ko if total == 0 => text(locale, Message::NoSessions).to_owned(),
        UiLocale::Ko if matched == total => format!("세션 {total}개"),
        UiLocale::Ko => format!("{total}개 중 {matched}개 일치"),
        UiLocale::En if total == 0 => text(locale, Message::NoSessions).to_owned(),
        UiLocale::En if matched == total && total == 1 => "1 session".to_owned(),
        UiLocale::En if matched == total => format!("{total} sessions"),
        UiLocale::En => format!("{matched} of {total} matched"),
    };
    if omitted > 0 {
        match locale {
            UiLocale::Ko => {
                let _ = write!(summary, " · +{omitted}개 생략");
            }
            UiLocale::En => {
                let _ = write!(summary, " · +{omitted} omitted");
            }
        }
    }
    summary
}

pub(crate) fn session_empty(locale: UiLocale, total: usize, matched: usize) -> &'static str {
    if total == 0 {
        text(locale, Message::NoSessions)
    } else if matched == 0 {
        text(locale, Message::NoMatches)
    } else {
        ""
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionFilterLabel {
    All,
    Idle,
    Working,
    Waiting,
    Completed,
    Unknown,
    Offline,
}

pub(crate) fn session_filter_label(locale: UiLocale, filter: SessionFilterLabel) -> &'static str {
    match filter {
        SessionFilterLabel::All => text(locale, Message::AllSessions),
        SessionFilterLabel::Idle => text(locale, Message::Idle),
        SessionFilterLabel::Working => text(locale, Message::Working),
        SessionFilterLabel::Waiting => text(locale, Message::Waiting),
        SessionFilterLabel::Completed => text(locale, Message::Completed),
        SessionFilterLabel::Unknown => text(locale, Message::Unknown),
        SessionFilterLabel::Offline => text(locale, Message::Offline),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionStatusLabel {
    Idle,
    Working,
    Waiting,
    Completed,
    Unknown,
}

pub(crate) fn session_status_label(locale: UiLocale, status: SessionStatusLabel) -> &'static str {
    match status {
        SessionStatusLabel::Idle => text(locale, Message::Idle),
        SessionStatusLabel::Working => text(locale, Message::Working),
        SessionStatusLabel::Waiting => text(locale, Message::Waiting),
        SessionStatusLabel::Completed => text(locale, Message::Completed),
        SessionStatusLabel::Unknown => text(locale, Message::Unknown),
    }
}

pub(crate) fn session_source(locale: UiLocale, source_id: u64, terminal: &str) -> String {
    format!(
        "{} #{source_id} · {} {terminal}",
        text(locale, Message::Source),
        text(locale, Message::Terminal)
    )
}

pub(crate) fn session_pane(locale: UiLocale, pane: &str) -> String {
    format!("{} {pane}", text(locale, Message::Pane))
}

pub(crate) fn offline_status(locale: UiLocale, status: &str) -> String {
    format!(
        "{} · {} {status}",
        text(locale, Message::Offline),
        text(locale, Message::LastKnown)
    )
}

pub(crate) fn operation_pending(locale: UiLocale) -> &'static str {
    text(locale, Message::ActivePending)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackOperationState<'a> {
    Accepted,
    Preparing,
    Applying,
    CommittedPendingApply,
    Completed,
    Failed,
    Canceled,
    DurabilityUnknown,
    Unknown(&'a str),
}

pub(crate) fn pack_operation_state(raw: &str) -> PackOperationState<'_> {
    match raw {
        "accepted" => PackOperationState::Accepted,
        "preparing" => PackOperationState::Preparing,
        "applying" => PackOperationState::Applying,
        "committed_pending_apply" => PackOperationState::CommittedPendingApply,
        "completed" => PackOperationState::Completed,
        "failed" => PackOperationState::Failed,
        "canceled" => PackOperationState::Canceled,
        "durability_unknown" => PackOperationState::DurabilityUnknown,
        other => PackOperationState::Unknown(other),
    }
}

pub(crate) fn pack_operation_state_label(locale: UiLocale, state: &str) -> Cow<'_, str> {
    match pack_operation_state(state) {
        PackOperationState::Accepted => Cow::Borrowed(text(locale, Message::Accepted)),
        PackOperationState::Preparing => Cow::Borrowed(text(locale, Message::Preparing)),
        PackOperationState::Applying => Cow::Borrowed(text(locale, Message::Applying)),
        PackOperationState::CommittedPendingApply => {
            Cow::Borrowed(text(locale, Message::CommittedPendingApply))
        }
        PackOperationState::Completed => Cow::Borrowed(text(locale, Message::Completed)),
        PackOperationState::Failed => Cow::Borrowed(text(locale, Message::Failed)),
        PackOperationState::Canceled => Cow::Borrowed(text(locale, Message::Canceled)),
        PackOperationState::DurabilityUnknown => {
            Cow::Borrowed(text(locale, Message::DurabilityUnknown))
        }
        PackOperationState::Unknown(raw) => Cow::Borrowed(raw),
    }
}

pub(crate) fn pack_operation(
    locale: UiLocale,
    operation_id: &str,
    state: &str,
    error: Option<&str>,
) -> String {
    let state = pack_operation_state_label(locale, state);
    let mut label = String::new();
    let _ = write!(
        label,
        "{} {operation_id}: {state}",
        text(locale, Message::Operation)
    );
    if let Some(error) = error {
        let _ = write!(label, " ({error})");
    }
    label
}

pub(crate) fn revision_label(locale: UiLocale, revision: u64) -> String {
    format!("{} {revision}", text(locale, Message::Revision))
}

pub(crate) fn language_save_failure(locale: UiLocale, error: &str) -> String {
    format!("{}: {error}", text(locale, Message::LanguageSaveFailed))
}

pub(crate) fn remove_character_confirmation(locale: UiLocale, id: &str) -> String {
    match locale {
        UiLocale::Ko => {
            format!("{id} 및 보관된 모든 리비전을 삭제할까요? 이 작업은 취소할 수 없습니다.")
        }
        UiLocale::En => {
            format!("Remove {id} and all retained revisions? This cannot be undone.")
        }
    }
}

pub(crate) fn pack_inspect_details(
    locale: UiLocale,
    name: &str,
    id: &str,
    head: u64,
    revisions: &[u64],
) -> String {
    let rev_list = if revisions.is_empty() {
        "-".to_owned()
    } else {
        revisions
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match locale {
        UiLocale::Ko => format!(
            "이름: {name}\n패키지 ID: {id}\n최신 리비전 (Head): rev.{head}\n보관된 리비전: [{rev_list}]"
        ),
        UiLocale::En => format!(
            "Name: {name}\nPackage ID: {id}\nHead Revision: rev.{head}\nRetained Revisions: [{rev_list}]"
        ),
    }
}

pub(crate) fn character_diagnostics_details(
    locale: UiLocale,
    selected: &str,
    active: Option<&str>,
    override_active: bool,
    generation: u64,
    error: Option<&str>,
    operation: Option<&str>,
) -> String {
    let active_str = active.unwrap_or(if matches!(locale, UiLocale::Ko) {
        "대기 중"
    } else {
        "Pending"
    });
    let err_str = error.unwrap_or("-");
    let op_str = operation.unwrap_or("-");
    let override_str = if override_active {
        if matches!(locale, UiLocale::Ko) {
            "예 (Override)"
        } else {
            "Yes (Override)"
        }
    } else if matches!(locale, UiLocale::Ko) {
        "아니오 (Normal)"
    } else {
        "No (Normal)"
    };
    match locale {
        UiLocale::Ko => format!(
            "선택된 캐릭터: {selected}\n활성 캐릭터: {active_str}\n재정의 상태: {override_str}\n세대 (Generation): {generation}\n작업 상태: {op_str}\n오류: {err_str}"
        ),
        UiLocale::En => format!(
            "Selected: {selected}\nActive: {active_str}\nOverride Active: {override_str}\nGeneration: {generation}\nOperation: {op_str}\nError: {err_str}"
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DefaultDialogue {
    Completion,
    HeadTap,
    BodyTap,
    Pet,
}

pub(crate) const fn default_dialogue(locale: UiLocale, key: DefaultDialogue) -> &'static str {
    match locale {
        UiLocale::Ko => match key {
            DefaultDialogue::Completion => "완료를 확인했어요.",
            DefaultDialogue::HeadTap => "안녕하세요.",
            DefaultDialogue::BodyTap => "간지러워요.",
            DefaultDialogue::Pet => "쓰다듬어 주셔서 좋아요.",
        },
        UiLocale::En => match key {
            DefaultDialogue::Completion => "Completion observed",
            DefaultDialogue::HeadTap => "Hello.",
            DefaultDialogue::BodyTap => "That tickles.",
            DefaultDialogue::Pet => "Nice petting.",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_honors_explicit_and_ordered_primary_tags() {
        assert_eq!(
            resolve_language(LanguagePreference::Ko, &["en-US"]),
            UiLocale::Ko
        );
        assert_eq!(
            resolve_language(LanguagePreference::En, &["ko-KR"]),
            UiLocale::En
        );
        assert_eq!(
            resolve_language(LanguagePreference::System, &["ja-JP", "KO_kr"]),
            UiLocale::Ko
        );
        assert_eq!(
            resolve_language(LanguagePreference::System, &["fr-FR", "en-US"]),
            UiLocale::En
        );
        assert_eq!(
            resolve_language(LanguagePreference::System, &["", "zh-Hant"]),
            UiLocale::En
        );
    }

    #[test]
    fn invalid_language_deserializes_to_system_and_serialization_is_canonical() {
        assert_eq!(
            serde_json::from_str::<LanguagePreference>("\"future\"").unwrap(),
            LanguagePreference::System
        );
        assert_eq!(
            serde_json::from_str::<LanguagePreference>("42").unwrap(),
            LanguagePreference::System
        );
        assert_eq!(
            serde_json::from_str::<LanguagePreference>("null").unwrap(),
            LanguagePreference::System
        );
        assert_eq!(
            serde_json::to_string(&LanguagePreference::Ko).unwrap(),
            "\"ko\""
        );
        assert_eq!(
            serde_json::to_string(&LanguagePreference::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn plural_boundaries_and_unknown_operation_text_are_preserved() {
        assert_eq!(task_disclosure(UiLocale::En, 1), "View 1 task");
        assert_eq!(task_disclosure(UiLocale::En, 2), "View 2 tasks");
        assert_eq!(
            disconnected_sources(UiLocale::En, 1),
            "1 disconnected source"
        );
        assert_eq!(
            disconnected_sources(UiLocale::En, 2),
            "2 disconnected sources"
        );
        assert_eq!(
            session_summary(UiLocale::En, 2, 1, 1),
            "1 of 2 matched · +1 omitted"
        );
        assert_eq!(
            pack_operation_state_label(UiLocale::Ko, "future_state"),
            "future_state"
        );
    }

    #[test]
    fn language_labels_and_system_tooltip_preserved() {
        assert_eq!(text(UiLocale::Ko, Message::LanguageSystem), "시스템");
        assert_eq!(text(UiLocale::En, Message::LanguageSystem), "System");
        assert_eq!(
            text(UiLocale::Ko, Message::FollowSystemSettings),
            "시스템 설정 따르기"
        );
        assert_eq!(
            text(UiLocale::En, Message::FollowSystemSettings),
            "Follow System Settings"
        );
    }
}
