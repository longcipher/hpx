use deno_core::{OpState, op2};

use crate::{
    dom::{NodeId, QualName},
    js_runtime::state::DomState,
};

// --- Read ops ---

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_document_node() -> i32 {
    NodeId::DOCUMENT.to_raw() as i32
}

#[op2]
#[string]
pub(crate) fn op_dom_get_tag_name(state: &mut OpState, #[smi] node_id: i32) -> String {
    let state = state.borrow::<DomState>();
    let id = NodeId::from_raw(node_id as u32);
    match state.dom.get(id) {
        Some(n) => n.as_element().map(|e| e.name.local.clone()),
        None => None,
    }
    .unwrap_or_default()
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_node_type(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    state.dom.node_type(NodeId::from_raw(node_id as u32)) as i32
}

#[op2]
#[string]
pub(crate) fn op_dom_get_text_content(state: &mut OpState, #[smi] node_id: i32) -> String {
    let state = state.borrow::<DomState>();
    state.dom.text_content(NodeId::from_raw(node_id as u32))
}

#[op2]
#[string]
pub(crate) fn op_dom_get_inner_html(state: &mut OpState, #[smi] node_id: i32) -> String {
    let state = state.borrow::<DomState>();
    state
        .dom
        .serialize_inner_html(NodeId::from_raw(node_id as u32))
}

#[op2]
#[string]
pub(crate) fn op_dom_get_outer_html(state: &mut OpState, #[smi] node_id: i32) -> String {
    let state = state.borrow::<DomState>();
    state.dom.serialize_html(NodeId::from_raw(node_id as u32))
}

#[op2]
#[string]
pub(crate) fn op_dom_get_attribute(
    state: &mut OpState,
    #[smi] node_id: i32,
    #[string] name: &str,
) -> String {
    let state = state.borrow::<DomState>();
    let id = NodeId::from_raw(node_id as u32);
    match state.dom.get(id) {
        Some(n) => n.as_element().and_then(|e| {
            e.attrs
                .iter()
                .find(|a| a.name.local.eq_ignore_ascii_case(name))
                .map(|a| a.value.clone())
        }),
        None => None,
    }
    .unwrap_or_default()
}

#[op2(fast)]
pub(crate) fn op_dom_has_attribute(
    state: &mut OpState,
    #[smi] node_id: i32,
    #[string] name: &str,
) -> bool {
    let state = state.borrow::<DomState>();
    let id = NodeId::from_raw(node_id as u32);
    match state.dom.get(id) {
        Some(n) => n.as_element().is_some_and(|e| {
            e.attrs
                .iter()
                .any(|a| a.name.local.eq_ignore_ascii_case(name))
        }),
        None => false,
    }
}

#[op2]
#[serde]
pub(crate) fn op_dom_get_attribute_names(state: &mut OpState, #[smi] node_id: i32) -> Vec<String> {
    let state = state.borrow::<DomState>();
    let id = NodeId::from_raw(node_id as u32);
    match state.dom.get(id) {
        Some(n) => n
            .as_element()
            .map(|e| e.attrs.iter().map(|a| a.name.local.clone()).collect()),
        None => None,
    }
    .unwrap_or_default()
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_parent(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    let id = NodeId::from_raw(node_id as u32);
    state
        .dom
        .get(id)
        .and_then(|n| n.parent)
        .map(|p| p.to_raw() as i32)
        .unwrap_or(-1)
}

#[op2]
#[serde]
pub(crate) fn op_dom_get_children(state: &mut OpState, #[smi] node_id: i32) -> Vec<i32> {
    let state = state.borrow::<DomState>();
    state
        .dom
        .children(NodeId::from_raw(node_id as u32))
        .iter()
        .map(|id| id.to_raw() as i32)
        .collect()
}

#[op2]
#[serde]
pub(crate) fn op_dom_get_child_elements(state: &mut OpState, #[smi] node_id: i32) -> Vec<i32> {
    let state = state.borrow::<DomState>();
    state
        .dom
        .child_elements(NodeId::from_raw(node_id as u32))
        .iter()
        .map(|id| id.to_raw() as i32)
        .collect()
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_first_child(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get(NodeId::from_raw(node_id as u32))
        .and_then(|n| n.first_child)
        .map(|id| id.to_raw() as i32)
        .unwrap_or(-1)
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_last_child(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get(NodeId::from_raw(node_id as u32))
        .and_then(|n| n.last_child)
        .map(|id| id.to_raw() as i32)
        .unwrap_or(-1)
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_next_sibling(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get(NodeId::from_raw(node_id as u32))
        .and_then(|n| n.next_sibling)
        .map(|id| id.to_raw() as i32)
        .unwrap_or(-1)
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_prev_sibling(state: &mut OpState, #[smi] node_id: i32) -> i32 {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get(NodeId::from_raw(node_id as u32))
        .and_then(|n| n.prev_sibling)
        .map(|id| id.to_raw() as i32)
        .unwrap_or(-1)
}

// --- Query ops ---

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_get_element_by_id(state: &mut OpState, #[string] id: &str) -> i32 {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get_element_by_id(id)
        .map(|n| n.to_raw() as i32)
        .unwrap_or(-1)
}

#[op2]
#[serde]
pub(crate) fn op_dom_get_elements_by_tag_name(
    state: &mut OpState,
    #[smi] node_id: i32,
    #[string] tag: String,
) -> Vec<i32> {
    let state = state.borrow::<DomState>();
    state
        .dom
        .get_elements_by_tag_name(NodeId::from_raw(node_id as u32), &tag)
        .iter()
        .map(|id| id.to_raw() as i32)
        .collect()
}

// --- Mutation ops ---

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_create_element(state: &mut OpState, #[string] tag: &str) -> i32 {
    let state = state.borrow_mut::<DomState>();
    state
        .dom
        .create_element(QualName::new(tag), vec![])
        .to_raw() as i32
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_create_text_node(state: &mut OpState, #[string] text: &str) -> i32 {
    let state = state.borrow_mut::<DomState>();
    state.dom.create_text(text.to_string()).to_raw() as i32
}

#[op2(fast)]
#[smi]
pub(crate) fn op_dom_create_document_fragment(state: &mut OpState) -> i32 {
    let state = state.borrow_mut::<DomState>();
    state.dom.create_document_fragment().to_raw() as i32
}

#[op2(fast)]
pub(crate) fn op_dom_append_child(state: &mut OpState, #[smi] parent: i32, #[smi] child: i32) {
    let state = state.borrow_mut::<DomState>();
    state.dom.append_child(
        NodeId::from_raw(parent as u32),
        NodeId::from_raw(child as u32),
    );
}

#[op2(fast)]
pub(crate) fn op_dom_insert_before(
    state: &mut OpState,
    #[smi] parent: i32,
    #[smi] child: i32,
    #[smi] reference: i32,
) {
    let state = state.borrow_mut::<DomState>();
    state.dom.insert_before(
        NodeId::from_raw(parent as u32),
        NodeId::from_raw(child as u32),
        NodeId::from_raw(reference as u32),
    );
}

#[op2(fast)]
pub(crate) fn op_dom_remove_child(state: &mut OpState, #[smi] parent: i32, #[smi] child: i32) {
    let state = state.borrow_mut::<DomState>();
    let _ = parent; // ponytail: detach is simpler, matches browser_oxide pattern
    state.dom.detach(NodeId::from_raw(child as u32));
}

#[op2(fast)]
pub(crate) fn op_dom_set_text_content(
    state: &mut OpState,
    #[smi] node_id: i32,
    #[string] text: String,
) {
    let state = state.borrow_mut::<DomState>();
    state
        .dom
        .set_text_content(NodeId::from_raw(node_id as u32), &text);
}

#[op2]
#[string]
pub(crate) fn op_dom_get_base_url(state: &mut OpState) -> String {
    let state = state.borrow::<DomState>();
    state
        .base_url
        .as_ref()
        .map(|u| u.to_string())
        .unwrap_or_else(|| "about:blank".to_string())
}

// --- Storage ops ---

#[op2]
#[string]
pub(crate) fn op_dom_storage_get(
    state: &mut OpState,
    #[string] area: String,
    #[string] key: String,
) -> Option<String> {
    let state = state.borrow::<DomState>();
    state.storage.get(&area).and_then(|m| m.get(&key)).cloned()
}

#[op2(fast)]
pub(crate) fn op_dom_storage_set(
    state: &mut OpState,
    #[string] area: String,
    #[string] key: String,
    #[string] value: String,
) {
    let state = state.borrow_mut::<DomState>();
    if let Some(m) = state.storage.get_mut(&area) {
        m.insert(key, value);
    }
}

#[op2(fast)]
pub(crate) fn op_dom_storage_remove(
    state: &mut OpState,
    #[string] area: String,
    #[string] key: String,
) {
    let state = state.borrow_mut::<DomState>();
    if let Some(m) = state.storage.get_mut(&area) {
        m.remove(&key);
    }
}

#[op2(fast)]
pub(crate) fn op_dom_storage_clear(state: &mut OpState, #[string] area: String) {
    let state = state.borrow_mut::<DomState>();
    if let Some(m) = state.storage.get_mut(&area) {
        m.clear();
    }
}

#[op2]
#[serde]
pub(crate) fn op_dom_storage_keys(state: &mut OpState, #[string] area: String) -> Vec<String> {
    let state = state.borrow::<DomState>();
    state
        .storage
        .get(&area)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

deno_core::extension!(
    dom_extension,
    ops = [
        op_dom_document_node,
        op_dom_get_tag_name,
        op_dom_get_node_type,
        op_dom_get_text_content,
        op_dom_get_inner_html,
        op_dom_get_outer_html,
        op_dom_get_attribute,
        op_dom_has_attribute,
        op_dom_get_attribute_names,
        op_dom_get_parent,
        op_dom_get_children,
        op_dom_get_child_elements,
        op_dom_get_first_child,
        op_dom_get_last_child,
        op_dom_get_next_sibling,
        op_dom_get_prev_sibling,
        op_dom_get_element_by_id,
        op_dom_get_elements_by_tag_name,
        op_dom_create_element,
        op_dom_create_text_node,
        op_dom_create_document_fragment,
        op_dom_append_child,
        op_dom_insert_before,
        op_dom_remove_child,
        op_dom_set_text_content,
        op_dom_get_base_url,
        op_dom_storage_get,
        op_dom_storage_set,
        op_dom_storage_remove,
        op_dom_storage_clear,
        op_dom_storage_keys,
    ],
);
