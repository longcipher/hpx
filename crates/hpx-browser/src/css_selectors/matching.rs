use crate::css_selectors::{ast::*, element::Element, parser::parse_selector_list};

/// Check if an element matches a selector.
pub fn matches_selector<E: Element>(element: &E, selector: &Selector) -> bool {
    let components = selector.components();
    match_components(element, components, 0)
}

/// Query the first matching element (depth-first pre-order).
pub fn query_selector<E: Element>(
    root: &E,
    selector_str: &str,
) -> Result<Option<E>, crate::css_selectors::error::SelectorParseError> {
    let selectors = parse_selector_list(selector_str)?;
    Ok(find_first(root, &selectors))
}

/// Query all matching elements (depth-first pre-order).
pub fn query_selector_all<E: Element>(
    root: &E,
    selector_str: &str,
) -> Result<Vec<E>, crate::css_selectors::error::SelectorParseError> {
    let selectors = parse_selector_list(selector_str)?;
    let mut results = Vec::new();
    find_all(root, &selectors, &mut results);
    Ok(results)
}

fn find_first<E: Element>(root: &E, selectors: &SelectorList) -> Option<E> {
    for child in root.child_elements() {
        if matches_any(&child, selectors) {
            return Some(child);
        }
        if let Some(found) = find_first(&child, selectors) {
            return Some(found);
        }
    }
    None
}

fn find_all<E: Element>(root: &E, selectors: &SelectorList, results: &mut Vec<E>) {
    for child in root.child_elements() {
        if matches_any(&child, selectors) {
            results.push(child.clone());
        }
        find_all(&child, selectors, results);
    }
}

/// Check if an element matches any selector in the list.
pub fn matches_any<E: Element>(element: &E, selectors: &SelectorList) -> bool {
    selectors.iter().any(|s| matches_selector(element, s))
}

fn match_components<E: Element>(element: &E, components: &[Component], pos: usize) -> bool {
    let mut i = pos;

    while i < components.len() {
        match &components[i] {
            Component::Combinator(_) => break,
            Component::Simple(simple) => {
                if !matches_simple(element, simple) {
                    return false;
                }
                i += 1;
            }
        }
    }

    if i >= components.len() {
        return true;
    }

    let combinator = match &components[i] {
        Component::Combinator(c) => *c,
        _ => return false,
    };
    i += 1;

    match combinator {
        Combinator::Child => {
            if let Some(parent) = element.parent_element() {
                match_components(&parent, components, i)
            } else {
                false
            }
        }
        Combinator::Descendant => {
            let mut ancestor = element.parent_element();
            while let Some(anc) = ancestor {
                if match_components(&anc, components, i) {
                    return true;
                }
                ancestor = anc.parent_element();
            }
            false
        }
        Combinator::NextSibling => {
            if let Some(prev) = element.prev_sibling_element() {
                match_components(&prev, components, i)
            } else {
                false
            }
        }
        Combinator::SubsequentSibling => {
            let mut prev = element.prev_sibling_element();
            while let Some(sib) = prev {
                if match_components(&sib, components, i) {
                    return true;
                }
                prev = sib.prev_sibling_element();
            }
            false
        }
    }
}

fn matches_simple<E: Element>(element: &E, simple: &SimpleSelector) -> bool {
    match simple {
        SimpleSelector::Type(name) => element.local_name().eq_ignore_ascii_case(name),
        SimpleSelector::Universal => true,
        SimpleSelector::Id(id) => element.id().is_some_and(|eid| eid == id),
        SimpleSelector::Class(class) => element.has_class(class),
        SimpleSelector::Attribute {
            name,
            operator,
            value,
            case_sensitivity,
        } => match_attribute(element, name, operator, value, case_sensitivity),
        SimpleSelector::PseudoClass(pc) => matches_pseudo_class(element, pc),
        SimpleSelector::PseudoElement(_) => true,
        SimpleSelector::Nesting => true,
    }
}

fn match_attribute<E: Element>(
    element: &E,
    name: &str,
    operator: &Option<AttributeOperator>,
    value: &Option<String>,
    case_sensitivity: &CaseSensitivity,
) -> bool {
    match operator {
        None => element.has_attribute(name),
        Some(op) => {
            let attr_val = match element.attribute_value(name) {
                Some(v) => v,
                None => return false,
            };
            let expected = match value {
                Some(v) => v.as_str(),
                None => return false,
            };

            let ci = matches!(case_sensitivity, CaseSensitivity::CaseInsensitive);

            match op {
                AttributeOperator::Exact => str_eq(attr_val, expected, ci),
                AttributeOperator::Includes => attr_val
                    .split_whitespace()
                    .any(|word| str_eq(word, expected, ci)),
                AttributeOperator::DashMatch => {
                    str_eq(attr_val, expected, ci)
                        || (if ci {
                            attr_val
                                .to_ascii_lowercase()
                                .starts_with(&format!("{}-", expected.to_ascii_lowercase()))
                        } else {
                            attr_val.starts_with(&format!("{}-", expected))
                        })
                }
                AttributeOperator::Prefix => {
                    if ci {
                        attr_val
                            .to_ascii_lowercase()
                            .starts_with(&expected.to_ascii_lowercase())
                    } else {
                        attr_val.starts_with(expected)
                    }
                }
                AttributeOperator::Suffix => {
                    if ci {
                        attr_val
                            .to_ascii_lowercase()
                            .ends_with(&expected.to_ascii_lowercase())
                    } else {
                        attr_val.ends_with(expected)
                    }
                }
                AttributeOperator::Substring => {
                    if ci {
                        attr_val
                            .to_ascii_lowercase()
                            .contains(&expected.to_ascii_lowercase())
                    } else {
                        attr_val.contains(expected)
                    }
                }
            }
        }
    }
}

fn str_eq(a: &str, b: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn matches_pseudo_class<E: Element>(element: &E, pc: &PseudoClass) -> bool {
    match pc {
        PseudoClass::Hover => element.is_hover(),
        PseudoClass::Active => element.is_active(),
        PseudoClass::Focus => element.is_focus(),
        PseudoClass::FocusWithin => element.is_focus_within(),
        PseudoClass::FocusVisible => element.is_focus_visible(),
        PseudoClass::Link => element.is_link(),
        PseudoClass::Visited => element.is_visited(),
        PseudoClass::AnyLink => element.is_any_link(),
        PseudoClass::Target => element.is_target(),
        PseudoClass::Enabled => element.is_enabled(),
        PseudoClass::Disabled => element.is_disabled(),
        PseudoClass::Checked => element.is_checked(),
        PseudoClass::Default => element.is_default(),
        PseudoClass::Indeterminate => element.is_indeterminate(),
        PseudoClass::Required => element.is_required(),
        PseudoClass::Optional => element.is_optional(),
        PseudoClass::Valid => element.is_valid(),
        PseudoClass::Invalid => element.is_invalid(),
        PseudoClass::InRange => element.is_in_range(),
        PseudoClass::OutOfRange => element.is_out_of_range(),
        PseudoClass::ReadWrite => element.is_read_write(),
        PseudoClass::ReadOnly => element.is_read_only(),
        PseudoClass::PlaceholderShown => element.is_placeholder_shown(),

        PseudoClass::Root => element.is_root(),
        PseudoClass::Empty => element.is_empty(),

        PseudoClass::FirstChild => element.sibling_index() == 1,
        PseudoClass::LastChild => element.sibling_index_from_end() == 1,
        PseudoClass::OnlyChild => {
            element.sibling_index() == 1 && element.sibling_index_from_end() == 1
        }

        PseudoClass::FirstOfType => element.sibling_type_index() == 1,
        PseudoClass::LastOfType => element.sibling_type_index_from_end() == 1,
        PseudoClass::OnlyOfType => element.sibling_type_count() == 1,

        PseudoClass::NthChild(nth, of_sel) => {
            let index = match of_sel {
                None => element.sibling_index(),
                Some(sel_list) => nth_of_index(element, sel_list, false),
            };
            nth.matches(index)
        }
        PseudoClass::NthLastChild(nth, of_sel) => {
            let index = match of_sel {
                None => element.sibling_index_from_end(),
                Some(sel_list) => nth_of_index(element, sel_list, true),
            };
            nth.matches(index)
        }
        PseudoClass::NthOfType(nth) => nth.matches(element.sibling_type_index()),
        PseudoClass::NthLastOfType(nth) => nth.matches(element.sibling_type_index_from_end()),

        PseudoClass::Lang(langs) => {
            if let Some(el_lang) = element.lang() {
                langs.iter().any(|l| {
                    el_lang.eq_ignore_ascii_case(l)
                        || el_lang
                            .to_ascii_lowercase()
                            .starts_with(&format!("{}-", l.to_ascii_lowercase()))
                })
            } else {
                false
            }
        }

        PseudoClass::Is(list) | PseudoClass::Where(list) => matches_any(element, list),
        PseudoClass::Not(list) => !matches_any(element, list),

        PseudoClass::Has(relatives) => {
            for rel in relatives {
                if has_matching_descendant(element, &rel.selector) {
                    return true;
                }
            }
            false
        }
    }
}

fn nth_of_index<E: Element>(element: &E, sel_list: &SelectorList, from_end: bool) -> i32 {
    let mut index = 1;
    let sib_fn = if from_end {
        Element::next_sibling_element
    } else {
        Element::prev_sibling_element
    };
    let mut sib = sib_fn(element);
    while let Some(s) = sib {
        if matches_any(&s, sel_list) {
            index += 1;
        }
        sib = sib_fn(&s);
    }
    index
}

fn has_matching_descendant<E: Element>(element: &E, selector: &Selector) -> bool {
    for child in element.child_elements() {
        if matches_selector(&child, selector) {
            return true;
        }
        if has_matching_descendant(&child, selector) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{Attribute, Dom, DomElement, NodeId, QualName};

    fn build_dom() -> Dom {
        let mut dom = Dom::new();
        let html = dom.create_element(QualName::new("html"), vec![]);
        dom.append_child(NodeId::DOCUMENT, html);

        let body = dom.create_element(QualName::new("body"), vec![]);
        dom.append_child(html, body);

        let div = dom.create_element(
            QualName::new("div"),
            vec![
                Attribute {
                    name: QualName::new("id"),
                    value: "main".to_string(),
                },
                Attribute {
                    name: QualName::new("class"),
                    value: "container active".to_string(),
                },
            ],
        );
        dom.append_child(body, div);

        let p = dom.create_element(QualName::new("p"), vec![]);
        dom.append_child(div, p);

        let text = dom.create_text("Hello".to_string());
        dom.append_child(p, text);

        dom
    }

    #[test]
    fn match_type_selector() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let html_el = DomElement::new(&dom, html).unwrap();
        let sels = parse_selector_list("html").unwrap();
        assert!(matches_selector(&html_el, &sels[0]));
    }

    #[test]
    fn match_id_selector() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let body = dom.children(html)[0];
        let div = dom.children(body)[0];
        let div_el = DomElement::new(&dom, div).unwrap();
        let sels = parse_selector_list("#main").unwrap();
        assert!(matches_selector(&div_el, &sels[0]));
    }

    #[test]
    fn match_class_selector() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let body = dom.children(html)[0];
        let div = dom.children(body)[0];
        let div_el = DomElement::new(&dom, div).unwrap();
        let sels = parse_selector_list(".container").unwrap();
        assert!(matches_selector(&div_el, &sels[0]));
    }

    #[test]
    fn match_descendant_combinator() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let body = dom.children(html)[0];
        let div = dom.children(body)[0];
        let p = dom.children(div)[0];
        let p_el = DomElement::new(&dom, p).unwrap();
        let sels = parse_selector_list("div p").unwrap();
        assert!(matches_selector(&p_el, &sels[0]));
    }

    #[test]
    fn match_child_combinator() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let body = dom.children(html)[0];
        let div = dom.children(body)[0];
        let p = dom.children(div)[0];
        let p_el = DomElement::new(&dom, p).unwrap();
        let sels = parse_selector_list("div > p").unwrap();
        assert!(matches_selector(&p_el, &sels[0]));
    }

    #[test]
    fn query_selector_finds_first() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let body = dom.children(html)[0];
        let body_el = DomElement::new(&dom, body).unwrap();
        let result = query_selector(&body_el, "p").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn query_selector_all_finds_multiple() {
        let dom = build_dom();
        let html = dom.children(NodeId::DOCUMENT)[0];
        let html_el = DomElement::new(&dom, html).unwrap();
        let results = query_selector_all(&html_el, "div, p").unwrap();
        assert_eq!(results.len(), 2); // div#main and p
    }
}
