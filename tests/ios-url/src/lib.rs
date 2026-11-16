use gpui_mobile::gpui::Platform;

#[no_mangle]
pub extern "C" fn platform_open_url(invalid: bool) {
    // The suffix is in the same allocation, but outside the Rust string slice.
    let storage = b"https://example.invalid/exact?text=hello__OUTSIDE_SLICE__\0";
    let length = "https://example.invalid/exact?text=hello".len();
    let url = if invalid {
        "http://[::1"
    } else {
        std::str::from_utf8(&storage[..length]).unwrap()
    };
    gpui_mobile::IosPlatform::new().open_url(url);
}
