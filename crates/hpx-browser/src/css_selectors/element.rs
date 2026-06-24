/// Trait for DOM elements that can be matched against CSS selectors.
pub trait Element: Sized + Clone {
    fn local_name(&self) -> &str;
    fn namespace(&self) -> Option<&str> {
        None
    }
    fn id(&self) -> Option<&str>;
    fn has_class(&self, name: &str) -> bool;
    fn has_attribute(&self, name: &str) -> bool;
    fn attribute_value(&self, name: &str) -> Option<&str>;
    fn parent_element(&self) -> Option<Self>;
    fn prev_sibling_element(&self) -> Option<Self>;
    fn next_sibling_element(&self) -> Option<Self>;
    fn first_child_element(&self) -> Option<Self>;
    fn last_child_element(&self) -> Option<Self>;

    fn is_root(&self) -> bool {
        self.parent_element().is_none()
    }
    fn is_empty(&self) -> bool {
        self.first_child_element().is_none()
    }
    fn is_link(&self) -> bool {
        false
    }
    fn is_visited(&self) -> bool {
        false
    }
    fn is_hover(&self) -> bool {
        false
    }
    fn is_active(&self) -> bool {
        false
    }
    fn is_focus(&self) -> bool {
        false
    }
    fn is_focus_within(&self) -> bool {
        false
    }
    fn is_focus_visible(&self) -> bool {
        false
    }
    fn is_enabled(&self) -> bool {
        false
    }
    fn is_disabled(&self) -> bool {
        false
    }
    fn is_checked(&self) -> bool {
        false
    }
    fn is_target(&self) -> bool {
        false
    }
    fn is_read_write(&self) -> bool {
        false
    }
    fn is_read_only(&self) -> bool {
        !self.is_read_write()
    }
    fn is_required(&self) -> bool {
        false
    }
    fn is_optional(&self) -> bool {
        !self.is_required()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn is_invalid(&self) -> bool {
        !self.is_valid()
    }
    fn is_default(&self) -> bool {
        false
    }
    fn is_indeterminate(&self) -> bool {
        false
    }
    fn is_placeholder_shown(&self) -> bool {
        false
    }
    fn is_any_link(&self) -> bool {
        self.is_link() || self.is_visited()
    }
    fn is_in_range(&self) -> bool {
        false
    }
    fn is_out_of_range(&self) -> bool {
        false
    }
    fn lang(&self) -> Option<&str> {
        None
    }

    fn child_elements(&self) -> Vec<Self> {
        let mut children = Vec::new();
        let mut child = self.first_child_element();
        while let Some(c) = child {
            let next = c.next_sibling_element();
            children.push(c);
            child = next;
        }
        children
    }

    fn sibling_index(&self) -> i32 {
        let mut index = 1;
        let mut sib = self.prev_sibling_element();
        while let Some(s) = sib {
            index += 1;
            sib = s.prev_sibling_element();
        }
        index
    }

    fn sibling_index_from_end(&self) -> i32 {
        let mut index = 1;
        let mut sib = self.next_sibling_element();
        while let Some(s) = sib {
            index += 1;
            sib = s.next_sibling_element();
        }
        index
    }

    fn sibling_type_index(&self) -> i32 {
        let name = self.local_name().to_ascii_lowercase();
        let mut index = 1;
        let mut sib = self.prev_sibling_element();
        while let Some(s) = sib {
            if s.local_name().eq_ignore_ascii_case(&name) {
                index += 1;
            }
            sib = s.prev_sibling_element();
        }
        index
    }

    fn sibling_type_index_from_end(&self) -> i32 {
        let name = self.local_name().to_ascii_lowercase();
        let mut index = 1;
        let mut sib = self.next_sibling_element();
        while let Some(s) = sib {
            if s.local_name().eq_ignore_ascii_case(&name) {
                index += 1;
            }
            sib = s.next_sibling_element();
        }
        index
    }

    fn sibling_type_count(&self) -> i32 {
        self.sibling_type_index() + self.sibling_type_index_from_end() - 1
    }
}
