use crate::tabs::TabId;
use crate::window_kind::TabKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPanelTab {
    pub tab_id: TabId,
    pub title: String,
    pub is_active: bool,
    pub kind: TabKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPanelGroup {
    pub id: usize,
    pub label: String,
    pub tabs: Vec<TabPanelTab>,
}

#[derive(Clone, Debug)]
pub enum TabPanelCommand {
    Focus(TabId),
    Move {
        tab_id: TabId,
        target_group: Option<usize>,
    },
}
