// DOM bootstrap — wires document.* and Element.prototype.* to Rust ops.
(() => {
  const core = Deno.core;

  // window === globalThis in browsers
  globalThis.window = globalThis;
  const DOCUMENT_NODE = core.ops.op_dom_document_node();

  // Node type constants
  const ELEMENT_NODE = 1;
  const TEXT_NODE = 3;
  const DOCUMENT_NODE_TYPE = 9;

  class Node {
    constructor(nodeId) {
      this._nodeId = nodeId;
    }
    get nodeType() {
      return core.ops.op_dom_get_node_type(this._nodeId);
    }
    get nodeName() {
      const type = this.nodeType;
      if (type === ELEMENT_NODE) return this.tagName;
      if (type === TEXT_NODE) return "#text";
      if (type === DOCUMENT_NODE_TYPE) return "#document";
      return "";
    }
    get parentNode() {
      const pid = core.ops.op_dom_get_parent(this._nodeId);
      return pid === -1 ? null : wrapNode(pid);
    }
    get parentElement() {
      const pid = core.ops.op_dom_get_parent(this._nodeId);
      if (pid === -1) return null;
      const n = wrapNode(pid);
      return n instanceof Element ? n : null;
    }
    get childNodes() {
      const ids = core.ops.op_dom_get_children(this._nodeId);
      return ids.map((id) => wrapNode(id));
    }
    get firstChild() {
      const id = core.ops.op_dom_get_first_child(this._nodeId);
      return id === -1 ? null : wrapNode(id);
    }
    get lastChild() {
      const id = core.ops.op_dom_get_last_child(this._nodeId);
      return id === -1 ? null : wrapNode(id);
    }
    get nextSibling() {
      const id = core.ops.op_dom_get_next_sibling(this._nodeId);
      return id === -1 ? null : wrapNode(id);
    }
    get previousSibling() {
      const id = core.ops.op_dom_get_prev_sibling(this._nodeId);
      return id === -1 ? null : wrapNode(id);
    }
    get textContent() {
      return core.ops.op_dom_get_text_content(this._nodeId);
    }
    set textContent(v) {
      core.ops.op_dom_set_text_content(this._nodeId, String(v));
    }
    appendChild(child) {
      if (child && child._nodeId != null) {
        core.ops.op_dom_append_child(this._nodeId, child._nodeId);
      }
      return child;
    }
    insertBefore(child, ref) {
      if (child && child._nodeId != null) {
        const refId = ref ? ref._nodeId : -1;
        core.ops.op_dom_insert_before(this._nodeId, child._nodeId, refId);
      }
      return child;
    }
    removeChild(child) {
      if (child && child._nodeId != null) {
        core.ops.op_dom_remove_child(this._nodeId, child._nodeId);
      }
      return child;
    }
  }

  class Element extends Node {
    get tagName() {
      return core.ops.op_dom_get_tag_name(this._nodeId).toUpperCase();
    }
    get innerHTML() {
      return core.ops.op_dom_get_inner_html(this._nodeId);
    }
    set innerHTML(v) {
      throw new Error("innerHTML setter not yet implemented");
    }
    get outerHTML() {
      return core.ops.op_dom_get_outer_html(this._nodeId);
    }
    get id() {
      return this.getAttribute("id") || "";
    }
    set id(v) {
      // ponytail: setAttribute not yet implemented — silently ignore
    }
    get className() {
      return this.getAttribute("class") || "";
    }
    set className(v) {
      // ponytail: setAttribute not yet implemented — silently ignore
    }
    getAttribute(name) {
      const v = core.ops.op_dom_get_attribute(this._nodeId, name);
      return v === "" &&
        !core.ops.op_dom_has_attribute(this._nodeId, name)
        ? null
        : v;
    }
    hasAttribute(name) {
      return core.ops.op_dom_has_attribute(this._nodeId, name);
    }
    setAttribute(name, value) {
      throw new Error("setAttribute not yet implemented");
    }
    removeAttribute(name) {
      throw new Error("removeAttribute not yet implemented");
    }
    querySelector(selector) {
      throw new Error("querySelector not yet implemented");
    }
    querySelectorAll(selector) {
      throw new Error("querySelectorAll not yet implemented");
    }
    get children() {
      const ids = core.ops.op_dom_get_child_elements(this._nodeId);
      return ids.map((id) => new Element(id));
    }
    get classList() {
      return {
        add() {},
        remove() {},
        contains() {
          return false;
        },
        toggle() {
          return false;
        },
      };
    }
  }

  class Document extends Node {
    constructor() {
      super(DOCUMENT_NODE);
    }
    get documentElement() {
      const children = this.childNodes;
      for (const c of children) {
        if (c instanceof Element) return c;
      }
      return null;
    }
    get body() {
      const html = this.documentElement;
      if (!html) return null;
      for (const c of html.childNodes) {
        if (c instanceof Element && c.tagName.toLowerCase() === 'body') return c;
      }
      return null;
    }
    get head() {
      return this.querySelector("head");
    }
    getElementById(id) {
      const nodeId = core.ops.op_dom_get_element_by_id(id);
      return nodeId === -1 ? null : new Element(nodeId);
    }
    getElementsByTagName(tag) {
      const ids = core.ops.op_dom_get_elements_by_tag_name(
        DOCUMENT_NODE,
        tag
      );
      return ids.map((id) => new Element(id));
    }
    createElement(tag) {
      const id = core.ops.op_dom_create_element(tag);
      return new Element(id);
    }
    createTextNode(text) {
      const id = core.ops.op_dom_create_text_node(text);
      return new TextNode(id);
    }
    createDocumentFragment() {
      const id = core.ops.op_dom_create_document_fragment();
      return new Element(id);
    }
    querySelector(selector) {
      throw new Error("querySelector not yet implemented");
    }
    querySelectorAll(selector) {
      throw new Error("querySelectorAll not yet implemented");
    }
    get URL() {
      return core.ops.op_dom_get_base_url();
    }
  }

  class TextNode extends Node {
    get data() {
      return core.ops.op_dom_get_text_content(this._nodeId);
    }
    set data(v) {
      core.ops.op_dom_set_text_content(this._nodeId, String(v));
    }
    get nodeName() {
      return "#text";
    }
  }

  function wrapNode(nodeId) {
    const type = core.ops.op_dom_get_node_type(nodeId);
    if (type === ELEMENT_NODE) return new Element(nodeId);
    if (type === TEXT_NODE) return new TextNode(nodeId);
    if (type === DOCUMENT_NODE_TYPE) return new Document();
    return new Node(nodeId);
  }

  globalThis.Node = Node;
  globalThis.Element = Element;
  globalThis.Document = Document;
  globalThis.Text = TextNode;
  globalThis.document = new Document();

  // Minimal getComputedStyle — reads CSS from inline <style> elements.
  globalThis.getComputedStyle = function(element) {
    if (!element || !element.tagName) return {};
    const tag = element.tagName.toLowerCase();
    const styles = {};
    const styleEls = document.getElementsByTagName('style');
    for (const styleEl of styleEls) {
      const css = styleEl.textContent || '';
      const regex = /([^{]+)\{([^}]+)\}/g;
      let match;
      while ((match = regex.exec(css)) !== null) {
        const selector = match[1].trim().toLowerCase();
        if (selector === tag) {
          const decls = match[2].split(';');
          for (const decl of decls) {
            const idx = decl.indexOf(':');
            if (idx > 0) {
              const prop = decl.substring(0, idx).trim();
              const val = decl.substring(idx + 1).trim();
              if (prop && val) styles[prop] = val;
            }
          }
        }
      }
    }
    return styles;
  };
})();
