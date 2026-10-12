//! Offscreen windows: GPUI windows drawn into a texture instead of onto the
//! screen, so an application can turn a view into an image — a share card, a
//! long screenshot of a conversation.
//!
//! [`OffscreenWindow::open`] opens an ordinary GPUI window whose platform side
//! has no UIKit / NDK surface: layout, text shaping and painting run exactly as
//! they do on screen, and each [`OffscreenWindow::render`] forces one frame and
//! reads its pixels back through a headless wgpu renderer.
//!
//! The renderer (one GPU device plus its pipelines) is created on first use and
//! kept for the rest of the process on the main thread, so later captures skip
//! device creation and shader compilation.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use anyhow::{Context as _, Result};
use gpui::{
    point, px, App, Bounds, Capslock, DevicePixels, DispatchEventResult, Entity, GpuSpecs,
    Modifiers, Pixels, PlatformAtlas, PlatformDisplay, PlatformInput, PlatformInputHandler,
    PlatformWindow, Point, PromptButton, PromptLevel, Render, RequestFrameOptions, Scene, Size,
    Window, WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControlArea,
    WindowHandle, WindowOptions, WindowParams, WindowVisibility,
};
use gpui_wgpu::WgpuHeadlessRenderer;
pub use image::RgbaImage;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

thread_local! {
    /// Set by [`OffscreenWindow::open`] for the platform's `open_window` call it
    /// makes, which then answers with an [`OffscreenPlatformWindow`].
    static PENDING: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
    static RENDERER: RefCell<Option<Rc<RefCell<WgpuHeadlessRenderer>>>> =
        const { RefCell::new(None) };
}

/// A GPUI window rendered into images instead of onto the screen.
///
/// Dropping the handle leaves the window open; call [`Self::close`].
pub struct OffscreenWindow<V> {
    handle: WindowHandle<V>,
    state: Rc<RefCell<State>>,
}

impl<V: Render + 'static> OffscreenWindow<V> {
    /// Opens a window of `size` logical pixels drawn at `scale_factor` device
    /// pixels per logical pixel, with `build` creating its root view.
    pub fn open(
        size: Size<Pixels>,
        scale_factor: f32,
        appearance: WindowAppearance,
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<Self> {
        let renderer = renderer()?;
        let sprite_atlas = renderer.borrow().sprite_atlas();
        let state = Rc::new(RefCell::new(State {
            bounds: Bounds::new(Point::default(), size),
            scale_factor,
            appearance,
            renderer,
            sprite_atlas,
            request_frame: None,
            resize: None,
            input_handler: None,
            read_back: false,
            image: None,
        }));
        PENDING.with(|pending| *pending.borrow_mut() = Some(state.clone()));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size,
            ))),
            focus: false,
            show: false,
            ..Default::default()
        };
        let handle = cx.open_window(options, build);
        // `open_window` takes it on success; clear it if the platform never got that far.
        PENDING.with(|pending| pending.borrow_mut().take());
        Ok(Self {
            handle: handle?,
            state,
        })
    }

    /// The GPUI handle of the window, to update its root view between renders.
    pub fn handle(&self) -> WindowHandle<V> {
        self.handle
    }

    /// The largest size one [`Self::render`] can produce: the GPU's texture
    /// limit divided by the window's scale factor.
    pub fn max_size(&self) -> Size<Pixels> {
        let state = self.state.borrow();
        let side = px(state.renderer.borrow().max_texture_size() as f32 / state.scale_factor);
        Size {
            width: side,
            height: side,
        }
    }

    /// Lays the window out at `size`, draws one frame and returns its pixels.
    ///
    /// GPUI delivers the frame through the window's own callbacks, which borrow
    /// the `App` themselves, so call this from an async task between `update`s
    /// (for example after `cx.spawn(async move |cx| …)` resumes), never from
    /// inside `App::update` or a view's `render`.
    pub fn render(&self, size: Size<Pixels>) -> Result<RgbaImage> {
        self.frame(size, true)?
            .context("GPUI did not draw the offscreen window")?
    }

    /// Lays the window out at `size` and paints one frame without reading it
    /// back, so views can measure their content (for example to size the next
    /// [`Self::render`]). The same calling rule as [`Self::render`] applies.
    pub fn layout(&self, size: Size<Pixels>) -> Result<()> {
        self.frame(size, false).map(|_| ())
    }

    fn frame(&self, size: Size<Pixels>, read_back: bool) -> Result<Option<Result<RgbaImage>>> {
        let resize = {
            let mut state = self.state.borrow_mut();
            let changed = state.bounds.size != size;
            state.bounds.size = size;
            state.read_back = read_back;
            state.image = None;
            changed.then(|| state.resize.take()).flatten()
        };
        if let Some(mut resize) = resize {
            let scale_factor = self.state.borrow().scale_factor;
            resize(size, scale_factor);
            self.state.borrow_mut().resize = Some(resize);
        }

        let mut request_frame = self
            .state
            .borrow_mut()
            .request_frame
            .take()
            .context("the offscreen window has no frame callback; was it closed?")?;
        request_frame(RequestFrameOptions {
            require_presentation: true,
            force_render: true,
            ..Default::default()
        });
        let mut state = self.state.borrow_mut();
        state.request_frame = Some(request_frame);
        Ok(state.image.take())
    }

    /// Closes the window, dropping its view tree.
    pub fn close(self, cx: &mut App) {
        let _ = self
            .handle
            .update(cx, |_, window, _| window.remove_window());
    }
}

/// Answers the platform's `open_window` with an offscreen window when
/// [`OffscreenWindow::open`] asked for one.
pub(crate) fn take_pending(_params: &WindowParams) -> Option<Box<dyn PlatformWindow>> {
    let state = PENDING.with(|pending| pending.borrow_mut().take())?;
    Some(Box::new(OffscreenPlatformWindow(state)))
}

fn renderer() -> Result<Rc<RefCell<WgpuHeadlessRenderer>>> {
    RENDERER.with(|slot| {
        if let Some(renderer) = slot.borrow().as_ref() {
            return Ok(renderer.clone());
        }
        let renderer = Rc::new(RefCell::new(create_renderer()?));
        *slot.borrow_mut() = Some(renderer.clone());
        Ok(renderer)
    })
}

/// iOS only has Metal, which the default wgpu instance leaves out.
#[cfg(target_os = "ios")]
fn create_renderer() -> Result<WgpuHeadlessRenderer> {
    let instance = gpui_wgpu::wgpu::Instance::new(gpui_wgpu::wgpu::InstanceDescriptor {
        backends: gpui_wgpu::wgpu::Backends::METAL,
        flags: gpui_wgpu::wgpu::InstanceFlags::default(),
        backend_options: gpui_wgpu::wgpu::BackendOptions::default(),
        memory_budget_thresholds: gpui_wgpu::wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    WgpuHeadlessRenderer::with_instance(instance)
}

#[cfg(not(target_os = "ios"))]
fn create_renderer() -> Result<WgpuHeadlessRenderer> {
    WgpuHeadlessRenderer::new()
}

/// GPUI's per-window frame callback, driven by [`OffscreenWindow::frame`].
type FrameCallback = Box<dyn FnMut(RequestFrameOptions)>;
/// GPUI's per-window resize callback.
type ResizeCallback = Box<dyn FnMut(Size<Pixels>, f32)>;

struct State {
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    appearance: WindowAppearance,
    renderer: Rc<RefCell<WgpuHeadlessRenderer>>,
    sprite_atlas: Arc<dyn PlatformAtlas>,
    request_frame: Option<FrameCallback>,
    resize: Option<ResizeCallback>,
    input_handler: Option<PlatformInputHandler>,
    /// Whether the next `draw` reads its pixels back ([`OffscreenWindow::render`])
    /// or only paints ([`OffscreenWindow::layout`]).
    read_back: bool,
    /// What the last `draw` produced, taken by [`OffscreenWindow::render`].
    image: Option<Result<RgbaImage>>,
}

struct OffscreenPlatformWindow(Rc<RefCell<State>>);

// No native window backs it, so there is no handle to report; `NotSupported`
// is `raw_window_handle`'s variant for exactly this.
impl HasWindowHandle for OffscreenPlatformWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl HasDisplayHandle for OffscreenPlatformWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::NotSupported)
    }
}

impl PlatformWindow for OffscreenPlatformWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        false
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.bounds().size
    }

    fn resize(&mut self, size: Size<Pixels>) {
        self.0.borrow_mut().bounds.size = size;
    }

    fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor
    }

    fn appearance(&self) -> WindowAppearance {
        self.0.borrow().appearance
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        None
    }

    fn mouse_position(&self) -> Point<Pixels> {
        Point::default()
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }

    fn capslock(&self) -> Capslock {
        Capslock::default()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.0.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {}

    fn is_active(&self) -> bool {
        false
    }

    fn visibility(&self) -> WindowVisibility {
        WindowVisibility::Visible
    }

    fn is_hovered(&self) -> bool {
        false
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, _title: &str) {}

    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {}

    fn minimize(&self) {}

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        false
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, _callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {}

    fn on_active_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}

    fn on_visibility_change(&self, _callback: Box<dyn FnMut(WindowVisibility)>) {}

    fn on_hover_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, _callback: Box<dyn FnMut()>) {}

    fn on_should_close(&self, _callback: Box<dyn FnMut() -> bool>) {}

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
    }

    fn on_close(&self, _callback: Box<dyn FnOnce()>) {}

    fn on_appearance_changed(&self, _callback: Box<dyn FnMut()>) {}

    fn draw(&self, scene: &Scene) {
        let mut state = self.0.borrow_mut();
        if !state.read_back {
            return;
        }
        let size: Size<DevicePixels> = state.bounds.size.to_device_pixels(state.scale_factor);
        let image = state.renderer.borrow_mut().render_to_image(scene, size);
        state.image = Some(image);
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.borrow().sprite_atlas.clone()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        None
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}
}
