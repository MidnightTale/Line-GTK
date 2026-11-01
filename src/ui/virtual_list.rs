use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Which end of the list a sentinel row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// The message a row belongs to, so a window of messages can be trimmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub id: String,
    pub created: i64,
}

/// Rows and message ids removed by a trim.
#[derive(Default)]
pub struct Trimmed {
    pub ids: Vec<String>,
    pub widgets: Vec<gtk::Widget>,
}

type EdgeHandler = Rc<dyn Fn(Edge)>;
type StagedRows = Vec<(gtk::Widget, Option<Owner>)>;

struct Edges {
    top: gtk::Box,
    bottom: gtk::Box,
    top_on: Cell<bool>,
    bottom_on: Cell<bool>,
    top_bound: Cell<bool>,
    bottom_bound: Cell<bool>,
    handler: RefCell<Option<EdgeHandler>>,
}

impl Edges {
    fn fire_later(self: &Rc<Self>, edge: Edge) {
        let edges = self.clone();
        // Never touch the model from inside a bind callback.
        gtk::glib::idle_add_local_once(move || {
            let handler = edges.handler.borrow().clone();
            if let Some(handler) = handler {
                handler(edge);
            }
        });
    }
}

fn sentinel() -> gtk::Box {
    let spinner = gtk::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Fill)
        .height_request(44)
        .css_classes(["line-history-sentinel"])
        .build();
    row.append(&spinner);
    row
}

fn set_spinning(row: &gtk::Box, on: bool) {
    if let Some(spinner) = row.first_child().and_downcast::<gtk::Spinner>() {
        spinner.set_spinning(on);
    }
}

/// A ListView-backed container for the existing rich message row widgets.
///
/// The model owns row widgets while the factory only attaches rows currently
/// needed by GTK. Optional sentinel rows at both ends report when they are
/// scrolled into view, which is how older or newer history gets requested.
#[derive(Clone)]
pub struct VirtualMessageList {
    view: gtk::ListView,
    store: gtk::gio::ListStore,
    owners: Rc<RefCell<Vec<Option<Owner>>>>,
    current_owner: Rc<RefCell<Option<Owner>>>,
    staging: Rc<RefCell<Option<StagedRows>>>,
    edges: Rc<Edges>,
}

impl VirtualMessageList {
    pub fn new() -> Self {
        let store = gtk::gio::ListStore::new::<gtk::glib::BoxedAnyObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let edges = Rc::new(Edges {
            top: sentinel(),
            bottom: sentinel(),
            top_on: Cell::new(false),
            bottom_on: Cell::new(false),
            top_bound: Cell::new(false),
            bottom_bound: Cell::new(false),
            handler: RefCell::new(None),
        });
        let factory = gtk::SignalListItemFactory::new();
        let bind_edges = edges.clone();
        factory.connect_bind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(item) = list_item.item().and_downcast::<gtk::glib::BoxedAnyObject>() else {
                return;
            };
            let widget = item.borrow::<gtk::Widget>().clone();
            list_item.set_child(Some(&widget));
            if widget == *bind_edges.top.upcast_ref::<gtk::Widget>() {
                bind_edges.top_bound.set(true);
                bind_edges.fire_later(Edge::Top);
            } else if widget == *bind_edges.bottom.upcast_ref::<gtk::Widget>() {
                bind_edges.bottom_bound.set(true);
                bind_edges.fire_later(Edge::Bottom);
            }
        });
        let unbind_edges = edges.clone();
        factory.connect_unbind(move |_, object| {
            if let Some(list_item) = object.downcast_ref::<gtk::ListItem>() {
                if let Some(child) = list_item.child() {
                    if child == *unbind_edges.top.upcast_ref::<gtk::Widget>() {
                        unbind_edges.top_bound.set(false);
                    } else if child == *unbind_edges.bottom.upcast_ref::<gtk::Widget>() {
                        unbind_edges.bottom_bound.set(false);
                    }
                }
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
        Self {
            view,
            store,
            owners: Rc::new(RefCell::new(Vec::new())),
            current_owner: Rc::new(RefCell::new(None)),
            staging: Rc::new(RefCell::new(None)),
            edges,
        }
    }

    pub fn widget(&self) -> &gtk::ListView {
        &self.view
    }

    pub fn set_edge_handler(&self, handler: impl Fn(Edge) + 'static) {
        *self.edges.handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// Rows added until the next `None` belong to this message.
    pub fn set_owner(&self, owner: Option<Owner>) {
        *self.current_owner.borrow_mut() = owner;
    }

    fn first_real(&self) -> usize {
        usize::from(self.edges.top_on.get())
    }

    fn end_real(&self) -> usize {
        self.owners.borrow().len() - usize::from(self.edges.bottom_on.get())
    }

    fn object(row: &gtk::Widget) -> gtk::glib::BoxedAnyObject {
        gtk::glib::BoxedAnyObject::new(row.clone())
    }

    pub fn append(&self, row: &impl IsA<gtk::Widget>) {
        let widget = row.clone().upcast::<gtk::Widget>();
        let owner = self.current_owner.borrow().clone();
        if let Some(staged) = self.staging.borrow_mut().as_mut() {
            staged.push((widget, owner));
            return;
        }
        let at = self.end_real();
        self.store.insert(at as u32, &Self::object(&widget));
        self.owners.borrow_mut().insert(at, owner);
    }

    /// Collect appended rows instead of showing them; `end_prepend` puts them
    /// above the current rows in a single model change.
    pub fn begin_prepend(&self) {
        *self.staging.borrow_mut() = Some(Vec::new());
    }

    pub fn end_prepend(&self) -> usize {
        let staged = self.staging.borrow_mut().take().unwrap_or_default();
        if staged.is_empty() {
            return 0;
        }
        let at = self.first_real();
        let objects: Vec<_> = staged.iter().map(|(w, _)| Self::object(w)).collect();
        self.store.splice(at as u32, 0, &objects);
        self.owners
            .borrow_mut()
            .splice(at..at, staged.into_iter().map(|(_, owner)| owner));
        objects.len()
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
                self.owners.borrow_mut().remove(index as usize);
                return;
            }
        }
    }

    /// Swap the rows of one message for freshly built ones. A day separator in
    /// front of the message stays. `before` sees the rows that are going away,
    /// `build` appends the new rows. Returns false when the message is not shown.
    pub fn replace_message(
        &self,
        id: &str,
        before: impl FnOnce(&[gtk::Widget]),
        build: impl FnOnce(),
    ) -> bool {
        if self.staging.borrow().is_some() {
            return false;
        }
        let (first, end) = (self.first_real(), self.end_real());
        let range = {
            let owners = self.owners.borrow();
            body_range(&owners, first, end, id, |index| {
                self.item_at(index as u32)
                    .is_some_and(|row| row.has_css_class("line-day-sep-row"))
            })
        };
        let Some(range) = range else {
            return false;
        };
        let Some(owner) = self.owners.borrow()[range.start].clone() else {
            return false;
        };
        let old: Vec<_> = range
            .clone()
            .filter_map(|index| self.item_at(index as u32))
            .collect();
        before(&old);

        *self.staging.borrow_mut() = Some(Vec::new());
        let previous = self.current_owner.replace(Some(owner.clone()));
        build();
        *self.current_owner.borrow_mut() = previous;
        let staged = self.staging.borrow_mut().take().unwrap_or_default();

        let objects: Vec<_> = staged.iter().map(|(w, _)| Self::object(w)).collect();
        self.store
            .splice(range.start as u32, range.len() as u32, &objects);
        self.owners.borrow_mut().splice(
            range,
            staged
                .into_iter()
                .map(|(_, row_owner)| row_owner.or_else(|| Some(owner.clone()))),
        );
        true
    }

    pub fn clear(&self) {
        self.store.remove_all();
        self.owners.borrow_mut().clear();
        *self.staging.borrow_mut() = None;
        *self.current_owner.borrow_mut() = None;
        self.edges.top_on.set(false);
        self.edges.bottom_on.set(false);
        self.edges.top_bound.set(false);
        self.edges.bottom_bound.set(false);
        set_spinning(&self.edges.top, false);
        set_spinning(&self.edges.bottom, false);
    }

    pub fn first_child(&self) -> Option<gtk::Widget> {
        let first = self.first_real();
        if first >= self.end_real() {
            return None;
        }
        self.item_at(first as u32)
    }

    pub fn queue_allocate(&self) {
        self.view.queue_allocate();
    }

    pub fn scroll_to_end(&self) {
        if let Some(last) = self.end_real().checked_sub(1) {
            self.view
                .scroll_to(last as u32, gtk::ListScrollFlags::NONE, None);
        }
    }

    /// Scroll to the content row, skipping a message's date separator if present.
    pub fn scroll_to_message(&self, id: &str) -> bool {
        let owners = self.owners.borrow();
        let Some(index) = message_row_index(&owners, id) else {
            return false;
        };
        self.view
            .scroll_to(index as u32, gtk::ListScrollFlags::FOCUS, None);
        true
    }

    pub fn set_sentinel(&self, edge: Edge, on: bool) {
        let (flag, row) = match edge {
            Edge::Top => (&self.edges.top_on, &self.edges.top),
            Edge::Bottom => (&self.edges.bottom_on, &self.edges.bottom),
        };
        if flag.get() == on {
            return;
        }
        let at = match (edge, on) {
            (Edge::Top, _) => 0,
            (Edge::Bottom, true) => self.owners.borrow().len(),
            (Edge::Bottom, false) => self.owners.borrow().len().saturating_sub(1),
        };
        if on {
            self.store
                .insert(at as u32, &Self::object(row.upcast_ref::<gtk::Widget>()));
            self.owners.borrow_mut().insert(at, None);
        } else {
            self.store.remove(at as u32);
            self.owners.borrow_mut().remove(at);
            set_spinning(row, false);
            match edge {
                Edge::Top => self.edges.top_bound.set(false),
                Edge::Bottom => self.edges.bottom_bound.set(false),
            }
        }
        flag.set(on);
    }

    pub fn set_sentinel_loading(&self, edge: Edge, loading: bool) {
        match edge {
            Edge::Top => set_spinning(&self.edges.top, loading),
            Edge::Bottom => set_spinning(&self.edges.bottom, loading),
        }
    }

    /// Whether a shown sentinel is currently attached to the view.
    pub fn sentinel_in_view(&self, edge: Edge) -> bool {
        match edge {
            Edge::Top => self.edges.top_on.get() && self.edges.top_bound.get(),
            Edge::Bottom => self.edges.bottom_on.get() && self.edges.bottom_bound.get(),
        }
    }

    /// Distinct messages currently in the list.
    pub fn message_count(&self) -> usize {
        count_messages(&self.owners.borrow())
    }

    /// First owner (oldest when `oldest`, newest otherwise) accepted by `keep`.
    pub fn edge_owner(&self, oldest: bool, keep: impl Fn(&Owner) -> bool) -> Option<Owner> {
        let owners = self.owners.borrow();
        let mut iter: Box<dyn Iterator<Item = &Option<Owner>>> = if oldest {
            Box::new(owners.iter())
        } else {
            Box::new(owners.iter().rev())
        };
        iter.find_map(|owner| owner.as_ref().filter(|o| keep(o)).cloned())
    }

    /// A row of the oldest or newest message, for anchoring the scroll position.
    pub fn edge_row(&self, oldest: bool) -> Option<gtk::Widget> {
        let (first, end) = (self.first_real(), self.end_real());
        let indexes: Box<dyn Iterator<Item = usize>> = if oldest {
            Box::new(first..end)
        } else {
            Box::new((first..end).rev())
        };
        let owners = self.owners.borrow();
        for index in indexes {
            if owners[index].is_some() {
                return self.item_at(index as u32);
            }
        }
        None
    }

    /// Drop the oldest messages until `keep` remain.
    pub fn trim_front(&self, keep: usize) -> Trimmed {
        self.trim(keep, true)
    }

    /// Drop the newest messages until `keep` remain.
    pub fn trim_back(&self, keep: usize) -> Trimmed {
        self.trim(keep, false)
    }

    fn trim(&self, keep: usize, front: bool) -> Trimmed {
        let (first, end) = (self.first_real(), self.end_real());
        let range = trim_range(&self.owners.borrow(), first, end, keep, front);
        let Some(range) = range else {
            return Trimmed::default();
        };
        let mut trimmed = Trimmed::default();
        for index in range.clone() {
            if let Some(widget) = self.item_at(index as u32) {
                trimmed.widgets.push(widget);
            }
        }
        let mut owners = self.owners.borrow_mut();
        for owner in owners.drain(range.clone()).flatten() {
            if trimmed.ids.last() != Some(&owner.id) {
                trimmed.ids.push(owner.id);
            }
        }
        drop(owners);
        self.store.splice(
            range.start as u32,
            range.len() as u32,
            &[] as &[gtk::glib::BoxedAnyObject],
        );
        trimmed
    }

    fn item_at(&self, index: u32) -> Option<gtk::Widget> {
        let item = self
            .store
            .item(index)
            .and_downcast::<gtk::glib::BoxedAnyObject>()?;
        Some(item.borrow::<gtk::Widget>().clone())
    }
}

/// Item range of a message's own rows, without a day separator in front.
fn body_range(
    owners: &[Option<Owner>],
    first: usize,
    end: usize,
    id: &str,
    is_separator: impl Fn(usize) -> bool,
) -> Option<std::ops::Range<usize>> {
    let matches = |index: usize| owners[index].as_ref().is_some_and(|owner| owner.id == id);
    let mut start = (first..end).find(|&index| matches(index))?;
    let mut stop = start;
    while stop < end && matches(stop) {
        stop += 1;
    }
    while start < stop && is_separator(start) {
        start += 1;
    }
    (start < stop).then_some(start..stop)
}

fn message_row_index(owners: &[Option<Owner>], id: &str) -> Option<usize> {
    // Message rows follow their optional separator; use the last matching row.
    owners
        .iter()
        .rposition(|owner| owner.as_ref().is_some_and(|owner| owner.id == id))
}

fn count_messages(owners: &[Option<Owner>]) -> usize {
    let mut count = 0;
    let mut previous: Option<&str> = None;
    for owner in owners.iter().flatten() {
        if previous != Some(owner.id.as_str()) {
            count += 1;
            previous = Some(owner.id.as_str());
        }
    }
    count
}

/// Item range to drop so only `keep` messages remain, taken from the front or
/// the back of `first..end`. Rows of one message are contiguous.
fn trim_range(
    owners: &[Option<Owner>],
    first: usize,
    end: usize,
    keep: usize,
    front: bool,
) -> Option<std::ops::Range<usize>> {
    let total = count_messages(&owners[first..end]);
    if total <= keep {
        return None;
    }
    let remove = total - keep;
    let indexes: Box<dyn Iterator<Item = usize>> = if front {
        Box::new(first..end)
    } else {
        Box::new((first..end).rev())
    };
    let mut seen = 0;
    let mut previous: Option<&str> = None;
    let mut stop = None;
    for index in indexes {
        if let Some(owner) = &owners[index]
            && previous != Some(owner.id.as_str())
        {
            if seen == remove {
                stop = Some(index);
                break;
            }
            seen += 1;
            previous = Some(owner.id.as_str());
        }
    }
    let range = match (front, stop) {
        (true, Some(index)) => first..index,
        (false, Some(index)) => index + 1..end,
        (_, None) => first..end,
    };
    (!range.is_empty()).then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(id: &str) -> Option<Owner> {
        Some(Owner {
            id: id.into(),
            created: 0,
        })
    }

    // sentinel, [sep a, row a], [row b], [sep c, row c], new-marker, sentinel
    fn items() -> Vec<Option<Owner>> {
        vec![
            None,
            owner("a"),
            owner("a"),
            owner("b"),
            owner("c"),
            owner("c"),
            None,
            None,
        ]
    }

    #[test]
    fn navigation_skips_separators_and_missing_messages() {
        let rows = items();
        assert_eq!(message_row_index(&rows, "a"), Some(2));
        assert_eq!(message_row_index(&rows, "b"), Some(3));
        assert_eq!(message_row_index(&rows, "c"), Some(5));
        assert_eq!(message_row_index(&rows, "missing"), None);
        assert_eq!(message_row_index(&[], "a"), None);
    }

    #[test]
    fn counts_each_message_once() {
        assert_eq!(count_messages(&items()), 3);
        assert_eq!(count_messages(&[]), 0);
    }

    #[test]
    fn trims_oldest_messages_with_all_of_their_rows() {
        let items = items();
        assert_eq!(trim_range(&items, 1, 7, 2, true), Some(1..3));
        assert_eq!(trim_range(&items, 1, 7, 1, true), Some(1..4));
    }

    #[test]
    fn trims_newest_messages_with_all_of_their_rows() {
        let items = items();
        assert_eq!(trim_range(&items, 1, 7, 2, false), Some(4..7));
        assert_eq!(trim_range(&items, 1, 7, 1, false), Some(3..7));
    }

    #[test]
    fn nothing_is_trimmed_within_the_limit() {
        let items = items();
        assert_eq!(trim_range(&items, 1, 7, 3, true), None);
        assert_eq!(trim_range(&items, 1, 7, 9, false), None);
    }

    #[test]
    fn trimming_everything_removes_the_whole_range() {
        let items = items();
        assert_eq!(trim_range(&items, 1, 7, 0, true), Some(1..7));
        assert_eq!(trim_range(&items, 1, 7, 0, false), Some(1..7));
    }

    #[test]
    fn replacing_a_message_keeps_its_day_separator() {
        let items = items();
        // "a" starts with a separator at index 1, "c" at index 4.
        assert_eq!(body_range(&items, 1, 7, "a", |i| i == 1), Some(2..3));
        assert_eq!(body_range(&items, 1, 7, "c", |i| i == 4), Some(5..6));
        assert_eq!(body_range(&items, 1, 7, "b", |_| false), Some(3..4));
        assert_eq!(body_range(&items, 1, 7, "missing", |_| false), None);
    }

    #[test]
    fn checked_last_index_handles_empty_models() {
        assert_eq!(0_u32.checked_sub(1), None);
        assert_eq!(3_u32.checked_sub(1), Some(2));
    }
}
