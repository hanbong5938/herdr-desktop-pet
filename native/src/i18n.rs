use crate::dialogue::DialogueSlot;
use crate::session_view::{DisplayStatus, SessionSort, SessionStatusSummary};
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

pub(crate) const fn cli_preference_feedback(locale: UiLocale, saved: bool) -> &'static str {
    match (locale, saved) {
        (UiLocale::Ko, true) => "자동화 설정 저장됨",
        (UiLocale::En, true) => "Automation settings saved",
        (UiLocale::Ko, false) => "자동화 설정 실패",
        (UiLocale::En, false) => "Automation settings failed",
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CliPromptFeedback {
    Sending,
    Acknowledged,
    Failed,
    UnknownDelivery,
}

pub(crate) const fn cli_prompt_feedback(
    locale: UiLocale,
    feedback: CliPromptFeedback,
) -> &'static str {
    match (locale, feedback) {
        (UiLocale::Ko, CliPromptFeedback::Sending) => "Herdr에 프롬프트 전송 중",
        (UiLocale::En, CliPromptFeedback::Sending) => "Sending prompt to Herdr",
        (UiLocale::Ko, CliPromptFeedback::Acknowledged) => "Herdr가 프롬프트를 수락함",
        (UiLocale::En, CliPromptFeedback::Acknowledged) => "Herdr acknowledged prompt",
        (UiLocale::Ko, CliPromptFeedback::Failed) => "프롬프트 전송 실패",
        (UiLocale::En, CliPromptFeedback::Failed) => "Prompt delivery failed",
        (UiLocale::Ko, CliPromptFeedback::UnknownDelivery) => "프롬프트 전달 여부 불확실",
        (UiLocale::En, CliPromptFeedback::UnknownDelivery) => "Prompt delivery uncertain",
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
    ResetBubbleSize,
    ResizeBubbleHelp,
    BubbleSizeSaveFailed,
    InvalidBubbleColor,
    BubbleColorsReload,
    BubbleColorsRebase,
    BubbleColorDraftConflict,
    PreferenceRevisionConflict,
    BubbleAppearanceSaveFailure,
    PresentationSaveFailure,
    StatusIndicatorsSaveFailure,
    MenuClickBehavior,
    MenuFullPassthrough,
    MenuAlphaPassthrough,
    MenuFullPassthroughHelp,
    MenuAlphaPassthroughHelp,
    MenuScale,
    MenuAppearance,
    MenuLanguage,
    MenuBarIcon,
    MenuBarVisibility,
    MenuBarIconDefault,
    MenuBarIconCustom,
    MenuBarIconChoose,
    MenuBarIconRestore,
    MenuBarIconHelp,
    MenuBarIconImportFailure,
    MenuBarIconSaveFailure,
    MenuBarIconLoadFailure,
    MenuBarAlways,
    MenuBarRecoveryOnly,
    MenuBarModeHelp,
    MenuBarModeSaveFailure,
    ContextSettings,
    ContextShowCharacter,
    ContextHideCharacter,
    ContextShowBubble,
    ContextHideBubble,
    MenuBarMenuTitle,
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
    AppUpdateTitle,
    AppUpdateAutoCheck,
    AppUpdateCheckNow,
    AppUpdateNotChecked,
    AppUpdateChecking,
    AppUpdateCurrent,
    AppUpdateAvailable,
    AppUpdateInstalled,
    AppUpdateApplied,
    AppUpdateLocal,
    AppUpdateUnsupported,
    AppUpdateFailed,
    AppUpdateLastChecked,
    AppUpdateNeverChecked,
    AppUpdateInstall,
    AppUpdateRebuild,
    AppUpdateApply,
    AppUpdateRunning,
    AppUpdateConfirmTitle,
    AppUpdateConfirmBody,
    AppUpdateLater,
    AppUpdateSource,
    AppUpdateAutomaticHelp,
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
    DialogueDraftConflict,
    DialogueReloadSaved,
    DialogueRebaseDraft,
    FinishMarkedText,
    CliDialoguePending,
    CliDialogueSaved,
    CliDialogueConflict,
    CliDialogueFailed,
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
    WorktreeRemove,
    WorktreeRemoveConfirm,
    WorktreeRemoveAction,
    WorktreeRemoving,
    WorktreeConfirmUnavailable,
    WorktreeCompactUnknownDelivery,
    WorktreeCompactRejected,
    WorktreeCompactUnavailable,
    WorktreeCompactFailed,
    WorktreeBusy,
    WorktreeOffline,
    WorktreeReadOnly,
    WorktreeStale,
    WorktreeNotLinked,
    WorktreeUnsupported,
    WorktreeUnknownDelivery,
    WorktreeRejected,
    WorktreeFailed,
    CliWorktreePending,
    CliWorktreeAcknowledged,
    CliWorktreeObserved,
    CliWorktreeUncertain,
    CliWorktreeConflict,
    FullSpeechBubbleMessage,
    AllSessions,
    SessionSearch,
    SessionStatusFilter,
    SessionSortLabel,
    SessionSortStable,
    SessionSortName,
    SessionSortSource,
    SessionRunningFirst,
    SessionListSaveFailure,
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
    Close,
    CharacterBrowserTitle,
    CharacterBrowserOpen,
    CharacterBrowserSearchPlaceholder,
    CharacterBrowserFilterAll,
    CharacterBrowserFilterInstalled,
    CharacterBrowserFilterOfficial,
    CharacterBrowserNoResults,
    CharacterBrowserDownloadAndApply,
    CharacterBrowserDownloading,
    CharacterBrowserCancelDownload,
    CharacterBrowserOfficialTag,
    CharacterBrowserSelect,
    CharacterBrowserSelected,
    CharacterBrowserUnsupportedFormat,
    OfficialCatalogUnavailable,
    OfficialUnsupportedFormat,
    OfficialLocalIdExists,
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
            Message::ResetBubbleSize => "자동 크기로 복원",
            Message::ResizeBubbleHelp => "말풍선의 가장자리나 모서리를 드래그하여 크기를 조절하세요.",
            Message::BubbleSizeSaveFailed => "말풍선 크기를 저장하지 못했습니다",
            Message::InvalidBubbleColor => "색상은 #RRGGBB 형식이어야 합니다",
            Message::BubbleColorsReload => "저장값 불러오기",
            Message::BubbleColorsRebase => "초안 기준 갱신",
            Message::BubbleColorDraftConflict => "저장 색상 변경됨 — 불러오기 / 기준 갱신",
            Message::PreferenceRevisionConflict => "설정 버전 충돌",
            Message::BubbleAppearanceSaveFailure => "말풍선 모양 설정 저장 실패",
            Message::PresentationSaveFailure => "펫 표시 설정 저장 실패",
            Message::StatusIndicatorsSaveFailure => "상태 아이콘·색상 설정 저장 실패",
            Message::MenuClickBehavior => "클릭 동작",
            Message::MenuFullPassthrough => "펫 전체 클릭 통과",
            Message::MenuAlphaPassthrough => "투명 영역만 클릭 통과",
            Message::MenuFullPassthroughHelp => "펫 전체에서 클릭이 통과합니다.",
            Message::MenuAlphaPassthroughHelp => "투명한 영역에서만 클릭이 통과합니다.",
            Message::MenuScale => "크기",
            Message::MenuAppearance => "외관",
            Message::MenuLanguage => "언어",
            Message::MenuBarIcon => "메뉴 막대 아이콘",
            Message::MenuBarVisibility => "표시 조건",
            Message::MenuBarIconDefault => "기본 아이콘",
            Message::MenuBarIconCustom => "사용자 지정 이미지",
            Message::MenuBarIconChoose => "이미지 선택…",
            Message::MenuBarIconRestore => "기본 아이콘 복원",
            Message::MenuBarIconHelp => "정적 PNG만 사용 가능 · 최대 4 MiB · 최대 100만 픽셀",
            Message::MenuBarIconImportFailure => "메뉴 막대 이미지 가져오기 실패",
            Message::MenuBarIconSaveFailure => "메뉴 막대 이미지 저장 실패",
            Message::MenuBarIconLoadFailure => "저장된 아이콘을 불러오지 못해 기본 아이콘을 표시합니다",
            Message::MenuBarAlways => "항상 표시",
            Message::MenuBarRecoveryOnly => "복구가 필요할 때만 표시",
            Message::MenuBarModeHelp => "조건부 모드에서는 캐릭터와 말풍선을 모두 숨기거나 전체 창 클릭 통과를 켜면 표시합니다.",
            Message::MenuBarModeSaveFailure => "메뉴 막대 아이콘 설정 저장 실패",
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
            Message::MenuBarMenuTitle => "Herdr Desktop Pet",
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
            Message::AppUpdateTitle => "앱 업데이트",
            Message::AppUpdateAutoCheck => "업데이트 자동 확인",
            Message::AppUpdateCheckNow => "지금 확인",
            Message::AppUpdateNotChecked => "아직 확인하지 않음",
            Message::AppUpdateChecking => "업데이트 확인 중…",
            Message::AppUpdateCurrent => "선택한 설치 소스에 업데이트 없음",
            Message::AppUpdateAvailable => "업데이트 가능",
            Message::AppUpdateInstalled => "업데이트 설치됨 · 재시작 필요",
            Message::AppUpdateApplied => "업데이트 적용 완료",
            Message::AppUpdateLocal => "로컬 빌드 사용 중",
            Message::AppUpdateUnsupported => "이 설치 방식은 앱에서 업데이트할 수 없음",
            Message::AppUpdateFailed => "업데이트 실패",
            Message::AppUpdateLastChecked => "마지막 확인",
            Message::AppUpdateNeverChecked => "확인 기록 없음",
            Message::AppUpdateInstall => "업데이트 설치…",
            Message::AppUpdateRebuild => "다시 빌드…",
            Message::AppUpdateApply => "설치된 버전으로 재시작…",
            Message::AppUpdateRunning => "실행 중인 버전",
            Message::AppUpdateConfirmTitle => "Herdr Desktop Pet을 업데이트할까요?",
            Message::AppUpdateConfirmBody => "업데이트를 적용하려면 현재 Herdr Desktop Pet을 종료하고 다시 시작해야 합니다. 설치 출처의 관리자가 패키지를 교체하거나 로컬 소스에서 빌드할 수 있습니다. 사전 빌드 파일을 사용할 수 없거나 설치가 실패하면 소스 빌드로 전환될 수도 있으므로 설치 출처와 빌드 과정을 신뢰해야 합니다. 새 버전을 시작할 수 없으면 이전 버전으로 자동 복구되지 않을 수 있으며 원래 설치 출처에서 직접 복구해야 할 수 있습니다.",
            Message::AppUpdateLater => "나중에",
            Message::AppUpdateSource => "설치 출처",
            Message::AppUpdateAutomaticHelp => "앱 실행 중 하루 간격으로 업데이트 여부만 확인합니다. 자동으로 설치하거나 재시작하지 않습니다.",
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
            Message::DialogueDraftConflict => "저장된 대사가 변경되었습니다. 초안을 다시 불러오거나 새 대사에 맞춰 재설정하세요.",
            Message::DialogueReloadSaved => "저장값 불러오기",
            Message::DialogueRebaseDraft => "초안 기준 갱신",
            Message::FinishMarkedText => "입력 중인 글자 조합을 완료하세요.",
            Message::CliDialoguePending => "자동화 대사 변경 적용 대기 중",
            Message::CliDialogueSaved => "자동화 대사 저장됨",
            Message::CliDialogueConflict => "자동화 대사 충돌 · 다시 읽어 확인하세요",
            Message::CliDialogueFailed => "자동화 대사 저장 실패",
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
            Message::WorktreeRemove => "워크트리 삭제…",
            Message::WorktreeRemoveConfirm => "이 워크트리를 삭제할까요?",
            Message::WorktreeRemoveAction => "워크트리 삭제",
            Message::WorktreeRemoving => "워크트리 삭제 중…",
            Message::WorktreeConfirmUnavailable => "삭제 확인 창을 안전하게 설정할 수 없습니다",
            Message::WorktreeCompactUnknownDelivery => "결과 불명.\n재시도 전 확인하세요",
            Message::WorktreeCompactRejected => "서버가 삭제를 거부했습니다",
            Message::WorktreeCompactUnavailable => "워크트리를 삭제할 수 없습니다",
            Message::WorktreeCompactFailed => "워크트리 삭제 실패",
            Message::WorktreeBusy => "다른 워크트리 삭제 작업이 진행 중입니다",
            Message::WorktreeOffline => "로컬 Herdr 소스가 오프라인입니다",
            Message::WorktreeReadOnly => "원격 관찰 소스는 읽기 전용입니다",
            Message::WorktreeStale => "워크트리 또는 영향받는 세션이 변경되었습니다. 다시 확인하세요",
            Message::WorktreeNotLinked => "연결된 워크트리가 아닙니다",
            Message::WorktreeUnsupported => "이 Herdr 서버는 worktree.remove를 지원하지 않습니다",
            Message::WorktreeUnknownDelivery => "삭제 결과를 확인할 수 없습니다. 확인 없이 다시 시도하지 마세요",
            Message::WorktreeRejected => "Herdr 서버가 워크트리 삭제를 거부했습니다",
            Message::WorktreeFailed => "워크트리를 삭제하지 못했습니다",
            Message::CliWorktreePending => "자동화 워크트리 삭제 요청 중",
            Message::CliWorktreeAcknowledged => "Herdr가 삭제 요청을 수락함 · 관찰 대기 중",
            Message::CliWorktreeObserved => "워크트리 삭제가 관찰됨",
            Message::CliWorktreeUncertain => "워크트리 삭제 결과 불확실 · 재시도 전 확인하세요",
            Message::CliWorktreeConflict => "워크트리 상태가 변경됨 · 다시 확인하세요",
            Message::FullSpeechBubbleMessage => "전체 말풍선 메시지",
            Message::AllSessions => "전체",
            Message::SessionSearch => "검색",
            Message::SessionStatusFilter => "상태",
            Message::SessionSortLabel => "정렬",
            Message::SessionSortStable => "기본순",
            Message::SessionSortName => "이름순",
            Message::SessionSortSource => "출처순",
            Message::SessionRunningFirst => "실행 중 우선",
            Message::SessionListSaveFailure => "세션 목록 설정 저장 실패",
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
            Message::NoMatches => "결과 없음",
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
            Message::Close => "닫기",
            Message::CharacterBrowserTitle => "캐릭터 브라우저",
            Message::CharacterBrowserOpen => "캐릭터 브라우저 열기…",
            Message::CharacterBrowserSearchPlaceholder => "이름, ID, 태그로 검색…",
            Message::CharacterBrowserFilterAll => "전체",
            Message::CharacterBrowserFilterInstalled => "설치됨",
            Message::CharacterBrowserFilterOfficial => "공식 카탈로그",
            Message::CharacterBrowserNoResults => "검색 결과가 없습니다",
            Message::CharacterBrowserDownloadAndApply => "다운로드 및 적용",
            Message::CharacterBrowserDownloading => "다운로드 중…",
            Message::CharacterBrowserCancelDownload => "다운로드 취소",
            Message::CharacterBrowserOfficialTag => "공식",
            Message::CharacterBrowserSelect => "선택",
            Message::CharacterBrowserSelected => "선택됨",
            Message::CharacterBrowserUnsupportedFormat => "지원되지 않는 형식",
            Message::OfficialCatalogUnavailable => "공식 카탈로그를 사용할 수 없습니다",
            Message::OfficialUnsupportedFormat => "지원되지 않는 공식 캐릭터 형식입니다",
            Message::OfficialLocalIdExists => "같은 ID의 팩이 이미 설치되어 있습니다. 캐릭터 카드에서 선택하세요",
        },
        UiLocale::En => match message {
            Message::ResetPosition => "Reset Position",
            Message::ScaleUp => "Scale Up",
            Message::ScaleDown => "Scale Down",
            Message::FollowSystemSettings => "Follow System Settings",
            Message::LanguageSystem => "System",
            Message::KoreanLanguage => "한국어",
            Message::EnglishLanguage => "English",
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
            Message::ResetBubbleSize => "Restore automatic size",
            Message::ResizeBubbleHelp => "Drag any edge or corner of the bubble to resize it.",
            Message::BubbleSizeSaveFailed => "Bubble size could not be saved",
            Message::InvalidBubbleColor => "Colors must use #RRGGBB",
            Message::BubbleAppearanceSaveFailure => "Bubble appearance could not be saved",
            Message::BubbleColorsReload => "Reload saved",
            Message::BubbleColorsRebase => "Rebase draft",
            Message::BubbleColorDraftConflict => "Saved colors changed — reload or rebase",
            Message::PreferenceRevisionConflict => "Settings revision conflict",
            Message::PresentationSaveFailure => "Pet display settings could not be saved",
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
            Message::MenuBarIcon => "Menu bar icon",
            Message::MenuBarVisibility => "Visibility",
            Message::MenuBarIconDefault => "Default icon",
            Message::MenuBarIconCustom => "Custom image",
            Message::MenuBarIconChoose => "Choose image…",
            Message::MenuBarIconRestore => "Restore default icon",
            Message::MenuBarIconHelp => "Static PNG only · up to 4 MiB · up to 1 million pixels",
            Message::MenuBarIconImportFailure => "Could not import menu bar image",
            Message::MenuBarIconSaveFailure => "Could not save menu bar image",
            Message::MenuBarIconLoadFailure => "Could not load saved icon; showing the default icon",
            Message::MenuBarAlways => "Always show",
            Message::MenuBarRecoveryOnly => "Only when recovery is needed",
            Message::MenuBarModeHelp => "In conditional mode, show when both character and bubble are hidden or full-window click-through is on.",
            Message::MenuBarModeSaveFailure => "Menu bar icon setting could not be saved",
            Message::ContextSettings => "Settings…",
            Message::ContextShowCharacter => "Show Character",
            Message::ContextHideCharacter => "Hide Character",
            Message::ContextShowBubble => "Show Bubble Window",
            Message::ContextHideBubble => "Hide Bubble Window",
            Message::MenuBarMenuTitle => "Herdr Desktop Pet",
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
            Message::AppUpdateTitle => "App updates",
            Message::AppUpdateAutoCheck => "Automatically check for updates",
            Message::AppUpdateCheckNow => "Check now",
            Message::AppUpdateNotChecked => "Not checked yet",
            Message::AppUpdateChecking => "Checking for updates…",
            Message::AppUpdateCurrent => "No update from selected source",
            Message::AppUpdateAvailable => "Update available",
            Message::AppUpdateInstalled => "Update installed · restart needed",
            Message::AppUpdateApplied => "Update applied",
            Message::AppUpdateLocal => "Local build",
            Message::AppUpdateUnsupported => "This installation cannot be updated in the app",
            Message::AppUpdateFailed => "Update failed",
            Message::AppUpdateLastChecked => "Last checked",
            Message::AppUpdateNeverChecked => "Never checked",
            Message::AppUpdateInstall => "Install update…",
            Message::AppUpdateRebuild => "Rebuild…",
            Message::AppUpdateApply => "Restart with installed version…",
            Message::AppUpdateRunning => "Running version",
            Message::AppUpdateConfirmTitle => "Update Herdr Desktop Pet?",
            Message::AppUpdateConfirmBody => "Applying the update will stop and restart Herdr Desktop Pet. Your installation manager may replace the package or build from local source. If a prebuilt download is unavailable or fails, installation may fall back to a source build, so you must trust the installation source and its build process. If the new version cannot start, the old version may not be restored automatically; you may need to recover manually from the original installation source.",
            Message::AppUpdateLater => "Later",
            Message::AppUpdateSource => "Installation source",
            Message::AppUpdateAutomaticHelp => "Checks for updates at most once per day while the app runs. It never installs updates or restarts automatically.",
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
            Message::DialogueDraftConflict => "Saved dialogue changed. Reload it or rebase your draft.",
            Message::DialogueReloadSaved => "Reload saved",
            Message::DialogueRebaseDraft => "Rebase draft",
            Message::FinishMarkedText => "Finish composing text first.",
            Message::CliDialoguePending => "Automation dialogue awaiting display",
            Message::CliDialogueSaved => "Automation dialogue saved",
            Message::CliDialogueConflict => "Automation dialogue conflict · read again",
            Message::CliDialogueFailed => "Automation dialogue save failed",
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
            Message::WorktreeRemove => "Remove Worktree…",
            Message::WorktreeRemoveConfirm => "Remove this worktree?",
            Message::WorktreeRemoveAction => "Remove Worktree",
            Message::WorktreeRemoving => "Removing worktree…",
            Message::WorktreeConfirmUnavailable => "Could not safely configure the removal confirmation",
            Message::WorktreeCompactUnknownDelivery => "Outcome unknown.\nCheck before retrying",
            Message::WorktreeCompactRejected => "Server refused removal",
            Message::WorktreeCompactUnavailable => "Cannot remove worktree",
            Message::WorktreeCompactFailed => "Worktree removal failed",
            Message::WorktreeBusy => "Another worktree removal is in progress",
            Message::WorktreeOffline => "The local Herdr source is offline",
            Message::WorktreeReadOnly => "Remote observation sources are read-only",
            Message::WorktreeStale => "The worktree or affected sessions changed. Review again",
            Message::WorktreeNotLinked => "This is not a linked worktree",
            Message::WorktreeUnsupported => "This Herdr server does not support worktree.remove",
            Message::WorktreeUnknownDelivery => "Removal outcome is unknown. Check before trying again",
            Message::WorktreeRejected => "The Herdr server rejected worktree removal",
            Message::WorktreeFailed => "Could not remove worktree",
            Message::CliWorktreePending => "Automation worktree removal requested",
            Message::CliWorktreeAcknowledged => "Herdr acknowledged removal · awaiting observation",
            Message::CliWorktreeObserved => "Worktree removal observed",
            Message::CliWorktreeUncertain => "Worktree removal uncertain · check before retrying",
            Message::CliWorktreeConflict => "Worktree changed · review again",
            Message::FullSpeechBubbleMessage => "Full speech bubble message",
            Message::AllSessions => "All",
            Message::SessionSearch => "Search",
            Message::SessionStatusFilter => "Status",
            Message::SessionSortLabel => "Sort",
            Message::SessionSortStable => "Stable",
            Message::SessionSortName => "Name",
            Message::SessionSortSource => "Source",
            Message::SessionRunningFirst => "Running first",
            Message::SessionListSaveFailure => "Could not save session list settings",
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
            Message::NoMatches => "No matching items",
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
            Message::Close => "Close",
            Message::CharacterBrowserTitle => "Character Browser",
            Message::CharacterBrowserOpen => "Browse Characters…",
            Message::CharacterBrowserSearchPlaceholder => "Search by name, ID, tags…",
            Message::CharacterBrowserFilterAll => "All",
            Message::CharacterBrowserFilterInstalled => "Installed",
            Message::CharacterBrowserFilterOfficial => "Official Catalog",
            Message::CharacterBrowserNoResults => "No characters found",
            Message::CharacterBrowserDownloadAndApply => "Download & Apply",
            Message::CharacterBrowserDownloading => "Downloading…",
            Message::CharacterBrowserCancelDownload => "Cancel Download",
            Message::CharacterBrowserOfficialTag => "Official",
            Message::CharacterBrowserSelect => "Select",
            Message::CharacterBrowserSelected => "Selected",
            Message::CharacterBrowserUnsupportedFormat => "Unsupported Format",
            Message::OfficialCatalogUnavailable => "Official catalog is unavailable",
            Message::OfficialUnsupportedFormat => "Unsupported official character format",
            Message::OfficialLocalIdExists => "A pack with this ID is already installed. Select it on its character card",
        },
    }
}

pub(crate) const fn session_local_source(locale: UiLocale) -> &'static str {
    text(locale, Message::ObservationLocal)
}

pub(crate) fn character_count_label(locale: UiLocale, count: usize) -> String {
    match locale {
        UiLocale::Ko => format!("캐릭터 {count}개"),
        UiLocale::En if count == 1 => "1 character".to_owned(),
        UiLocale::En => format!("{count} characters"),
    }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
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

pub(crate) fn session_list_summary(locale: UiLocale, matched: usize, omitted: usize) -> String {
    match locale {
        UiLocale::Ko => format!("일치: {matched}개\n제외: {omitted}개"),
        UiLocale::En => format!("Matched: {matched}\nOmitted: {omitted}"),
    }
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

pub(crate) fn session_sort_title(
    locale: UiLocale,
    sort: SessionSort,
    running_first: bool,
) -> &'static str {
    match (locale, sort, running_first) {
        (UiLocale::Ko, SessionSort::Stable, false) => text(locale, Message::SessionSortStable),
        (UiLocale::Ko, SessionSort::TitleAsc, false) => text(locale, Message::SessionSortName),
        (UiLocale::Ko, SessionSort::SourceAsc, false) => text(locale, Message::SessionSortSource),
        (UiLocale::En, SessionSort::Stable, false) => text(locale, Message::SessionSortStable),
        (UiLocale::En, SessionSort::TitleAsc, false) => text(locale, Message::SessionSortName),
        (UiLocale::En, SessionSort::SourceAsc, false) => text(locale, Message::SessionSortSource),
        (UiLocale::Ko, SessionSort::Stable, true) => "기본순 · 실행 중 우선",
        (UiLocale::Ko, SessionSort::TitleAsc, true) => "이름순 · 실행 중 우선",
        (UiLocale::Ko, SessionSort::SourceAsc, true) => "출처순 · 실행 중 우선",
        (UiLocale::En, SessionSort::Stable, true) => "Stable · Running first",
        (UiLocale::En, SessionSort::TitleAsc, true) => "Name · Running first",
        (UiLocale::En, SessionSort::SourceAsc, true) => "Source · Running first",
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

pub(crate) fn worktree_remove_confirmation(
    locale: UiLocale,
    repo_name: &str,
    checkout_path: &str,
    tabs: usize,
    panes: usize,
) -> String {
    match locale {
        UiLocale::Ko => format!(
            "저장소: {repo_name}\n삭제할 체크아웃: {checkout_path}\n\n이 워크트리의 전체 작업 공간에서 탭 {tabs}개와 패널 {panes}개가 닫히며 실행 중인 프로세스와 에이전트가 종료됩니다. 선택한 카드만 닫는 작업이 아닙니다.\n\n무시된 파일(예: 빌드 결과물)은 삭제됩니다. 변경된 추적 파일이나 추적되지 않은 파일이 있으면 서버가 삭제를 거부할 수 있습니다. Git 브랜치는 유지됩니다. 이 작업은 되돌릴 수 없습니다."
        ),
        UiLocale::En => format!(
            "Repository: {repo_name}\nCheckout to remove: {checkout_path}\n\nAcross this entire workspace, {tabs} tab(s) and {panes} pane(s) will close, terminating running processes and agents—not just the selected card.\n\nIgnored files (such as build outputs) will be deleted. The server may refuse removal if tracked or untracked files are dirty. The Git branch remains. This cannot be undone."
        ),
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
            pack_operation_state_label(UiLocale::Ko, "future_state"),
            "future_state"
        );
    }

    #[test]
    fn worktree_confirmation_names_exact_checkout_impact_and_ignored_files() {
        for locale in [UiLocale::Ko, UiLocale::En] {
            let warning = worktree_remove_confirmation(locale, "repo", "/checkout/linked-b", 2, 3);
            assert!(warning.contains("/checkout/linked-b"));
            assert!(warning.contains('2'));
            assert!(warning.contains('3'));
            assert!(warning.contains(match locale {
                UiLocale::Ko => "무시된 파일",
                UiLocale::En => "Ignored files",
            }));
        }
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
