//! Real AppKit composer geometry regression, executed by Cargo without libtest's worker threads.
#[path = "../src/composer_layout.rs"]
mod composer_layout;

use composer_layout::{
    configure_composer, layout_composer, reveal_composer_selection, ComposerMetrics,
};
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType,
    NSColor, NSFont, NSScreen, NSScrollView, NSScrollerStyle, NSSelectionAffinity, NSTextView,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRange, NSRect, NSSize, NSString};
use std::path::Path;

const SHORT: &str = "초안 테스트: 가운데 선택 유지 ABC 123 🐾";
const UNIT: &str = "초안 Korean ABC 123 🐾 ";

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn overlap(a: NSRect, b: NSRect) -> NSRect {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    rect(
        x,
        y,
        (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - x,
        (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - y,
    )
}

fn inside(outer: NSRect, inner: NSRect, epsilon: f64) -> bool {
    inner.origin.x >= outer.origin.x - epsilon
        && inner.origin.y >= outer.origin.y - epsilon
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width + epsilon
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height + epsilon
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

struct Fixture {
    window: objc2::rc::Retained<NSWindow>,
    root: objc2::rc::Retained<NSView>,
    second: objc2::rc::Retained<NSView>,
    reply: objc2::rc::Retained<NSView>,
    scroll: objc2::rc::Retained<NSScrollView>,
    view: objc2::rc::Retained<NSTextView>,
    metrics: ComposerMetrics,
}

impl Fixture {
    fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: mtm guarantees the main thread; the frame and AppKit enum arguments are valid.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(120.0, 120.0, 520.0, 190.0),
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let root = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 520.0, 190.0));
        let first = NSView::initWithFrame(NSView::alloc(mtm), rect(8.0, 8.0, 500.0, 85.0));
        let second = NSView::initWithFrame(NSView::alloc(mtm), rect(8.0, 97.0, 500.0, 85.0));
        let reply = NSView::initWithFrame(NSView::alloc(mtm), rect(5.0, 3.0, 470.0, 50.0));
        let view = NSTextView::initWithFrame(NSTextView::alloc(mtm), rect(0.0, 0.0, 220.0, 22.0));
        view.setEditable(true);
        view.setSelectable(true);
        view.setRichText(false);
        view.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        view.setTextColor(Some(&NSColor::blackColor()));
        view.setDrawsBackground(false);
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(2.0, 23.0, 220.0, 22.0));
        scroll.setDrawsBackground(true);
        scroll.setBackgroundColor(&NSColor::whiteColor());
        root.addSubview(&first);
        root.addSubview(&second);
        first.addSubview(&reply);
        reply.addSubview(&scroll);
        window.setContentView(Some(&root));
        let metrics = configure_composer(&view, &scroll);
        let fixture = Self {
            window,
            root,
            second,
            reply,
            scroll,
            view,
            metrics,
        };
        fixture.resize(220.0);
        fixture.window.orderFront(None);
        fixture.settle();
        fixture
    }

    fn settle(&self) {
        let text = self.view.string().to_string();
        let ranges = self.view.selectedRanges();
        let affinity = self.view.selectionAffinity();
        self.scroll.tile();
        self.root.layoutSubtreeIfNeeded();
        layout_composer(&self.view, &self.scroll, self.metrics);
        self.root.layoutSubtreeIfNeeded();
        assert_eq!(
            self.view.string().to_string(),
            text,
            "geometry changed composer text"
        );
        assert_eq!(
            self.view.selectedRanges(),
            ranges,
            "geometry changed UTF-16 selection ranges"
        );
        assert_eq!(
            self.view.selectionAffinity(),
            affinity,
            "geometry changed selection affinity"
        );
    }

    fn resize(&self, width: f64) {
        assert!(width > 0.0);
        self.reply
            .setFrame(rect(5.0, 3.0, width + 70.0, self.metrics.reply_height));
        self.scroll
            .setFrame(rect(2.0, 23.0, width, self.metrics.input_height));
        self.settle();
    }

    fn style(&self, requested: NSScrollerStyle) {
        // Setting style before mounting may be overridden by AppKit during attachment.
        self.scroll.setScrollerStyle(requested);
        self.settle();
        assert_eq!(
            self.scroll.scrollerStyle(),
            requested,
            "AppKit did not honor requested scroller style; cannot pass this cell"
        );
    }

    fn text(&self, value: &str) {
        self.view.setString(&NSString::from_str(value));
        self.settle();
        assert_eq!(self.view.string().to_string(), value);
    }

    fn viewport(&self) -> NSRect {
        let mut visible = self.view.bounds();
        let mut ancestor = unsafe { self.view.superview() };
        let mut count = 0;
        while let Some(parent) = ancestor {
            count += 1;
            assert!(count <= 8, "unexpected view hierarchy cycle");
            assert!(
                !parent.isHiddenOrHasHiddenAncestor(),
                "composer hidden during geometry assertion"
            );
            visible = overlap(
                visible,
                self.view
                    .convertRect_fromView(parent.bounds(), Some(&parent)),
            );
            ancestor = unsafe { parent.superview() };
        }
        assert!(
            count >= 4,
            "composer not attached to actual window hierarchy"
        );
        assert!(self.window.isVisible(), "composer window not visible");
        // Overlay scrollers can occupy pixels inside the clip: do not count those as text viewport.
        if let Some(bar) = self.scroll.horizontalScroller() {
            if !bar.isHiddenOrHasHiddenAncestor() {
                let bar_rect = self.view.convertRect_fromView(bar.bounds(), Some(&bar));
                let intersection = overlap(visible, bar_rect);
                if intersection.size.width > 0.0 && intersection.size.height > 0.0 {
                    if bar_rect.origin.y < visible.origin.y + visible.size.height / 2.0 {
                        let bottom =
                            (bar_rect.origin.y + bar_rect.size.height).max(visible.origin.y);
                        visible.size.height =
                            (visible.origin.y + visible.size.height - bottom).max(0.0);
                        visible.origin.y = bottom;
                    } else {
                        visible.size.height = (bar_rect.origin.y - visible.origin.y).max(0.0);
                    }
                }
            }
        }
        visible
    }

    fn assert_scroll_within_document(&self, label: &str) {
        let clip = self.scroll.contentView().bounds();
        let document = self.view.frame();
        let epsilon = 1.0 / self.window.backingScaleFactor().max(1.0);
        let maximum =
            (document.origin.x + document.size.width - clip.size.width).max(document.origin.x);
        assert!(
            clip.origin.x >= document.origin.x - epsilon && clip.origin.x <= maximum + epsilon,
            "{label}: viewport stranded beyond document: clip={clip:?} document={document:?}"
        );
    }

    fn assert_geometry(&self, label: &str, expect_fit: bool) {
        let visible = self.viewport();
        let clip = self.scroll.contentView();
        let container = unsafe { self.view.textContainer() }.expect("text container");
        let manager = unsafe { self.view.layoutManager() }.expect("layout manager");
        manager.ensureLayoutForTextContainer(&container);
        let range = manager.glyphRangeForTextContainer(&container);
        let used = manager.usedRectForTextContainer(&container);
        let origin = self.view.textContainerOrigin();
        let epsilon = 1.0 / self.window.backingScaleFactor().max(1.0);
        let text_end = origin.x + used.origin.x + used.size.width;
        if expect_fit {
            assert!(
                text_end <= visible.size.width + epsilon,
                "{label}: expected text to fit but used extent exceeds viewport: used={used:?} visible={visible:?}"
            );
        }
        // Horizontal overflow is allowed; a short text must be wholly visible
        // even when the preceding long draft left the viewport at its end.
        let fits_horizontally = text_end <= visible.size.width + epsilon;
        let line = if range.length == 0 {
            manager.extraLineFragmentRect()
        } else {
            unsafe {
                manager.lineFragmentRectForGlyphAtIndex_effectiveRange(
                    range.location,
                    std::ptr::null_mut(),
                )
            }
        };
        let line = rect(
            origin.x + line.origin.x,
            origin.y + line.origin.y,
            line.size.width,
            line.size.height,
        );
        assert!(
            visible.size.width > 0.0 && visible.size.height > 0.0,
            "{label}: zero usable viewport: {visible:?}, clip={:?}, doc={:?}",
            clip.bounds(),
            self.view.frame()
        );
        assert!(line.size.height > 0.0 && line.origin.y >= visible.origin.y - epsilon && line.origin.y + line.size.height <= visible.origin.y + visible.size.height + epsilon,
            "{label}: complete line/caret clipped: line={line:?} visible={visible:?} used={used:?} clip={:?} doc={:?} metrics={:?}", clip.bounds(), self.view.frame(), self.metrics);
        if range.length == 0 {
            assert!(line.origin.x >= visible.origin.x - epsilon && line.origin.x <= visible.origin.x + visible.size.width + epsilon,
                "{label}: empty insertion caret is outside the viewport: line={line:?} visible={visible:?}");
        }
        let mut ink_count = 0;
        for glyph in range.location..range.location + range.length {
            let glyph_rect = manager
                .boundingRectForGlyphRange_inTextContainer(NSRange::new(glyph, 1), &container);
            if glyph_rect.size.height <= 0.0 || glyph_rect.size.width <= 0.0 {
                continue;
            }
            let ink = rect(
                origin.x + glyph_rect.origin.x,
                origin.y + glyph_rect.origin.y,
                glyph_rect.size.width,
                glyph_rect.size.height,
            );
            if !fits_horizontally
                && (ink.origin.x >= visible.origin.x + visible.size.width
                    || ink.origin.x + ink.size.width <= visible.origin.x)
            {
                continue;
            }
            assert!(ink.origin.y >= visible.origin.y - epsilon && ink.origin.y + ink.size.height <= visible.origin.y + visible.size.height + epsilon,
                "{label}: glyph {glyph} vertically clipped: ink={ink:?}, usable={visible:?}, clip={:?}, doc={:?}", clip.bounds(), self.view.frame());
            if fits_horizontally {
                assert!(
                    inside(visible, ink, epsilon),
                    "{label}: short glyph {glyph} clipped: ink={ink:?}, usable={visible:?}"
                );
            }
            ink_count += 1;
        }
        if range.length != 0 {
            assert!(ink_count > 0, "{label}: no glyphs intersect viewport");
        }
        // A one-line input must not silently wrap even in the narrow fixture.
        if range.length > 0 {
            let last_line = unsafe {
                manager.lineFragmentRectForGlyphAtIndex_effectiveRange(
                    range.location + range.length - 1,
                    std::ptr::null_mut(),
                )
            };
            assert!(
                (last_line.origin.y - (line.origin.y - origin.y)).abs() <= epsilon,
                "{label}: unexpected text wrapping"
            );
        }
        println!("composer label={label:?} style={} outer_h={:.2} clip_h={:.2} document_h={:.2} usable_h={:.2} line_h={:.2} glyphs={} used_w={:.2} origin_x={:.2}",
            self.scroll.scrollerStyle().0, self.scroll.frame().size.height, clip.bounds().size.height,
            self.view.frame().size.height, visible.size.height, line.size.height, range.length,
            used.size.width, clip.bounds().origin.x);
    }

    fn assert_selection_visible(&self, label: &str) {
        let container = unsafe { self.view.textContainer() }.expect("text container");
        let manager = unsafe { self.view.layoutManager() }.expect("layout manager");
        manager.ensureLayoutForTextContainer(&container);
        let glyphs = unsafe {
            manager.glyphRangeForCharacterRange_actualCharacterRange(
                self.view.selectedRange(),
                std::ptr::null_mut(),
            )
        };
        let selected = manager.boundingRectForGlyphRange_inTextContainer(glyphs, &container);
        let origin = self.view.textContainerOrigin();
        let selected = rect(
            origin.x + selected.origin.x,
            origin.y + selected.origin.y,
            selected.size.width,
            selected.size.height,
        );
        let visible = self.viewport();
        let epsilon = 1.0 / self.window.backingScaleFactor().max(1.0);
        assert!(
            selected.size.width > 0.0 && inside(visible, selected, epsilon),
            "{label}: selected text clipped: selection={selected:?} visible={visible:?}"
        );
        assert!(
            self.scroll.contentView().bounds().size.height >= self.metrics.content_height,
            "{label}: editable line does not fit clip"
        );
        self.assert_geometry(label, false);
    }
    fn assert_reachable(&self, label: &str) {
        let length = self.view.string().length();
        let container = unsafe { self.view.textContainer() }.expect("text container");
        let manager = unsafe { self.view.layoutManager() }.expect("layout manager");
        let glyphs = manager.glyphRangeForTextContainer(&container);
        let used = manager.usedRectForTextContainer(&container);
        let epsilon = 1.0 / self.window.backingScaleFactor().max(1.0);
        for index in [0, length / 2, length] {
            self.view.setSelectedRange(NSRange::new(index, 0));
            self.view.scrollRangeToVisible(NSRange::new(index, 0));
            self.settle();
            self.assert_geometry(label, false);
            let visible = self.viewport();
            let glyph = unsafe {
                manager.glyphRangeForCharacterRange_actualCharacterRange(
                    NSRange::new(index.min(length.saturating_sub(1)), 1),
                    std::ptr::null_mut(),
                )
            };
            let glyph_index = glyph
                .location
                .min(glyphs.location + glyphs.length.saturating_sub(1));
            let ink = manager.boundingRectForGlyphRange_inTextContainer(
                NSRange::new(glyph_index, 1),
                &container,
            );
            let x = self.view.textContainerOrigin().x + ink.origin.x;
            assert!(
                x + ink.size.width >= visible.origin.x - epsilon
                    && x <= visible.origin.x + visible.size.width + epsilon,
                "{label}: cannot scroll to UTF-16 index {index}, glyph={ink:?} visible={visible:?}"
            );
            assert_eq!(
                self.view.selectedRange(),
                NSRange::new(index, 0),
                "selection moved during scrolling"
            );
        }
        self.view.setSelectedRange(NSRange::new(length, 0));
        self.view.scrollRangeToVisible(NSRange::new(length, 0));
        self.settle();
        let visible = self.viewport();
        assert!(
            visible.origin.x + visible.size.width
                >= self.view.textContainerOrigin().x + used.origin.x + used.size.width - epsilon,
            "{label}: end of draft unreachable: used={used:?} visible={visible:?}"
        );
    }

    fn render(&self, path: &Path) {
        let image = self
            .reply
            .bitmapImageRepForCachingDisplayInRect(self.reply.bounds())
            .expect("AppKit bitmap caching unavailable");
        self.reply
            .cacheDisplayInRect_toBitmapImageRep(self.reply.bounds(), &image);
        let props = NSDictionary::new();
        let data =
            unsafe { image.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }
                .expect("AppKit PNG encoding unavailable");
        assert!(
            data.writeToFile_atomically(&NSString::from_str(&path.to_string_lossy()), true),
            "cannot write PNG to {path:?}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.window.close();
    }
}

fn state_transitions(f: &Fixture, style: &str) {
    let original = SHORT;
    f.text(original);
    let prefix = "초안 테스트: ";
    let target = "가운데";
    let selection = NSRange::new(utf16_len(prefix), utf16_len(target));
    f.view.setSelectedRange_affinity_stillSelecting(
        selection,
        NSSelectionAffinity::Upstream,
        false,
    );
    let affinity = f.view.selectionAffinity();
    let before = f.view.selectedRanges();
    f.reply.setHidden(true);
    f.reply.removeFromSuperview();
    f.second.addSubview(&f.reply);
    f.reply.setHidden(false);
    f.settle();
    assert_eq!(
        f.view.string().to_string(),
        original,
        "{style}: reparent lost draft"
    );
    assert_eq!(
        f.view.selectedRanges(),
        before,
        "{style}: reparent lost selected UTF-16 range(s)"
    );
    assert_eq!(
        f.view.selectionAffinity(),
        affinity,
        "{style}: reparent lost affinity"
    );
    f.assert_scroll_within_document("reparented selected input");
    let container = unsafe { f.view.textContainer() }.expect("text container");
    let manager = unsafe { f.view.layoutManager() }.expect("layout manager");
    manager.ensureLayoutForTextContainer(&container);
    let selected_glyphs = unsafe {
        manager.glyphRangeForCharacterRange_actualCharacterRange(selection, std::ptr::null_mut())
    };
    let selected = manager.boundingRectForGlyphRange_inTextContainer(selected_glyphs, &container);
    let origin = f.view.textContainerOrigin();
    let selected = rect(
        origin.x + selected.origin.x,
        origin.y + selected.origin.y,
        selected.size.width,
        selected.size.height,
    );
    let visible = f.viewport();
    let epsilon = 1.0 / f.window.backingScaleFactor().max(1.0);
    assert!(
        selected.size.width > 0.0 && inside(visible, selected, epsilon),
        "{style}: reparented selection is not visible: selected={selected:?} visible={visible:?}"
    );
    // Native reparenting can scroll the selection into view. Once attached,
    // our own layout pass must leave its valid origin unchanged.
    let settled_scroll = f.scroll.contentView().bounds().origin;
    f.settle();
    assert_eq!(
        f.scroll.contentView().bounds().origin,
        settled_scroll,
        "{style}: layout shifted an unchanged reparented selection"
    );
    f.assert_geometry("reparented selected input", false);
    let replacement = NSString::from_str("중앙🐈");
    let _: () =
        unsafe { msg_send![&*f.view, insertText: &*replacement, replacementRange: selection] };
    let expected = original.replacen(target, "중앙🐈", 1);
    assert_eq!(
        f.view.string().to_string(),
        expected,
        "{style}: native insertion replaced wrong UTF-16 span"
    );
    assert_eq!(
        f.view.selectedRange(),
        NSRange::new(selection.location + utf16_len("중앙🐈"), 0)
    );
    f.settle();
    f.assert_geometry("middle UTF-16 replacement", false);

    let start = NSRange::new(utf16_len(prefix), 0);
    f.view.setSelectedRange(start);
    let marked = NSString::from_str("한글");
    let _: () = unsafe {
        msg_send![&*f.view, setMarkedText: &*marked, selectedRange: NSRange::new(2, 0), replacementRange: start]
    };
    let has_marked: bool = unsafe { msg_send![&*f.view, hasMarkedText] };
    assert!(
        has_marked,
        "native marked-text callback did not establish preedit"
    );
    let preedit = f.view.string().to_string();
    let preedit_selection = f.view.selectedRange();
    let frame = f.view.frame();
    let scroll_frame = f.scroll.frame();
    let container_size = unsafe { f.view.textContainer() }
        .expect("text container")
        .containerSize();
    let clip_bounds = f.scroll.contentView().bounds();
    layout_composer(&f.view, &f.scroll, f.metrics);
    reveal_composer_selection(&f.view, &f.scroll);
    assert_eq!(
        f.view.frame(),
        frame,
        "marked layout mutated document geometry"
    );
    assert_eq!(
        f.scroll.frame(),
        scroll_frame,
        "marked layout mutated scroll geometry"
    );
    assert_eq!(
        unsafe { f.view.textContainer() }
            .expect("text container")
            .containerSize(),
        container_size,
        "marked layout mutated text container geometry"
    );
    assert_eq!(
        f.scroll.contentView().bounds(),
        clip_bounds,
        "marked layout moved viewport"
    );
    assert_eq!(
        f.view.string().to_string(),
        preedit,
        "marked layout changed preedit text"
    );
    assert_eq!(
        f.view.selectedRange(),
        preedit_selection,
        "marked layout changed preedit selection"
    );
    let _: () = unsafe { msg_send![&*f.view, unmarkText] };
    let has_marked: bool = unsafe { msg_send![&*f.view, hasMarkedText] };
    assert!(!has_marked, "marked text not cleared after unmark");
    assert_eq!(
        f.view.string().to_string(),
        preedit,
        "unmark lost committed text"
    );
    f.settle();
    f.assert_geometry("after unmark", false);
}

fn focused_reflow(f: &Fixture, style: &str) {
    let draft = UNIT.repeat(18);
    f.resize(390.0);
    f.style(if style == "legacy" {
        NSScrollerStyle::Legacy
    } else {
        NSScrollerStyle::Overlay
    });
    f.text(&draft);
    let selection = NSRange::new(utf16_len(&draft) - utf16_len(UNIT), utf16_len("초안"));
    f.view.setSelectedRange_affinity_stillSelecting(
        selection,
        NSSelectionAffinity::Upstream,
        false,
    );
    let ranges = f.view.selectedRanges();
    let affinity = f.view.selectionAffinity();
    let clip = f.scroll.contentView();
    clip.scrollToPoint(NSPoint::new(0.0, 0.0));
    f.scroll.reflectScrolledClipView(&clip);
    f.resize(95.0);
    // settle/resize only reconciles geometry; the focused viewport-change
    // path must invoke the production reveal, not a test-only native scroll.
    reveal_composer_selection(&f.view, &f.scroll);
    f.assert_selection_visible(&format!("{style}: focused narrow viewport"));
    assert_eq!(f.view.string().to_string(), draft);
    assert_eq!(f.view.selectedRanges(), ranges);
    assert_eq!(f.view.selectionAffinity(), affinity);
    f.assert_scroll_within_document("focused shrink");

    f.reply.removeFromSuperview();
    f.root.addSubview(&f.reply);
    f.settle();
    reveal_composer_selection(&f.view, &f.scroll);
    f.assert_selection_visible(&format!("{style}: root reparent"));
    f.second.addSubview(&f.reply);
    f.settle();
    reveal_composer_selection(&f.view, &f.scroll);
    f.assert_selection_visible(&format!("{style}: card reparent"));
    assert_eq!(f.view.string().to_string(), draft);
    assert_eq!(f.view.selectedRanges(), ranges);
    assert_eq!(f.view.selectionAffinity(), affinity);
    f.assert_scroll_within_document("focused reparent");
}

fn run(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    assert!(NSScreen::mainScreen(mtm).is_some(), "composer AppKit regression needs a logged-in macOS Window Server with an available display");
    app.finishLaunching();
    let render_dir = std::env::var_os("HERDR_COMPOSER_RENDER_DIR");
    if let Some(dir) = &render_dir {
        assert!(
            Path::new(dir).is_dir(),
            "HERDR_COMPOSER_RENDER_DIR must be an existing parent-owned directory"
        );
    }
    let f = Fixture::new(mtm);
    assert!(
        f.metrics.content_height > 0.0
            && f.metrics.input_height > f.metrics.content_height
            && f.metrics.reply_height >= f.metrics.input_height + 23.0,
        "invalid production measured composer metrics: {:?}",
        f.metrics
    );
    for (name, style) in [
        ("legacy", NSScrollerStyle::Legacy),
        ("overlay", NSScrollerStyle::Overlay),
    ] {
        for width in [95.0, 220.0, 390.0] {
            f.resize(width);
            f.style(style);
            f.text("");
            f.assert_geometry("empty", true);
            f.text("한 A 🐾");
            f.assert_geometry("short Korean/Latin/emoji", true);
            if width == 220.0 {
                if let Some(dir) = &render_dir {
                    f.render(&Path::new(dir).join(format!("composer-{name}-short.png")));
                }
            }
            let long = UNIT.repeat(18);
            f.text(&long);
            f.assert_geometry("overflowing Korean/Latin/emoji", false);
            let clip = f.scroll.contentView();
            if style == NSScrollerStyle::Legacy {
                let bar = f
                    .scroll
                    .horizontalScroller()
                    .expect("Legacy horizontal scroller missing");
                assert!(
                    !bar.isHiddenOrHasHiddenAncestor(),
                    "Legacy overflow scrollbar hidden"
                );
                let bar_in_reply = f.reply.convertRect_fromView(bar.bounds(), Some(&bar));
                let clip_in_reply = f.reply.convertRect_fromView(clip.bounds(), Some(&clip));
                assert!(bar_in_reply.size.height > 0.0 && clip_in_reply.size.height > 0.0
                    && overlap(bar_in_reply, clip_in_reply).size.height <= 0.0,
                    "Legacy scrollbar does not occupy separate area: bar={bar_in_reply:?}, clip={clip_in_reply:?}");
            }
            if style == NSScrollerStyle::Overlay {
                // Force the overlay into its visible phase without changing system preferences.
                f.scroll.setAutohidesScrollers(false);
                f.settle();
                let bar = f
                    .scroll
                    .horizontalScroller()
                    .expect("Overlay horizontal scroller missing");
                assert!(
                    !bar.isHiddenOrHasHiddenAncestor(),
                    "Overlay scrollbar not visible in forced-visible phase"
                );
                f.assert_geometry("visible Overlay bar", false);
                if width == 220.0 {
                    if let Some(dir) = &render_dir {
                        f.render(&Path::new(dir).join("composer-overlay-visible.png"));
                    }
                }
                f.scroll.setAutohidesScrollers(true);
                f.settle();
            }
            f.assert_reachable("overflow scroll beginning/middle/end");
            if width == 220.0 {
                if let Some(dir) = &render_dir {
                    f.render(&Path::new(dir).join(format!("composer-{name}-long.png")));
                }
            }
            // setString replaces the draft without implying that the old
            // end-of-document scroll position is a valid new viewport.
            f.text(SHORT);
            f.assert_scroll_within_document("long to mixed short draft");
            f.assert_geometry("mixed short after long", false);
            // This short draft itself overflows the narrow clip. Navigate with
            // real AppKit selection/scrolling before requiring its caret reachable.
            f.assert_reachable("mixed short draft beginning/middle/end");
            f.text("짧게 A 🐾");
            f.assert_scroll_within_document("mixed short to fitting draft");
            f.assert_geometry("short after long", true);
            f.scroll.tile();
            f.settle();
            f.assert_geometry("after retile", true);
            f.text(&long);
            f.assert_reachable("long draft before clearing");
            f.text("");
            f.assert_scroll_within_document("long to empty draft");
            f.assert_geometry("empty after long", true);
        }
        f.resize(220.0);
        state_transitions(&f, name);
        focused_reflow(&f, name);
    }
    println!("composer AppKit geometry and native insertion passed on main thread");
}

fn main() {
    let mtm = MainThreadMarker::new()
        .expect("composer AppKit regression must start on the process main thread");
    run(mtm);
}
