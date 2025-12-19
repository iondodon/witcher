use crate::types::WindowEntry;

#[derive(Default)]
pub struct MruState {
    order: Vec<u64>,
}

impl MruState {
    pub fn update_on_focus(&mut self, id: u64) {
        self.order.retain(|&existing| existing != id);
        self.order.insert(0, id);
        if self.order.len() > 256 {
            self.order.truncate(256);
        }
    }

    pub fn order_windows(&self, windows: Vec<WindowEntry>) -> Vec<WindowEntry> {
        let focused = windows.iter().find(|w| w.is_focused).map(|w| w.id);
        let mut order_index = std::collections::HashMap::new();
        for (idx, id) in self.order.iter().enumerate() {
            order_index.insert(*id, idx);
        }

        let mut ranked = Vec::with_capacity(windows.len());
        for (idx, window) in windows.into_iter().enumerate() {
            let rank = if Some(window.id) == focused {
                0usize
            } else if let Some(mru_idx) = order_index.get(&window.id) {
                1 + *mru_idx
            } else {
                1 + order_index.len() + idx
            };
            ranked.push((rank, idx, window));
        }

        ranked.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        ranked.into_iter().map(|(_, _, window)| window).collect()
    }
}
