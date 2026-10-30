#[cfg(windows)]
fn main() {
    use std::env;
    use std::fs::File;
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let source_png = manifest_dir.join("assets").join("icon_window_bar.png");

    println!("cargo:rerun-if-changed={}", source_png.display());

    let img = image::open(&source_png).expect("failed to read icon PNG");
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

    // Bundle several common icon sizes so Windows can pick the best one.
    for size in [16u32, 32, 48, 64, 128, 256] {
        let resized = img
            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let icon_image = ico::IconImage::from_rgba_data(size, size, resized.into_raw());
        let entry = ico::IconDirEntry::encode(&icon_image).expect("failed to encode icon entry");
        icon_dir.add_entry(entry);
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let icon_path = out_dir.join("humaipsy.ico");

    let mut icon_file = File::create(&icon_path).expect("failed to create generated .ico");
    icon_dir
        .write(&mut icon_file)
        .expect("failed to write generated .ico");

    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_path.to_str().expect("invalid icon path"));
    res.compile().expect("failed to compile windows resources");
}

#[cfg(not(windows))]
fn main() {}
