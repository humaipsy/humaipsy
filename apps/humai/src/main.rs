use humai::HumaiApp;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[cfg(windows)]
use eframe::egui::IconData;
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, GetSystemMetrics, SendMessageW, HICON, ICON_BIG, ICON_SMALL,
    LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
};

#[cfg(windows)]
fn create_hicon_from_png(png_bytes: &[u8], target_size: i32) -> HICON {
    // CreateIconFromResourceEx can consume PNG bytes directly when fIcon=true.
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

#[cfg(windows)]
fn set_windows_icons(cc: &eframe::CreationContext<'_>) {
    let Ok(window_handle) = cc.window_handle() else {
        return;
    };

    let RawWindowHandle::Win32(win32_handle) = window_handle.as_raw() else {
        return;
    };

    let hwnd = win32_handle.hwnd.get() as *mut core::ffi::c_void;

    let small_png = include_bytes!("../assets/icon_topbar_small.png");
    let big_png = include_bytes!("../assets/icon_window_bar.png");

    let small_size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let big_size = unsafe { GetSystemMetrics(SM_CXICON) };

    let small_icon = create_hicon_from_png(small_png, small_size);
    if !small_icon.is_null() {
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
        }
    }

    let big_icon = create_hicon_from_png(big_png, big_size);
    if !big_icon.is_null() {
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big_icon as isize);
        }
    }
}

fn main() -> eframe::Result<()> {
    // Create a Tokio runtime for async operations
    let rt = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
    let rt_handle = rt.handle().clone();

    // Keep the runtime alive by storing it
    std::thread::spawn(move || {
        rt.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });

    #[allow(unused_mut)]
    let mut viewport = eframe::egui::ViewportBuilder::default().with_inner_size([1080.0, 800.0]);

    #[cfg(windows)]
    {
        // Disable eframe's default icon so we can explicitly set small + big icons ourselves.
        viewport = viewport.with_icon(IconData::default());
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Humai v.0.1",
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            set_windows_icons(cc);

            Ok(Box::new(HumaiApp::new(rt_handle.clone())))
        }),
    )
}
