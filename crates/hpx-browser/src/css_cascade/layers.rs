pub type LayerId = u32;

pub struct LayerOrder {
    name_to_id: std::collections::HashMap<String, LayerId>,
    order: Vec<LayerId>,
    next_id: LayerId,
}

impl LayerOrder {
    pub fn new() -> Self {
        Self {
            name_to_id: std::collections::HashMap::new(),
            order: Vec::new(),
            next_id: 1,
        }
    }

    pub fn register(&mut self, name: &str) -> LayerId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.name_to_id.insert(name.to_string(), id);
        self.order.push(id);
        id
    }

    pub fn get(&self, name: &str) -> Option<LayerId> {
        self.name_to_id.get(name).copied()
    }

    pub fn compare(&self, a: LayerId, b: LayerId) -> std::cmp::Ordering {
        let a_pos = self.order.iter().position(|&id| id == a);
        let b_pos = self.order.iter().position(|&id| id == b);
        a_pos.cmp(&b_pos)
    }
}

impl Default for LayerOrder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    #[test]
    fn layer_ordering() {
        let mut layers = LayerOrder::new();
        let base = layers.register("base");
        let components = layers.register("components");
        assert_eq!(layers.compare(base, components), Ordering::Less);
    }
}
