include!("src/command_manifest.rs");

fn main() {
    write_source_development_png();
    let icon_path = write_development_icon();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().window_icon_path(icon_path))
            .app_manifest(tauri_build::AppManifest::new().commands(REGISTERED_COMMANDS)),
    )
    .expect("failed to build Optimizer Desktop Tauri manifest");
}

fn write_source_development_png() {
    const WIDTH: usize = 32;
    const HEIGHT: usize = 32;
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"),
    );
    let icon_directory = manifest.join("icons");
    let icon_path = icon_directory.join("icon.png");
    std::fs::create_dir_all(&icon_directory).expect("development icon directory can be created");

    let mut scanlines = Vec::with_capacity(HEIGHT * (1 + WIDTH * 4));
    for y in 0..HEIGHT {
        scanlines.push(0);
        for x in 0..WIDTH {
            let accent = if (x + y) % 7 == 0 { 0x89 } else { 0x6b };
            scanlines.extend([0x2f, accent, 0x50, 0xff]);
        }
    }
    let compressed = zlib_uncompressed(&scanlines);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend((WIDTH as u32).to_be_bytes());
    header.extend((HEIGHT as u32).to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    push_png_chunk(&mut png, b"IHDR", &header);
    push_png_chunk(&mut png, b"IDAT", &compressed);
    push_png_chunk(&mut png, b"IEND", &[]);
    std::fs::write(icon_path, png).expect("development PNG icon can be written");
}

fn zlib_uncompressed(input: &[u8]) -> Vec<u8> {
    assert!(input.len() <= u16::MAX as usize);
    let length = input.len() as u16;
    let mut output = vec![0x78, 0x01, 0x01];
    output.extend(length.to_le_bytes());
    output.extend((!length).to_le_bytes());
    output.extend(input);
    output.extend(adler32(input).to_be_bytes());
    output
}

fn adler32(input: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for byte in input {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn push_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    output.extend((payload.len() as u32).to_be_bytes());
    output.extend(kind);
    output.extend(payload);
    let mut checksum_input = kind.to_vec();
    checksum_input.extend(payload);
    output.extend(crc32(&checksum_input).to_be_bytes());
}

fn crc32(input: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in input {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
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
