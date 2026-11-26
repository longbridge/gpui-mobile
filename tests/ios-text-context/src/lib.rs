//! A synthetic GPUI document. UIKit tests exercise the backend's real UITextView.
use gpui_mobile::{gpui::*, ios::ffi};
use std::{
    cell::RefCell,
    ffi::{c_char, c_void, CStr, CString},
    ops::Range,
    rc::Rc,
};

thread_local! {
    // The UIKit test host owns this app for the process lifetime, including TLS teardown.
    static APP: RefCell<Option<&'static ApplicationHandle>> = const { RefCell::new(None) };
    static DOCUMENT: RefCell<Option<Entity<Document>>> = const { RefCell::new(None) };
    static FRAMES: RefCell<usize> = const { RefCell::new(0) };
    static HANDLER_PAINTS: RefCell<usize> = const { RefCell::new(0) };
}

struct Document {
    focus: FocusHandle,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    changes: Vec<(Option<Range<usize>>, String)>,
    backspace_keys: usize,
    editable: Option<Range<usize>>,
    reports_length: bool,
}

fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut count = 0;
    for (byte, character) in text.char_indices() {
        if count >= utf16 {
            return byte;
        }
        count += character.len_utf16();
    }
    text.len()
}

impl Render for Document {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "backspace" {
                    this.backspace_keys += 1;
                    let mut range = this.selection.clone();
                    if range.is_empty() {
                        let caret = byte_offset(&this.text, range.start);
                        if let Some((byte, _)) = this.text[..caret].char_indices().last() {
                            range.start = this.text[..byte].encode_utf16().count();
                        }
                    }
                    if !range.is_empty() {
                        this.replace_text_in_range(Some(range), "", window, cx);
                    }
                }
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        HANDLER_PAINTS.with(|paints| *paints.borrow_mut() += 1);
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for Document {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = byte_offset(&self.text, range.start);
        let end = byte_offset(&self.text, range.end);
        *adjusted = Some(
            self.text[..start].encode_utf16().count()..self.text[..end].encode_utf16().count(),
        );
        Some(self.text[start..end].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone()
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.changes.push((range.clone(), text.into()));
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = byte_offset(&self.text, range.start);
        let end = byte_offset(&self.text, range.end);
        self.text.replace_range(start..end, text);
        let caret = range.start + text.encode_utf16().count();
        self.selection = caret..caret;
        self.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range
            .as_ref()
            .or(self.marked.as_ref())
            .unwrap_or(&self.selection)
            .start;
        self.replace_text_in_range(range, text, window, cx);
        self.marked = Some(start..start + text.encode_utf16().count());
        if let Some(selection) = selection {
            self.selection = start + selection.start..start + selection.end;
        }
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.selection.start)
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.reports_length
            .then(|| self.text.encode_utf16().count())
    }

    fn text_input_editable_range(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.editable.clone()
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = range;
        cx.notify();
    }
}

#[no_mangle]
pub extern "C" fn regression_start() -> *mut c_void {
    ffi::gpui_ios_set_embedded();
    ffi::gpui_ios_initialize();
    let app =
        Application::with_platform(Rc::new(gpui_mobile::IosPlatform::new())).run_embedded(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let document = cx.new(|cx| Document {
                    focus: cx.focus_handle(),
                    text: String::new(),
                    selection: 0..0,
                    marked: None,
                    changes: vec![],
                    backspace_keys: 0,
                    editable: None,
                    reports_length: true,
                });
                let focus = document.read(cx).focus.clone();
                window.focus(&focus, cx);
                DOCUMENT.with(|slot| *slot.borrow_mut() = Some(document.clone()));
                document
            })
            .unwrap();
        });
    APP.with(|slot| *slot.borrow_mut() = Some(Box::leak(Box::new(app))));
    ffi::gpui_ios_did_finish_launching(std::ptr::null_mut());
    unsafe { ffi::gpui_ios_view_controller(ffi::gpui_ios_get_window()) }
}

#[no_mangle]
pub extern "C" fn regression_frame() {
    FRAMES.with(|frames| *frames.borrow_mut() += 1);
    ffi::gpui_ios_request_frame(ffi::gpui_ios_get_window());
}

#[no_mangle]
pub unsafe extern "C" fn regression_set_document(text: *const c_char, caret: usize) {
    let text = CStr::from_ptr(text).to_str().unwrap().to_owned();
    APP.with(|app| {
        app.borrow().as_ref().unwrap().update(|cx| {
            DOCUMENT.with(|document| {
                document.borrow().as_ref().unwrap().update(cx, |this, cx| {
                    this.text = text;
                    this.selection = caret..caret;
                    this.marked = None;
                    this.changes.clear();
                    this.backspace_keys = 0;
                    this.editable = None;
                    this.reports_length = true;
                    cx.notify();
                })
            });
        })
    });
}

#[no_mangle]
pub extern "C" fn regression_snapshot() -> *mut c_char {
    APP.with(|app| {
        app.borrow().as_ref().unwrap().update(|cx| {
            DOCUMENT.with(|document| {
                let slot = document.borrow();
                let doc = slot.as_ref().unwrap().read(cx);
                let changes: Vec<_> = doc
                    .changes
                    .iter()
                    .map(|(range, text)| {
                        serde_json::json!({
                            "range": range.as_ref().map(|r| [r.start, r.end]), "text": text,
                        })
                    })
                    .collect();
                let result = serde_json::json!({
                    "text": doc.text, "selection": [doc.selection.start, doc.selection.end],
                    "marked": doc.marked.as_ref().map(|r| [r.start, r.end]), "changes": changes,
                "backspaceKeys": doc.backspace_keys, "frames": FRAMES.with(|f| *f.borrow()),
                "handlerPaints": HANDLER_PAINTS.with(|paints| *paints.borrow()),
                });
                CString::new(result.to_string()).unwrap().into_raw()
            })
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn regression_free(text: *mut c_char) {
    drop(CString::from_raw(text));
}

#[no_mangle]
pub extern "C" fn regression_hide_keyboard() {
    ffi::gpui_ios_hide_keyboard(ffi::gpui_ios_get_window());
}

#[no_mangle]
pub extern "C" fn regression_set_editable_range(start: usize, end: usize, reports_length: bool) {
    APP.with(|app| {
        app.borrow().as_ref().unwrap().update(|cx| {
            DOCUMENT.with(|document| {
                document.borrow().as_ref().unwrap().update(cx, |this, cx| {
                    this.editable = Some(start..end);
                    this.reports_length = reports_length;
                    cx.notify();
                })
            });
        })
    });
}
