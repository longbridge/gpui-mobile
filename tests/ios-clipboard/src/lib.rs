use gpui_mobile::gpui::{ClipboardItem, Platform};

const TEXT: &str = "Rust\0中文🙂";

#[no_mangle]
pub extern "C" fn clipboard_write() {
    gpui_mobile::IosPlatform::new().write_to_clipboard(ClipboardItem::new_string(TEXT.into()));
}

#[no_mangle]
pub extern "C" fn clipboard_check() -> bool {
    gpui_mobile::IosPlatform::new()
        .read_from_clipboard()
        .and_then(|item| item.text())
        .as_deref()
        == Some(TEXT)
}
