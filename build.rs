//! Windows PE resources. GPUI `load_icon` reads `HICON` resource id 1 from
//! this exe; Explorer and the taskbar use the same ICON. Do not embed a
//! second RT_MANIFEST here — gpui already ships PerMonitorV2.

#[path = "src/icon_mark.rs"]
mod icon_mark;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_mark.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_windows_icon();
}

fn embed_windows_icon() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico_path = out_dir.join("localtex.ico");
    write_ico(&ico_path);

    let icon = ico_path.to_string_lossy().replace('\\', "\\\\");
    let package_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut fields = package_version
        .split(['.', '-', '+'])
        .map(|field| field.parse::<u16>().unwrap_or(0))
        .chain(std::iter::repeat(0));
    let file_version = format!(
        "{},{},{},{}",
        fields.next().unwrap_or(0),
        fields.next().unwrap_or(0),
        fields.next().unwrap_or(0),
        fields.next().unwrap_or(0),
    );
    let description = rc_escape(
        &std::env::var("CARGO_PKG_DESCRIPTION")
            .unwrap_or_default()
            .replace('\u{2014}', "-")
            .replace('\u{2013}', "-"),
    );

    let resources = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {file_version}
PRODUCTVERSION {file_version}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "{description}\0"
            VALUE "FileVersion", "{package_version}\0"
            VALUE "InternalName", "localtex\0"
            VALUE "OriginalFilename", "localtex.exe\0"
            VALUE "ProductName", "LocalTeX\0"
            VALUE "ProductVersion", "{package_version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    );

    let script = out_dir.join("localtex.rc");
    std::fs::write(&script, resources).expect("write the resource script");

    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .expect("compile Windows resources");
}

fn write_ico(path: &std::path::Path) {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    use image::imageops::{self, FilterType};
    use image::ExtendedColorType;

    let src = icon_mark::raster(256);
    let sizes = [16_u32, 32, 48, 256];
    let rasters: Vec<image::RgbaImage> = sizes
        .iter()
        .copied()
        .map(|size| {
            if size == 256 {
                src.clone()
            } else {
                imageops::resize(&src, size, size, FilterType::Lanczos3)
            }
        })
        .collect();
    let frames: Vec<IcoFrame<'_>> = rasters
        .iter()
        .map(|img| {
            IcoFrame::as_png(
                img.as_raw(),
                img.width(),
                img.height(),
                ExtendedColorType::Rgba8,
            )
            .expect("encode ICO PNG frame")
        })
        .collect();
    let file = std::fs::File::create(path).expect("create ICO");
    IcoEncoder::new(file)
        .encode_images(&frames)
        .expect("encode ICO");
}

fn rc_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
