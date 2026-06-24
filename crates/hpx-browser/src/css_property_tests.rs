#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parse_stylesheet_never_panics(input in ".*") {
            let (stylesheet, _errors) = crate::css_parser::parse_stylesheet(&input);
            // Should not panic; errors are collected, not thrown
            let _ = stylesheet.rules.len();
        }

        #[test]
        fn parse_declaration_list_never_panics(input in ".*") {
            let (decls, _errors) = crate::css_parser::parse_declaration_list(&input);
            let _ = decls.len();
        }

        #[test]
        fn tokenize_never_panics(input in ".*") {
            let tokens: Vec<_> = crate::css_parser::Tokenizer::new(&input).collect();
            let _ = tokens.len();
        }

        #[test]
        fn parse_selector_list_no_panic_on_any_input(input in ".*") {
            let _ = crate::css_selectors::parse_selector_list(&input);
        }

        #[test]
        fn parse_selector_list_forgiving_no_panic(input in ".*") {
            let list = crate::css_selectors::parse_selector_list_forgiving(&input);
            let _ = list.len();
        }
    }
}
