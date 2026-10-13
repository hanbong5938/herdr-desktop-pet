//! On-demand embedded plugin management; no host work before an explicit open.
use crate::plugin_manager_backend::{
    Action, OperationObservation, Plugin, ProfileOperationStore, Target,
};
use crate::plugin_manager_i18n::{text, Message, UiLocale};
use desktop_pet_contracts::{ProfilePaths, ProfileStore};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertSecondButtonReturn, NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType,
    NSBezelStyle, NSBezierPath, NSButton, NSColor, NSFloatingWindowLevel, NSFont, NSPopUpButton,
    NSTextField, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) const CARD_HEIGHT: f64 = 164.0;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
fn request_id() -> u64 {
    NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
}
fn tr(
    locale: UiLocale,
    en: &'static str,
    ko: &'static str,
    zh_hans: &'static str,
    ja: &'static str,
) -> &'static str {
    match locale {
        UiLocale::En => en,
        UiLocale::Ko => ko,
        UiLocale::ZhHans => zh_hans,
        UiLocale::Ja => ja,
    }
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
fn label(value: &str, frame: NSRect, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    field.setFrame(frame);
    field
}
fn button(
    value: &str,
    action: objc2::runtime::Sel,
    target: &AnyObject,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(value),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setFrame(frame);
    button
}

// SAFETY: both views/controllers are constructed and accessed on the AppKit main thread.
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetPluginManagerCard"]
    struct CardView;
    unsafe impl NSObjectProtocol for CardView {}
    impl CardView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.161, 0.161, 0.176, 1.0).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(), 10.0, 10.0).fill();
        }
    }
);

pub(crate) struct PluginManagerCard {
    view: Retained<CardView>,
    heading: Retained<NSTextField>,
    help: Retained<NSTextField>,
    status: Retained<NSTextField>,
    open: Retained<NSButton>,
    install: Retained<NSButton>,
    remove: Retained<NSButton>,
    locale: UiLocale,
    installed: bool,
    busy: bool,
    error: Option<String>,
}
impl PluginManagerCard {
    pub(crate) fn new(locale: UiLocale, target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let view: Retained<CardView> = unsafe {
            msg_send![super(CardView::alloc(mtm).set_ivars(())), initWithFrame: NSRect::default()]
        };
        let heading = label("", NSRect::default(), mtm);
        heading.setFont(Some(&NSFont::boldSystemFontOfSize(12.5)));
        let help = label("", NSRect::default(), mtm);
        let status = label("", NSRect::default(), mtm);
        status.setMaximumNumberOfLines(4);
        let create = |action| {
            let b = button("", action, target, NSRect::default(), mtm);
            b.setBezelStyle(NSBezelStyle::Push);
            b
        };
        let open = create(sel!(openPluginManager:));
        let install = create(sel!(installPluginManager:));
        let remove = create(sel!(removePluginManager:));
        for child in [
            &*heading as &NSView,
            &*help,
            &*status,
            &*open,
            &*install,
            &*remove,
        ] {
            view.addSubview(child);
        }
        let mut card = Self {
            view,
            heading,
            help,
            status,
            open,
            install,
            remove,
            locale,
            installed: false,
            busy: false,
            error: None,
        };
        card.set_locale(locale);
        card
    }
    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }
    pub(crate) fn layout(&self, frame: NSRect) {
        self.view.setFrame(frame);
        let width = frame.size.width;
        let full = (width - 28.0).max(1.0);
        self.heading.setFrame(rect(14.0, 8.0, full, 20.0));
        self.help.setFrame(rect(14.0, 29.0, full, 34.0));
        let third = (width - 42.0) / 3.0;
        self.open.setFrame(rect(14.0, 68.0, third, 28.0));
        self.install.setFrame(rect(21.0 + third, 68.0, third, 28.0));
        self.remove
            .setFrame(rect(28.0 + 2.0 * third, 68.0, third, 28.0));
        self.status.setFrame(rect(14.0, 99.0, full, 60.0));
    }
    pub(crate) fn locale(&self) -> UiLocale {
        self.locale
    }
    pub(crate) fn installed(&self) -> bool {
        self.installed
    }
    pub(crate) fn sync(&mut self, installed: bool) {
        self.installed = installed;
        self.update();
    }
    pub(crate) fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        self.update();
    }
    pub(crate) fn report_error(&mut self, error: Option<String>) {
        self.error = error;
        self.update();
    }
    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.locale = locale;
        self.heading.setStringValue(&NSString::from_str(text(
            locale,
            Message::PluginManagerTitle,
        )));
        self.help.setStringValue(&NSString::from_str(text(
            locale,
            Message::PluginManagerHelp,
        )));
        self.open.setTitle(&NSString::from_str(text(
            locale,
            Message::PluginManagerOpen,
        )));
        self.install.setTitle(&NSString::from_str(text(
            locale,
            Message::PluginManagerInstall,
        )));
        self.remove.setTitle(&NSString::from_str(text(
            locale,
            Message::PluginManagerRemove,
        )));
        self.update();
    }
    fn update(&self) {
        self.open.setEnabled(self.installed && !self.busy);
        self.install.setEnabled(!self.installed && !self.busy);
        self.remove.setEnabled(self.installed && !self.busy);
        let status = if self.busy {
            text(self.locale, Message::PluginManagerSaving)
        } else if let Some(error) = &self.error {
            error
        } else if self.installed {
            text(self.locale, Message::PluginManagerAvailable)
        } else {
            text(self.locale, Message::PluginManagerUnavailable)
        };
        self.status.setStringValue(&NSString::from_str(status));
        self.status.setToolTip(Some(&NSString::from_str(status)));
        let accessibility = NSString::from_str(status);
        unsafe {
            let _: () = msg_send![&*self.status, setAccessibilityLabel: Some(&*accessibility)];
        }
        let status_color = if self.error.is_some() {
            NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.54, 0.50, 1.0)
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.60, 0.61, 0.66, 1.0)
        };
        self.status.setTextColor(Some(&status_color));
    }
}

struct Inventory {
    target: Option<Target>,
    plugins: Vec<Plugin>,
    protection: Vec<Option<String>>,
    support: Result<(), String>,
    observation: OperationObservation,
}
struct WindowState {
    window: Retained<NSWindow>,
    _target: Retained<Controller>,
    controller: u64,
    paths: ProfilePaths,
    profile: ProfileStore,
    socket: PathBuf,
    locale: UiLocale,
    generation: u64,
    busy: bool,
    feedback: Option<String>,
    inventory: Option<Inventory>,
    selection: Retained<NSPopUpButton>,
    details: Retained<NSTextField>,
    status: Retained<NSTextField>,
    refresh: Retained<NSButton>,
    review: Retained<NSButton>,
    actions: [Retained<NSButton>; 5],
}
thread_local! { static WINDOW: RefCell<Option<WindowState>> = const { RefCell::new(None) }; }

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetEmbeddedPluginManagerController"]
    #[ivars = u64]
    struct Controller;
    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &objc2_foundation::NSNotification) {
            // Do not release the delegate from inside its own AppKit callback.
            let controller = *self.ivars();
            WINDOW.with(|slot| {
                if let Some(window) = slot.borrow_mut().as_mut().filter(|window| window.controller == controller) {
                    window.generation = request_id();
                }
            });
            DispatchQueue::main().exec_async(move || close_controller(controller));
        }
    }
    impl Controller {
        #[unsafe(method(refreshPlugins:))]
        fn refresh(&self, _sender: &AnyObject) { if self.is_current() { WINDOW.with(|slot| { if let Some(s) = slot.borrow_mut().as_mut() { s.refresh(); } }); } }
        #[unsafe(method(selectManagedPlugin:))]
        fn select(&self, _sender: &AnyObject) { if self.is_current() { WINDOW.with(|slot| { if let Some(s) = slot.borrow().as_ref() { s.show_selection(); } }); } }
        #[unsafe(method(reviewPluginEvidence:))]
        fn review(&self, _sender: &AnyObject) { if self.is_current() { review_unknown(self.mtm()); } }
        #[unsafe(method(enableManagedPlugin:))]
        fn enable(&self, _sender: &AnyObject) { if self.is_current() { act(Action::Enable, self.mtm()); } }
        #[unsafe(method(disableManagedPlugin:))]
        fn disable(&self, _sender: &AnyObject) { if self.is_current() { act(Action::Disable, self.mtm()); } }
        #[unsafe(method(unlinkManagedPlugin:))]
        fn unlink(&self, _sender: &AnyObject) { if self.is_current() { act(Action::Unlink, self.mtm()); } }
        #[unsafe(method(uninstallManagedPlugin:))]
        fn uninstall(&self, _sender: &AnyObject) { if self.is_current() { act(Action::Uninstall, self.mtm()); } }
        #[unsafe(method(updateManagedPlugin:))]
        fn update(&self, _sender: &AnyObject) { if self.is_current() { act(Action::Update, self.mtm()); } }
    }
);
impl Controller {
    fn is_current(&self) -> bool {
        WINDOW.with(|slot| {
            slot.borrow().as_ref().is_some_and(|window| {
                window.controller == *self.ivars() && window.window.isVisible()
            })
        })
    }
}
fn is_github(p: &Plugin) -> bool {
    p.source.get("kind").and_then(serde_json::Value::as_str) == Some("github")
}
fn managed_checkout(p: &Plugin) -> Option<&str> {
    let path = p
        .source
        .get("managed_path")
        .and_then(serde_json::Value::as_str)?;
    (is_github(p) && Path::new(path).is_absolute()).then_some(path)
}
fn eligible(action: Action, p: &Plugin) -> bool {
    match action {
        Action::Enable => !p.enabled,
        Action::Disable => p.enabled,
        Action::Unlink => p.source.get("kind").and_then(serde_json::Value::as_str) == Some("local"),
        Action::Uninstall | Action::Update => managed_checkout(p).is_some(),
    }
}
fn action_name(action: Action, locale: UiLocale) -> &'static str {
    match action {
        Action::Enable => tr(locale, "Enable", "활성화", "启用", "有効にする"),
        Action::Disable => tr(locale, "Disable", "비활성화", "禁用", "無効にする"),
        Action::Unlink => tr(locale, "Unlink", "연결 해제", "解除关联", "登録を解除"),
        Action::Uninstall => tr(
            locale,
            "Delete plugin",
            "플러그인 삭제",
            "删除插件",
            "プラグインを削除",
        ),
        Action::Update => tr(locale, "Update", "업데이트", "更新", "更新"),
    }
}
fn set_status(s: &WindowState, message: &str) {
    s.status.setStringValue(&NSString::from_str(message));
    s.status.setToolTip(Some(&NSString::from_str(message)));
}
impl WindowState {
    fn selected(&self) -> Option<(usize, &Plugin)> {
        let index = usize::try_from(self.selection.indexOfSelectedItem()).ok()?;
        self.inventory
            .as_ref()?
            .plugins
            .get(index)
            .map(|p| (index, p))
    }
    fn controls(&self) {
        self.refresh.setEnabled(!self.busy);
        let inventory = self.inventory.as_ref();
        self.review.setEnabled(
            !self.busy
                && inventory.is_some_and(|i| {
                    matches!(
                        i.observation,
                        OperationObservation::Unknown {
                            acknowledged: false,
                            ..
                        }
                    )
                }),
        );
        self.selection.setEnabled(!self.busy && inventory.is_some());
        for (index, button) in self.actions.iter().enumerate() {
            let action = [
                Action::Enable,
                Action::Disable,
                Action::Unlink,
                Action::Uninstall,
                Action::Update,
            ][index];
            button.setEnabled(
                !self.busy
                    && inventory.is_some_and(|i| {
                        i.target.is_some()
                            && i.support.is_ok()
                            && !matches!(
                                i.observation,
                                OperationObservation::Running { .. }
                                    | OperationObservation::Unknown {
                                        acknowledged: false,
                                        ..
                                    }
                            )
                            && self.selected().is_some_and(|(n, p)| {
                                i.protection[n].is_none() && !p.protected() && eligible(action, p)
                            })
                    }),
            );
        }
    }
    fn show_selection(&self) {
        let value = if let (Some(i), Some((n, p))) = (&self.inventory, self.selected()) {
            let source_note = if let Some(path) = managed_checkout(p) {
                format!(
                    "{}: {path}",
                    tr(
                        self.locale,
                        "Managed checkout",
                        "관리형 체크아웃",
                        "托管检出目录",
                        "管理対象チェックアウト"
                    )
                )
            } else if is_github(p) {
                tr(
                    self.locale,
                    "Managed checkout unavailable; deletion and update are disabled.",
                    "관리형 체크아웃 경로가 없어 삭제 및 업데이트할 수 없습니다.",
                    "托管检出目录不可用，无法删除或更新。",
                    "管理対象チェックアウトが利用できないため、削除と更新は無効です。",
                )
                .to_owned()
            } else {
                tr(self.locale, "Local source remains on disk. Unlink only removes its host registration; update the source yourself.", "로컬 원본 파일은 유지됩니다. 연결 해제는 호스트 등록만 제거하며 원본 업데이트는 직접 진행하세요.", "本地源文件会保留在磁盘上。解除关联仅移除其在宿主中的注册；请自行更新源文件。", "ローカルのソースはディスクに残ります。登録解除はホストへの登録のみを削除します。ソースはご自身で更新してください。").to_owned()
            };
            format!(
                "{} ({}) • {}\nID: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}\n{}",
                p.name,
                p.version,
                if p.enabled {
                    tr(self.locale, "Enabled", "활성", "已启用", "有効")
                } else {
                    tr(self.locale, "Disabled", "비활성", "已禁用", "無効")
                },
                p.id,
                tr(self.locale, "Host", "호스트", "宿主", "ホスト"),
                i.target
                    .as_ref()
                    .map(|t| t.host.display().to_string())
                    .unwrap_or_default(),
                tr(
                    self.locale,
                    "Installation",
                    "설치 경로",
                    "安装路径",
                    "インストール先"
                ),
                p.root.display(),
                tr(self.locale, "Source", "소스", "来源", "ソース"),
                p.source,
                tr(self.locale, "Warnings", "경고", "警告", "警告"),
                if p.warnings.is_empty() {
                    tr(self.locale, "None", "없음", "无", "なし").to_owned()
                } else {
                    p.warnings.join("; ")
                },
                source_note,
                i.protection[n].as_deref().unwrap_or("")
            )
        } else {
            tr(
                self.locale,
                "Select a plugin to inspect its read-only details.",
                "읽기 전용 상세 정보를 보려면 플러그인을 선택하세요.",
                "选择插件以查看其只读详细信息。",
                "プラグインを選択すると、読み取り専用の詳細情報を確認できます。",
            )
            .to_owned()
        };
        self.details.setStringValue(&NSString::from_str(&value));
        self.details.setToolTip(Some(&NSString::from_str(&value)));
        self.controls();
    }
    fn refresh(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.generation = request_id();
        let generation = self.generation;
        let socket = self.socket.clone();
        let paths = self.paths.clone();
        let profile = self.profile.clone();
        self.inventory = None;
        self.selection.removeAllItems();
        self.show_selection();
        set_status(
            self,
            tr(
                self.locale,
                "Loading inventory and operation evidence…",
                "플러그인 목록과 작업 기록 불러오는 중…",
                "正在加载插件列表和操作记录…",
                "プラグイン一覧と操作記録を読み込み中…",
            ),
        );
        std::thread::spawn(move || {
            let result = (|| {
                let store = ProfileOperationStore::new(&paths, &profile)?;
                let observation = store.observe_active()?;
                let listing = Target::from_selected_socket(socket).and_then(|target| {
                    let plugins = target.list()?;
                    let protection = plugins
                        .iter()
                        .map(|p| target.protection_reason(p, &plugins))
                        .collect();
                    let support = target.mutation_support();
                    Ok::<_, String>((target, plugins, protection, support))
                });
                let (target, plugins, protection, support) = match listing {
                    Ok((target, plugins, protection, support)) => {
                        (Some(target), plugins, protection, support)
                    }
                    Err(error) => (None, Vec::new(), Vec::new(), Err(error)),
                };
                Ok::<_, String>(Inventory {
                    target,
                    plugins,
                    protection,
                    support,
                    observation,
                })
            })();
            DispatchQueue::main().exec_async(move || {
                WINDOW.with(|slot| {
                    if let Some(s) = slot
                        .borrow_mut()
                        .as_mut()
                        .filter(|s| s.generation == generation)
                    {
                        s.busy = false;
                        match result {
                            Ok(inventory) => s.present(inventory),
                            Err(error) => {
                                set_status(
                                    s,
                                    &format!(
                                        "{}: {error}",
                                        tr(
                                            s.locale,
                                            "Inventory error",
                                            "목록 오류",
                                            "插件列表错误",
                                            "一覧取得エラー"
                                        )
                                    ),
                                );
                                s.controls();
                            }
                        }
                    }
                })
            });
        });
    }
    fn present(&mut self, inventory: Inventory) {
        self.selection.removeAllItems();
        for (p, reason) in inventory.plugins.iter().zip(&inventory.protection) {
            self.selection
                .addItemWithTitle(&NSString::from_str(&format!(
                    "{} — {} ({}){}",
                    p.name,
                    p.id,
                    if p.enabled {
                        tr(self.locale, "enabled", "활성", "已启用", "有効")
                    } else {
                        tr(self.locale, "disabled", "비활성", "已禁用", "無効")
                    },
                    if reason.is_some() || p.protected() {
                        tr(
                            self.locale,
                            " • read-only",
                            " • 읽기 전용",
                            " • 只读",
                            " • 読み取り専用",
                        )
                    } else {
                        ""
                    }
                )));
        }
        let evidence = match &inventory.observation {
            OperationObservation::Idle => tr(
                self.locale,
                "No active operation.",
                "진행 중인 작업이 없습니다.",
                "当前没有进行中的操作。",
                "実行中の操作はありません。",
            )
            .to_owned(),
            OperationObservation::Running {
                operation_id,
                journal,
            } => format!(
                "{}: {operation_id}. {}: {}. {}",
                tr(
                    self.locale,
                    "Operation running",
                    "작업 진행 중",
                    "操作进行中",
                    "操作実行中"
                ),
                tr(self.locale, "Journal", "기록 경로", "日志路径", "記録先"),
                journal.display(),
                tr(
                    self.locale,
                    "Closing this window does not cancel it.",
                    "창을 닫아도 작업은 취소되지 않습니다.",
                    "关闭此窗口不会取消操作。",
                    "このウィンドウを閉じても操作はキャンセルされません。"
                )
            ),
            OperationObservation::Terminal {
                operation_id,
                journal,
                result,
            } => format!(
                "{} {operation_id}: {result}. {}: {}",
                tr(
                    self.locale,
                    "Recorded result",
                    "기록된 결과",
                    "已记录的结果",
                    "記録された結果"
                ),
                tr(self.locale, "Journal", "기록 경로", "日志路径", "記録先"),
                journal.display()
            ),
            OperationObservation::Unknown {
                operation_id,
                detail,
                acknowledged,
                ..
            } => format!(
                "{} {operation_id}: {detail}. {}",
                tr(
                    self.locale,
                    "Outcome unknown",
                    "결과 불명",
                    "结果未知",
                    "結果不明"
                ),
                if *acknowledged {
                    tr(
                        self.locale,
                        "Reviewed.",
                        "검토됨.",
                        "已审阅。",
                        "確認済み。",
                    )
                } else {
                    tr(
                        self.locale,
                        "Review evidence before another mutation.",
                        "다른 변경 전에 기록을 검토하세요.",
                        "再次修改前请先审阅操作记录。",
                        "次の変更を行う前に操作記録を確認してください。",
                    )
                }
            ),
        };
        let support = inventory
            .support
            .as_ref()
            .err()
            .map(|e| {
                format!(
                    " {}: {e}",
                    tr(
                        self.locale,
                        "Read-only; host mutation unavailable",
                        "읽기 전용: 호스트 변경 불가",
                        "只读；无法修改宿主",
                        "読み取り専用：ホストの変更は利用できません"
                    )
                )
            })
            .unwrap_or_default();
        self.inventory = Some(inventory);
        set_status(
            self,
            &format!(
                "{}: {}. {evidence}{support}{}",
                tr(
                    self.locale,
                    "Installed plugins",
                    "설치된 플러그인",
                    "已安装的插件",
                    "インストール済みプラグイン"
                ),
                self.inventory.as_ref().unwrap().plugins.len(),
                self.feedback
                    .as_ref()
                    .map(|f| format!(" {f}"))
                    .unwrap_or_default()
            ),
        );
        self.show_selection();
    }
}
fn confirmation(
    action: Action,
    p: &Plugin,
    target: &Target,
    locale: UiLocale,
    mtm: MainThreadMarker,
) -> bool {
    let detail = match action {
        Action::Enable => tr(locale, "Enable in this host context; files and data remain.", "이 호스트에서 활성화합니다. 파일과 데이터는 유지됩니다.", "在此宿主环境中启用；文件和数据会保留。", "このホスト環境で有効にします。ファイルとデータは保持されます。"),
        Action::Disable => tr(locale, "Disable in this host context; files and data remain.", "이 호스트에서 비활성화합니다. 파일과 데이터는 유지됩니다.", "在此宿主环境中禁用；文件和数据会保留。", "このホスト環境で無効にします。ファイルとデータは保持されます。"),
        Action::Unlink => tr(locale, "Remove host registration only. Local files, configuration and state remain.", "호스트 등록만 해제합니다. 로컬 파일과 설정 및 상태는 유지됩니다.", "仅移除宿主注册。本地文件、配置和状态会保留。", "ホストへの登録のみを削除します。ローカルファイル、設定、状態は保持されます。"),
        Action::Uninstall => tr(locale, "Delete the entire managed checkout, not the pet. Plugin configuration and state remain.", "펫이 아닌 관리형 체크아웃 전체를 삭제합니다. 플러그인 설정과 상태는 유지됩니다.", "删除整个托管检出目录，而非宠物。插件配置和状态会保留。", "ペットではなく、管理対象チェックアウト全体を削除します。プラグインの設定と状態は保持されます。"),
        Action::Update => tr(locale, "Update from the recorded GitHub source. Preserve configuration, state and disabled status. Review partial results before retrying.", "기록된 GitHub 소스에서 업데이트합니다. 설정, 상태 및 비활성 상태는 유지됩니다. 부분 실패 시 재시도 전에 결과를 검토하세요.", "从记录的 GitHub 来源更新。保留配置、状态以及禁用状态。重试前请审阅部分完成的结果。", "記録された GitHub ソースから更新します。設定、状態、無効化状態は保持されます。再試行する前に部分的な結果を確認してください。"),
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&format!(
        "{}: {} ({})",
        tr(locale, "Confirm", "확인", "确认", "確認"),
        action_name(action, locale),
        p.id
    )));
    alert.setInformativeText(&NSString::from_str(&format!(
        "{} ({})\nID: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n\n{detail}",
        p.name,
        p.version,
        p.id,
        tr(locale, "Socket", "소켓", "套接字", "ソケット"),
        target.socket.display(),
        tr(
            locale,
            "Authenticated host",
            "인증된 호스트",
            "已验证的宿主",
            "認証済みホスト"
        ),
        target.host.display(),
        tr(locale, "Root", "설치 경로", "安装路径", "インストール先"),
        p.root.display(),
        tr(locale, "Source", "소스", "来源", "ソース"),
        p.source,
        tr(
            locale,
            "Managed checkout",
            "관리형 체크아웃",
            "托管检出目录",
            "管理対象チェックアウト"
        ),
        managed_checkout(p).unwrap_or(tr(
            locale,
            "Not applicable",
            "해당 없음",
            "不适用",
            "該当なし"
        ))
    )));
    alert.addButtonWithTitle(&NSString::from_str(tr(
        locale,
        "Cancel",
        "취소",
        "取消",
        "キャンセル",
    )));
    alert.addButtonWithTitle(&NSString::from_str(action_name(action, locale)));
    alert.runModal() == NSAlertSecondButtonReturn
}
fn act(action: Action, mtm: MainThreadMarker) {
    let prepared = WINDOW.with(|slot| {
        let slot = slot.borrow();
        let s = slot.as_ref()?;
        if s.busy {
            return None;
        }
        let i = s.inventory.as_ref()?;
        let (n, p) = s.selected()?;
        if i.support.is_err()
            || i.protection[n].is_some()
            || p.protected()
            || !eligible(action, p)
            || matches!(
                i.observation,
                OperationObservation::Running { .. }
                    | OperationObservation::Unknown {
                        acknowledged: false,
                        ..
                    }
            )
        {
            return None;
        }
        Some((
            s.generation,
            i.target.as_ref()?.clone(),
            s.paths.clone(),
            s.profile.clone(),
            s.locale,
            p.clone(),
        ))
    });
    let Some((generation, target, paths, profile, locale, plugin)) = prepared else {
        return;
    };
    if !confirmation(action, &plugin, &target, locale, mtm) {
        return;
    }
    WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot
            .as_mut()
            .filter(|s| s.generation == generation && !s.busy)
        else {
            return;
        };
        if !s.inventory.as_ref().is_some_and(|i| {
            i.target.is_some()
                && i.support.is_ok()
                && s.selected().is_some_and(|(n, p)| {
                    i.protection[n].is_none()
                        && !p.protected()
                        && eligible(action, p)
                        && p.raw == plugin.raw
                        && p.registration == plugin.registration
                        && p.id == plugin.id
                        && p.root == plugin.root
                })
        }) {
            return;
        }
        s.busy = true;
        s.generation = request_id();
        let generation = s.generation;
        set_status(
            s,
            tr(locale, "Rechecking installed metadata and submitting…", "설치 정보를 다시 확인하고 작업 제출 중…", "正在重新检查已安装插件的元数据并提交操作…", "インストール済みメタデータを再確認して送信中…"),
        );
        s.controls();
        std::thread::spawn(move || {
            let result = (|| {
                let store = ProfileOperationStore::new(&paths, &profile)?;
                // Reuse the authenticated inventory target. Backend requests reject
                // socket, peer PID/executable, and host-image replacements.
                target.mutation_support()?;
                let plugins = target.list()?;
                let current = plugins
                    .iter()
                    .find(|p| {
                        p.id == plugin.id
                            && p.root == plugin.root
                            && p.raw == plugin.raw
                            && p.registration == plugin.registration
                    })
                    .ok_or_else(|| {
                        tr(locale, "Plugin metadata changed; refresh and confirm again.", "플러그인 정보가 변경되었습니다. 새로고침 후 다시 확인하세요.", "插件元数据已更改；请刷新后重新确认。", "プラグインのメタデータが変更されました。更新して再度確認してください。")
                        .to_owned()
                    })?;
                if current.protected()
                    || target.protection_reason(current, &plugins).is_some()
                    || !eligible(action, current)
                {
                    return Err(tr(locale, "The selected action is no longer permitted.", "선택한 작업이 더 이상 허용되지 않습니다.", "所选操作已不再允许。", "選択した操作は許可されなくなりました。")
                    .into());
                }
                let handle = store.submit(&target, action, current)?;
                let journal = handle.journal.clone();
                handle
                    .wait(Duration::from_secs(300))
                    .map(|observation| (observation, journal))
            })();
            // Store owns the result even if Pet quits or this window is hidden.
            DispatchQueue::main().exec_async(move || {
                WINDOW.with(|slot| {
                    if let Some(s) = slot
                        .borrow_mut()
                        .as_mut()
                        .filter(|s| s.generation == generation)
                    {
                        s.busy = false;
                        s.feedback = Some(match result {
                            Ok((observation, journal)) => format!(
                                "{}: {}. {}: {}",
                                tr(locale, "Operation evidence", "작업 기록", "操作记录", "操作記録"),
                                observation_detail(&observation, locale),
                                tr(locale, "Journal", "기록 경로", "日志路径", "記録先"),
                                journal.display()
                            ),
                            Err(error) => format!(
                                "{}: {error}",
                                tr(locale, "Submission or observation error; review before retrying", "작업 제출 또는 확인 오류: 재시도 전 검토하세요", "提交或结果检查出错；重试前请先审阅记录", "送信または結果確認のエラー：再試行する前に確認してください")
                            ),
                        });
                        s.refresh();
                    }
                })
            });
        });
    });
}
fn observation_detail(value: &OperationObservation, locale: UiLocale) -> String {
    match value {
        OperationObservation::Idle => tr(locale, "Idle", "대기 중", "空闲", "待機中").into(),
        OperationObservation::Running { operation_id, .. } => {
            format!(
                "{} {operation_id}",
                tr(locale, "Running", "진행 중", "进行中", "実行中")
            )
        }
        OperationObservation::Terminal {
            operation_id,
            result,
            ..
        } => format!("{operation_id}: {result}"),
        OperationObservation::Unknown {
            operation_id,
            detail,
            ..
        } => format!("{operation_id}: {detail}"),
    }
}
fn review_unknown(mtm: MainThreadMarker) {
    let data = WINDOW.with(|slot| {
        let slot = slot.borrow();
        let s = slot.as_ref()?;
        let OperationObservation::Unknown {
            operation_id,
            journal,
            detail,
            acknowledged: false,
        } = &s.inventory.as_ref()?.observation
        else {
            return None;
        };
        Some((
            s.generation,
            s.paths.clone(),
            s.profile.clone(),
            s.locale,
            operation_id.clone(),
            journal.clone(),
            detail.clone(),
        ))
    });
    let Some((generation, paths, profile, locale, id, journal, detail)) = data else {
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr(
        locale,
        "Review unknown operation evidence",
        "결과 불명 작업 기록 검토",
        "审阅结果未知的操作记录",
        "結果不明の操作記録を確認",
    )));
    alert.setInformativeText(&NSString::from_str(&format!("{}: {id}\n{}: {}\n{}: {detail}\n\n{}",tr(locale, "Operation ID", "작업 ID", "操作 ID", "操作 ID"),tr(locale, "Journal", "기록 경로", "日志路径", "記録先"),journal.display(),tr(locale, "Evidence", "기록", "操作记录", "操作記録"),tr(locale, "The result cannot be proven. Inspect the installed plugin and journal before acknowledging. Acknowledgment does not retry or reverse anything; a new confirmation is required for any subsequent change.", "결과를 증명할 수 없습니다. 확인 전에 설치된 플러그인과 기록을 검토하세요. 확인은 재시도나 원상 복구가 아니며, 다음 변경에는 새 확인이 필요합니다.", "无法确定操作结果。确认前请检查已安装的插件和日志。确认不会重试或撤销任何操作；后续的任何更改都需要重新确认。", "結果を確認できません。了承する前に、インストール済みプラグインと記録を確認してください。了承しても再試行や取り消しは行われません。その後の変更には新たな確認が必要です。"))));
    alert.addButtonWithTitle(&NSString::from_str(tr(
        locale,
        "Keep blocked",
        "차단 유지",
        "继续阻止操作",
        "ブロックを維持",
    )));
    alert.addButtonWithTitle(&NSString::from_str(tr(
        locale,
        "I reviewed this operation",
        "이 작업을 검토했습니다",
        "我已审阅此操作",
        "この操作を確認しました",
    )));
    if alert.runModal() != NSAlertSecondButtonReturn {
        return;
    }
    WINDOW.with(|slot| {
        let mut slot=slot.borrow_mut();let Some(s)=slot.as_mut().filter(|s|s.generation==generation&&!s.busy)else{return;};
        if !s.inventory.as_ref().is_some_and(|i|matches!(&i.observation,OperationObservation::Unknown{operation_id,acknowledged:false,..} if operation_id==&id)){return;}
        s.busy=true;s.generation=request_id();let generation=s.generation;s.controls();
        std::thread::spawn(move || {
            let result=ProfileOperationStore::new(&paths, &profile).and_then(|store|store.acknowledge_unknown(&id));
            DispatchQueue::main().exec_async(move || WINDOW.with(|slot| {
                if let Some(s)=slot.borrow_mut().as_mut().filter(|s|s.generation==generation){
                    s.busy=false;
                    if let Err(error)=result{s.feedback=Some(format!("{}: {error}",tr(locale, "Could not acknowledge evidence", "기록 확인 실패", "无法确认操作记录", "操作記録を了承できませんでした")));}
                    s.refresh();
                }
            }));
        });
    });
}
fn close_controller(controller: u64) {
    if WINDOW.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.controller == controller)
    }) {
        close();
    }
}
pub(crate) fn close() {
    WINDOW.with(|slot| {
        if let Some(s) = slot.borrow_mut().take() {
            s.window.setDelegate(None);
            s.window.orderOut(None);
        }
    });
}
pub(crate) fn open(
    paths: ProfilePaths,
    profile: ProfileStore,
    socket: PathBuf,
    locale: UiLocale,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    let same_presentation = WINDOW.with(|slot| {
        slot.borrow().as_ref().is_some_and(|s| {
            s.window.isVisible()
                && s.socket == socket
                && s.paths.config_dir == paths.config_dir
                && s.paths.state_dir == paths.state_dir
                && s.locale == locale
        })
    });
    if same_presentation {
        WINDOW.with(|slot| {
            if let Some(s) = slot.borrow().as_ref() {
                s.window.makeKeyAndOrderFront(None);
            }
        });
        return Ok(());
    }
    close();
    let controller = request_id();
    let target: Retained<Controller> =
        unsafe { msg_send![super(Controller::alloc(mtm).set_ivars(controller)), init] };
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, 760.0, 610.0),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setLevel(NSFloatingWindowLevel);
    if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }) {
        let _: () = unsafe { msg_send![&*window, setAppearance: Some(&*appearance)] };
    }
    window.setTitle(&NSString::from_str(text(
        locale,
        Message::PluginManagerTitle,
    )));
    window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    let content = window.contentView().ok_or(tr(
        locale,
        "manager window has no content view",
        "관리자 창에 콘텐츠 뷰가 없습니다",
        "管理器窗口没有内容视图",
        "マネージャーウィンドウにコンテンツビューがありません",
    ))?;
    let heading = label(
        &format!(
            "{}: {}",
            tr(
                locale,
                "Herdr socket",
                "Herdr 소켓",
                "Herdr 套接字",
                "Herdr ソケット"
            ),
            socket.display()
        ),
        rect(24.0, 560.0, 710.0, 30.0),
        mtm,
    );
    content.addSubview(&heading);
    let selection: Retained<NSPopUpButton> = unsafe {
        msg_send![NSPopUpButton::alloc(mtm),initWithFrame:rect(24.0,510.0,590.0,30.0),pullsDown:false]
    };
    unsafe {
        selection.setTarget(Some(&*target));
        selection.setAction(Some(sel!(selectManagedPlugin:)));
    }
    content.addSubview(&selection);
    let refresh = button(
        tr(locale, "Refresh", "새로고침", "刷新", "再読み込み"),
        sel!(refreshPlugins:),
        &target,
        rect(625.0, 510.0, 110.0, 30.0),
        mtm,
    );
    content.addSubview(&refresh);
    let details = label("", rect(24.0, 225.0, 710.0, 265.0), mtm);
    details.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)));
    content.addSubview(&details);
    let actions = [
        button(
            action_name(Action::Enable, locale),
            sel!(enableManagedPlugin:),
            &target,
            rect(24.0, 170.0, 130.0, 32.0),
            mtm,
        ),
        button(
            action_name(Action::Disable, locale),
            sel!(disableManagedPlugin:),
            &target,
            rect(165.0, 170.0, 130.0, 32.0),
            mtm,
        ),
        button(
            action_name(Action::Unlink, locale),
            sel!(unlinkManagedPlugin:),
            &target,
            rect(306.0, 170.0, 130.0, 32.0),
            mtm,
        ),
        button(
            action_name(Action::Uninstall, locale),
            sel!(uninstallManagedPlugin:),
            &target,
            rect(447.0, 170.0, 140.0, 32.0),
            mtm,
        ),
        button(
            action_name(Action::Update, locale),
            sel!(updateManagedPlugin:),
            &target,
            rect(598.0, 170.0, 137.0, 32.0),
            mtm,
        ),
    ];
    for item in &actions {
        content.addSubview(item);
    }
    let review = button(
        tr(
            locale,
            "Review unknown outcome",
            "결과 불명 기록 검토",
            "审阅未知结果",
            "不明な結果を確認",
        ),
        sel!(reviewPluginEvidence:),
        &target,
        rect(24.0, 117.0, 260.0, 32.0),
        mtm,
    );
    content.addSubview(&review);
    let notice=label(tr(locale, "Removing the manager feature never deletes plugins. Deleting a managed plugin requires separate confirmation.", "관리자 기능을 제거해도 플러그인은 삭제되지 않습니다. 관리형 플러그인 삭제에는 별도 확인이 필요합니다.", "移除管理器功能不会删除插件。删除托管插件需要单独确认。", "マネージャー機能を削除してもプラグインは削除されません。管理対象プラグインの削除には別途確認が必要です。"),rect(24.0,80.0,710.0,32.0),mtm);
    content.addSubview(&notice);
    let status_field = label("", rect(24.0, 10.0, 710.0, 68.0), mtm);
    content.addSubview(&status_field);
    WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        *slot = Some(WindowState {
            window,
            _target: target,
            controller,
            paths,
            profile,
            socket,
            locale,
            generation: request_id(),
            busy: false,
            feedback: None,
            inventory: None,
            selection,
            details,
            status: status_field,
            refresh,
            review,
            actions,
        });
        let s = slot.as_mut().expect("window state initialized");
        s.window.center();
        s.window.makeKeyAndOrderFront(None);
        s.refresh();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plugin(source: serde_json::Value, enabled: bool) -> Plugin {
        Plugin {
            id: "example".into(),
            name: "Example".into(),
            version: "1".into(),
            enabled,
            root: PathBuf::from("/tmp/example"),
            source,
            warnings: Vec::new(),
            raw: serde_json::Value::Null,
            registration: None,
        }
    }
    #[test]
    fn toggles_apply_to_all_sources_and_removal_requires_appropriate_source() {
        let local = plugin(serde_json::json!({"kind":"local"}), false);
        assert!(eligible(Action::Enable, &local));
        assert!(!eligible(Action::Disable, &local));
        assert!(eligible(Action::Unlink, &local));
        assert!(!eligible(Action::Update, &local));
        assert!(!eligible(Action::Uninstall, &local));
        let enabled_local = plugin(serde_json::json!({"kind":"local"}), true);
        assert!(!eligible(Action::Enable, &enabled_local));
        assert!(eligible(Action::Disable, &enabled_local));
        assert!(eligible(Action::Unlink, &enabled_local));
        assert!(!eligible(Action::Uninstall, &enabled_local));
        let disabled_managed = plugin(
            serde_json::json!({"kind":"github","managed_path":"/tmp/checkout"}),
            false,
        );
        assert!(eligible(Action::Enable, &disabled_managed));
        assert!(!eligible(Action::Disable, &disabled_managed));
        assert!(!eligible(Action::Unlink, &disabled_managed));
        assert!(eligible(Action::Update, &disabled_managed));
        assert!(eligible(Action::Uninstall, &disabled_managed));
        let enabled_managed = plugin(
            serde_json::json!({"kind":"github","managed_path":"/tmp/checkout"}),
            true,
        );
        assert!(!eligible(Action::Enable, &enabled_managed));
        assert!(eligible(Action::Disable, &enabled_managed));
        assert!(!eligible(Action::Unlink, &enabled_managed));
        assert!(eligible(Action::Update, &enabled_managed));
        assert!(eligible(Action::Uninstall, &enabled_managed));
        let unmanaged = plugin(
            serde_json::json!({"kind":"github","managed_path":"relative"}),
            true,
        );
        assert!(!eligible(Action::Enable, &unmanaged));
        assert!(eligible(Action::Disable, &unmanaged));
        assert!(!eligible(Action::Unlink, &unmanaged));
        assert!(!eligible(Action::Update, &unmanaged));
        assert!(!eligible(Action::Uninstall, &unmanaged));
    }
}
