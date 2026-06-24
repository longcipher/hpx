//! Global font database backed by `fontdb`.

use std::sync::OnceLock;

use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};

pub struct FontDatabase {
    inner: Database,
}

const LIBERATION_SANS_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const LIBERATION_SANS_BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");
const LIBERATION_SANS_ITALIC: &[u8] = include_bytes!("../fonts/LiberationSans-Italic.ttf");
const LIBERATION_SANS_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/LiberationSans-BoldItalic.ttf");
const LIBERATION_SERIF_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSerif-Regular.ttf");
const LIBERATION_SERIF_BOLD: &[u8] = include_bytes!("../fonts/LiberationSerif-Bold.ttf");
const LIBERATION_SERIF_ITALIC: &[u8] = include_bytes!("../fonts/LiberationSerif-Italic.ttf");
const LIBERATION_SERIF_BOLD_ITALIC: &[u8] =
    include_bytes!("../fonts/LiberationSerif-BoldItalic.ttf");
const LIBERATION_MONO_REGULAR: &[u8] = include_bytes!("../fonts/LiberationMono-Regular.ttf");
const LIBERATION_MONO_BOLD: &[u8] = include_bytes!("../fonts/LiberationMono-Bold.ttf");
const LIBERATION_MONO_ITALIC: &[u8] = include_bytes!("../fonts/LiberationMono-Italic.ttf");
const LIBERATION_MONO_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/LiberationMono-BoldItalic.ttf");
const DEJAVU_SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");

pub const BUNDLED_FACE_COUNT: usize = 14;

impl FontDatabase {
    pub fn get() -> &'static FontDatabase {
        static INSTANCE: OnceLock<FontDatabase> = OnceLock::new();
        INSTANCE.get_or_init(Self::init_bundled)
    }

    fn init_bundled() -> FontDatabase {
        let mut db = Database::new();
        db.load_font_data(LIBERATION_SANS_REGULAR.to_vec());
        db.load_font_data(LIBERATION_SANS_BOLD.to_vec());
        db.load_font_data(LIBERATION_SANS_ITALIC.to_vec());
        db.load_font_data(LIBERATION_SANS_BOLD_ITALIC.to_vec());
        db.load_font_data(LIBERATION_SERIF_REGULAR.to_vec());
        db.load_font_data(LIBERATION_SERIF_BOLD.to_vec());
        db.load_font_data(LIBERATION_SERIF_ITALIC.to_vec());
        db.load_font_data(LIBERATION_SERIF_BOLD_ITALIC.to_vec());
        db.load_font_data(LIBERATION_MONO_REGULAR.to_vec());
        db.load_font_data(LIBERATION_MONO_BOLD.to_vec());
        db.load_font_data(LIBERATION_MONO_ITALIC.to_vec());
        db.load_font_data(LIBERATION_MONO_BOLD_ITALIC.to_vec());
        db.load_font_data(DEJAVU_SANS.to_vec());
        db.load_font_data(NOTO_SANS_REGULAR.to_vec());

        db.set_sans_serif_family("Liberation Sans");
        db.set_serif_family("Liberation Serif");
        db.set_monospace_family("Liberation Mono");
        db.set_cursive_family("Liberation Sans");
        db.set_fantasy_family("Liberation Sans");

        FontDatabase { inner: db }
    }

    pub fn query(&self, family: &str, weight: u16, italic: bool, os_name: &str) -> Option<ID> {
        let style = if italic { Style::Italic } else { Style::Normal };
        let families = resolve_family(family, os_name);
        let query = Query {
            families: &families,
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style,
        };
        if let Some(id) = self.inner.query(&query) {
            return Some(id);
        }
        let fallback = [Family::SansSerif];
        self.inner.query(&Query {
            families: &fallback,
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style,
        })
    }

    fn query_strict(&self, family: &str, weight: u16, italic: bool, os_name: &str) -> Option<ID> {
        let style = if italic { Style::Italic } else { Style::Normal };
        let families = resolve_family(family, os_name);
        self.inner.query(&Query {
            families: &families,
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style,
        })
    }

    pub fn query_chain(
        &self,
        families: &[String],
        weight: u16,
        italic: bool,
        os_name: &str,
    ) -> Option<ID> {
        for fam in families {
            if let Some(id) = self.query_strict(fam, weight, italic, os_name) {
                return Some(id);
            }
        }
        let style = if italic { Style::Italic } else { Style::Normal };
        let fallback = [Family::SansSerif];
        self.inner.query(&Query {
            families: &fallback,
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style,
        })
    }

    pub fn face_data(&self, id: ID) -> Option<(&[u8], u32)> {
        let face = self.inner.face(id)?;
        let fontdb::Source::Binary(data) = &face.source else {
            return None;
        };
        let slice: &[u8] = (**data).as_ref();
        Some((slice, face.index))
    }
}

fn resolve_family<'a>(name: &'a str, os_name: &str) -> Vec<Family<'a>> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "sans-serif" => vec![Family::SansSerif],
        "serif" => vec![Family::Serif],
        "monospace" => vec![Family::Monospace],
        "cursive" => vec![Family::Cursive],
        "fantasy" => vec![Family::Fantasy],
        "arial" | "helvetica" | "helvetica neue" | "tahoma" | "verdana" | "segoe ui"
        | "calibri" => {
            if os_name == "Windows" || os_name == "macOS" || os_name == "Linux" {
                vec![Family::Name("Liberation Sans"), Family::SansSerif]
            } else {
                vec![Family::Name(name)]
            }
        }
        "times" | "times new roman" | "georgia" => {
            if os_name == "Windows" || os_name == "macOS" || os_name == "Linux" {
                vec![Family::Name("Liberation Serif"), Family::Serif]
            } else {
                vec![Family::Name(name)]
            }
        }
        "courier" | "courier new" | "consolas" | "menlo" | "monaco" => {
            if os_name == "Windows" || os_name == "macOS" || os_name == "Linux" {
                vec![Family::Name("Liberation Mono"), Family::Monospace]
            } else {
                vec![Family::Name(name)]
            }
        }
        _ => vec![Family::Name(name)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_sans_serif_generic() {
        let db = FontDatabase::get();
        assert!(db.query("sans-serif", 400, false, "Linux").is_some());
    }

    #[test]
    fn resolves_arial_alias() {
        let db = FontDatabase::get();
        let arial_id = db
            .query("Arial", 400, false, "Linux")
            .expect("Arial should resolve");
        let libsans_id = db
            .query("Liberation Sans", 400, false, "Linux")
            .expect("Liberation Sans should resolve");
        assert_eq!(arial_id, libsans_id);
    }

    #[test]
    fn fallback_when_family_unknown() {
        let db = FontDatabase::get();
        assert!(
            db.query("NonexistentFamily42", 400, false, "Linux")
                .is_some()
        );
    }

    #[test]
    fn face_data_returns_bytes() {
        let db = FontDatabase::get();
        let id = db
            .query("Arial", 400, false, "Linux")
            .expect("should resolve");
        let (bytes, _idx) = db.face_data(id).expect("should have face data");
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn query_chain_walks_fallback() {
        let db = FontDatabase::get();
        let chain = vec![
            "NonexistentA".to_string(),
            "NonexistentB".to_string(),
            "Arial".to_string(),
        ];
        let id = db
            .query_chain(&chain, 400, false, "Linux")
            .expect("should resolve");
        let arial_id = db
            .query("Arial", 400, false, "Linux")
            .expect("should resolve");
        assert_eq!(id, arial_id);
    }
}
