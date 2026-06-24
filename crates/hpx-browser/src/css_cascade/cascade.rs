use crate::{
    css_cascade::layers::LayerId,
    css_selectors::Specificity,
    css_values::property::{CssValue, PropertyDeclaration, PropertyId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent = 0,
    User = 1,
    Author = 2,
}

#[derive(Debug, Clone)]
pub struct CascadeEntry {
    pub declaration: PropertyDeclaration,
    pub origin: Origin,
    pub layer: Option<LayerId>,
    pub specificity: Specificity,
    pub source_order: u32,
}

pub fn cascade_sort(
    entries: &mut [CascadeEntry],
) -> std::collections::HashMap<PropertyId, CssValue> {
    entries.sort_by(cascade_compare);

    let mut result = std::collections::HashMap::new();
    for entry in entries.iter() {
        result.insert(
            entry.declaration.property.clone(),
            entry.declaration.value.clone(),
        );
    }
    result
}

fn cascade_compare(a: &CascadeEntry, b: &CascadeEntry) -> std::cmp::Ordering {
    let a_important = a.declaration.important;
    let b_important = b.declaration.important;

    let a_priority = origin_priority(a.origin, a_important);
    let b_priority = origin_priority(b.origin, b_important);
    let ord = a_priority.cmp(&b_priority);
    if ord != std::cmp::Ordering::Equal {
        return ord;
    }

    match (&a.layer, &b.layer) {
        (None, Some(_)) if !a_important => return std::cmp::Ordering::Greater,
        (Some(_), None) if !a_important => return std::cmp::Ordering::Less,
        (None, Some(_)) if a_important => return std::cmp::Ordering::Less,
        (Some(_), None) if a_important => return std::cmp::Ordering::Greater,
        (Some(la), Some(lb)) => {
            let layer_ord = la.cmp(lb);
            if layer_ord != std::cmp::Ordering::Equal {
                return if a_important {
                    layer_ord.reverse()
                } else {
                    layer_ord
                };
            }
        }
        _ => {}
    }

    let spec_ord = a.specificity.cmp(&b.specificity);
    if spec_ord != std::cmp::Ordering::Equal {
        return spec_ord;
    }

    a.source_order.cmp(&b.source_order)
}

fn origin_priority(origin: Origin, important: bool) -> u8 {
    if important {
        match origin {
            Origin::Author => 4,
            Origin::User => 5,
            Origin::UserAgent => 6,
        }
    } else {
        match origin {
            Origin::UserAgent => 1,
            Origin::User => 2,
            Origin::Author => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_values::types::color::Color;

    fn entry(
        origin: Origin,
        specificity: (u32, u32, u32),
        order: u32,
        important: bool,
        value: CssValue,
    ) -> CascadeEntry {
        CascadeEntry {
            declaration: PropertyDeclaration {
                property: PropertyId::Color,
                value,
                important,
            },
            origin,
            layer: None,
            specificity: Specificity::new(specificity.0, specificity.1, specificity.2),
            source_order: order,
        }
    }

    #[test]
    fn higher_specificity_wins() {
        let mut entries = vec![
            entry(
                Origin::Author,
                (0, 1, 0),
                0,
                false,
                CssValue::Color(Color::Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 1.0,
                }),
            ),
            entry(
                Origin::Author,
                (1, 0, 0),
                1,
                false,
                CssValue::Color(Color::Rgba {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 1.0,
                }),
            ),
        ];
        let result = cascade_sort(&mut entries);
        assert!(matches!(
            result.get(&PropertyId::Color),
            Some(CssValue::Color(Color::Rgba {
                r: 0,
                g: 0,
                b: 255,
                ..
            }))
        ));
    }

    #[test]
    fn important_beats_normal() {
        let mut entries = vec![
            entry(
                Origin::Author,
                (1, 0, 0),
                1,
                false,
                CssValue::Color(Color::Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 1.0,
                }),
            ),
            entry(
                Origin::Author,
                (0, 0, 1),
                0,
                true,
                CssValue::Color(Color::Rgba {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 1.0,
                }),
            ),
        ];
        let result = cascade_sort(&mut entries);
        assert!(matches!(
            result.get(&PropertyId::Color),
            Some(CssValue::Color(Color::Rgba {
                r: 0,
                g: 0,
                b: 255,
                ..
            }))
        ));
    }
}
