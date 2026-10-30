//! Platform-specific window icon setup.
//!
//! - Linux: set the title-bar icon via [`ViewportBuilder::with_icon`] (X11 only; Wayland ignores it).
//! - Windows: set small + title-bar icons via Win32 APIs after the window is created.

const WINDOW_BAR_ICON_PNG: &[u8] = include_bytes!("../assets/icon_window_bar.png");

#[cfg(windows)]
mod platform {
    use super::WINDOW_BAR_ICON_PNG;
    use eframe::CreationContext;
    use eframe::egui::{IconData, ViewportBuilder};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, GetSystemMetrics, SendMessageW, HICON, ICON_BIG, ICON_SMALL,
        LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    const TOPBAR_SMALL_ICON_PNG: &[u8] = include_bytes!("../assets/icon_topbar_small.png");

    pub fn configure_viewport(viewport: ViewportBuilder) -> ViewportBuilder {
        viewport.with_icon(IconData::default())
    }

    pub fn apply_runtime_icons(cc: &CreationContext<'_>) {
        let Ok(window_handle) = cc.window_handle() else {
            return;
        };

        let RawWindowHandle::Win32(win32_handle) = window_handle.as_raw() else {
            return;
        };

        let hwnd = win32_handle.hwnd.get() as *mut core::ffi::c_void;

        let small_size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        let big_size = unsafe { GetSystemMetrics(SM_CXICON) };

        let small_icon = create_hicon_from_png(TOPBAR_SMALL_ICON_PNG, small_size);
        if !small_icon.is_null() {
            unsafe {
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
            }
        }

        let big_icon = create_hicon_from_png(WINDOW_BAR_ICON_PNG, big_size);
        if !big_icon.is_null() {
            unsafe {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big_icon as isize);
            }
        }
    }

    fn create_hicon_from_png(png_bytes: &[u8], target_size: i32) -> HICON {
        unsafe {
            CreateIconFromResourceEx(
                png_bytes.as_ptr() as *mut u8,
                png_bytes.len() as u32,
                1,
                0x0003_0000,
                target_size,
                target_size,
                LR_DEFAULTCOLOR,
            )
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::WINDOW_BAR_ICON_PNG;
    use eframe::CreationContext;
    use eframe::egui::{IconData, ViewportBuilder};

    pub fn configure_viewport(viewport: ViewportBuilder) -> ViewportBuilder {
        viewport.with_icon(decode_png_icon(WINDOW_BAR_ICON_PNG))
    }

    pub fn apply_runtime_icons(_cc: &CreationContext<'_>) {}

    fn decode_png_icon(png: &[u8]) -> IconData {
        let image = image::load_from_memory(png).expect("failed to decode window icon PNG");
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        }
    }
}

pub use platform::{apply_runtime_icons, configure_viewport};
