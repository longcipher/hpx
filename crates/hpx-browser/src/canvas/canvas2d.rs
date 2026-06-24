//! CPU-based Canvas 2D rendering context backed by Skia via `skia-safe`.

#[allow(deprecated)]
use skia_safe::gradient_shader;
use skia_safe::{
    AlphaType, BlendMode, Canvas as SkCanvas, Color4f, ColorFilter, ColorType, Font, FontHinting,
    FontMgr, ImageInfo, Matrix, Paint, PaintStyle, Point, Rect as SkRect, TileMode, image_filters,
    surfaces,
};

use crate::canvas::{
    path::Path2D,
    text::{self, ParsedFont, TextMetrics},
};

/// Simple 0-255 RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn red(self) -> f32 {
        self.r as f32 / 255.0
    }
    pub fn green(self) -> f32 {
        self.g as f32 / 255.0
    }
    pub fn blue(self) -> f32 {
        self.b as f32 / 255.0
    }
    pub fn alpha(self) -> f32 {
        self.a as f32 / 255.0
    }

    fn to_color4f(self) -> Color4f {
        Color4f::new(self.red(), self.green(), self.blue(), self.alpha())
    }
}

/// Gradient definition with color stops.
#[derive(Debug, Clone)]
pub enum Gradient {
    Linear {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<(f32, Color)>,
    },
    Radial {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<(f32, Color)>,
    },
    Conic {
        cx: f32,
        cy: f32,
        start_angle: f32,
        stops: Vec<(f32, Color)>,
    },
}

impl Gradient {
    #[allow(deprecated)]
    fn to_shader(&self) -> Option<skia_safe::Shader> {
        match self {
            Gradient::Linear {
                x0,
                y0,
                x1,
                y1,
                stops,
            } => {
                if stops.len() < 2 {
                    return None;
                }
                let (colors, positions) = split_stops(stops);
                gradient_shader::linear(
                    (Point::new(*x0, *y0), Point::new(*x1, *y1)),
                    gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
                    Some(&positions[..]),
                    TileMode::Clamp,
                    None,
                    None,
                )
            }
            Gradient::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            } => {
                if stops.len() < 2 {
                    return None;
                }
                let (colors, positions) = split_stops(stops);
                gradient_shader::two_point_conical(
                    Point::new(*x0, *y0),
                    *r0,
                    Point::new(*x1, *y1),
                    *r1,
                    gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
                    Some(&positions[..]),
                    TileMode::Clamp,
                    None,
                    None,
                )
            }
            Gradient::Conic {
                cx,
                cy,
                start_angle,
                stops,
            } => {
                if stops.len() < 2 {
                    return None;
                }
                let (colors, positions) = split_stops(stops);
                let start_deg = start_angle.to_degrees();
                gradient_shader::sweep(
                    (*cx, *cy),
                    gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
                    Some(&positions[..]),
                    TileMode::Clamp,
                    (start_deg, start_deg + 360.0),
                    None,
                    None,
                )
            }
        }
    }
}

fn split_stops(stops: &[(f32, Color)]) -> (Vec<Color4f>, Vec<f32>) {
    let mut colors = Vec::with_capacity(stops.len());
    let mut positions = Vec::with_capacity(stops.len());
    for (offset, color) in stops {
        colors.push(color.to_color4f());
        positions.push(*offset);
    }
    (colors, positions)
}

#[derive(Clone)]
enum FillStyle {
    Solid(Color),
    Gradient(Gradient),
    Pattern(Pattern),
}

/// CSS `createPattern(image, repetition)`.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub repetition: PatternRepetition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternRepetition {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

impl PatternRepetition {
    pub fn parse(s: &str) -> Self {
        match s {
            "repeat" | "" => Self::Repeat,
            "repeat-x" => Self::RepeatX,
            "repeat-y" => Self::RepeatY,
            "no-repeat" => Self::NoRepeat,
            _ => Self::Repeat,
        }
    }

    fn tile_modes(self) -> (TileMode, TileMode) {
        match self {
            Self::Repeat => (TileMode::Repeat, TileMode::Repeat),
            Self::RepeatX => (TileMode::Repeat, TileMode::Decal),
            Self::RepeatY => (TileMode::Decal, TileMode::Repeat),
            Self::NoRepeat => (TileMode::Decal, TileMode::Decal),
        }
    }
}

impl Pattern {
    fn to_shader(&self) -> Option<skia_safe::Shader> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let info = ImageInfo::new(
            (self.width as i32, self.height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let row_bytes = self.width as usize * 4;
        let data = skia_safe::Data::new_copy(&self.rgba);
        let image = skia_safe::images::raster_from_data(&info, data, row_bytes)?;
        let (tx, ty) = self.repetition.tile_modes();
        image.to_shader(Some((tx, ty)), skia_safe::SamplingOptions::default(), None)
    }
}

#[derive(Clone)]
struct CanvasState {
    fill_style: FillStyle,
    stroke_style: FillStyle,
    line_width: f32,
    global_alpha: f32,
    transform: Matrix,
    font: ParsedFont,
    global_composite_operation: BlendMode,
    shadow_blur: f32,
    shadow_offset_x: f32,
    shadow_offset_y: f32,
    shadow_color: Color,
    filter_chain: Vec<FilterOp>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            fill_style: FillStyle::Solid(Color::BLACK),
            stroke_style: FillStyle::Solid(Color::BLACK),
            line_width: 1.0,
            global_alpha: 1.0,
            transform: Matrix::new_identity(),
            font: ParsedFont::default_font(),
            global_composite_operation: BlendMode::SrcOver,
            shadow_blur: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_color: Color::TRANSPARENT,
            filter_chain: Vec::new(),
        }
    }
}

/// Parsed `ctx.filter` operation.
#[derive(Debug, Clone, Copy)]
pub enum FilterOp {
    Blur(f32),
    Grayscale(f32),
    Sepia(f32),
    Invert(f32),
    Brightness(f32),
    Contrast(f32),
    Saturate(f32),
    Opacity(f32),
}

pub fn parse_composite_operation(s: &str) -> BlendMode {
    match s {
        "source-over" => BlendMode::SrcOver,
        "source-in" => BlendMode::SrcIn,
        "source-out" => BlendMode::SrcOut,
        "source-atop" => BlendMode::SrcATop,
        "destination-over" => BlendMode::DstOver,
        "destination-in" => BlendMode::DstIn,
        "destination-out" => BlendMode::DstOut,
        "destination-atop" => BlendMode::DstATop,
        "lighter" => BlendMode::Plus,
        "copy" => BlendMode::Src,
        "xor" => BlendMode::Xor,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => BlendMode::SrcOver,
    }
}

pub fn parse_filter_chain(input: &str) -> Vec<FilterOp> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "none" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut chars = trimmed.chars().peekable();
    while chars.peek().is_some() {
        while matches!(chars.peek(), Some(&c) if c.is_whitespace()) {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '(' {
                chars.next();
                break;
            }
            name.push(c);
            chars.next();
        }
        let mut arg = String::new();
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == ')' {
                break;
            }
            arg.push(c);
        }
        let op = match name.trim() {
            "blur" => FilterOp::Blur(parse_length_px(&arg).unwrap_or(0.0)),
            "grayscale" => FilterOp::Grayscale(parse_percent(&arg).unwrap_or(0.0)),
            "sepia" => FilterOp::Sepia(parse_percent(&arg).unwrap_or(0.0)),
            "invert" => FilterOp::Invert(parse_percent(&arg).unwrap_or(0.0)),
            "brightness" => FilterOp::Brightness(parse_percent(&arg).unwrap_or(1.0)),
            "contrast" => FilterOp::Contrast(parse_percent(&arg).unwrap_or(1.0)),
            "saturate" => FilterOp::Saturate(parse_percent(&arg).unwrap_or(1.0)),
            "opacity" => FilterOp::Opacity(parse_percent(&arg).unwrap_or(1.0)),
            _ => continue,
        };
        out.push(op);
    }
    out
}

fn parse_length_px(s: &str) -> Option<f32> {
    let t = s.trim();
    if let Some(v) = t.strip_suffix("px") {
        return v.trim().parse().ok();
    }
    if let Some(v) = t.strip_suffix("pt") {
        return v.trim().parse::<f32>().ok().map(|n| n * 96.0 / 72.0);
    }
    t.parse().ok()
}

fn parse_percent(s: &str) -> Option<f32> {
    let t = s.trim();
    if let Some(v) = t.strip_suffix('%') {
        return v.trim().parse::<f32>().ok().map(|n| n / 100.0);
    }
    t.parse().ok()
}

fn compose_filter_chain(
    chain: &[FilterOp],
    mut inner: Option<skia_safe::ImageFilter>,
) -> Option<skia_safe::ImageFilter> {
    for op in chain {
        let next = match op {
            FilterOp::Blur(radius) => {
                let sigma = radius / 2.0;
                if sigma > 0.0 {
                    image_filters::blur((sigma, sigma), TileMode::Decal, inner.take(), None)
                } else {
                    inner.take()
                }
            }
            FilterOp::Grayscale(amount) => {
                let cf = grayscale_filter(amount.clamp(0.0, 1.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Sepia(amount) => {
                let cf = sepia_filter(amount.clamp(0.0, 1.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Invert(amount) => {
                let cf = invert_filter(amount.clamp(0.0, 1.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Brightness(amount) => {
                let cf = brightness_filter(amount.max(0.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Contrast(amount) => {
                let cf = contrast_filter(amount.max(0.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Saturate(amount) => {
                let cf = saturate_filter(amount.max(0.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
            FilterOp::Opacity(amount) => {
                let cf = opacity_filter(amount.clamp(0.0, 1.0));
                image_filters::color_filter(cf, inner.take(), None)
            }
        };
        if let Some(filter) = next {
            inner = Some(filter);
        }
    }
    inner
}

fn matrix_color_filter(m: [f32; 20]) -> ColorFilter {
    skia_safe::color_filters::matrix_row_major(&m, None)
}

fn grayscale_filter(amount: f32) -> ColorFilter {
    let a = amount;
    let r = 0.2126 + 0.7874 * (1.0 - a);
    let rg = 0.7152 - 0.7152 * (1.0 - a);
    let rb = 0.0722 - 0.0722 * (1.0 - a);
    let gr = 0.2126 - 0.2126 * (1.0 - a);
    let g = 0.7152 + 0.2848 * (1.0 - a);
    let gb = 0.0722 - 0.0722 * (1.0 - a);
    let br = 0.2126 - 0.2126 * (1.0 - a);
    let bg = 0.7152 - 0.7152 * (1.0 - a);
    let b = 0.0722 + 0.9278 * (1.0 - a);
    matrix_color_filter([
        r, rg, rb, 0.0, 0.0, gr, g, gb, 0.0, 0.0, br, bg, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

fn sepia_filter(amount: f32) -> ColorFilter {
    let a = amount;
    let r = 0.393 + 0.607 * (1.0 - a);
    let rg = 0.769 - 0.769 * (1.0 - a);
    let rb = 0.189 - 0.189 * (1.0 - a);
    let gr = 0.349 - 0.349 * (1.0 - a);
    let g = 0.686 + 0.314 * (1.0 - a);
    let gb = 0.168 - 0.168 * (1.0 - a);
    let br = 0.272 - 0.272 * (1.0 - a);
    let bg = 0.534 - 0.534 * (1.0 - a);
    let b = 0.131 + 0.869 * (1.0 - a);
    matrix_color_filter([
        r, rg, rb, 0.0, 0.0, gr, g, gb, 0.0, 0.0, br, bg, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

fn invert_filter(amount: f32) -> ColorFilter {
    let a = amount;
    let diag = 1.0 - 2.0 * a;
    let off = a;
    matrix_color_filter([
        diag, 0.0, 0.0, 0.0, off, 0.0, diag, 0.0, 0.0, off, 0.0, 0.0, diag, 0.0, off, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ])
}

fn brightness_filter(amount: f32) -> ColorFilter {
    matrix_color_filter([
        amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
    ])
}

fn contrast_filter(amount: f32) -> ColorFilter {
    let a = amount;
    let t = 0.5 * (1.0 - a);
    matrix_color_filter([
        a, 0.0, 0.0, 0.0, t, 0.0, a, 0.0, 0.0, t, 0.0, 0.0, a, 0.0, t, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

fn saturate_filter(amount: f32) -> ColorFilter {
    let a = amount;
    let r = 0.213 + 0.787 * a;
    let rg = 0.715 - 0.715 * a;
    let rb = 0.072 - 0.072 * a;
    let gr = 0.213 - 0.213 * a;
    let g = 0.715 + 0.285 * a;
    let gb = 0.072 - 0.072 * a;
    let br = 0.213 - 0.213 * a;
    let bg = 0.715 - 0.715 * a;
    let b = 0.072 + 0.928 * a;
    matrix_color_filter([
        r, rg, rb, 0.0, 0.0, gr, g, gb, 0.0, 0.0, br, bg, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

fn opacity_filter(amount: f32) -> ColorFilter {
    matrix_color_filter([
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        amount, 0.0,
    ])
}

/// CPU-based Canvas 2D rendering context backed by Skia.
pub struct Canvas2D {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    os_name: String,
    seed: u64,
    state: CanvasState,
    state_stack: Vec<CanvasState>,
    path: Path2D,
}

impl Canvas2D {
    pub fn new(width: u32, height: u32, os_name: String, seed: u64) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        Some(Self {
            pixels,
            width,
            height,
            os_name,
            seed,
            state: CanvasState::default(),
            state_stack: Vec::new(),
            path: Path2D::new(),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    // --- State ---

    pub fn save(&mut self) {
        self.state_stack.push(self.state.clone());
    }

    pub fn restore(&mut self) {
        if let Some(state) = self.state_stack.pop() {
            self.state = state;
        }
    }

    // --- Style ---

    pub fn set_fill_color(&mut self, r: u8, g: u8, b: u8, a: f32) {
        self.state.fill_style = FillStyle::Solid(Color::from_rgba8(
            r,
            g,
            b,
            (a.clamp(0.0, 1.0) * 255.0) as u8,
        ));
    }

    pub fn set_fill_color_str(&mut self, color_str: &str) {
        if let Some(c) = parse_css_color(color_str) {
            self.state.fill_style = FillStyle::Solid(c);
        }
    }

    pub fn set_stroke_color(&mut self, r: u8, g: u8, b: u8, a: f32) {
        self.state.stroke_style = FillStyle::Solid(Color::from_rgba8(
            r,
            g,
            b,
            (a.clamp(0.0, 1.0) * 255.0) as u8,
        ));
    }

    pub fn set_stroke_color_str(&mut self, color_str: &str) {
        if let Some(c) = parse_css_color(color_str) {
            self.state.stroke_style = FillStyle::Solid(c);
        }
    }

    pub fn set_fill_gradient(&mut self, gradient: Gradient) {
        self.state.fill_style = FillStyle::Gradient(gradient);
    }

    pub fn set_stroke_gradient(&mut self, gradient: Gradient) {
        self.state.stroke_style = FillStyle::Gradient(gradient);
    }

    pub fn set_fill_pattern(&mut self, pattern: Pattern) {
        self.state.fill_style = FillStyle::Pattern(pattern);
    }

    pub fn set_stroke_pattern(&mut self, pattern: Pattern) {
        self.state.stroke_style = FillStyle::Pattern(pattern);
    }

    pub fn set_line_width(&mut self, width: f32) {
        self.state.line_width = width;
    }

    pub fn set_global_alpha(&mut self, alpha: f32) {
        self.state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    pub fn set_global_composite_operation(&mut self, op: &str) {
        self.state.global_composite_operation = parse_composite_operation(op);
    }

    // --- Shadow ---

    pub fn set_shadow_blur(&mut self, blur: f32) {
        self.state.shadow_blur = blur.max(0.0);
    }

    pub fn set_shadow_offset_x(&mut self, x: f32) {
        self.state.shadow_offset_x = x;
    }

    pub fn set_shadow_offset_y(&mut self, y: f32) {
        self.state.shadow_offset_y = y;
    }

    pub fn set_shadow_color(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.state.shadow_color = Color::from_rgba8(r, g, b, a);
    }

    pub fn set_shadow_color_str(&mut self, color_str: &str) {
        if let Some(c) = parse_css_color(color_str) {
            self.state.shadow_color = c;
        }
    }

    // --- Filter ---

    pub fn set_filter(&mut self, filter_str: &str) {
        self.state.filter_chain = parse_filter_chain(filter_str);
    }

    pub fn set_font(&mut self, font_str: &str) {
        if let Some(parsed) = ParsedFont::parse(font_str) {
            self.state.font = parsed;
        }
    }

    // --- Transform ---

    pub fn translate(&mut self, x: f32, y: f32) {
        self.state.transform.pre_translate((x, y));
    }

    pub fn rotate(&mut self, angle: f32) {
        self.state.transform.pre_rotate(angle.to_degrees(), None);
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.state.transform.pre_scale((sx, sy), None);
    }

    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        let mut m = Matrix::new_identity();
        m.set_9(&[a, c, e, b, d, f, 0.0, 0.0, 1.0]);
        self.state.transform = m;
    }

    pub fn reset_transform(&mut self) {
        self.state.transform = Matrix::new_identity();
    }

    // --- Rectangle ops ---

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = SkRect::from_xywh(x, y, w, h);
        let paint = self.build_fill_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_rect(rect, &paint);
            canvas.restore();
        });
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = SkRect::from_xywh(x, y, w, h);
        let paint = self.build_stroke_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_rect(rect, &paint);
            canvas.restore();
        });
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = SkRect::from_xywh(x, y, w, h);
        let mut paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 0.0), None);
        paint.set_blend_mode(BlendMode::Clear);
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_rect(rect, &paint);
            canvas.restore();
        });
    }

    // --- Path ops ---

    pub fn begin_path(&mut self) {
        self.path.clear();
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x, y);
    }
    pub fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x, y);
    }
    pub fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        self.path.bezier_curve_to(cp1x, cp1y, cp2x, cp2y, x, y);
    }
    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        self.path.quadratic_curve_to(cpx, cpy, x, y);
    }
    pub fn arc(&mut self, x: f32, y: f32, r: f32, start: f32, end: f32, ccw: bool) {
        self.path.arc(x, y, r, start, end, ccw);
    }
    pub fn arc_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) {
        self.path.arc_to(x1, y1, x2, y2, radius);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        rotation: f32,
        start: f32,
        end: f32,
        ccw: bool,
    ) {
        self.path.ellipse(cx, cy, rx, ry, rotation, start, end, ccw);
    }

    pub fn close_path(&mut self) {
        self.path.close_path();
    }

    pub fn fill(&mut self) {
        let Some(sk_path) = self.path.to_skia_path() else {
            return;
        };
        let paint = self.build_fill_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_path(&sk_path, &paint);
            canvas.restore();
        });
    }

    pub fn stroke(&mut self) {
        let Some(sk_path) = self.path.to_skia_path() else {
            return;
        };
        let paint = self.build_stroke_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_path(&sk_path, &paint);
            canvas.restore();
        });
    }

    // --- Pixel ops ---

    pub fn get_image_data(&self, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in y..(y + h).min(self.height) {
            for col in x..(x + w).min(self.width) {
                let offset = ((row * self.width + col) * 4) as usize;
                if offset + 3 >= self.pixels.len() {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                let r = self.pixels[offset];
                let g = self.pixels[offset + 1];
                let b = self.pixels[offset + 2];
                let a = self.pixels[offset + 3];
                if a > 0 {
                    out.push(((r as u16 * 255 + a as u16 / 2) / a as u16) as u8);
                    out.push(((g as u16 * 255 + a as u16 / 2) / a as u16) as u8);
                    out.push(((b as u16 * 255 + a as u16 / 2) / a as u16) as u8);
                    out.push(a);
                } else {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        out
    }

    pub fn put_image_data(&mut self, data: &[u8], x: u32, y: u32, w: u32, h: u32) {
        let stride = self.width as usize * 4;
        for row in 0..h.min(self.height.saturating_sub(y)) {
            for col in 0..w.min(self.width.saturating_sub(x)) {
                let src_offset = ((row * w + col) * 4) as usize;
                let dst_offset = ((y + row) as usize * stride) + ((x + col) as usize * 4);
                if src_offset + 3 >= data.len() || dst_offset + 3 >= self.pixels.len() {
                    continue;
                }
                let r = data[src_offset];
                let g = data[src_offset + 1];
                let b = data[src_offset + 2];
                let a = data[src_offset + 3];
                self.pixels[dst_offset] = ((r as u16 * a as u16) / 255) as u8;
                self.pixels[dst_offset + 1] = ((g as u16 * a as u16) / 255) as u8;
                self.pixels[dst_offset + 2] = ((b as u16 * a as u16) / 255) as u8;
                self.pixels[dst_offset + 3] = a;
            }
        }
    }

    // --- Text ---

    pub fn measure_text(&self, text: &str) -> f64 {
        text::measure_text_width(text, &self.state.font, &self.os_name)
    }

    pub fn measure_text_metrics(&self, text: &str) -> TextMetrics {
        text::measure_text_metrics(text, &self.state.font, &self.os_name)
    }

    pub fn fill_text(&mut self, text: &str, x: f32, y: f32) {
        let Some((data, idx, run)) = text::shape_run(text, &self.state.font, &self.os_name) else {
            return;
        };
        let size_px = self.state.font.size_px;
        let Some(typeface) = FontMgr::new().new_from_data(data, Some(idx as usize)) else {
            // Fallback: legacy swash raster.
            let color = self.state.fill_style.color();
            let glyphs = text::rasterize_text(
                text,
                x,
                y,
                &self.state.font,
                color.r,
                color.g,
                color.b,
                self.state.global_alpha * color.alpha(),
                &self.os_name,
            );
            let w = self.width;
            let h = self.height;
            for glyph in &glyphs {
                text::composite_glyph(glyph, &mut self.pixels, w, h);
            }
            return;
        };
        let mut font = Font::from_typeface(typeface, Some(size_px));
        font.set_edging(skia_safe::font::Edging::AntiAlias);
        font.set_subpixel(true);
        font.set_hinting(FontHinting::None);

        let mut ids: Vec<u16> = Vec::with_capacity(run.glyphs.len());
        let mut pos: Vec<Point> = Vec::with_capacity(run.glyphs.len());
        let mut cx = x;
        for g in &run.glyphs {
            ids.push(g.glyph_id as u16);
            pos.push(Point::new(cx + g.x_offset, y - g.y_offset));
            cx += g.x_advance;
        }

        let paint = self.build_fill_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_glyphs_at(&ids, &pos[..], Point::new(0.0, 0.0), &font, &paint);
            canvas.restore();
        });
    }

    pub fn stroke_text(&mut self, text: &str, x: f32, y: f32) {
        let mut path = Path2D::new();
        let any = text::append_text_outline_to_path(
            &mut path,
            text,
            x,
            y,
            &self.state.font,
            &self.os_name,
        );
        if !any {
            return;
        }
        let Some(sk_path) = path.to_skia_path() else {
            return;
        };
        let paint = self.build_stroke_paint();
        let matrix = self.state.transform;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.concat(&matrix);
            canvas.draw_path(&sk_path, &paint);
            canvas.restore();
        });
    }

    // --- Image ---

    pub fn draw_image_pixels(&mut self, rgba: &[u8], src_w: u32, src_h: u32, dx: f32, dy: f32) {
        self.put_image_data(rgba, dx as u32, dy as u32, src_w, src_h);
    }

    pub fn decode_image(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
        let img = image::load_from_memory(bytes).ok()?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Some((rgba.into_raw(), w, h))
    }

    // --- Encoding ---

    pub fn to_data_url(&self) -> String {
        let png_bytes = self.to_png_bytes();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
        format!("data:image/png;base64,{b64}")
    }

    pub fn to_png_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("PNG header write failed");
            let unpremultiplied = self.get_image_data(0, 0, self.width, self.height);
            writer
                .write_image_data(&unpremultiplied)
                .expect("PNG data write failed");
        }
        buf
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, crate::canvas::CanvasError> {
        Ok(self.to_png_bytes())
    }

    pub fn has_content(&self) -> bool {
        self.pixels.chunks(4).any(|px| px[3] > 0)
    }

    // --- Internal ---

    fn with_canvas<F>(&mut self, f: F)
    where
        F: FnOnce(&SkCanvas),
    {
        let info = ImageInfo::new(
            (self.width as i32, self.height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let row_bytes = self.width as usize * 4;
        let Some(mut surface) =
            surfaces::wrap_pixels(&info, &mut self.pixels, Some(row_bytes), None)
        else {
            return;
        };
        f(surface.canvas());
    }

    fn build_fill_paint(&self) -> Paint {
        self.build_paint(&self.state.fill_style, PaintStyle::Fill, 0.0)
    }

    fn build_stroke_paint(&self) -> Paint {
        self.build_paint(
            &self.state.stroke_style,
            PaintStyle::Stroke,
            self.state.line_width,
        )
    }

    fn build_paint(&self, style: &FillStyle, paint_style: PaintStyle, stroke_width: f32) -> Paint {
        let mut paint = match style {
            FillStyle::Solid(color) => Paint::new(color.to_color4f(), None),
            FillStyle::Gradient(_) | FillStyle::Pattern(_) => {
                Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None)
            }
        };
        paint.set_anti_alias(true);
        paint.set_style(paint_style);
        if paint_style == PaintStyle::Stroke {
            paint.set_stroke_width(stroke_width);
        }
        match style {
            FillStyle::Gradient(grad) => {
                if let Some(shader) = grad.to_shader() {
                    paint.set_shader(shader);
                }
            }
            FillStyle::Pattern(pat) => {
                if let Some(shader) = pat.to_shader() {
                    paint.set_shader(shader);
                }
            }
            FillStyle::Solid(_) => {}
        }
        let base_alpha = paint.alpha_f();
        paint.set_alpha_f(base_alpha * self.state.global_alpha);
        paint.set_blend_mode(self.state.global_composite_operation);

        // Shadow
        let shadow_active = (self.state.shadow_blur > 0.0
            || self.state.shadow_offset_x != 0.0
            || self.state.shadow_offset_y != 0.0)
            && self.state.shadow_color.a > 0;

        let mut img_filter = None;
        if shadow_active {
            let sigma = self.state.shadow_blur / 2.0;
            let shadow_col = skia_safe::Color::from_argb(
                self.state.shadow_color.a,
                self.state.shadow_color.r,
                self.state.shadow_color.g,
                self.state.shadow_color.b,
            );
            img_filter = image_filters::drop_shadow(
                (self.state.shadow_offset_x, self.state.shadow_offset_y),
                (sigma, sigma),
                shadow_col,
                None,
                None,
                None,
            );
        }

        if !self.state.filter_chain.is_empty() {
            img_filter = compose_filter_chain(&self.state.filter_chain, img_filter);
        }

        if let Some(f) = img_filter {
            paint.set_image_filter(f);
        }

        paint
    }
}

impl FillStyle {
    fn color(&self) -> Color {
        match self {
            FillStyle::Solid(c) => *c,
            FillStyle::Gradient(_) | FillStyle::Pattern(_) => Color::BLACK,
        }
    }
}

fn parse_css_color(s: &str) -> Option<Color> {
    let s = s.trim();
    match s {
        "black" => Some(Color::from_rgba8(0, 0, 0, 255)),
        "white" => Some(Color::from_rgba8(255, 255, 255, 255)),
        "red" => Some(Color::from_rgba8(255, 0, 0, 255)),
        "green" => Some(Color::from_rgba8(0, 128, 0, 255)),
        "blue" => Some(Color::from_rgba8(0, 0, 255, 255)),
        "yellow" => Some(Color::from_rgba8(255, 255, 0, 255)),
        "cyan" => Some(Color::from_rgba8(0, 255, 255, 255)),
        "magenta" => Some(Color::from_rgba8(255, 0, 255, 255)),
        "transparent" => Some(Color::from_rgba8(0, 0, 0, 0)),
        s if s.starts_with('#') => parse_hex_color(s),
        s if s.starts_with("rgb") => parse_rgb_color(s),
        _ => None,
    }
}

fn parse_hex_color(s: &str) -> Option<Color> {
    let hex = &s[1..];
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some(Color::from_rgba8(r, g, b, 255))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::from_rgba8(r, g, b, 255))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color::from_rgba8(r, g, b, a))
        }
        _ => None,
    }
}

fn parse_rgb_color(s: &str) -> Option<Color> {
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() >= 3 {
        let r: u8 = parts[0].trim().parse().ok()?;
        let g: u8 = parts[1].trim().parse().ok()?;
        let b: u8 = parts[2].trim().parse().ok()?;
        let a = if parts.len() >= 4 {
            (parts[3].trim().parse::<f32>().ok()? * 255.0) as u8
        } else {
            255
        };
        Some(Color::from_rgba8(r, g, b, a))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_canvas() {
        let c = Canvas2D::new(200, 100, "Linux".to_string(), 0).expect("canvas");
        assert_eq!(c.width(), 200);
        assert_eq!(c.height(), 100);
        assert!(!c.has_content());
    }

    #[test]
    fn fill_rect_produces_pixels() {
        let mut c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(255, 0, 0, 1.0);
        c.fill_rect(10.0, 10.0, 50.0, 50.0);
        assert!(c.has_content());
    }

    #[test]
    fn clear_rect_clears() {
        let mut c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(255, 0, 0, 1.0);
        c.fill_rect(0.0, 0.0, 100.0, 100.0);
        assert!(c.has_content());
        c.clear_rect(0.0, 0.0, 100.0, 100.0);
        assert!(!c.has_content());
    }

    #[test]
    fn path_fill() {
        let mut c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(0, 0, 255, 1.0);
        c.begin_path();
        c.move_to(10.0, 10.0);
        c.line_to(90.0, 10.0);
        c.line_to(50.0, 90.0);
        c.close_path();
        c.fill();
        assert!(c.has_content());
    }

    #[test]
    fn stroke_rect() {
        let mut c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        c.set_stroke_color(0, 255, 0, 1.0);
        c.set_line_width(2.0);
        c.stroke_rect(10.0, 10.0, 80.0, 80.0);
        assert!(c.has_content());
    }

    #[test]
    fn get_image_data_red() {
        let mut c = Canvas2D::new(10, 10, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(255, 0, 0, 1.0);
        c.fill_rect(0.0, 0.0, 10.0, 10.0);
        let data = c.get_image_data(0, 0, 10, 10);
        assert_eq!(data.len(), 10 * 10 * 4);
        assert_eq!(data[0], 255);
        assert_eq!(data[1], 0);
        assert_eq!(data[2], 0);
        assert_eq!(data[3], 255);
    }

    #[test]
    fn to_png_bytes_magic() {
        let mut c = Canvas2D::new(10, 10, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(0, 0, 255, 1.0);
        c.fill_rect(0.0, 0.0, 10.0, 10.0);
        let bytes = c.to_png_bytes();
        assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4e, 0x47]);
    }

    #[test]
    fn to_data_url() {
        let mut c = Canvas2D::new(10, 10, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(255, 0, 0, 1.0);
        c.fill_rect(0.0, 0.0, 10.0, 10.0);
        let url = c.to_data_url();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn save_restore() {
        let mut c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(255, 0, 0, 1.0);
        c.save();
        c.set_fill_color(0, 255, 0, 1.0);
        c.fill_rect(0.0, 0.0, 50.0, 50.0);
        c.restore();
        c.fill_rect(50.0, 0.0, 50.0, 50.0);
        assert!(c.has_content());
    }

    #[test]
    fn measure_text_nonzero() {
        let c = Canvas2D::new(100, 100, "Linux".to_string(), 0).expect("canvas");
        let width = c.measure_text("Hello");
        assert!(width > 0.0);
    }

    #[test]
    fn fill_text_produces_content() {
        let mut c = Canvas2D::new(200, 50, "Linux".to_string(), 0).expect("canvas");
        c.set_fill_color(0, 0, 0, 1.0);
        c.set_font("16px sans-serif");
        c.fill_text("Hello World", 10.0, 30.0);
        assert!(c.has_content());
    }

    #[test]
    fn zero_canvas_returns_none() {
        assert!(Canvas2D::new(0, 100, "Linux".to_string(), 0).is_none());
    }

    #[test]
    fn parse_hex_color_test() {
        assert_eq!(
            parse_css_color("#ff0000"),
            Some(Color::from_rgba8(255, 0, 0, 255))
        );
        assert_eq!(
            parse_css_color("black"),
            Some(Color::from_rgba8(0, 0, 0, 255))
        );
    }
}
