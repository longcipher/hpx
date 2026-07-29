use ahash::AHashSet;
use blitz_dom::{
    Attribute as BlitzAttribute, BaseDocument, DocumentConfig, ElementData as BlitzElementData,
    Node as BlitzNode, NodeData as BlitzNodeData, QualName as H5QualName, ns,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) u64);

impl NodeId {
    #[must_use]
    pub fn from_raw(v: u32) -> Self {
        // JS-side raw IDs are slot indices (low 32 bits) with no version.
        // Reconstruct a blitz NodeId using only the index; the version is
        // resolved at lookup time by `BaseDocument::get_node` (returns None
        // for stale versions).
        Self(u64::from(v))
    }

    #[must_use]
    pub fn to_raw(self) -> u32 {
        // Expose only the slot index to JS. The version lives in the high
        // 32 bits and is an internal detail of the blitz DOM.
        #[expect(clippy::cast_possible_truncation, reason = "extracting slot index")]
        {
            self.0 as u32
        }
    }

    /// Convert to `f64` for JS interop. Carries the full 64-bit value
    /// (slot index + version) so versioned NodeIds round-trip through JS
    /// numbers, which can represent integers up to 2^53 exactly. Use this
    /// instead of [`to_raw`](Self::to_raw) when the version must survive a
    /// hop through JavaScript (e.g. the document node has a non-zero
    /// version and `from_raw(0)` would not resolve).
    #[must_use]
    pub fn to_f64(self) -> f64 {
        self.0 as f64
    }

    /// Reconstruct a NodeId from an `f64` produced by [`to_f64`](Self::to_f64).
    #[must_use]
    pub fn from_f64(v: f64) -> Self {
        Self(v as u64)
    }

    /// Convert to the blitz-dom node id (`usize`).
    pub(crate) fn to_blitz(self) -> usize {
        self.0 as usize
    }

    /// Wrap a blitz-dom node id (`usize`) in our local newtype.
    pub(crate) fn from_blitz(id: usize) -> Self {
        Self(id as u64)
    }
}

/// Iterator over the children of a node, reading directly from the
/// underlying `BaseDocument` children vec without allocating.
pub struct ChildrenIter<'a> {
    iter: std::slice::Iter<'a, usize>,
}

impl<'a> Iterator for ChildrenIter<'a> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        self.iter.next().map(|&id| NodeId::from_blitz(id))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> DoubleEndedIterator for ChildrenIter<'a> {
    fn next_back(&mut self) -> Option<NodeId> {
        self.iter.next_back().map(|&id| NodeId::from_blitz(id))
    }
}

impl<'a> ExactSizeIterator for ChildrenIter<'a> {}

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

/// Lightweight borrowed reference to a DOM node that reads directly from the
/// underlying `BaseDocument`. Does **not** allocate `Vec<Attribute>`, `String`
/// for `QualName`, or compute sibling positions — unlike [`Dom::get`].
#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    dom: &'a Dom,
    id: NodeId,
}

impl<'a> NodeRef<'a> {
    pub fn id(&self) -> NodeId {
        self.id
    }

    fn blitz_data(&self) -> Option<&'a BlitzNodeData> {
        self.dom.inner.get_node(self.id.to_blitz()).map(|n| &n.data)
    }

    pub fn is_element(&self) -> bool {
        matches!(
            self.blitz_data(),
            Some(BlitzNodeData::Element(_)) | Some(BlitzNodeData::AnonymousBlock(_))
        )
    }

    pub fn is_text(&self) -> bool {
        matches!(self.blitz_data(), Some(BlitzNodeData::Text(_)))
    }

    pub fn is_comment(&self) -> bool {
        matches!(self.blitz_data(), Some(BlitzNodeData::Comment))
    }

    pub fn is_document(&self) -> bool {
        matches!(self.blitz_data(), Some(BlitzNodeData::Document))
    }

    pub fn text(&self) -> Option<&'a str> {
        match self.blitz_data() {
            Some(BlitzNodeData::Text(t)) => Some(&t.content),
            _ => None,
        }
    }

    pub fn tag_name(&self) -> Option<&'a str> {
        match self.blitz_data() {
            Some(BlitzNodeData::Element(e)) | Some(BlitzNodeData::AnonymousBlock(e)) => {
                Some(&e.name.local)
            }
            _ => None,
        }
    }

    pub fn get_attr(&self, name: &str) -> Option<&'a str> {
        match self.blitz_data() {
            Some(BlitzNodeData::Element(e)) | Some(BlitzNodeData::AnonymousBlock(e)) => e
                .attrs
                .iter()
                .find(|a| &*a.name.local == name)
                .map(|a| a.value.as_str()),
            _ => None,
        }
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.get_attr("class")
            .is_some_and(|v| v.split_whitespace().any(|c| c == class))
    }

    pub fn first_child(&self) -> Option<NodeId> {
        self.dom
            .inner
            .get_node(self.id.to_blitz())
            .and_then(|n| n.children.first().map(|&c| NodeId::from_blitz(c)))
    }

    pub fn next_sibling(&self) -> Option<NodeId> {
        let node = self.dom.inner.get_node(self.id.to_blitz())?;
        let parent_id = node.parent?;
        let parent = self.dom.inner.get_node(parent_id)?;
        let pos = parent
            .children
            .iter()
            .position(|&c| c == self.id.to_blitz())?;
        parent.children.get(pos + 1).map(|&c| NodeId::from_blitz(c))
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.dom
            .inner
            .get_node(self.id.to_blitz())
            .and_then(|n| n.parent.map(NodeId::from_blitz))
    }

    pub fn node_type(&self) -> u32 {
        match self.blitz_data() {
            Some(BlitzNodeData::Element(_)) | Some(BlitzNodeData::AnonymousBlock(_)) => 1,
            Some(BlitzNodeData::Text(_)) => 3,
            Some(BlitzNodeData::Comment) => 8,
            Some(BlitzNodeData::Document) => 9,
            None => 0,
        }
    }
}

impl<'a> std::fmt::Debug for NodeRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.blitz_data() {
            Some(BlitzNodeData::Element(e)) | Some(BlitzNodeData::AnonymousBlock(e)) => {
                write!(f, "<{}", e.name.local)?;
                for attr in e.attrs.iter() {
                    write!(f, " {}=\"{}\"", attr.name.local, attr.value)?;
                }
                write!(f, ">")
            }
            Some(BlitzNodeData::Text(t)) => write!(f, "Text({:?})", t.content),
            Some(BlitzNodeData::Comment) => write!(f, "Comment"),
            Some(BlitzNodeData::Document) => write!(f, "Document"),
            None => write!(f, "NodeRef(<invalid {}>)", self.id.0),
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
        NodeId::from_blitz(self.inner.root_node().id)
    }

    pub fn inner(&self) -> &BaseDocument {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut BaseDocument {
        &mut self.inner
    }

    /// Returns a lightweight borrowed reference to a node without allocating.
    pub fn node_ref(&self, id: NodeId) -> NodeRef<'_> {
        NodeRef { dom: self, id }
    }

    /// Returns an iterator over the children of a node, reading directly
    /// from the `BaseDocument` children vec without allocating.
    pub fn children_of(&self, id: NodeId) -> ChildrenIter<'_> {
        let slice = self
            .inner
            .get_node(id.to_blitz())
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        ChildrenIter { iter: slice.iter() }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn get(&self, id: NodeId) -> Option<Node> {
        let blitz_node = self.inner.get_node(id.to_blitz())?;
        Some(self.convert_node(blitz_node))
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut BaseDocument> {
        self.inner.get_node_mut(id.to_blitz())?;
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
        NodeId::from_blitz(id)
    }

    pub fn create_text(&mut self, text: String) -> NodeId {
        let id = self.inner.create_text_node(&text);
        NodeId::from_blitz(id)
    }

    pub fn create_comment(&mut self, _text: String) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Comment);
        NodeId::from_blitz(id)
    }

    pub fn create_document_fragment(&mut self) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId::from_blitz(id)
    }

    pub fn create_shadow_root(&mut self, _host: NodeId, _mode: ShadowRootMode) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId::from_blitz(id)
    }

    pub fn allocate_pi(&mut self, _target: String, _data: String) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Comment);
        NodeId::from_blitz(id)
    }

    pub fn create_doctype(
        &mut self,
        _name: String,
        _public_id: String,
        _system_id: String,
    ) -> NodeId {
        let id = self.inner.create_node(BlitzNodeData::Document);
        NodeId::from_blitz(id)
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if self.inner.get_node(parent.to_blitz()).is_none()
            || self.inner.get_node(child.to_blitz()).is_none()
        {
            return;
        }
        self.detach(child);
        if let Some(parent_node) = self.inner.get_node_mut(parent.to_blitz()) {
            parent_node.children.push(child.to_blitz());
        }
        if let Some(child_node) = self.inner.get_node_mut(child.to_blitz()) {
            child_node.parent = Some(parent.to_blitz());
        }
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if self.inner.get_node(parent.to_blitz()).is_none()
            || self.inner.get_node(child.to_blitz()).is_none()
            || self.inner.get_node(reference.to_blitz()).is_none()
        {
            return;
        }
        self.detach(child);
        if let Some(parent_node) = self.inner.get_node_mut(parent.to_blitz()) {
            if let Some(idx) = parent_node
                .children
                .iter()
                .position(|&id| id == reference.to_blitz())
            {
                parent_node.children.insert(idx, child.to_blitz());
            } else {
                parent_node.children.push(child.to_blitz());
            }
        }
        if let Some(child_node) = self.inner.get_node_mut(child.to_blitz()) {
            child_node.parent = Some(parent.to_blitz());
        }
    }

    pub fn detach(&mut self, id: NodeId) {
        let parent_id = match self.inner.get_node(id.to_blitz()) {
            Some(n) => n.parent,
            None => return,
        };
        if let Some(pid) = parent_id {
            if let Some(parent) = self.inner.get_node_mut(pid) {
                parent.children.retain(|&c| c != id.to_blitz());
            }
        }
        if let Some(node) = self.inner.get_node_mut(id.to_blitz()) {
            node.parent = None;
        }
    }

    pub fn remove(&mut self, id: NodeId) {
        self.detach(id);
        let children: Vec<usize> = self
            .inner
            .get_node(id.to_blitz())
            .map(|n| n.children.to_vec())
            .unwrap_or_default();
        for child_id in children {
            self.remove(NodeId::from_blitz(child_id));
        }
    }

    pub fn reparent_children(&mut self, source: NodeId, target: NodeId) {
        let children: Vec<usize> = self
            .inner
            .get_node(source.to_blitz())
            .map(|n| n.children.to_vec())
            .unwrap_or_default();
        for child_id in children {
            self.append_child(target, NodeId::from_blitz(child_id));
        }
    }

    pub fn children(&self, parent: NodeId) -> Vec<NodeId> {
        self.children_of(parent).collect()
    }

    pub fn child_elements(&self, parent: NodeId) -> Vec<NodeId> {
        self.children_of(parent)
            .filter(|&id| NodeRef { dom: self, id }.is_element())
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
            let nr = NodeRef { dom: self, id };
            if let Some(text) = nr.text() {
                result.push_str(text);
            } else {
                stack.extend(self.children_of(id).rev());
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
        self.find_element(self.document(), &|nr| nr.get_attr("id") == Some(id_value))
    }

    pub fn get_elements_by_tag_name(&self, root: NodeId, tag: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.collect_elements(
            root,
            &|nr| nr.tag_name().is_some_and(|t| t.eq_ignore_ascii_case(tag)),
            &mut results,
        );
        results
    }

    pub fn get_elements_by_class_name(&self, root: NodeId, class: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.collect_elements(root, &|nr| nr.has_class(class), &mut results);
        results
    }

    pub fn serialize_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize_node(id, &mut out);
        out
    }

    pub fn serialize_inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        if self.inner.get_node(id.to_blitz()).is_none() {
            return out;
        }
        for c in self.children_of(id) {
            self.serialize_node(c, &mut out);
        }
        out
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn serialize_node(&self, root: NodeId, out: &mut String) {
        enum SerWork {
            Open(NodeId),
            Close(NodeId),
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
                SerWork::Close(id) => {
                    if let Some(blitz_node) = self.inner.get_node(id.to_blitz()) {
                        if let BlitzNodeData::Element(e) | BlitzNodeData::AnonymousBlock(e) =
                            &blitz_node.data
                        {
                            out.push_str("</");
                            out.push_str(&e.name.local);
                            out.push('>');
                        }
                    }
                }
                SerWork::Open(id) => {
                    if !visited.insert(id) {
                        continue;
                    }
                    steps += 1;
                    if steps > WALK_LIMIT {
                        break;
                    }
                    let blitz_node = match self.inner.get_node(id.to_blitz()) {
                        Some(n) => n,
                        None => continue,
                    };
                    match &blitz_node.data {
                        BlitzNodeData::Element(e) | BlitzNodeData::AnonymousBlock(e) => {
                            out.push('<');
                            out.push_str(&e.name.local);
                            for attr in e.attrs.iter() {
                                out.push(' ');
                                out.push_str(&attr.name.local);
                                out.push_str("=\"");
                                out.push_str(
                                    &attr.value.replace('&', "&amp;").replace('"', "&quot;"),
                                );
                                out.push('"');
                            }
                            out.push('>');
                            let tag: &str = &e.name.local;
                            if !VOID_ELEMENTS.contains(&tag) {
                                stack.push(SerWork::Close(id));
                            }
                            for c in self.children_of(id).rev() {
                                stack.push(SerWork::Open(c));
                            }
                        }
                        BlitzNodeData::Text(t) => {
                            out.push_str(
                                &t.content
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;"),
                            );
                        }
                        BlitzNodeData::Comment => {
                            out.push_str("<!--");
                            out.push_str("-->");
                        }
                        BlitzNodeData::Document => {
                            for c in self.children_of(id).rev() {
                                stack.push(SerWork::Open(c));
                            }
                        }
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

        for c in source.children_of(source_root) {
            queue.push((c, new_root));
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
            for c in source.children_of(src_id) {
                queue.push((c, new_id));
            }
        }

        new_root
    }

    pub fn node_type(&self, id: NodeId) -> u32 {
        NodeRef { dom: self, id }.node_type()
    }

    fn find_element(
        &self,
        root: NodeId,
        predicate: &dyn Fn(&NodeRef<'_>) -> bool,
    ) -> Option<NodeId> {
        let mut stack: Vec<NodeId> = self.children_of(root).rev().collect();
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
            let nr = NodeRef { dom: self, id };
            if predicate(&nr) {
                return Some(id);
            }
            stack.extend(self.children_of(id).rev());
        }
        None
    }

    fn collect_elements(
        &self,
        root: NodeId,
        predicate: &dyn Fn(&NodeRef<'_>) -> bool,
        results: &mut Vec<NodeId>,
    ) {
        let mut stack: Vec<NodeId> = self.children_of(root).rev().collect();
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
            let nr = NodeRef { dom: self, id };
            if predicate(&nr) {
                results.push(id);
            }
            stack.extend(self.children_of(id).rev());
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn convert_node(&self, blitz_node: &BlitzNode) -> Node {
        let id = NodeId::from_blitz(blitz_node.id);
        let parent = blitz_node.parent.map(NodeId::from_blitz);
        let children: Vec<NodeId> = blitz_node
            .children
            .iter()
            .map(|&c| NodeId::from_blitz(c))
            .collect();
        let first_child = children.first().copied();
        let last_child = children.last().copied();
        let prev_sibling = blitz_node
            .parent
            .and_then(|pid| self.inner.get_node(pid))
            .and_then(|parent| {
                let pos = parent.children.iter().position(|&c| c == blitz_node.id)?;
                if pos > 0 {
                    parent.children.get(pos - 1).map(|&c| NodeId::from_blitz(c))
                } else {
                    None
                }
            });
        let next_sibling = blitz_node
            .parent
            .and_then(|pid| self.inner.get_node(pid))
            .and_then(|parent| {
                let pos = parent.children.iter().position(|&c| c == blitz_node.id)?;
                parent.children.get(pos + 1).map(|&c| NodeId::from_blitz(c))
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
}

impl<'a> DomElement<'a> {
    pub fn new(dom: &'a Dom, id: NodeId) -> Option<Self> {
        let nr = NodeRef { dom, id };
        if !nr.is_element() {
            return None;
        }
        Some(Self { dom, id })
    }

    pub fn node_id(&self) -> NodeId {
        self.id
    }

    fn nr(&self) -> NodeRef<'a> {
        NodeRef {
            dom: self.dom,
            id: self.id,
        }
    }

    pub fn local_name(&self) -> &'a str {
        self.nr().tag_name().unwrap_or("")
    }

    pub fn id(&self) -> Option<&'a str> {
        self.nr().get_attr("id")
    }

    pub fn has_class(&self, name: &str) -> bool {
        self.nr().has_class(name)
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.nr().get_attr(name).is_some()
    }

    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.nr().get_attr(name)
    }
}

impl<'a> std::fmt::Debug for DomElement<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nr = self.nr();
        write!(f, "<{}", nr.tag_name().unwrap_or("?"))?;
        if let Some(BlitzNodeData::Element(e)) | Some(BlitzNodeData::AnonymousBlock(e)) =
            nr.blitz_data()
        {
            for attr in e.attrs.iter() {
                write!(f, " {}=\"{}\"", attr.name.local, attr.value)?;
            }
        }
        write!(f, ">")
    }
}
