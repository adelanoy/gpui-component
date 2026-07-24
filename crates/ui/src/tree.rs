use crate::{
    Selectable as _, StyledExt,
    actions::{Confirm, SelectDown, SelectLeft, SelectRight, SelectUp},
    list::ListItem,
    menu::{ContextMenuExt as _, PopupMenu},
    scroll::ScrollableElement,
};
use gpui::{
    App, Context, ElementId, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, ListSizingBehavior, MouseButton, ParentElement, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, uniform_list,
};
use std::collections::VecDeque;
use std::fmt::Debug;
use std::{cell::RefCell, ops::Range, rc::Rc};

const CONTEXT: &str = "Tree";
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(CONTEXT)),
    ]);
}

/// Create a [`Tree`].
///
/// # Arguments
///
/// * `state` - The shared state managing the tree items.
/// * `render_item` - A closure to render each tree item.
///
/// ```ignore
/// let state = cx.new(|_| {
///     TreeState::new().items(vec![
///         TreeItem::new("src")
///             .child(TreeItem::new("lib.rs"),
///         TreeItem::new("Cargo.toml"),
///         TreeItem::new("README.md"),
///     ])
/// });
///
/// tree(&state, |ix, entry, selected, window, cx| {
///     let item = entry.item();
///     ListItem::new(ix).pl(px(16.) * entry.depth()).child(item.label.clone())
/// })
/// ```
pub fn tree<R, I>(state: &Entity<TreeState<I>>, render_item: R) -> Tree<I>
where
    I: Clone + Eq,
    R: Fn(usize, &TreeEntry<I>, bool, &mut Window, &mut Context<TreeState<I>>) -> ListItem + 'static,
{
    Tree::new(state, render_item)
}

struct TreeItemState {
    expanded: bool,
    disabled: bool,
}

/// A tree item with a label, children, and an expanded state.
#[derive(Clone)]
pub struct TreeItem<I>
where
    I: Clone + Eq,
{
    pub id: Vec<I>,
    pub label: SharedString,
    pub children: Vec<TreeItem<I>>,
    state: Rc<RefCell<TreeItemState>>,
}

/// A flat representation of a tree item with its depth.
#[derive(Clone)]
pub struct TreeEntry<I>
where
    I: Clone + Eq,
{
    item: TreeItem<I>,
    depth: usize,
}

impl<I> TreeEntry<I>
where
    I: Clone + Eq,
{
    /// Get the source tree item.
    #[inline]
    pub fn item(&self) -> &TreeItem<I> {
        &self.item
    }

    /// The depth of this item in the tree.
    #[inline]
    pub fn depth(&self) -> usize {
        self.depth
    }

    #[inline]
    fn is_root(&self) -> bool {
        self.depth == 0
    }

    /// Whether this item is a folder (has children).
    #[inline]
    pub fn is_folder(&self) -> bool {
        self.item.is_folder()
    }

    /// Return true if the item is expanded.
    #[inline]
    pub fn is_expanded(&self) -> bool {
        self.item.is_expanded()
    }

    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.item.is_disabled()
    }
}

/// Event emitted by [`TreeState`] when user-visible state changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeEvent<I>
where
    I: Clone + Eq,
{
    /// A tree node was expanded.
    Expanded(Vec<I>),
    /// A tree node was collapsed.
    Collapsed(Vec<I>),
}

impl<I> TreeItem<I>
where
    I: Clone + Eq,
{
    /// Create a new tree item with the given label.
    ///
    /// - The `id` for you to uniquely identify this item, then later you can use it for selection or other purposes.
    /// - The `label` is the text to display for this item.
    ///
    /// For example, the `id` is the full file path, and the `label` is the file name.
    ///
    /// ```ignore
    /// TreeItem::new("src/ui/button.rs", "button.rs")
    /// ```
    pub fn new(id: Vec<I>, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            label: label.into(),
            children: Vec::new(),
            state: Rc::new(RefCell::new(TreeItemState {
                expanded: false,
                disabled: false,
            })),
        }
    }

    /// Add a child item to this tree item.
    pub fn child(mut self, child: TreeItem<I>) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple child items to this tree item.
    pub fn children(mut self, children: impl IntoIterator<Item = TreeItem<I>>) -> Self {
        self.children.extend(children);
        self
    }

    /// Set expanded state for this tree item.
    pub fn expanded(self, expanded: bool) -> Self {
        self.state.borrow_mut().expanded = expanded;
        self
    }

    /// Set disabled state for this tree item.
    pub fn disabled(self, disabled: bool) -> Self {
        self.state.borrow_mut().disabled = disabled;
        self
    }

    /// Whether this item is a folder (has children).
    #[inline]
    pub fn is_folder(&self) -> bool {
        self.children.len() > 0
    }

    /// Return true if the item is disabled.
    pub fn is_disabled(&self) -> bool {
        self.state.borrow().disabled
    }

    /// Return true if the item is expanded.
    #[inline]
    pub fn is_expanded(&self) -> bool {
        self.state.borrow().expanded
    }

    fn find_ancestors(&self, target_id: &mut VecDeque<I>) -> Option<Vec<TreeItem<I>>> {
        let Some(child) = self
            .children
            .iter()
            .find(|entry| entry.id.last() == target_id.get(0))
        else {
            return None;
        };

        if target_id.len() == 1 {
            return if target_id.get(0) == child.id.last() {
                Some(vec![self.clone()])
            } else {
                None
            };
        };

        target_id.pop_front();
        if let Some(mut path) = child.find_ancestors(target_id) {
            path.push(self.clone());
            return Some(path);
        }

        None
    }
}

/// State for managing tree items.
pub struct TreeState<I>
where
    I: Clone + Eq,
{
    focus_handle: FocusHandle,
    entries: Vec<TreeEntry<I>>,
    scroll_handle: UniformListScrollHandle,
    selected_ix: Option<usize>,
    right_clicked_ix: Option<usize>,
    render_item: Rc<dyn Fn(usize, &TreeEntry<I>, bool, &mut Window, &mut Context<TreeState<I>>) -> ListItem>,
    context_menu_builder: Option<
        Rc<
            dyn Fn(
                usize,
                &TreeEntry<I>,
                PopupMenu,
                &mut Window,
                &mut Context<TreeState<I>>,
            ) -> PopupMenu,
        >,
    >,
}

impl<I> EventEmitter<TreeEvent<I>> for TreeState<I> where I: Clone + Eq + 'static {}

impl<I> TreeState<I>
where
    I: Clone + Eq + 'static,
{
    /// Create a new empty tree state.
    pub fn new(cx: &mut App) -> Self {
        Self {
            selected_ix: None,
            right_clicked_ix: None,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
            entries: Vec::new(),
            render_item: Rc::new(|_, _, _, _, _| ListItem::new(0)),
            context_menu_builder: None,
        }
    }

    /// Set the tree items.
    pub fn items(mut self, items: impl Into<Vec<TreeItem<I>>>) -> Self {
        let items = items.into();
        self.entries.clear();
        for item in items.into_iter() {
            self.add_entry(item, 0);
        }
        self
    }

    /// Set the tree items.
    pub fn set_items(&mut self, items: impl Into<Vec<TreeItem<I>>>, cx: &mut Context<Self>) {
        let items = items.into();
        self.entries.clear();
        for item in items.into_iter() {
            self.add_entry(item, 0);
        }
        self.selected_ix = None;
        self.right_clicked_ix = None;
        cx.notify();
    }

    /// Get the currently selected index, if any.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_ix
    }

    /// Set the selected index, or `None` to clear selection.
    pub fn set_selected_index(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.selected_ix = ix;
        cx.notify();
    }

    /// Set the selected index by tree item, or `None` to clear selection.
    pub fn set_selected_item(&mut self, item: Option<&TreeItem<I>>, cx: &mut Context<Self>) {
        if let Some(item) = item {
            let ix = self
                .entries
                .iter()
                .position(|entry| entry.item.id == item.id);
            if ix.is_some() {
                self.selected_ix = ix;
            } else {
                self.expand_ancestors(item.id.clone().into(), cx);
                self.selected_ix = self
                    .entries
                    .iter()
                    .position(|entry| entry.item.id == item.id);
            }
        } else {
            self.selected_ix = None;
        }
        cx.notify();
    }

    /// Get the currently selected tree item, if any.
    pub fn selected_item(&self) -> Option<&TreeItem<I>> {
        self.selected_ix
            .and_then(|ix| self.entries.get(ix).map(|entry| &entry.item))
    }

    pub fn scroll_to_item(&mut self, ix: usize, strategy: gpui::ScrollStrategy) {
        self.scroll_handle.scroll_to_item(ix, strategy);
    }

    /// Find the flat index of the entry whose `item.id` matches, if present.
    pub(crate) fn index_of(&self, id: &I) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.item.id.last() == Some(id))
    }

    /// Expand all ancestors of the node with `id` and scroll it into view.
    /// No-op if `id` is not found. Does not change the selected index.
    pub fn reveal_item(
        &mut self,
        id: &VecDeque<I>,
        strategy: gpui::ScrollStrategy,
        cx: &mut Context<Self>,
    ) {
        self.expand_ancestors(id.clone(), cx);
        let Some(id) = id.get(0) else {
            return;
        };
        if let Some(ix) = self.index_of(id) {
            self.scroll_to_item(ix, strategy);
        }
    }

    /// Get the currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&TreeEntry<I>> {
        self.selected_ix.and_then(|ix| self.entries.get(ix))
    }

    fn expand_ancestors(&mut self, mut target_id: VecDeque<I>, cx: &mut Context<Self>) {
        let mut ancestors = Vec::new();

        // find the root entry whose path must be only one segment, and the first segment of the target path
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.item.id.last() == target_id.get(0))
        else {
            return;
        };

        target_id.pop_front();
        if let Some(found_ancestors) = entry.item.find_ancestors(&mut target_id) {
            ancestors = found_ancestors;
        }

        if ancestors.is_empty() {
            return;
        }

        for ancestor in ancestors.into_iter().rev() {
            if !ancestor.is_expanded() {
                ancestor.state.borrow_mut().expanded = true;
                cx.emit(TreeEvent::Expanded(ancestor.id.clone()));
            }
        }

        self.rebuild_entries();
    }

    fn add_entry(&mut self, item: TreeItem<I>, depth: usize) {
        self.entries.push(TreeEntry {
            item: item.clone(),
            depth,
        });
        if item.is_expanded() {
            for child in &item.children {
                self.add_entry(child.clone(), depth + 1);
            }
        }
    }

    fn toggle_expand(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get_mut(ix) else {
            return;
        };
        if !entry.is_folder() {
            return;
        }

        let expanded = !entry.is_expanded();
        let id = entry.item.id.clone();
        entry.item.state.borrow_mut().expanded = expanded;

        if expanded {
            cx.emit(TreeEvent::Expanded(id));
        } else {
            cx.emit(TreeEvent::Collapsed(id));
        }

        self.right_clicked_ix = None;
        self.rebuild_entries();
    }

    fn rebuild_entries(&mut self) {
        let root_items: Vec<TreeItem<I>> = self
            .entries
            .iter()
            .filter(|e| e.is_root())
            .map(|e| e.item.clone())
            .collect();
        self.entries.clear();
        for item in root_items.into_iter() {
            self.add_entry(item, 0);
        }
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    fn on_action_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected_ix) = self.selected_ix {
            if let Some(entry) = self.entries.get(selected_ix) {
                if entry.is_folder() {
                    self.toggle_expand(selected_ix, cx);
                    cx.notify();
                }
            }
        }
    }

    fn on_action_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected_ix) = self.selected_ix {
            if let Some(entry) = self.entries.get(selected_ix) {
                if entry.is_folder() && entry.is_expanded() {
                    self.toggle_expand(selected_ix, cx);
                    cx.notify();
                }
            }
        }
    }

    fn on_action_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected_ix) = self.selected_ix {
            if let Some(entry) = self.entries.get(selected_ix) {
                if entry.is_folder() && !entry.is_expanded() {
                    self.toggle_expand(selected_ix, cx);
                    cx.notify();
                }
            }
        }
    }

    fn on_action_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let mut selected_ix = self.selected_ix.unwrap_or(0);

        if selected_ix > 0 {
            selected_ix = selected_ix - 1;
        } else {
            selected_ix = self.entries.len().saturating_sub(1);
        }

        self.selected_ix = Some(selected_ix);
        self.scroll_handle
            .scroll_to_item(selected_ix, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    fn on_action_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let mut selected_ix = self.selected_ix.unwrap_or(0);
        if selected_ix + 1 < self.entries.len() {
            selected_ix = selected_ix + 1;
        } else {
            selected_ix = 0;
        }

        self.selected_ix = Some(selected_ix);
        self.scroll_handle
            .scroll_to_item(selected_ix, gpui::ScrollStrategy::Bottom);
        cx.notify();
    }

    fn on_entry_click(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_ix = Some(ix);
        self.toggle_expand(ix, cx);
        cx.notify();
    }
}

impl<I> Render for TreeState<I>
where
    I: Clone + Eq + 'static,
{
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_item = self.render_item.clone();
        let state = cx.entity().clone();

        div()
            .id("tree-state")
            .size_full()
            .relative()
            .context_menu({
                let state = state.clone();
                move |menu, window, cx: &mut Context<PopupMenu>| {
                    if state.read(cx).context_menu_builder.is_none() {
                        return menu;
                    }

                    let (ix, entry) = {
                        let state = state.read(cx);
                        let entry = state
                            .right_clicked_ix
                            .and_then(|ix| state.entries.get(ix).cloned());
                        (state.right_clicked_ix, entry)
                    };

                    if let (Some(ix), Some(entry)) = (ix, entry) {
                        state.update(cx, |state, cx| {
                            if let Some(build) = state.context_menu_builder.clone() {
                                build(ix, &entry, menu, window, cx)
                            } else {
                                menu
                            }
                        })
                    } else {
                        menu
                    }
                }
            })
            .child(
                uniform_list("entries", self.entries.len(), {
                    cx.processor(move |state, visible_range: Range<usize>, window, cx| {
                        let mut items = Vec::with_capacity(visible_range.len());
                        for ix in visible_range {
                            let entry = &state.entries[ix];
                            let selected = Some(ix) == state.selected_ix;
                            let right_clicked = Some(ix) == state.right_clicked_ix;
                            let item = (render_item)(ix, entry, selected, window, cx);

                            let el = div()
                                .id(ix)
                                .child(
                                    item.disabled(entry.item().is_disabled())
                                        .selected(selected)
                                        .secondary_selected(right_clicked),
                                )
                                .when(!entry.item().is_disabled(), |this| {
                                    this.on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener({
                                            move |this, _, window, cx| {
                                                this.on_entry_click(ix, window, cx);
                                            }
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |this, _, _, cx| {
                                            this.right_clicked_ix = Some(ix);
                                            cx.notify();
                                        }),
                                    )
                                });

                            items.push(el)
                        }

                        items
                    })
                })
                .flex_grow_1()
                .size_full()
                .track_scroll(&self.scroll_handle)
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .into_any_element(),
            )
    }
}

/// A tree view element that displays hierarchical data.
#[derive(IntoElement)]
pub struct Tree<I>
where
    I: Clone + Eq + 'static,
{
    id: ElementId,
    state: Entity<TreeState<I>>,
    style: StyleRefinement,
    render_item: Rc<dyn Fn(usize, &TreeEntry<I>, bool, &mut Window, &mut Context<TreeState<I>>) -> ListItem>,
    context_menu_builder: Option<
        Rc<
            dyn Fn(
                usize,
                &TreeEntry<I>,
                PopupMenu,
                &mut Window,
                &mut Context<TreeState<I>>,
            ) -> PopupMenu,
        >,
    >,
}

impl<I> Tree<I>
where
    I: Clone + Eq,
{
    pub fn new<R>(state: &Entity<TreeState<I>>, render_item: R) -> Self
    where
        R: Fn(usize, &TreeEntry<I>, bool, &mut Window, &mut Context<TreeState<I>>) -> ListItem + 'static,
    {
        Self {
            id: ElementId::Name(format!("tree-{}", state.entity_id()).into()),
            state: state.clone(),
            style: StyleRefinement::default(),
            render_item: Rc::new(move |ix, item, selected, window, app| {
                render_item(ix, item, selected, window, app)
            }),
            context_menu_builder: None,
        }
    }

    /// Add a context menu to the tree.
    ///
    /// The closure receives:
    /// - `ix`: the index of the right-clicked entry
    /// - `entry`: the right-clicked tree entry
    /// - `menu`: the popup menu builder
    pub fn context_menu<F>(mut self, f: F) -> Self
    where
        F: Fn(
                usize,
                &TreeEntry<I>,
                PopupMenu,
                &mut Window,
                &mut Context<TreeState<I>>,
            ) -> PopupMenu
            + 'static,
    {
        self.context_menu_builder = Some(Rc::new(f));
        self
    }
}

impl<I> Styled for Tree<I>
where
    I: Clone + Eq,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<I> RenderOnce for Tree<I>
where
    I: Clone + Eq + 'static,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let scroll_handle = self.state.read(cx).scroll_handle.clone();

        self.state.update(cx, |state, _| {
            state.render_item = self.render_item;
            state.context_menu_builder = self.context_menu_builder;
        });

        div()
            .id(self.id)
            .key_context(CONTEXT)
            .track_focus(&focus_handle)
            .on_action(window.listener_for(&self.state, TreeState::on_action_confirm))
            .on_action(window.listener_for(&self.state, TreeState::on_action_left))
            .on_action(window.listener_for(&self.state, TreeState::on_action_right))
            .on_action(window.listener_for(&self.state, TreeState::on_action_up))
            .on_action(window.listener_for(&self.state, TreeState::on_action_down))
            .size_full()
            .child(self.state)
            .refine_style(&self.style)
            .vertical_scrollbar(&scroll_handle)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use indoc::indoc;

    use super::{TreeEvent, TreeState};
    use gpui::{AppContext as _, Render, SharedString, Subscription};

    struct TestCollector<I>
    where
        I: Clone + Eq,
    {
        _state: gpui::Entity<TreeState<I>>,
        events: Rc<RefCell<Vec<TreeEvent<I>>>>,
        _subscription: Subscription,
    }

    impl<I> TestCollector<I>
    where
        I: Clone + Eq + 'static,
    {
        fn new(state: &gpui::Entity<TreeState<I>>, cx: &mut gpui::Context<Self>) -> Self {
            let events = Rc::new(RefCell::new(Vec::new()));
            let events_clone = events.clone();
            let _subscription = cx.subscribe(state, move |_, _, ev: &TreeEvent<I>, _| {
                events_clone.borrow_mut().push(ev.clone());
            });
            Self {
                _state: state.clone(),
                events,
                _subscription,
            }
        }
    }

    impl<I> Render for TestCollector<I>
    where
        I: Clone + Eq + 'static,
    {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    fn assert_entries<I>(entries: &Vec<super::TreeEntry<I>>, expected: &str)
    where
        I: Clone + Eq + 'static,
    {
        let actual: Vec<String> = entries
            .iter()
            .map(|e| {
                let mut s = String::new();
                s.push_str(&"    ".repeat(e.depth));
                s.push_str(e.item().label.as_str());
                s
            })
            .collect();
        let actual = actual.join("\n");
        assert_eq!(actual.trim(), expected.trim());
    }

    #[gpui::test]
    fn test_tree_entry(cx: &mut gpui::TestAppContext) {
        use super::TreeItem;

        let items = vec![
            TreeItem::<SharedString>::new(vec!["src".into()], "src")
                .expanded(true)
                .child(
                    TreeItem::new(vec!["src".into(), "ui".into()], "ui")
                        .expanded(true)
                        .child(TreeItem::new(
                            vec!["src".into(), "ui".into(), "button.rs".into()],
                            "button.rs",
                        ))
                        .child(TreeItem::new(
                            vec!["src".into(), "ui".into(), "icon.rs".into()],
                            "icon.rs",
                        ))
                        .child(TreeItem::new(
                            vec!["src".into(), "ui".into(), "mod.rs".into()],
                            "mod.rs",
                        )),
                )
                .child(TreeItem::new(vec!["src/lib.rs".into()], "lib.rs")),
            TreeItem::new(vec!["Cargo.toml".into()], "Cargo.toml"),
            TreeItem::new(vec!["Cargo.lock".into()], "Cargo.lock").disabled(true),
            TreeItem::new(vec!["README.md".into()], "README.md"),
        ];

        let state = cx.new(|cx| TreeState::new(cx).items(items));
        state.update(cx, |state, cx| {
            assert_entries(
                &state.entries,
                indoc! {
                    r#"
                src
                    ui
                        button.rs
                        icon.rs
                        mod.rs
                    lib.rs
                Cargo.toml
                Cargo.lock
                README.md
                "#
                },
            );

            let entry = state.entries.get(0).unwrap();
            assert_eq!(entry.depth(), 0);
            assert_eq!(entry.is_root(), true);
            assert_eq!(entry.is_folder(), true);
            assert_eq!(entry.is_expanded(), true);

            let entry = state.entries.get(1).unwrap();
            assert_eq!(entry.depth(), 1);
            assert_eq!(entry.is_root(), false);
            assert_eq!(entry.is_folder(), true);
            assert_eq!(entry.is_expanded(), true);
            assert_eq!(entry.item().label.as_str(), "ui");

            state.toggle_expand(1, cx);
            let entry = state.entries.get(1).unwrap();
            assert_eq!(entry.is_expanded(), false);
            assert_entries(
                &state.entries,
                indoc! {
                    r#"
                src
                    ui
                    lib.rs
                Cargo.toml
                Cargo.lock
                README.md
                "#
                },
            );
        })
    }

    #[gpui::test]
    fn test_emits_expanded_event(cx: &mut gpui::TestAppContext) {
        let expanded_id = vec!["src".into()];
        let items = vec![
            super::TreeItem::<SharedString>::new(expanded_id.clone(), "src").child(
                super::TreeItem::new(vec!["src".into(), "lib.rs".into()], "lib.rs"),
            ),
        ];
        let state = cx.new(|cx| TreeState::new(cx).items(items));
        let collector = cx.new(|cx| TestCollector::new(&state, cx));

        state.update(cx, |state, cx| {
            state.toggle_expand(0, cx);
        });

        let events = collector.read_with(cx, |c, _| c.events.borrow().clone());
        assert_eq!(events, vec![TreeEvent::Expanded(expanded_id)]);
    }

    #[gpui::test]
    fn test_emits_collapsed_event(cx: &mut gpui::TestAppContext) {
        let collapsed_id = vec!["src".into()];
        let items = vec![
            super::TreeItem::<SharedString>::new(collapsed_id.clone(), "src")
                .expanded(true)
                .child(super::TreeItem::new(
                    vec!["src".into(), "lib.rs".into()],
                    "lib.rs",
                )),
        ];
        let state = cx.new(|cx| TreeState::new(cx).items(items));
        let collector = cx.new(|cx| TestCollector::new(&state, cx));

        state.update(cx, |state, cx| {
            state.toggle_expand(0, cx);
        });

        let events = collector.read_with(cx, |c, _| c.events.borrow().clone());
        assert_eq!(events, vec![TreeEvent::Collapsed(collapsed_id)]);
    }

    #[gpui::test]
    fn test_set_items_does_not_emit_expansion_events(cx: &mut gpui::TestAppContext) {
        let items = vec![
            super::TreeItem::<SharedString>::new(vec!["src".into()], "src")
                .expanded(true)
                .child(super::TreeItem::new(
                    vec!["src".into(), "lib.rs".into()],
                    "lib.rs",
                )),
        ];
        let state = cx.new(|cx| TreeState::new(cx).items(items));
        let collector = cx.new(|cx| TestCollector::new(&state, cx));

        let new_items = vec![
            super::TreeItem::new(vec!["docs".into()], "docs")
                .expanded(true)
                .child(super::TreeItem::new(
                    vec!["docs".into(), "readme.md".into()],
                    "readme.md",
                )),
        ];
        state.update(cx, |state, cx| {
            state.set_items(new_items, cx);
        });

        let events = collector.read_with(cx, |c, _| c.events.borrow().clone());
        assert!(
            events.is_empty(),
            "set_items should not emit Expanded/Collapsed events"
        );
    }

    #[gpui::test]
    fn test_event_carries_item_id(cx: &mut gpui::TestAppContext) {
        let expanded_id = vec!["src".into(), "ui".into()];
        let items = vec![
            super::TreeItem::<SharedString>::new(vec!["src".into()], "src")
                .expanded(true)
                .child(super::TreeItem::new(expanded_id.clone(), "ui").child(
                    super::TreeItem::new(
                        vec!["src".into(), "ui".into(), "button.rs".into()],
                        "button.rs",
                    ),
                )),
        ];
        let state = cx.new(|cx| TreeState::new(cx).items(items));
        let collector = cx.new(|cx| TestCollector::new(&state, cx));

        // Toggle the child at index 1 ("src/ui"), event payload should be the id not the index.
        state.update(cx, |state, cx| {
            state.toggle_expand(1, cx);
        });

        let events = collector.read_with(cx, |c, _| c.events.borrow().clone());
        assert_eq!(events, vec![TreeEvent::Expanded(expanded_id)]);
    }

    #[gpui::test]
    fn test_set_selected_item_emits_expanded_events_for_hidden_ancestors(
        cx: &mut gpui::TestAppContext,
    ) {
        let target = super::TreeItem::new(vec![1, 10, 100], "button.rs");
        let src_id = vec![1];
        let mut ui_id = src_id.clone();
        ui_id.push(10);
        let items = vec![
            super::TreeItem::new(src_id.clone(), "src")
                .child(super::TreeItem::new(ui_id.clone(), "ui").child(target.clone())),
        ];
        let state = cx.new(|cx| TreeState::new(cx).items(items));
        let collector = cx.new(|cx| TestCollector::new(&state, cx));

        state.update(cx, |state, cx| {
            state.set_selected_item(Some(&target), cx);
        });

        let events = collector.read_with(cx, |c, _| c.events.borrow().clone());
        assert_eq!(
            events,
            vec![TreeEvent::Expanded(src_id), TreeEvent::Expanded(ui_id)]
        );
    }
}
