#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Rgba { r: u8, g: u8, b: u8, a: f32 },
    Hsl { h: f64, s: f64, l: f64, a: f32 },
    CurrentColor,
    Transparent,
}

impl Color {
    pub fn to_rgba(&self) -> (u8, u8, u8, f32) {
        match self {
            Color::Rgba { r, g, b, a } => (*r, *g, *b, *a),
            Color::Transparent => (0, 0, 0, 0.0),
            Color::CurrentColor => (0, 0, 0, 1.0),
            _ => (0, 0, 0, 1.0),
        }
    }
}

pub fn named_color(name: &str) -> Option<Color> {
    let rgba = |r, g, b| Some(Color::Rgba { r, g, b, a: 1.0 });

    match name.to_ascii_lowercase().as_str() {
        "transparent" => Some(Color::Transparent),
        "currentcolor" => Some(Color::CurrentColor),
        "black" => rgba(0, 0, 0),
        "white" => rgba(255, 255, 255),
        "red" => rgba(255, 0, 0),
        "green" => rgba(0, 128, 0),
        "blue" => rgba(0, 0, 255),
        "yellow" => rgba(255, 255, 0),
        "cyan" | "aqua" => rgba(0, 255, 255),
        "magenta" | "fuchsia" => rgba(255, 0, 255),
        "orange" => rgba(255, 165, 0),
        "gray" | "grey" => rgba(128, 128, 128),
        "silver" => rgba(192, 192, 192),
        "maroon" => rgba(128, 0, 0),
        "olive" => rgba(128, 128, 0),
        "lime" => rgba(0, 255, 0),
        "purple" => rgba(128, 0, 128),
        "teal" => rgba(0, 128, 128),
        "navy" => rgba(0, 0, 128),
        "pink" => rgba(255, 192, 203),
        "brown" => rgba(165, 42, 42),
        "coral" => rgba(255, 127, 80),
        "gold" => rgba(255, 215, 0),
        "indigo" => rgba(75, 0, 130),
        "khaki" => rgba(240, 230, 140),
        "lavender" => rgba(230, 230, 250),
        "salmon" => rgba(250, 128, 114),
        "turquoise" => rgba(64, 224, 208),
        "violet" => rgba(238, 130, 238),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_colors_work() {
        assert!(
            matches!(named_color("red"), Some(Color::Rgba { r: 255, g: 0, b: 0, a }) if a == 1.0)
        );
        assert!(matches!(
            named_color("transparent"),
            Some(Color::Transparent)
        ));
        assert!(named_color("notacolor").is_none());
    }
}
