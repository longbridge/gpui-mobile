//! GPUI element for embedding native platform views.
//!
//! `PlatformViewElement` is a GPUI component that reserves layout space for
//! a native view (Android `View` or iOS `UIView`) and synchronizes its
//! position and size during the paint phase.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use gpui_mobile::components::platform_view_element::platform_view_element;
//! use gpui_mobile::platform_view::PlatformViewHandle;
//!
//! fn my_component(handle: Arc<PlatformViewHandle>) -> impl IntoElement {
//!     div()
//!         .flex()
//!         .child(platform_view_element(handle).size_full())
//! }
//! ```

use crate::platform_view::{PlatformViewBounds, PlatformViewHandle};
use gpui::{
    div, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Refineable as _, Style, StyleRefinement, Styled, Window,
};
use std::sync::{Arc, Weak};

/// Create a GPUI element that hosts a native platform view.
///
/// The element reserves layout space and, when painted, moves the native view
/// to its bounds and clips it to the visible area of its scroll containers
/// (native views draw above GPUI content). When the element stops being painted
/// (its screen closes, a tab switches away) the native view is hidden, and shown
/// again when it comes back. [`PlatformViewHandle::set_visible`] still hides it
/// on request, for instance under a GPUI dialog.
///
/// Size it like any element: `.w()`, `.h()`, `.size_full()`, `.flex_grow()`.
pub fn platform_view_element(handle: Arc<PlatformViewHandle>) -> PlatformViewElement {
    PlatformViewElement {
        handle,
        style: StyleRefinement::default(),
    }
}

/// The element returned by [`platform_view_element`].
pub struct PlatformViewElement {
    handle: Arc<PlatformViewHandle>,
    style: StyleRefinement,
}

impl Styled for PlatformViewElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for PlatformViewElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Kept in the element's state while it is painted; GPUI drops the state of an
/// element that wasn't painted in a frame, which hides the view.
struct OnScreen(Weak<PlatformViewHandle>);

impl Drop for OnScreen {
    fn drop(&mut self) {
        if let Some(handle) = self.0.upgrade() {
            handle.set_on_screen(false);
        }
    }
}

fn logical(bounds: Bounds<Pixels>) -> PlatformViewBounds {
    PlatformViewBounds {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
    }
}

impl Element for PlatformViewElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::NamedInteger(
            "platform-view".into(),
            self.handle.id().0,
        ))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let handle = &self.handle;
        let visible = bounds.intersect(&window.content_mask().bounds);
        let on_screen = visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO;
        if on_screen {
            handle.set_bounds(logical(bounds));
            handle.set_clip((visible != bounds).then(|| logical(visible)));
        }
        handle.set_on_screen(on_screen);
        if let Some(id) = id {
            window.with_element_state(id, |state: Option<OnScreen>, _| {
                (
                    (),
                    state.unwrap_or_else(|| OnScreen(Arc::downgrade(handle))),
                )
            });
        }
    }
}

/// A higher-level wrapper that creates the platform view from the registry
/// and manages its lifecycle.
///
/// The view is created on first render and disposed when dropped.
pub struct ManagedPlatformView {
    handle: Option<Arc<PlatformViewHandle>>,
    view_type: String,
    creation_params: std::collections::HashMap<String, String>,
}

impl ManagedPlatformView {
    /// Create a new managed platform view for the given type.
    pub fn new(view_type: impl Into<String>) -> Self {
        Self {
            handle: None,
            view_type: view_type.into(),
            creation_params: std::collections::HashMap::new(),
        }
    }

    /// Add a creation parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.creation_params.insert(key.into(), value.into());
        self
    }

    /// Get or create the platform view handle.
    pub fn ensure_created(&mut self) -> Result<Arc<PlatformViewHandle>, String> {
        if let Some(handle) = &self.handle {
            return Ok(handle.clone());
        }

        let registry = crate::platform_view::PlatformViewRegistry::global();
        let params = crate::platform_view::PlatformViewParams {
            bounds: PlatformViewBounds::default(),
            creation_params: self.creation_params.clone(),
        };

        let handle = Arc::new(registry.create_view(&self.view_type, params)?);
        self.handle = Some(handle.clone());
        Ok(handle)
    }

    /// Get the handle if the view has been created.
    pub fn handle(&self) -> Option<&Arc<PlatformViewHandle>> {
        self.handle.as_ref()
    }

    /// Render the platform view element, or an empty element if the view
    /// fails to create.
    pub fn render(&mut self) -> gpui::AnyElement {
        match self.ensure_created() {
            Ok(handle) => platform_view_element(handle).into_any_element(),
            Err(e) => {
                log::error!("Failed to create platform view '{}': {}", self.view_type, e);
                div().into_any_element()
            }
        }
    }
}

impl Drop for ManagedPlatformView {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.dispose();
        }
    }
}
