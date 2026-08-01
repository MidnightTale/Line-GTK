use gtk::prelude::*;

/// A ListView-backed container for the existing rich message row widgets.
///
/// The model owns row widgets while the factory only attaches rows currently
/// needed by GTK. This avoids keeping every row in the rendered widget tree and
/// provides an incremental migration path toward data-only message items.
#[derive(Clone)]
pub struct VirtualMessageList {
    view: gtk::ListView,
    store: gtk::gio::ListStore,
}

impl VirtualMessageList {
    pub fn new() -> Self {
        let store = gtk::gio::ListStore::new::<gtk::glib::BoxedAnyObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_bind(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(item) = list_item.item().and_downcast::<gtk::glib::BoxedAnyObject>() else {
                return;
            };
            let widget = item.borrow::<gtk::Widget>().clone();
            list_item.set_child(Some(&widget));
        });
        factory.connect_unbind(|_, object| {
            if let Some(list_item) = object.downcast_ref::<gtk::ListItem>() {
                list_item.set_child(None::<&gtk::Widget>);
            }
        });
        let view = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .css_classes(["line-msg-list"])
            .valign(gtk::Align::End)
            .margin_start(8)
            .margin_end(8)
            .margin_top(8)
            .margin_bottom(8)
            .single_click_activate(false)
            .build();
        Self { view, store }
    }

    pub fn widget(&self) -> &gtk::ListView {
        &self.view
    }

    pub fn append(&self, row: &impl IsA<gtk::Widget>) {
        let widget = row.clone().upcast::<gtk::Widget>();
        self.store.append(&gtk::glib::BoxedAnyObject::new(widget));
    }

    pub fn remove(&self, row: &impl IsA<gtk::Widget>) {
        let target = row.clone().upcast::<gtk::Widget>();
        for index in 0..self.store.n_items() {
            let Some(item) = self
                .store
                .item(index)
                .and_downcast::<gtk::glib::BoxedAnyObject>()
            else {
                continue;
            };
            if *item.borrow::<gtk::Widget>() == target {
                self.store.remove(index);
                return;
            }
        }
    }

    pub fn clear(&self) {
        self.store.remove_all();
    }

    pub fn first_child(&self) -> Option<gtk::Widget> {
        self.item_at(0)
    }

    pub fn queue_allocate(&self) {
        self.view.queue_allocate();
    }

    pub fn scroll_to_end(&self) {
        if let Some(last) = self.store.n_items().checked_sub(1) {
            self.view.scroll_to(last, gtk::ListScrollFlags::NONE, None);
        }
    }

    fn item_at(&self, index: u32) -> Option<gtk::Widget> {
        let item = self
            .store
            .item(index)
            .and_downcast::<gtk::glib::BoxedAnyObject>()?;
        Some(item.borrow::<gtk::Widget>().clone())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked_last_index_handles_empty_models() {
        assert_eq!(0_u32.checked_sub(1), None);
        assert_eq!(3_u32.checked_sub(1), Some(2));
    }
}
