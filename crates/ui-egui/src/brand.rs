//! Lumen Video Editor's brand: the product name, version, author and links, the terms and licence
//! texts shown in About, and the app icon (`assets/app-icon/`, see its README) drawn in the header,
//! the About dialog and the Home screen.

/// The product name people see.
pub const PRODUCT_NAME: &str = "Lumen Video Editor";

/// Who makes Lumen Video Editor.
pub const AUTHOR: &str = "SifG22";

/// The app's version (About, the `.exe` details, the installers and the update check).
pub const VERSION: &str = "0.0.3";

/// SifG22's website (Help menu, About, Home).
pub const WEBSITE: &str = "https://sifg.net";

/// Lumen Video Editor's repository: releases, the update manifest and issues.
pub const REPO: &str = "https://github.com/SifG22/lumen_video_editor";

/// Native project files: `.lumenv` (Lumen Video). The older `.fcproj` name of the same format
/// still opens.
pub const PROJECT_EXTENSION: &str = "lumenv";

/// SifG22's terms and conditions of use (About › Terms, and the installer's licence page).
pub const TERMS_TEXT: &str = include_str!("../../../assets/legal/TERMINOS.txt");

/// The licence notices the app ships with (MIT, as its licence requires) and the third-party
/// material it bundles. Shown in About › Licenses.
pub const LICENSE_TEXT: &str = concat!(include_str!("../../../LICENSE-MIT"), "\n\n", include_str!("../../../NOTICE"));

/// `0.0.1`, plus the commit and date when the build recorded them: `0.0.1 (3f2a9c1d0, 2026-10-19)`.
pub fn long_version() -> String {
    let sha = option_env!("LUMEN_BUILD_SHA").map(|s| s.chars().take(9).collect::<String>()).filter(|s| !s.is_empty());
    let date = option_env!("LUMEN_BUILD_DATE").filter(|d| !d.is_empty());
    match (sha, date) {
        (Some(s), Some(d)) => format!("{VERSION} ({s}, {d})"),
        (Some(s), None) => format!("{VERSION} ({s})"),
        (None, Some(d)) => format!("{VERSION} ({d})"),
        (None, None) => VERSION.to_string(),
    }
}

const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/net.sifg.lumen.png");

/// The app icon texture, decoded on first use and kept in the context.
pub fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("lumen-app-icon");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let tex = ctx.load_texture("lumen-app-icon", color, options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Draw the app icon `size` points square with its left edge at `left_center` (vertically
/// centred). Returns the drawn rect.
pub fn paint_app_icon(ui: &egui::Ui, left_center: egui::Pos2, size: f32) -> Option<egui::Rect> {
    let tex = app_icon(ui.ctx())?;
    let r = egui::Rect::from_min_size(egui::pos2(left_center.x, left_center.y - size / 2.0), egui::vec2(size, size));
    ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn the_icon_decodes_to_a_256_px_square_with_transparent_corners() {
        let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (256, 256));
        assert_eq!(img.get_pixel(0, 0)[3], 0, "rounded corner");
        assert_eq!(img.get_pixel(128, 128)[3], 255, "opaque tile");
    }

    #[test]
    fn the_texture_is_decoded_once() {
        let ctx = egui::Context::default();
        let a = app_icon(&ctx).unwrap();
        let b = app_icon(&ctx).unwrap();
        assert_eq!(a.id(), b.id());
        assert_eq!(a.size(), [256, 256]);
    }

    #[test]
    fn ui_strings_name_lumen_video_editor() {
        assert_eq!(Language::En.tr("About Lumen Video Editor"), "About Lumen Video Editor");
        assert_eq!(Language::Es.tr("About Lumen Video Editor"), "Acerca de Lumen Video Editor");
    }

    #[test]
    fn the_version_is_shown_with_the_build_details_it_has() {
        assert!(long_version().starts_with(VERSION));
        assert!(TERMS_TEXT.contains(PRODUCT_NAME) && TERMS_TEXT.contains(AUTHOR));
        assert!(LICENSE_TEXT.contains("MIT License"));
    }
}
