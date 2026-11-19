//! Vector icons: pictorial glyphs from the Atlas icon fonts (MIT, Vectopus) and a few
//! geometric primitives drawn with strokes. Both are resolution independent, so they stay
//! crisp at any pixel density, unlike the pre-rasterized FigCraft atlas.
use eframe::egui::{self, Color32, Pos2, Shape, Stroke, Vec2};

const PACKS: [(&str, &[u8]); 9] = [
    (
        "basic-ui",
        include_bytes!("../assets/fonts/atlas/basic-ui.ttf"),
    ),
    ("arrow", include_bytes!("../assets/fonts/atlas/arrow.ttf")),
    ("audio", include_bytes!("../assets/fonts/atlas/audio.ttf")),
    (
        "electronic-gadget",
        include_bytes!("../assets/fonts/atlas/electronic-gadget.ttf"),
    ),
    (
        "web-design",
        include_bytes!("../assets/fonts/atlas/web-design.ttf"),
    ),
    ("love", include_bytes!("../assets/fonts/atlas/love.ttf")),
    (
        "data-document",
        include_bytes!("../assets/fonts/atlas/data-document.ttf"),
    ),
    ("school", include_bytes!("../assets/fonts/atlas/school.ttf")),
    (
        "furniture",
        include_bytes!("../assets/fonts/atlas/furniture.ttf"),
    ),
];

pub fn register_fonts(fonts: &mut egui::FontDefinitions) {
    for (pack, bytes) in PACKS {
        let name = format!("atlas-{pack}");
        fonts.font_data.insert(
            name.clone(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        fonts
            .families
            .insert(egui::FontFamily::Name(name.clone().into()), vec![name]);
    }
}

#[derive(Clone, Copy)]
enum Prim {
    Plus,
    Minus,
    Check,
    Close,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    ChevronLeft,
    Play,
    Pause,
    Refresh,
    Power,
    Gauge,
    HeartFilled,
    Grip,
    Sparkles,
}

#[derive(Clone, Copy)]
enum Glyph {
    Font(&'static str, char),
    Prim(Prim),
}

fn glyph(key: &str) -> Option<Glyph> {
    use Glyph::{Font, Prim as P};
    Some(match key {
        "playlist-down" | "chevron-down" => P(Prim::ChevronDown),
        "playlist-up" | "chevron-up" => P(Prim::ChevronUp),
        "chevron-right" => P(Prim::ChevronRight),
        "chevron-left" => P(Prim::ChevronLeft),
        "playlist-remove" | "dialog-close" | "remove-close" | "close" => P(Prim::Close),
        "playlist-grip" | "grip" => P(Prim::Grip),
        "interval-minus" | "minus" => P(Prim::Minus),
        "interval-plus" | "plus" => P(Prim::Plus),
        "playlist-check" | "check" => P(Prim::Check),
        "play" | "▷" | "▶" | "play-large" => P(Prim::Play),
        "pause" => P(Prim::Pause),
        "skip-next" => Font("audio", '\u{e950}'),
        "skip-previous" => Font("audio", '\u{e93b}'),
        "shuffle" => Font("audio", '\u{e956}'),
        "refresh" => P(Prim::Refresh),
        "power" => P(Prim::Power),
        "gauge" => P(Prim::Gauge),
        "heart-filled" => P(Prim::HeartFilled),
        "sparkles" | "✧" => P(Prim::Sparkles),
        "remove-trash" | "trash" | "♲" => Font("basic-ui", '\u{e9c9}'),
        "gear" => Font("basic-ui", '\u{e996}'),
        "search" => Font("basic-ui", '\u{e9aa}'),
        "info" => Font("basic-ui", '\u{e99d}'),
        "list" => Font("basic-ui", '\u{e9a7}'),
        "more" => Font("basic-ui", '\u{e98d}'),
        "sliders" => Font("basic-ui", '\u{e999}'),
        "arrow-right" => Font("arrow", '\u{e99d}'),
        "import-upload" | "upload" => Font("arrow", '\u{ea19}'),
        "download" => Font("arrow", '\u{e9e0}'),
        "volume-x" => Font("audio", '\u{e94f}'),
        "volume" => Font("audio", '\u{e941}'),
        "audio-lines" => Font("audio", '\u{e939}'),
        "monitor" => Font("electronic-gadget", '\u{e95d}'),
        "music" | "♫" => Font("audio", '\u{e94a}'),
        "keyboard" => Font("electronic-gadget", '\u{e96f}'),
        "image" => Font("web-design", '\u{e98e}'),
        "grid" => Font("web-design", '\u{e98c}'),
        "layers" => Font("web-design", '\u{e991}'),
        "heart" | "♡" => Font("love", '\u{e94e}'),
        "folder" | "folder-open" | "▱" => Font("data-document", '\u{e9d2}'),
        "library" | "▥" => Font("school", '\u{e945}'),
        "clock" => Font("furniture", '\u{e94f}'),
        _ => return None,
    })
}

/// Maps a FigCraft node id (the id every call site already uses) to an icon key.
pub fn key_for_node(node: u32) -> Option<&'static str> {
    Some(match node {
        1251 | 72 => "chevron-down",
        1252 => "chevron-up",
        633 => "chevron-right",
        1253 | 1187 | 1317 | 769 => "close",
        1246 => "grip",
        1296 | 47 => "minus",
        1298 | 292 => "plus",
        1312 | 962 => "check",
        1352 | 974 | 137 | 286 => "play",
        618 => "pause",
        288 => "skip-next",
        284 => "skip-previous",
        282 => "shuffle",
        128 => "refresh",
        643 => "power",
        774 => "gauge",
        169 => "heart-filled",
        42 | 1221 => "sparkles",
        1325 | 1369 => "trash",
        45 | 780 => "gear",
        102 => "search",
        786 => "info",
        123 => "list",
        60 => "more",
        92 => "sliders",
        968 => "arrow-right",
        1234 | 125 => "upload",
        638 => "volume-x",
        64 => "volume",
        777 => "audio-lines",
        69 => "monitor",
        1356 => "music",
        791 => "keyboard",
        1216 => "image",
        121 => "grid",
        634 => "layers",
        1359 => "heart",
        1362 | 646 | 783 => "folder",
        1365 => "library",
        116 => "clock",
        _ => return None,
    })
}

/// Draws `key` centred on `center` inside a `size`-pixel box. Returns false when the key has
/// no vector equivalent, so the caller can fall back to the raster atlas.
pub fn paint(painter: &egui::Painter, center: Pos2, key: &str, color: Color32, size: f32) -> bool {
    let Some(glyph) = glyph(key) else {
        return false;
    };
    match glyph {
        Glyph::Font(pack, ch) => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                ch,
                egui::FontId::new(size, egui::FontFamily::Name(format!("atlas-{pack}").into())),
                color,
            );
        }
        Glyph::Prim(prim) => paint_prim(painter, center, prim, color, size),
    }
    true
}

fn line(painter: &egui::Painter, points: Vec<Pos2>, width: f32, color: Color32) {
    let (first, last) = (points[0], points[points.len() - 1]);
    painter.add(Shape::line(points, Stroke::new(width, color)));
    // Round caps.
    painter.circle_filled(first, width * 0.5, color);
    painter.circle_filled(last, width * 0.5, color);
}

fn fan(painter: &egui::Painter, center: Pos2, ring: &[Pos2], color: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for point in ring {
        mesh.colored_vertex(*point, color);
    }
    let n = ring.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
}

fn paint_prim(painter: &egui::Painter, c: Pos2, prim: Prim, color: Color32, size: f32) {
    // Unit box of half-size `u`; shapes keep ~15% padding like the pictorial glyphs.
    let u = size * 0.5 * 1.0;
    let w = (size * 0.09).clamp(1.3, 2.2);
    let p = |x: f32, y: f32| c + Vec2::new(x, y) * u;
    match prim {
        Prim::Plus => {
            line(painter, vec![p(-0.7, 0.0), p(0.7, 0.0)], w, color);
            line(painter, vec![p(0.0, -0.7), p(0.0, 0.7)], w, color);
        }
        Prim::Minus => line(painter, vec![p(-0.7, 0.0), p(0.7, 0.0)], w, color),
        Prim::Check => line(
            painter,
            vec![p(-0.75, 0.05), p(-0.25, 0.55), p(0.8, -0.5)],
            w,
            color,
        ),
        Prim::Close => {
            line(painter, vec![p(-0.6, -0.6), p(0.6, 0.6)], w, color);
            line(painter, vec![p(-0.6, 0.6), p(0.6, -0.6)], w, color);
        }
        Prim::ChevronDown => line(
            painter,
            vec![p(-0.6, -0.3), p(0.0, 0.3), p(0.6, -0.3)],
            w,
            color,
        ),
        Prim::ChevronUp => line(
            painter,
            vec![p(-0.6, 0.3), p(0.0, -0.3), p(0.6, 0.3)],
            w,
            color,
        ),
        Prim::ChevronRight => line(
            painter,
            vec![p(-0.3, -0.6), p(0.3, 0.0), p(-0.3, 0.6)],
            w,
            color,
        ),
        Prim::ChevronLeft => line(
            painter,
            vec![p(0.3, -0.6), p(-0.3, 0.0), p(0.3, 0.6)],
            w,
            color,
        ),
        Prim::Play => {
            let tri = vec![p(-0.5, -0.8), p(0.8, 0.0), p(-0.5, 0.8)];
            painter.add(Shape::convex_polygon(
                tri,
                color,
                Stroke::new(w * 0.9, color),
            ));
        }
        Prim::Pause => {
            for x in [-0.62, 0.2] {
                painter.rect_filled(
                    egui::Rect::from_min_max(p(x, -0.7), p(x + 0.42, 0.7)),
                    1.5,
                    color,
                );
            }
        }
        Prim::Refresh => {
            let (start, end) = (35.0_f32.to_radians(), 330.0_f32.to_radians());
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = start + (end - start) * i as f32 / 24.0;
                    p(0.7 * a.sin(), -0.7 * a.cos())
                })
                .collect();
            let tip = *pts.last().unwrap();
            line(painter, pts, w, color);
            line(
                painter,
                vec![
                    tip + Vec2::new(-0.05, 0.55) * u * 0.7,
                    tip,
                    tip + Vec2::new(0.5, 0.25) * u * 0.7,
                ],
                w,
                color,
            );
        }
        Prim::Power => {
            let (start, end) = (40.0_f32.to_radians(), 320.0_f32.to_radians());
            let pts: Vec<Pos2> = (0..=28)
                .map(|i| {
                    let a = start + (end - start) * i as f32 / 28.0;
                    p(0.7 * a.sin(), 0.1 - 0.7 * a.cos())
                })
                .collect();
            line(painter, pts, w, color);
            line(painter, vec![p(0.0, -0.85), p(0.0, -0.05)], w, color);
        }
        Prim::Gauge => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a =
                        std::f32::consts::PI * (1.0 + 0.0) + std::f32::consts::PI * i as f32 / 24.0;
                    p(0.85 * a.cos(), 0.55 + 0.85 * a.sin())
                })
                .collect();
            line(painter, pts, w, color);
            line(painter, vec![p(0.0, 0.55), p(0.45, 0.0)], w, color);
            painter.circle_filled(p(0.0, 0.55), w * 0.9, color);
        }
        Prim::HeartFilled => {
            let ring: Vec<Pos2> = (0..48)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / 48.0;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos()
                        - 5.0 * (2.0 * t).cos()
                        - 2.0 * (3.0 * t).cos()
                        - (4.0 * t).cos();
                    p(x / 17.0, -y / 17.0 + 0.05)
                })
                .collect();
            fan(painter, p(0.0, -0.05), &ring, color);
        }
        Prim::Grip => {
            for x in [-0.35, 0.35] {
                for y in [-0.6, 0.0, 0.6] {
                    painter.circle_filled(p(x, y), (size * 0.08).max(1.1), color);
                }
            }
        }
        Prim::Sparkles => {
            let ring = [
                p(0.0, -0.95),
                p(0.18, -0.18),
                p(0.95, 0.0),
                p(0.18, 0.18),
                p(0.0, 0.95),
                p(-0.18, 0.18),
                p(-0.95, 0.0),
                p(-0.18, -0.18),
            ];
            fan(painter, c, &ring, color);
        }
    }
}
