use objc2::ClassType;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBorderType, NSLineBreakMode, NSScrollView, NSScroller,
    NSScrollerStyle, NSTextView,
};
use objc2_foundation::NSSize;

const BASELINE_LINE_HEIGHT: f64 = 18.0;
pub(crate) const INPUT_ORIGIN_Y: f64 = 23.0;
pub(crate) const BOTTOM_INSET: f64 = 3.0;
const TEXT_INSET: f64 = 3.0;
const CONTAINER_WIDTH: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ComposerMetrics {
    pub content_height: f64,
    pub input_height: f64,
    pub reply_height: f64,
}

pub(crate) fn configure_composer(view: &NSTextView, scroll: &NSScrollView) -> ComposerMetrics {
    view.setHorizontallyResizable(true);
    view.setVerticallyResizable(false);
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
    view.setTextContainerInset(NSSize::new(TEXT_INSET, TEXT_INSET));
    if let Some(container) = unsafe { view.textContainer() } {
        container.setWidthTracksTextView(false);
        container.setLineBreakMode(NSLineBreakMode::ByClipping);
        container.setLineFragmentPadding(4.0);
        container.setHeightTracksTextView(true);
    }

    scroll.setScrollerStyle(NSScrollerStyle::Overlay);
    scroll.setBorderType(NSBorderType::LineBorder);
    scroll.setDocumentView(Some(view));
    scroll.setHasVerticalScroller(false);
    scroll.setHasHorizontalScroller(true);
    scroll.setAutohidesScrollers(true);

    // The font's native line height (not its point size) and the baseline row
    // determine the minimum *clip* height; the inset lies outside that line.
    let font = view.font().expect("composer font must be configured first");
    let line_height = unsafe { view.layoutManager() }
        .expect("composer has a layout manager")
        .defaultLineHeightForFont(&font);
    let content_height = line_height.max(BASELINE_LINE_HEIGHT).ceil() + 2.0 * TEXT_INSET;
    let scroller = scroll
        .horizontalScroller()
        .expect("horizontal scroller configured");
    // Reserve the native space-consuming style even if the current preference
    // chooses overlay or autohide temporarily removes the horizontal bar.
    let outer = unsafe {
        NSScrollView::frameSizeForContentSize_horizontalScrollerClass_verticalScrollerClass_borderType_controlSize_scrollerStyle(
            NSSize::new(220.0, content_height),
            Some(NSScroller::class()),
            None,
            scroll.borderType(),
            scroller.controlSize(),
            NSScrollerStyle::Legacy,
            objc2::MainThreadMarker::new().expect("composer is main-thread only"),
        )
    };
    let metrics = ComposerMetrics {
        content_height,
        input_height: outer.height,
        reply_height: INPUT_ORIGIN_Y + outer.height + BOTTOM_INSET,
    };
    view.setMinSize(NSSize::new(0.0, content_height));
    view.setMaxSize(NSSize::new(CONTAINER_WIDTH, f64::MAX));
    if let Some(container) = unsafe { view.textContainer() } {
        container.setContainerSize(NSSize::new(
            CONTAINER_WIDTH,
            content_height - 2.0 * TEXT_INSET,
        ));
    }
    metrics
}

pub(crate) fn layout_composer(view: &NSTextView, scroll: &NSScrollView, metrics: ComposerMetrics) {
    let marked: bool = unsafe { objc2::msg_send![view, hasMarkedText] };
    if marked
        || scroll.frame().size.height < metrics.input_height
        || scroll.frame().size.width <= 0.0
    {
        return;
    }
    // Native style/autohide changes can retile the clip without changing the
    // outer frame. Reconcile the live viewport, not the cached reply label.
    let mut clip = scroll.contentSize();
    if clip.height < metrics.content_height {
        scroll.tile();
        clip = scroll.contentSize();
    }
    if clip.height < metrics.content_height {
        return;
    }
    let container = unsafe { view.textContainer() }.expect("composer has a text container");
    let size = container.containerSize();
    let available_height = (clip.height - 2.0 * TEXT_INSET).max(0.0);
    if size.width != CONTAINER_WIDTH || size.height != available_height {
        container.setContainerSize(NSSize::new(CONTAINER_WIDTH, available_height));
    }

    // Use the current laid-out text rather than the previous document width:
    // resizing the viewport or replacing a long draft must also shrink the
    // horizontal scrollable extent. Cached layout is cheap when text is unchanged.
    let manager = unsafe { view.layoutManager() }.expect("composer has a layout manager");
    manager.ensureLayoutForTextContainer(&container);
    let used = manager.usedRectForTextContainer(&container);
    let extra = manager.extraLineFragmentRect();
    let origin = view.textContainerOrigin();
    let content_right = (used.origin.x + used.size.width).max(extra.origin.x + extra.size.width);
    let width = clip
        .width
        .max((origin.x + content_right + TEXT_INSET).ceil());
    let frame = view.frame();
    if frame.size.width != width || frame.size.height != clip.height {
        view.setFrameSize(NSSize::new(width, clip.height));
        // Preserve any still-valid horizontal offset; constrain only offsets
        // stranded beyond the document after the text or viewport shrinks.
        let clip_view = scroll.contentView();
        let bounds = clip_view.bounds();
        let constrained = clip_view.constrainBoundsRect(bounds).origin;
        if constrained.x != bounds.origin.x || constrained.y != bounds.origin.y {
            clip_view.scrollToPoint(constrained);
            scroll.reflectScrolledClipView(&clip_view);
        }
    }
}
