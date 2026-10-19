#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod window_icon;

use humaipsy::HumaiPsyApp;
use std::sync::Arc;
use tokio::runtime::Runtime;

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

    let viewport = window_icon::configure_viewport(
        eframe::egui::ViewportBuilder::default().with_inner_size([1080.0, 800.0]),
    );

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        // The version comes from the manifest, so a release bump names itself:
        // this used to be a hardcoded "Humai v.0.1" that drifted three releases behind.
        concat!("HumaiPsy v", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(move |cc| {
            window_icon::apply_runtime_icons(cc);
            Ok(Box::new(HumaiPsyApp::new(rt_handle.clone())))
        }),
    )
}
