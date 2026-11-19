//! Icons rendered from the cached FigCraft vectors, including their exact bounds.
use eframe::egui::{self, Color32, Vec2};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone, Deserialize)]
struct Entry {
    rect: [f32; 4],
    size: [f32; 2],
    pad: f32,
}
#[derive(Deserialize)]
struct Manifest {
    width: f32,
    height: f32,
    scale: f32,
    icons: HashMap<String, Entry>,
}
#[derive(Clone)]
struct Atlas {
    texture: egui::TextureHandle,
    entries: HashMap<String, Entry>,
    size: Vec2,
    scale: f32,
}

/// The atlas is baked at `scale` (4x). Sampling it with plain bilinear filtering at 1x or
/// 1.5x skips most texels and makes icons look aliased, so it is box/Lanczos-resampled once
/// to the physical pixel density actually used on screen (quantized to 0.25 steps).
fn atlas(ctx: &egui::Context) -> Atlas {
    let manifest: Manifest =
        serde_json::from_str(include_str!("../assets/figcraft-icons.json")).expect("icon manifest");
    let target = ((ctx.pixels_per_point() * 4.0).round() / 4.0).clamp(0.5, manifest.scale);
    let id = egui::Id::new(("figcraft-icon-atlas", (target * 4.0) as u32));
    if let Some(atlas) = ctx.data(|data| data.get_temp::<Atlas>(id)) {
        return atlas;
    }
    let mut image = image::load_from_memory(include_bytes!("../assets/figcraft-icons.png"))
        .expect("icon atlas")
        .to_rgba8();
    if target < manifest.scale {
        // Resample premultiplied so transparent texels do not bleed colour into edges.
        for px in image.pixels_mut() {
            let a = px[3] as u32;
            for c in 0..3 {
                px[c] = ((px[c] as u32 * a + 127) / 255) as u8;
            }
        }
        let ratio = target / manifest.scale;
        let w = ((image.width() as f32 * ratio).round() as u32).max(1);
        let h = ((image.height() as f32 * ratio).round() as u32).max(1);
        image = image::imageops::resize(&image, w, h, image::imageops::FilterType::Lanczos3);
        for px in image.pixels_mut() {
            let a = px[3] as u32;
            if a > 0 {
                for c in 0..3 {
                    px[c] = ((px[c] as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
    let atlas = Atlas {
        texture: ctx.load_texture(
            "figcraft-icons",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                &image,
            ),
            egui::TextureOptions::LINEAR,
        ),
        entries: manifest.icons,
        size: Vec2::new(manifest.width, manifest.height),
        scale: manifest.scale,
    };
    ctx.data_mut(|data| data.insert_temp(id, atlas.clone()));
    atlas
}

pub fn paint_node(
    painter: &egui::Painter,
    center: egui::Pos2,
    node: u32,
    color: Color32,
    size: Option<Vec2>,
) {
    let atlas = atlas(painter.ctx());
    let Some(entry) = atlas.entries.get(&node.to_string()) else {
        return;
    };
    let native = Vec2::new(entry.size[0], entry.size[1]);
    if let Some(key) = crate::icon_set::key_for_node(node) {
        let target = size.unwrap_or(native);
        if crate::icon_set::paint(painter, center, key, color, target.x.max(target.y)) {
            return;
        }
    }
    let scale = size.unwrap_or(native) / native;
    let dimensions = Vec2::new(entry.rect[2], entry.rect[3]) / atlas.scale * scale;
    let min = center - native * scale * 0.5 - Vec2::splat(entry.pad) * scale;
    let rect = egui::Rect::from_min_size(min, dimensions);
    let uv = egui::Rect::from_min_max(
        egui::pos2(entry.rect[0] / atlas.size.x, entry.rect[1] / atlas.size.y),
        egui::pos2(
            (entry.rect[0] + entry.rect[2]) / atlas.size.x,
            (entry.rect[1] + entry.rect[3]) / atlas.size.y,
        ),
    );
    painter.image(atlas.texture.id(), rect, uv, color);
}

pub fn paint(painter: &egui::Painter, center: egui::Pos2, key: &str, color: Color32) -> bool {
    let node = match key {
        "playlist-down" => 1251,
        "playlist-up" => 1252,
        "playlist-remove" => 1253,
        "playlist-grip" => 1246,
        "interval-minus" => 1296,
        "interval-plus" => 1298,
        "dialog-close" => 1187,
        "remove-close" => 1317,
        "remove-trash" => 1325,
        "import-upload" => 1234,
        "playlist-check" => 1312,
        "play" | "▷" | "▶" => 1352,
        "play-large" => 974,
        "heart" | "♡" => 1359,
        "music" | "♫" => 1356,
        "library" | "▥" => 1365,
        "folder" | "▱" => 1362,
        "trash" | "♲" => 1369,
        "gear" => 45,
        "refresh" => 128,
        "close" => 769,
        "more" => 60,
        "plus" => 292,
        "minus" => 47,
        "check" => 962,
        "arrow-right" => 968,
        "upload" => 125,
        "shuffle" => 282,
        "repeat" => 290,
        "skip-next" => 288,
        "skip-previous" => 284,
        "monitor" => 69,
        "sliders" => 92,
        "sparkles" | "✧" => 42,
        "maximize" => 48,
        "minimize" => 47,
        "heart-filled" => 169,
        "power" => 643,
        "pause" => 618,
        "chevron-down" => 72,
        "chevron-right" => 633,
        "keyboard" => 791,
        "info" => 786,
        "layers" => 634,
        "volume-x" => 638,
        "folder-open" => 646,
        "image" => 1216,
        "search" => 102,
        "clock" => 116,
        "grid" => 121,
        "list" => 123,
        _ => return false,
    };
    paint_node(painter, center, node, color, None);
    true
}
