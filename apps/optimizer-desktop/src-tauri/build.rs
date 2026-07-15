include!("src/command_manifest.rs");

fn main() {
    let icon_path = write_development_icon();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().window_icon_path(icon_path))
            .app_manifest(tauri_build::AppManifest::new().commands(REGISTERED_COMMANDS)),
    )
    .expect("failed to build Optimizer Desktop Tauri manifest");
}

fn write_development_icon() -> std::path::PathBuf {
    const WIDTH: usize = 16;
    const HEIGHT: usize = 16;
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let icon_path = output.join("optimizer-development.ico");
    let pixel_bytes = WIDTH * HEIGHT * 4;
    let mask_bytes = HEIGHT * 4;
    let image_bytes = 40 + pixel_bytes + mask_bytes;
    let mut icon = Vec::with_capacity(22 + image_bytes);
    push_u16(&mut icon, 0);
    push_u16(&mut icon, 1);
    push_u16(&mut icon, 1);
    icon.extend([WIDTH as u8, HEIGHT as u8, 0, 0]);
    push_u16(&mut icon, 1);
    push_u16(&mut icon, 32);
    push_u32(&mut icon, image_bytes as u32);
    push_u32(&mut icon, 22);
    push_u32(&mut icon, 40);
    push_i32(&mut icon, WIDTH as i32);
    push_i32(&mut icon, (HEIGHT * 2) as i32);
    push_u16(&mut icon, 1);
    push_u16(&mut icon, 32);
    push_u32(&mut icon, 0);
    push_u32(&mut icon, (pixel_bytes + mask_bytes) as u32);
    push_i32(&mut icon, 0);
    push_i32(&mut icon, 0);
    push_u32(&mut icon, 0);
    push_u32(&mut icon, 0);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let accent = if (x + y) % 5 == 0 { 0xD8 } else { 0x80 };
            icon.extend([0x70, accent, 0x32, 0xFF]);
        }
    }
    icon.resize(icon.len() + mask_bytes, 0);
    std::fs::write(&icon_path, icon).expect("development icon can be written to OUT_DIR");
    icon_path
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend(value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend(value.to_le_bytes());
}

fn push_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend(value.to_le_bytes());
}
