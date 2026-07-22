use ahash::AHashSet;
use blitz_dom::{
    Attribute as BlitzAttribute, BaseDocument, DocumentConfig, ElementData as BlitzElementData,
    Node as BlitzNode, NodeData as BlitzNodeData, QualName as H5QualName, ns,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    pub const DOCUMENT: NodeId = NodeId(0);

    #[must_use]
    pub fn from_raw(v: u32) -> Self {
        Self(v as usize)
    }

    #[must_use]
    pub fn to_raw(self) -> u32 {
        self.0 as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualName {
    pub ns: Option<String>,
    pub local: String,
}

impl QualName {
    pub fn new(local: impl Into<String>) -> Self {
        Self {
            ns: None,
            local: local.into(),
        }
    }

    pub fn with_ns(ns: impl Into<String>, local: impl Into<String>) -> Self {
        Self {
            ns: Some(ns.into()),
            local: local.into(),
        }
    }
}

impl QualName {
    fn to_h5(&self) -> H5QualName {
        let ns = match &self.ns {
            Some(ns) => ns.as_str().into(),
            None => ns!(html),
        };
        H5QualName::new(None, ns, self.local.as_str().into())
    }

    fn from_h5(qn: &H5QualName) -> Self {
        let ns_str = qn.ns.to_string();
        let ns = if ns_str.is_empty() || ns_str == "http://www.w3.org/1999/xhtml" {
            None
        } else {
            Some(ns_str)
        };
        QualName {
            ns,
            local: qn.local.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: QualName,
    pub value: String,
}

impl Attribute {
    pub(crate) fn to_blitz(&self) -> BlitzAttribute {
        BlitzAttribute {
            name: self.name.to_h5(),
            value: self.value.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum NodeData {
    Document,
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element(ElementData),
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
    DocumentFragment,
    ShadowRoot {
        mode: ShadowRootMode,
        host: NodeId,
    },
}

#[derive(Debug, Clone)]
pub struct ElementData {
    pub name: QualName,
    pub attrs: Vec<Attribute>,
    pub shadow_root: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootMode {
    Open,
    Closed,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub data: NodeData,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
}

impl Node {
    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    pub fn as_element(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match &mut self.data {
            NodeData::Element(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn is_element_with_tag(&self, tag: &str) -> bool {
        match &self.data {
            NodeData::Element(e) => e.name.local.eq_ignore_ascii_case(tag),
            _ => false,
        }
    }
}

pub struct Dom {
    inner: BaseDocument,
}

const WALK_LIMIT: usize = 2_000_000;
const ANCESTOR_LIMIT: usize = 10_000;

impl Dom {
    pub fn new() -> Self {
        let mut config = DocumentConfig::default();
        config.style_threading = blitz_dom::StyleThreading::Sequential;
        let inner = BaseDocument::new(config);
        Self { inner }
    }

    pub fn from_base(inner: BaseDocument) -> Self {
        Self { inner }
    }

    pub fn document(&self) -> NodeId {
        NodeId::DOCUMENT
    }

    pub fn inner(&self) -> &BaseDocument {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut BaseDocument {
        &mut self.inner
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn get(&self, id: NodeId) -> Option<Node> {
        let blitz_node = self.inner.get_node(id.0)?;
        Some(self.convert_node(blitz_node))
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut BaseDocument> {
        self.inner.get_node_mut(id.0)?;
        Some(&mut self.inner)
    }

    pub fn len(&self) -> usize {
        self.inner.tree().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.tree().is_empty()
    }

    pub fn create_element(&mut self, name: QualName, attrs: Vec<Attribute>) -> NodeId {
        let blitz_attrs: Vec<BlitzAttribute> = attrs.iter().map(|a| a.to_blitz()).collect();
        let h5_name = name.to_h5();
        let elem_data = BlitzElementData::new(h5_name, blitz_attrs);
        let id = self.inner.create_node(BlitzNodeData::Element(elem_data));
        NodeId(id)
    }

    pub fn create_text(&mut self, text: String) -> NodeId {
        let id = self.inner.create_text_node(&text);
        NodeId(id)
    }

    pub fn create_comment(&mut self, _text: String) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Comment);
        NodeId(id)
    }

    pub fn create_document_fragment(&mut self) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId(id)
    }

    pub fn create_shadow_root(&mut self, _host: NodeId, _mode: ShadowRootMode) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId(id)
    }

    pub fn allocate_pi(&mut self, _target: String, _data: String) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Comment);
        NodeId(id)
    }

    pub fn create_doctype(
        &mut self,
        _name: String,
        _public_id: String,
        _system_id: String,
    ) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId(id)
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if self.inner.get_node(parent.0).is_none() || self.inner.get_node(child.0).is_none() {
            return;
        }
        self.detach(child);
        if let Some(parent_node) = self.inner.get_node_mut(parent.0) {
            parent_node.children.push(child.0);
        }
        if let Some(child_node) = self.inner.get_node_mut(child.0) {
            child_node.parent = Some(parent.0);
        }
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if self.inner.get_node(parent.0).is_none()
            || self.inner.get_node(child.0).is_none()
            || self.inner.get_node(reference.0).is_none()
        {
            return;
        }
        self.detach(child);
        if let Some(parent_node) = self.inner.get_node_mut(parent.0) {
            if let Some(idx) = parent_node
                .children
                .iter()
                .position(|&id| id == reference.0)
            {
                parent_node.children.insert(idx, child.0);
            } else {
                parent_node.children.push(child.0);
            }
        }
        if let Some(child_node) = self.inner.get_node_mut(child.0) {
            child_node.parent = Some(parent.0);
        }
    }

    pub fn detach(&mut self, id: NodeId) {
        let parent_id = match self.inner.get_node(id.0) {
            Some(n) => n.parent,
            None => return,
        };
        if let Some(pid) = parent_id {
            if let Some(parent) = self.inner.get_node_mut(pid) {
                parent.children.retain(|&c| c != id.0);
            }
        }
        if let Some(node) = self.inner.get_node_mut(id.0) {
            node.parent = None;
        }
    }

    pub fn remove(&mut self, id: NodeId) {
        self.detach(id);
        let children: Vec<usize> = self
            .inner
            .get_node(id.0)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            self.remove(NodeId(child_id));
        }
    }

    pub fn reparent_children(&mut self, source: NodeId, target: NodeId) {
        let children: Vec<usize> = self
            .inner
            .get_node(source.0)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            self.append_child(target, NodeId(child_id));
        }
    }

    pub fn children(&self, parent: NodeId) -> Vec<NodeId> {
        self.inner
            .get_node(parent.0)
            .map(|n| n.children.iter().map(|&id| NodeId(id)).collect())
            .unwrap_or_default()
    }

    pub fn child_elements(&self, parent: NodeId) -> Vec<NodeId> {
        self.children(parent)
            .into_iter()
            .filter(|id| self.get(*id).is_some_and(|n| n.is_element()))
            .collect()
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn text_content(&self, id: NodeId) -> String {
        let mut result = String::new();
        self.collect_text(id, &mut result);
        result
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn collect_text(&self, root: NodeId, result: &mut String) {
        let mut stack: Vec<NodeId> = vec![root];
        let mut visited: AHashSet<NodeId> = AHashSet::with_capacity(64);
        let mut steps: usize = 0;
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            steps += 1;
            if steps > WALK_LIMIT {
                break;
            }
            let node = match self.get(id) {
                Some(n) => n,
                None => continue,
            };
            match &node.data {
                NodeData::Text(t) => result.push_str(t),
                _ => {
                    let mut kids: Vec<NodeId> = Vec::new();
                    let mut child = node.first_child;
                    while let Some(c) = child {
                        kids.push(c);
                        child = self.get(c).and_then(|n| n.next_sibling);
                    }
                    stack.extend(kids.into_iter().rev());
                }
            }
        }
    }

    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        let children: Vec<NodeId> = self.children(id);
        for child in children {
            self.remove(child);
        }
        if !text.is_empty() {
            let text_id = self.create_text(text.to_string());
            self.append_child(id, text_id);
        }
    }

    pub fn get_element_by_id(&self, id_value: &str) -> Option<NodeId> {
        self.find_element(NodeId::DOCUMENT, &|node| {
            node.as_element()
                .and_then(|e| e.attrs.iter().find(|a| a.name.local == "id"))
                .is_some_and(|a| a.value == id_value)
        })
    }

    pub fn get_elements_by_tag_name(&self, root: NodeId, tag: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.collect_elements(
            root,
            &|node| {
                node.as_element()
                    .is_some_and(|e| e.name.local.eq_ignore_ascii_case(tag))
            },
            &mut results,
        );
        results
    }

    pub fn get_elements_by_class_name(&self, root: NodeId, class: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.collect_elements(
            root,
            &|node| {
                node.as_element()
                    .and_then(|e| e.attrs.iter().find(|a| a.name.local == "class"))
                    .is_some_and(|a| a.value.split_whitespace().any(|c| c == class))
            },
            &mut results,
        );
        results
    }

    pub fn serialize_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize_node(id, &mut out);
        out
    }

    pub fn serialize_inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        let node = match self.get(id) {
            Some(n) => n,
            None => return out,
        };
        let mut kids: Vec<NodeId> = Vec::new();
        let mut child = node.first_child;
        while let Some(child_id) = child {
            kids.push(child_id);
            child = self.get(child_id).and_then(|n| n.next_sibling);
        }
        for c in kids {
            self.serialize_node(c, &mut out);
        }
        out
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn serialize_node(&self, root: NodeId, out: &mut String) {
        enum SerWork {
            Open(NodeId),
            Close(String),
        }
        const VOID_ELEMENTS: &[&str] = &[
            "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
            "source", "track", "wbr",
        ];

        let mut stack: Vec<SerWork> = vec![SerWork::Open(root)];
        let mut visited: AHashSet<NodeId> = AHashSet::with_capacity(64);
        let mut steps: usize = 0;
        while let Some(work) = stack.pop() {
            match work {
                SerWork::Close(s) => out.push_str(&s),
                SerWork::Open(id) => {
                    if !visited.insert(id) {
                        continue;
                    }
                    steps += 1;
                    if steps > WALK_LIMIT {
                        break;
                    }
                    let node = match self.get(id) {
                        Some(n) => n,
                        None => continue,
                    };
                    match &node.data {
                        NodeData::Element(elem) => {
                            out.push('<');
                            out.push_str(&elem.name.local);
                            for attr in &elem.attrs {
                                out.push(' ');
                                out.push_str(&attr.name.local);
                                out.push_str("=\"");
                                out.push_str(
                                    &attr.value.replace('&', "&amp;").replace('"', "&quot;"),
                                );
                                out.push('"');
                            }
                            out.push('>');
                            let is_void = VOID_ELEMENTS.contains(&elem.name.local.as_str());
                            if !is_void {
                                stack.push(SerWork::Close(format!("</{}>", elem.name.local)));
                            }
                            let mut kids: Vec<NodeId> = Vec::new();
                            let mut child = node.first_child;
                            while let Some(c) = child {
                                kids.push(c);
                                child = self.get(c).and_then(|n| n.next_sibling);
                            }
                            for c in kids.into_iter().rev() {
                                stack.push(SerWork::Open(c));
                            }
                        }
                        NodeData::Text(text) => {
                            out.push_str(
                                &text
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;"),
                            );
                        }
                        NodeData::Comment(text) => {
                            out.push_str("<!--");
                            out.push_str(text);
                            out.push_str("-->");
                        }
                        NodeData::DocumentType { name, .. } => {
                            out.push_str("<!DOCTYPE ");
                            out.push_str(name);
                            out.push('>');
                        }
                        NodeData::Document | NodeData::DocumentFragment => {
                            let mut kids: Vec<NodeId> = Vec::new();
                            let mut child = node.first_child;
                            while let Some(c) = child {
                                kids.push(c);
                                child = self.get(c).and_then(|n| n.next_sibling);
                            }
                            for c in kids.into_iter().rev() {
                                stack.push(SerWork::Open(c));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub fn merge_subtree(&mut self, source: &Dom, source_root: NodeId) -> NodeId {
        fn create_from(this: &mut Dom, source: &Dom, src_id: NodeId) -> Option<NodeId> {
            let src = source.get(src_id)?;
            Some(match &src.data {
                NodeData::Element(elem) => {
                    this.create_element(elem.name.clone(), elem.attrs.clone())
                }
                NodeData::Text(t) => this.create_text(t.clone()),
                NodeData::Comment(t) => this.create_comment(t.clone()),
                NodeData::DocumentFragment | NodeData::Document => this.create_document_fragment(),
                _ => this.create_document_fragment(),
            })
        }

        let new_root = match create_from(self, source, source_root) {
            Some(id) => id,
            None => return self.create_document_fragment(),
        };

        let mut queue: Vec<(NodeId, NodeId)> = Vec::new();
        let mut visited: AHashSet<NodeId> = AHashSet::with_capacity(64);
        visited.insert(source_root);

        let mut child = source.get(source_root).and_then(|n| n.first_child);
        while let Some(c) = child {
            queue.push((c, new_root));
            child = source.get(c).and_then(|n| n.next_sibling);
        }

        let mut steps: usize = 0;
        let mut i = 0usize;
        while i < queue.len() {
            let (src_id, dest_parent) = queue[i];
            i += 1;
            steps += 1;
            if steps > WALK_LIMIT {
                break;
            }
            if !visited.insert(src_id) {
                continue;
            }
            let new_id = match create_from(self, source, src_id) {
                Some(id) => id,
                None => continue,
            };
            self.append_child(dest_parent, new_id);
            let mut child = source.get(src_id).and_then(|n| n.first_child);
            while let Some(c) = child {
                queue.push((c, new_id));
                child = source.get(c).and_then(|n| n.next_sibling);
            }
        }

        new_root
    }

    pub fn node_type(&self, id: NodeId) -> u32 {
        match self.get(id).map(|n| n.data) {
            Some(NodeData::Element(_)) => 1,
            Some(NodeData::Text(_)) => 3,
            Some(NodeData::ProcessingInstruction { .. }) => 7,
            Some(NodeData::Comment(_)) => 8,
            Some(NodeData::Document) => 9,
            Some(NodeData::DocumentType { .. }) => 10,
            Some(NodeData::DocumentFragment) => 11,
            Some(NodeData::ShadowRoot { .. }) => 11,
            None => 0,
        }
    }

    fn find_element(&self, root: NodeId, predicate: &dyn Fn(&Node) -> bool) -> Option<NodeId> {
        let mut stack: Vec<NodeId> = Vec::new();
        let mut child = self.get(root).and_then(|n| n.first_child);
        let mut seed: Vec<NodeId> = Vec::new();
        while let Some(c) = child {
            seed.push(c);
            child = self.get(c).and_then(|n| n.next_sibling);
        }
        stack.extend(seed.into_iter().rev());

        let mut visited: AHashSet<NodeId> = AHashSet::with_capacity(64);
        let mut steps: usize = 0;
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            steps += 1;
            if steps > WALK_LIMIT {
                break;
            }
            let node = match self.get(id) {
                Some(n) => n,
                None => continue,
            };
            if predicate(&node) {
                return Some(id);
            }
            let mut kids: Vec<NodeId> = Vec::new();
            let mut child = node.first_child;
            while let Some(c) = child {
                kids.push(c);
                child = self.get(c).and_then(|n| n.next_sibling);
            }
            stack.extend(kids.into_iter().rev());
        }
        None
    }

    fn collect_elements(
        &self,
        root: NodeId,
        predicate: &dyn Fn(&Node) -> bool,
        results: &mut Vec<NodeId>,
    ) {
        let mut stack: Vec<NodeId> = Vec::new();
        let mut seed: Vec<NodeId> = Vec::new();
        let mut child = self.get(root).and_then(|n| n.first_child);
        while let Some(c) = child {
            seed.push(c);
            child = self.get(c).and_then(|n| n.next_sibling);
        }
        stack.extend(seed.into_iter().rev());

        let mut visited: AHashSet<NodeId> = AHashSet::with_capacity(64);
        let mut steps: usize = 0;
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            steps += 1;
            if steps > WALK_LIMIT {
                break;
            }
            let node = match self.get(id) {
                Some(n) => n,
                None => continue,
            };
            if predicate(&node) {
                results.push(id);
            }
            let mut kids: Vec<NodeId> = Vec::new();
            let mut child = node.first_child;
            while let Some(c) = child {
                kids.push(c);
                child = self.get(c).and_then(|n| n.next_sibling);
            }
            stack.extend(kids.into_iter().rev());
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn convert_node(&self, blitz_node: &BlitzNode) -> Node {
        let id = NodeId(blitz_node.id);
        let parent = blitz_node.parent.map(NodeId);
        let children: Vec<NodeId> = blitz_node.children.iter().map(|&c| NodeId(c)).collect();
        let first_child = children.first().copied();
        let last_child = children.last().copied();
        let prev_sibling = blitz_node
            .parent
            .and_then(|pid| self.inner.get_node(pid))
            .and_then(|parent| {
                let pos = parent.children.iter().position(|&c| c == blitz_node.id)?;
                if pos > 0 {
                    parent.children.get(pos - 1).map(|&c| NodeId(c))
                } else {
                    None
                }
            });
        let next_sibling = blitz_node
            .parent
            .and_then(|pid| self.inner.get_node(pid))
            .and_then(|parent| {
                let pos = parent.children.iter().position(|&c| c == blitz_node.id)?;
                parent.children.get(pos + 1).map(|&c| NodeId(c))
            });
        let data = match &blitz_node.data {
            BlitzNodeData::Document => NodeData::Document,
            BlitzNodeData::Element(e) => {
                let name = QualName::from_h5(&e.name);
                let attrs = e
                    .attrs
                    .iter()
                    .map(|a| Attribute {
                        name: QualName::from_h5(&a.name),
                        value: a.value.clone(),
                    })
                    .collect();
                NodeData::Element(ElementData {
                    name,
                    attrs,
                    shadow_root: None,
                })
            }
            BlitzNodeData::AnonymousBlock(e) => {
                let name = QualName::from_h5(&e.name);
                let attrs = e
                    .attrs
                    .iter()
                    .map(|a| Attribute {
                        name: QualName::from_h5(&a.name),
                        value: a.value.clone(),
                    })
                    .collect();
                NodeData::Element(ElementData {
                    name,
                    attrs,
                    shadow_root: None,
                })
            }
            BlitzNodeData::Text(t) => NodeData::Text(t.content.clone()),
            BlitzNodeData::Comment => NodeData::Comment(String::new()),
        };
        Node {
            id,
            data,
            parent,
            first_child,
            last_child,
            prev_sibling,
            next_sibling,
        }
    }
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Dom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Dom");
        s.field("len", &self.inner.tree().len());
        s.finish()
    }
}

#[derive(Clone)]
pub struct DomElement<'a> {
    pub dom: &'a Dom,
    pub id: NodeId,
    data: ElementData,
}

impl<'a> DomElement<'a> {
    pub fn new(dom: &'a Dom, id: NodeId) -> Option<Self> {
        let node = dom.get(id)?;
        if !node.is_element() {
            return None;
        }
        let data = node.as_element()?.clone();
        Some(Self { dom, id, data })
    }

    pub fn node_id(&self) -> NodeId {
        self.id
    }

    pub fn local_name(&self) -> &str {
        &self.data.name.local
    }

    pub fn id(&self) -> Option<&str> {
        self.data
            .attrs
            .iter()
            .find(|a| a.name.local == "id")
            .map(|a| a.value.as_str())
    }

    pub fn has_class(&self, name: &str) -> bool {
        self.data
            .attrs
            .iter()
            .find(|a| a.name.local == "class")
            .is_some_and(|a| a.value.split_whitespace().any(|c| c == name))
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.data
            .attrs
            .iter()
            .any(|a| a.name.local.eq_ignore_ascii_case(name))
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.data
            .attrs
            .iter()
            .find(|a| a.name.local.eq_ignore_ascii_case(name))
            .map(|a| a.value.as_str())
    }

    fn node(&self) -> Node {
        self.dom
            .get(self.id)
            .unwrap_or_else(|| panic!("DomElement::node invariant: node {} must exist", self.id.0))
    }

    fn element_data(&self) -> &ElementData {
        &self.data
    }
}

impl<'a> std::fmt::Debug for DomElement<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.element_data();
        write!(f, "<{}", data.name.local)?;
        for attr in &data.attrs {
            write!(f, " {}=\"{}\"", attr.name.local, attr.value)?;
        }
        write!(f, ">")
    }
}
