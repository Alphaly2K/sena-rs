use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

static DEFAULT_TTF_BYTES: &[u8] = include_bytes!("default.ttf");

#[derive(Clone, Debug)]
pub struct PalFontSystem {
    fallback: PalFontFallback,
    begun: bool,
    font_size: u16,
    font_type: u16,
    effect: u16,
    color: u32,
    effect_color: u32,
    ex_font_loaded: bool,
}

impl Default for PalFontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl PalFontSystem {
    pub fn new() -> Self {
        Self {
            fallback: PalFontFallback::default_ttf(),
            begun: false,
            font_size: 28,
            font_type: 1,
            effect: 0,
            color: 0xFF00_0000,
            effect_color: 0xFFFF_FFFF,
            ex_font_loaded: false,
        }
    }

    pub fn begin(&mut self) -> bool {
        if self.font_type == 4 {
            self.font_type = 1;
        }
        self.begun = true;
        true
    }

    pub fn end(&mut self) -> bool {
        self.begun = false;
        true
    }

    pub fn is_begun(&self) -> bool {
        self.begun
    }

    pub fn set_color(&mut self, color: u32, effect_color: u32) {
        self.color = color;
        self.effect_color = effect_color;
    }

    pub fn color(&self) -> (u32, u32) {
        (self.color, self.effect_color)
    }

    pub fn set_effect(&mut self, effect: u16) {
        self.effect = effect;
    }

    pub fn effect(&self) -> u16 {
        self.effect
    }

    pub fn set_font_size(&mut self, font_size: u16) {
        self.font_size = font_size.max(1);
    }

    pub fn font_size(&self) -> u16 {
        self.font_size
    }

    pub fn set_type(&mut self, font_type: u16) -> bool {
        if font_type == 4 && !self.ex_font_loaded {
            return false;
        }
        self.font_type = font_type;
        true
    }

    pub fn font_type(&self) -> u16 {
        self.font_type
    }

    pub fn set_ex_font_loaded(&mut self, loaded: bool) {
        self.ex_font_loaded = loaded;
        if !loaded && self.font_type == 4 {
            self.font_type = 1;
        }
    }

    pub fn measure(&self, text: &str) -> (u32, u32) {
        let font_size = f32::from(self.font_size.max(1));
        self.fallback.measure_line(text, font_size)
    }

    pub fn rasterize(&self, text: &str) -> (u32, u32, Vec<u8>) {
        let color = argb_to_bgra(self.color);
        let (width, height, mut pixels) =
            self.fallback
                .rasterize_line(text, f32::from(self.font_size.max(1)), color);
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        if self.effect == 0 || text.is_empty() {
            return (width, height, pixels);
        }
        let edge = argb_to_rgba(self.effect_color);
        if edge[3] == 0 {
            return (width, height, pixels);
        }
        apply_text_edge(&pixels, width, height, edge)
    }
}

#[derive(Clone, Debug)]
pub struct PalFontFallback {
    font: FontRef<'static>,
}

impl PalFontFallback {
    /// Construct using the embedded default.ttf.  Panics only if the embedded file
    /// is corrupt (which would be a build-time error, not a runtime condition).
    pub fn default_ttf() -> Self {
        for path in SYSTEM_CJK_FONT_CANDIDATES {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            if let Ok(font) = FontRef::try_from_slice(leaked) {
                return Self { font };
            }
        }
        let font = FontRef::try_from_slice(DEFAULT_TTF_BYTES)
            .expect("default.ttf embedded in pal-vm is not a valid TrueType font");
        Self { font }
    }

    /// Measure the pixel width of a single text line at the given pixel height.
    /// Returns `(width_px, height_px)`.
    pub fn measure_line(&self, text: &str, px_height: f32) -> (u32, u32) {
        let scale = PxScale::from(px_height);
        let scaled = self.font.as_scaled(scale);
        let width: f32 = text
            .chars()
            .map(|c| scaled.h_advance(self.font.glyph_id(c)))
            .sum();
        (width.ceil() as u32, px_height.ceil() as u32)
    }

    /// Rasterize a single line of text into BGRA8 pixels.
    ///
    /// Returns `(width, height, pixels_bgra)`.  If the text is empty or all
    /// glyphs have zero advance, returns a 1×height blank buffer.
    ///
    /// Color is `[B, G, R, A]` matching the PAL surface format.
    pub fn rasterize_line(
        &self,
        text: &str,
        px_height: f32,
        color_bgra: [u8; 4],
    ) -> (u32, u32, Vec<u8>) {
        let scale = PxScale::from(px_height);
        let scaled = self.font.as_scaled(scale);

        let (width, height) = self.measure_line(text, px_height);
        let w = width.max(1) as usize;
        let h = height.max(1) as usize;
        let mut pixels = vec![0u8; w * h * 4];

        let mut cursor_x = 0.0f32;
        let baseline_y = scaled.ascent();

        for ch in text.chars() {
            let glyph_id = self.font.glyph_id(ch);
            let glyph =
                glyph_id.with_scale_and_position(scale, ab_glyph::point(cursor_x, baseline_y));
            cursor_x += scaled.h_advance(glyph_id);

            if let Some(outlined) = self.font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                // Some fallback fonts place shorter glyphs a pixel or two
                // above the bottom of the ideographic cell. PAL text uses a
                // shared cell baseline, so align substantial short glyphs to
                // that baseline while leaving centered marks and low commas
                // at their font-defined positions.
                let y_shift = glyph_baseline_shift(
                    bounds.min.y as i32,
                    bounds.max.y as i32,
                    baseline_y,
                    px_height,
                );
                outlined.draw(|gx, gy, cov| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32 + y_shift;
                    if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                        return;
                    }
                    let idx = (py as usize * w + px as usize) * 4;
                    let alpha = (cov * color_bgra[3] as f32) as u8;
                    pixels[idx] = color_bgra[0];
                    pixels[idx + 1] = color_bgra[1];
                    pixels[idx + 2] = color_bgra[2];
                    pixels[idx + 3] = alpha;
                });
            }
        }

        (w as u32, h as u32, pixels)
    }
}

fn glyph_baseline_shift(min_y: i32, max_y: i32, ascent: f32, cell_height: f32) -> i32 {
    if (max_y - min_y) as f32 <= cell_height * 0.45 {
        return 0;
    }
    let baseline_bottom = ascent.ceil() as i32 - 1;
    (baseline_bottom - (max_y - 1)).clamp(0, (cell_height * 0.1).ceil() as i32)
}

const SYSTEM_CJK_FONT_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "C:/Windows/Fonts/msyh.ttc",
    "C:/Windows/Fonts/msgothic.ttc",
];

fn argb_to_bgra(color: u32) -> [u8; 4] {
    [
        (color & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 24) & 0xFF) as u8,
    ]
}

fn argb_to_rgba(color: u32) -> [u8; 4] {
    [
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
        ((color >> 24) & 0xFF) as u8,
    ]
}

fn apply_text_edge(src: &[u8], width: u32, height: u32, edge: [u8; 4]) -> (u32, u32, Vec<u8>) {
    let pad = 1u32;
    let out_w = width.saturating_add(pad * 2).max(1);
    let out_h = height.saturating_add(pad * 2).max(1);
    let mut dst = vec![0u8; out_w as usize * out_h as usize * 4];
    let stamp = |dst: &mut [u8], x: i32, y: i32, px: &[u8]| {
        if x < 0 || y < 0 || x >= out_w as i32 || y >= out_h as i32 || px.len() < 4 || px[3] == 0 {
            return;
        }
        let index = (y as usize * out_w as usize + x as usize) * 4;
        dst[index..index + 4].copy_from_slice(&px[..4]);
    };
    for y in 0..height {
        for x in 0..width {
            let src_index = (y as usize * width as usize + x as usize) * 4;
            if src.get(src_index + 3).copied().unwrap_or(0) == 0 {
                continue;
            }
            let ox = x as i32 + pad as i32;
            let oy = y as i32 + pad as i32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    stamp(&mut dst, ox + dx, oy + dy, &edge);
                }
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            let src_index = (y as usize * width as usize + x as usize) * 4;
            let Some(px) = src.get(src_index..src_index + 4) else {
                continue;
            };
            stamp(&mut dst, x as i32 + pad as i32, y as i32 + pad as i32, px);
        }
    }
    (out_w, out_h, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_glyphs_share_the_cell_baseline_without_moving_centered_marks() {
        assert_eq!(glyph_baseline_shift(7, 23, 24.0, 28.0), 1);
        assert_eq!(glyph_baseline_shift(7, 22, 24.0, 28.0), 2);
        assert_eq!(glyph_baseline_shift(12, 16, 24.0, 28.0), 0);
        assert_eq!(glyph_baseline_shift(18, 25, 24.0, 28.0), 0);
    }

    #[test]
    fn default_ttf_loads() {
        let font = PalFontFallback::default_ttf();
        let (w, h) = font.measure_line("A", 16.0);
        assert!(w > 0, "glyph width must be positive");
        assert!(h > 0, "glyph height must be positive");
    }

    #[test]
    fn rasterize_produces_correct_dimensions() {
        let font = PalFontFallback::default_ttf();
        let (w, h, pixels) = font.rasterize_line("Hi", 20.0, [255, 255, 255, 255]);
        assert_eq!(pixels.len(), w as usize * h as usize * 4);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn effect_adds_edge_pixels_around_the_glyph() {
        let mut font = PalFontSystem::new();
        font.set_font_size(28);
        font.set_color(0xFFFF_0000, 0xFF00_0000);
        font.set_effect(0);
        let (_, _, plain) = font.rasterize("A");
        font.set_effect(1);
        let (_, _, edged) = font.rasterize("A");
        let ink = |pixels: &[u8]| pixels.chunks_exact(4).filter(|px| px[3] > 0).count();
        assert!(ink(&edged) > ink(&plain));
        assert!(edged.chunks_exact(4).any(|px| px[0] == 255 && px[3] > 0));
    }

    #[test]
    fn rasterize_empty_string_returns_blank() {
        let font = PalFontFallback::default_ttf();
        let (w, h, pixels) = font.rasterize_line("", 16.0, [0, 0, 0, 255]);
        assert_eq!(pixels.len(), w as usize * h as usize * 4);
        assert!(h > 0);
    }
}
