use std::time::Instant;

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton};
use winit::keyboard::{Key, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::CursorIcon;

use unicode_width::UnicodeWidthChar;

#[cfg(target_os = "macos")]
use crossfont::GlyphKey;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::MIN_COLUMNS;

use crate::config::UiConfig;
use crate::display::color::Rgb;
use crate::display::SizeInfo;
use crate::renderer::rects::RenderRect;
use crate::renderer::{GlyphCache, Renderer};
use crate::tab_panel::{TabPanelCommand, TabPanelGroup, TabPanelTab};
use crate::tabs::TabId;

const RESIZE_HANDLE_WIDTH_PX: f64 = 6.0;
const PANEL_ICON_SCALE: f32 = 2.0;
const PANEL_ROW_PADDING_PX: f32 = 4.0;
const ACTIVITY_INDICATOR_COLS: usize = 2;
const ACTIVITY_INDICATOR_FILLED: char = '\u{25CF}';
const ACTIVITY_INDICATOR_OUTLINE: char = '\u{25CB}';

#[derive(Default, Clone, Copy)]
pub struct PanelDimensions {
    pub columns: usize,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPanelEditTarget {
    Tab(TabId),
    Group(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPanelEditCommit {
    pub target: TabPanelEditTarget,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabPanelEditOutcome {
    None,
    Changed,
    Commit(TabPanelEditCommit),
    Cancelled,
}

pub fn compute_panel_dimensions(
    config: &UiConfig,
    cell_width: f32,
    viewport_width: f32,
    padding_x: f32,
    scale_factor: f32,
) -> PanelDimensions {
    if !config.window.tab_panel.enabled {
        return PanelDimensions::default();
    }

    let available_cols = ((viewport_width - 2.0 * padding_x) / cell_width).floor() as isize;
    let max_panel_cols = (available_cols - MIN_COLUMNS as isize).max(0) as usize;
    if max_panel_cols == 0 {
        return PanelDimensions::default();
    }

    let requested_width = config.window.tab_panel.width as f32 * scale_factor;
    let max_width = max_panel_cols as f32 * cell_width;
    let width = requested_width.min(max_width);
    let columns = (width / cell_width).floor().min(max_panel_cols as f32) as usize;

    if columns == 0 {
        return PanelDimensions::default();
    }

    PanelDimensions { columns, width }
}

#[derive(Default)]
pub struct TabPanel {
    enabled: bool,
    width_cols: usize,
    width_px: f32,
    groups: Vec<TabPanelGroup>,
    edit: Option<EditState>,
    hover: HoverState,
    drag: Option<DragState>,
    resize: Option<ResizeState>,
    drop_target: Option<DropTarget>,
    last_mouse_pos: Option<PhysicalPosition<f64>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditState {
    target: TabPanelEditTarget,
    text: String,
    cursor: usize,
}

impl TabPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_dimensions(&mut self, dimensions: PanelDimensions) {
        self.width_cols = dimensions.columns;
        self.width_px = dimensions.width;
    }

    pub fn width(&self) -> f32 {
        self.width_px
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.width_cols > 0
    }

    pub fn set_groups(&mut self, groups: Vec<TabPanelGroup>) -> bool {
        if self.groups == groups {
            return false;
        }

        self.groups = groups;
        self.validate_edit_target();
        true
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    pub fn begin_edit_tab(&mut self, tab_id: TabId, title: String) -> bool {
        self.begin_edit(TabPanelEditTarget::Tab(tab_id), title)
    }

    pub fn begin_edit_group(&mut self, group_id: usize, name: String) -> bool {
        self.begin_edit(TabPanelEditTarget::Group(group_id), name)
    }

    pub fn cancel_edit(&mut self) -> bool {
        self.edit.take().is_some()
    }

    pub fn handle_key_event(&mut self, key: &KeyEvent) -> TabPanelEditOutcome {
        let Some(edit) = self.edit.as_mut() else {
            return TabPanelEditOutcome::None;
        };

        if key.state == ElementState::Released {
            return TabPanelEditOutcome::None;
        }

        match key.logical_key.as_ref() {
            Key::Named(NamedKey::Escape) => {
                self.edit = None;
                return TabPanelEditOutcome::Cancelled;
            },
            Key::Named(NamedKey::Enter) => {
                let commit = self.take_edit_commit();
                return TabPanelEditOutcome::Commit(commit);
            },
            Key::Named(NamedKey::Backspace) => {
                if edit.backspace() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::Delete) => {
                if edit.delete() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::ArrowLeft) => {
                if edit.move_left() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::ArrowRight) => {
                if edit.move_right() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::Home) => {
                if edit.move_home() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::End) => {
                if edit.move_end() {
                    return TabPanelEditOutcome::Changed;
                }
            },
            Key::Named(NamedKey::Tab) => {
                return TabPanelEditOutcome::None;
            },
            _ => (),
        }

        let text = key.text_with_all_modifiers().unwrap_or_default();
        if edit.insert_text(&text) {
            TabPanelEditOutcome::Changed
        } else {
            TabPanelEditOutcome::None
        }
    }

    pub fn handle_ime_commit(&mut self, text: &str) -> TabPanelEditOutcome {
        let Some(edit) = self.edit.as_mut() else {
            return TabPanelEditOutcome::None;
        };

        if edit.insert_text(text) {
            TabPanelEditOutcome::Changed
        } else {
            TabPanelEditOutcome::None
        }
    }

    fn begin_edit(&mut self, target: TabPanelEditTarget, text: String) -> bool {
        let cursor = text.chars().count();
        let next = EditState { target, text, cursor };
        let changed = self.edit.as_ref() != Some(&next);
        self.edit = Some(next);
        self.drag = None;
        self.resize = None;
        self.drop_target = None;
        changed
    }

    fn validate_edit_target(&mut self) {
        let Some(edit) = &self.edit else {
            return;
        };

        let valid = match edit.target {
            TabPanelEditTarget::Tab(tab_id) => self
                .groups
                .iter()
                .any(|group| group.tabs.iter().any(|tab| tab.tab_id == tab_id)),
            TabPanelEditTarget::Group(group_id) => {
                self.groups.iter().any(|group| group.id == group_id)
            },
        };

        if !valid {
            self.edit = None;
        }
    }

    fn take_edit_commit(&mut self) -> TabPanelEditCommit {
        let edit = self.edit.take().expect("edit state required");
        TabPanelEditCommit { target: edit.target, text: edit.text }
    }

    pub fn cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
        size_info: &SizeInfo,
    ) -> TabPanelCursorUpdate {
        self.last_mouse_pos = Some(position);

        let panel_size_info = self.panel_size_info(size_info);
        let resizing = self.resize.is_some();
        let resize_hit = !self.drag.is_some() && self.is_on_resize_handle(position);
        let capture = self.should_capture(Some(position)) || resize_hit;

        if resizing {
            let width_px = self.resize.as_ref().unwrap().width(position);
            return TabPanelCursorUpdate {
                capture: true,
                needs_redraw: true,
                cursor: Some(CursorIcon::EwResize),
                resize_width: Some(width_px),
            };
        }

        if !capture {
            let needs_redraw = self.hover != HoverState::default() || self.drop_target.is_some();
            self.hover = HoverState::default();
            self.drop_target = None;
            return TabPanelCursorUpdate {
                capture: false,
                needs_redraw,
                cursor: None,
                resize_width: None,
            };
        }

        let hit = if resize_hit { None } else { self.hit_test(position, &panel_size_info) };
        let next_hover = HoverState::from_hit(&hit);
        let drag_started = self.update_drag(position);
        let needs_redraw = drag_started
            || next_hover != self.hover
            || self.update_drop_target(position, &panel_size_info);
        self.hover = next_hover;

        let cursor = if resize_hit {
            Some(CursorIcon::EwResize)
        } else {
            match hit {
                Some(PanelHit::Tab { .. }) => Some(CursorIcon::Pointer),
                _ => Some(CursorIcon::Default),
            }
        };

        TabPanelCursorUpdate { capture: true, needs_redraw, cursor, resize_width: None }
    }

    pub fn mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
        size_info: &SizeInfo,
    ) -> TabPanelMouseUpdate {
        let position = match self.last_mouse_pos {
            Some(position) => position,
            None => return TabPanelMouseUpdate::default(),
        };

        let panel_size_info = self.panel_size_info(size_info);
        let capture = self.should_capture(Some(position));
        if !capture {
            return TabPanelMouseUpdate::default();
        }

        if button == MouseButton::Right {
            if !matches!(state, ElementState::Released) {
                return TabPanelMouseUpdate { capture, needs_redraw: false, command: None };
            }

            let hit = self.hit_test(position, &panel_size_info);
            let command = match hit {
                Some(PanelHit::Tab { tab_id }) => Some(TabPanelCommand::RenameTab(tab_id)),
                Some(PanelHit::Group { group_index }) => self
                    .groups
                    .get(group_index)
                    .map(|group| TabPanelCommand::RenameGroup(group.id)),
                None => None,
            };

            return TabPanelMouseUpdate {
                capture,
                needs_redraw: command.is_some(),
                command,
            };
        }

        if button != MouseButton::Left {
            return TabPanelMouseUpdate { capture, needs_redraw: false, command: None };
        }

        if matches!(state, ElementState::Pressed) && self.is_on_resize_handle(position) {
            self.resize = Some(ResizeState::new(self.width_px, position));
            return TabPanelMouseUpdate { capture: true, needs_redraw: true, command: None };
        }

        if matches!(state, ElementState::Released) && self.resize.take().is_some() {
            return TabPanelMouseUpdate { capture: true, needs_redraw: true, command: None };
        }

        let hit = self.hit_test(position, &panel_size_info);
        let mut needs_redraw = false;
        let mut command = None;

        match state {
            ElementState::Pressed => {
                if let Some(PanelHit::Tab { tab_id }) = hit {
                    if !self.is_close_hit(position, &panel_size_info) {
                        self.drag = Some(DragState::new(tab_id, position));
                        needs_redraw = true;
                    }
                }
            },
            ElementState::Released => {
                if let Some(drag) = self.drag.take() {
                    if drag.dragging {
                        if let Some(target) = self.compute_drop_target(position, &panel_size_info) {
                            command = Some(TabPanelCommand::Move {
                                tab_id: drag.tab_id,
                                target_group_id: Some(target.group_id),
                                target_index: Some(target.index),
                            });
                        } else if self.is_inside_panel(position) {
                            command = Some(TabPanelCommand::Move {
                                tab_id: drag.tab_id,
                                target_group_id: None,
                                target_index: None,
                            });
                        }
                    } else if let Some(PanelHit::Tab { tab_id }) = hit {
                        if self.is_close_hit(position, &panel_size_info)
                            && self.hover.tab == Some(tab_id)
                        {
                            command = Some(TabPanelCommand::Close(tab_id));
                        } else {
                            command = Some(TabPanelCommand::Focus(tab_id));
                        }
                    }

                    self.drop_target = None;
                    needs_redraw = true;
                } else if let Some(PanelHit::Tab { tab_id }) = hit {
                    if self.is_close_hit(position, &panel_size_info) && self.hover.tab == Some(tab_id)
                    {
                        command = Some(TabPanelCommand::Close(tab_id));
                    } else {
                        command = Some(TabPanelCommand::Focus(tab_id));
                    }
                    needs_redraw = true;
                }
            },
        }

        TabPanelMouseUpdate { capture, needs_redraw, command }
    }

    pub fn push_rects(&self, size_info: &SizeInfo, config: &UiConfig, rects: &mut Vec<RenderRect>) {
        if !self.is_enabled() {
            return;
        }

        let panel_size_info = self.panel_size_info(size_info);
        let layout = self.layout(&panel_size_info);
        let base = config.colors.primary.background;
        let fg = config.colors.primary.foreground;
        let panel_bg = mix(base, fg, 0.04);
        let header_bg = mix(base, fg, 0.08);
        let active_bg = mix(base, fg, 0.18);
        let drop_bg = mix(base, fg, 0.24);
        let divider = mix(base, fg, 0.2);

        rects.push(RenderRect::new(0., 0., self.width_px, size_info.height(), panel_bg, 1.));

        if self.width_px >= 1.0 {
            rects.push(RenderRect::new(
                self.width_px - 1.0,
                0.,
                1.0,
                size_info.height(),
                divider,
                1.0,
            ));
        }

        let line_height = panel_size_info.cell_height();
        let start_y = panel_size_info.padding_y();

        for item in layout.items {
            let y = start_y + item.line as f32 * line_height;
            let bg = match item.kind {
                PanelItemKind::GroupHeader { group_index } => {
                    if self.drop_target.is_some_and(|target| target.group_index == group_index) {
                        drop_bg
                    } else {
                        header_bg
                    }
                },
                PanelItemKind::Tab { tab, group_index } => {
                    if self.drop_target.is_some_and(|target| target.group_index == group_index) {
                        drop_bg
                    } else if tab.is_active {
                        active_bg
                    } else {
                        panel_bg
                    }
                },
            };

            rects.push(RenderRect::new(0., y, self.width_px, line_height, bg, 1.));
        }
    }

    pub fn draw_text(
        &self,
        size_info: &SizeInfo,
        config: &UiConfig,
        renderer: &mut Renderer,
        glyph_cache: &mut GlyphCache,
    ) {
        if !self.is_enabled() {
            return;
        }

        let panel_size_info = self.panel_size_info(size_info);
        let layout = self.layout(&panel_size_info);

        #[cfg(target_os = "macos")]
        {
            let font_key = glyph_cache.font_key;
            let font_size = glyph_cache.font_size;
            let metrics = glyph_cache.font_metrics();
            let mut missing = Vec::new();

            for item in &layout.items {
                if let PanelItemKind::Tab { tab, .. } = &item.kind {
                    if let Some(favicon) = &tab.favicon {
                        let key =
                            GlyphKey { font_key, size: font_size, character: favicon.character };
                        if !glyph_cache.has_glyph(&key) {
                            missing.push((key, favicon.clone()));
                        }
                    }
                }
            }

            if !missing.is_empty() {
                renderer.with_loader(|mut api| {
                    for (key, favicon) in missing {
                        let rasterized =
                            favicon.image.rasterized_glyph(favicon.character, &panel_size_info, metrics);
                        glyph_cache.insert_custom_glyph(key, rasterized, &mut api);
                    }
                });
            }
        }

        renderer.set_viewport(&panel_size_info);
        renderer.set_text_projection(&panel_size_info);

        let base = config.colors.primary.background;
        let fg = config.colors.primary.foreground;
        let panel_bg = mix(base, fg, 0.04);
        let header_bg = mix(base, fg, 0.08);
        let active_bg = mix(base, fg, 0.18);
        let drop_bg = mix(base, fg, 0.24);
        let header_fg = mix(fg, base, 0.2);
        let now = Instant::now();

        for item in layout.items {
            match item.kind {
                PanelItemKind::GroupHeader { group_index } => {
                    if let Some(group) = self.groups.get(group_index) {
                        let label = match &self.edit {
                            Some(edit)
                                if edit.target == TabPanelEditTarget::Group(group.id) =>
                            {
                                let name = render_edit_text(&edit.text, edit.cursor);
                                format!("{} {}", group.id, name)
                            },
                            _ => group.label.clone(),
                        };
                        let title = format!("{}:", label);
                        let text = truncate_to_columns(&title, self.width_cols.saturating_sub(1));
                        let bg = if self
                            .drop_target
                            .is_some_and(|target| target.group_index == group_index)
                        {
                            drop_bg
                        } else {
                            header_bg
                        };
                        let point = Point::new(item.line, Column(0));
                        renderer.draw_string(
                            point,
                            header_fg,
                            bg,
                            text.chars(),
                            &panel_size_info,
                            glyph_cache,
                        );
                    }
                },
                PanelItemKind::Tab { tab, group_index } => {
                    let indent = 1;
                    let indicator_cols =
                        if tab.activity.is_some() { ACTIVITY_INDICATOR_COLS } else { 0 };
                    let text_col = indent + indicator_cols;
                    let close_col = self.width_cols.saturating_sub(1);
                    let max_cols = self.width_cols.saturating_sub(text_col + 1);
                    let title = match &self.edit {
                        Some(edit) if edit.target == TabPanelEditTarget::Tab(tab.tab_id) => {
                            render_edit_text(&edit.text, edit.cursor)
                        },
                        _ => tab.title.clone(),
                    };
                    #[cfg(target_os = "macos")]
                    let label = if let Some(favicon) = &tab.favicon {
                        format!("{}  {}", favicon.character, title)
                    } else {
                        title
                    };
                    #[cfg(not(target_os = "macos"))]
                    let label = title;
                    let text = truncate_to_columns(&label, max_cols);
                    let bg = if self
                        .drop_target
                        .is_some_and(|target| target.group_index == group_index)
                    {
                        drop_bg
                    } else if tab.is_active {
                        active_bg
                    } else {
                        panel_bg
                    };

                    if let Some(indicator) = tab_activity_indicator(&tab, now, base, fg, config) {
                        let point = Point::new(item.line, Column(indent));
                        renderer.draw_string(
                            point,
                            indicator.color,
                            bg,
                            std::iter::once(indicator.glyph),
                            &panel_size_info,
                            glyph_cache,
                        );
                    }

                    let point = Point::new(item.line, Column(text_col));
                    renderer.draw_string(
                        point,
                        fg,
                        bg,
                        text.chars(),
                        &panel_size_info,
                        glyph_cache,
                    );

                    if close_col > text_col && self.hover.tab == Some(tab.tab_id) {
                        let point = Point::new(item.line, Column(close_col));
                        renderer.draw_string(
                            point,
                            fg,
                            bg,
                            "x".chars(),
                            &panel_size_info,
                            glyph_cache,
                        );
                    }
                },
            }
        }

        renderer.set_viewport(size_info);
        renderer.set_text_projection(size_info);
    }

    pub fn should_capture(&self, position: Option<PhysicalPosition<f64>>) -> bool {
        if !self.is_enabled() {
            return false;
        }

        if self.drag.is_some() || self.resize.is_some() {
            return true;
        }

        position.is_some_and(|pos| self.is_inside_panel(pos) || self.is_on_resize_handle(pos))
    }

    pub fn should_capture_last(&self) -> bool {
        self.should_capture(self.last_mouse_pos)
    }

    pub fn update_drag(&mut self, position: PhysicalPosition<f64>) -> bool {
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };

        if drag.dragging {
            return false;
        }

        let dx = (position.x - drag.start_pos.x).abs();
        let dy = (position.y - drag.start_pos.y).abs();
        if dx.max(dy) > DRAG_THRESHOLD_PX {
            drag.dragging = true;
            return true;
        }

        false
    }

    fn panel_cell_height(&self, size_info: &SizeInfo) -> f32 {
        let min_height = (size_info.cell_width() * PANEL_ICON_SCALE).ceil();
        size_info.cell_height().max(min_height) + PANEL_ROW_PADDING_PX
    }

    fn panel_size_info(&self, size_info: &SizeInfo) -> SizeInfo {
        SizeInfo::new(
            self.width_px,
            size_info.height(),
            size_info.cell_width(),
            self.panel_cell_height(size_info),
            0.,
            0.,
            size_info.padding_y(),
            false,
        )
    }

    fn is_close_hit(&self, position: PhysicalPosition<f64>, size_info: &SizeInfo) -> bool {
        if self.width_cols == 0 {
            return false;
        }

        let cell_width = size_info.cell_width() as f64;
        if cell_width <= 0.0 {
            return false;
        }

        let close_col = self.width_cols.saturating_sub(1);
        if close_col <= 1 {
            return false;
        }

        let col = (position.x / cell_width).floor() as usize;
        col == close_col
    }

    fn is_inside_panel(&self, position: PhysicalPosition<f64>) -> bool {
        position.x >= 0.0 && position.x < self.width_px as f64
    }

    fn is_on_resize_handle(&self, position: PhysicalPosition<f64>) -> bool {
        if !self.is_enabled() {
            return false;
        }

        let left = (self.width_px as f64 - RESIZE_HANDLE_WIDTH_PX).max(0.0);
        let right = self.width_px as f64 + RESIZE_HANDLE_WIDTH_PX;
        position.x >= left && position.x <= right
    }

    fn update_drop_target(&mut self, position: PhysicalPosition<f64>, size_info: &SizeInfo) -> bool {
        let dragging = self.drag.as_ref().is_some_and(|drag| drag.dragging);
        if !dragging {
            if self.drop_target.take().is_some() {
                return true;
            }
            return false;
        }

        let next = self.compute_drop_target(position, size_info);
        if next != self.drop_target {
            self.drop_target = next;
            return true;
        }

        false
    }

    fn compute_drop_target(
        &self,
        position: PhysicalPosition<f64>,
        size_info: &SizeInfo,
    ) -> Option<DropTarget> {
        if !self.is_inside_panel(position) {
            return None;
        }

        let top = size_info.padding_y() as f64;
        if position.y < top {
            return None;
        }

        let line_height = size_info.cell_height() as f64;
        let mut line = ((position.y - top) / line_height).floor() as usize;
        let max_lines = size_info.screen_lines();
        if max_lines == 0 {
            return None;
        }
        if line >= max_lines {
            line = max_lines - 1;
        }

        let mut current_line = 0;

        for (group_index, group) in self.groups.iter().enumerate() {
            if current_line >= max_lines {
                break;
            }

            let header_line = current_line;
            let remaining_lines = max_lines.saturating_sub(header_line + 1);
            let visible_tabs = group.tabs.len().min(remaining_lines);
            let tabs_start = header_line + 1;
            let tabs_end = header_line + visible_tabs;
            let blank_line = if visible_tabs < remaining_lines {
                Some(tabs_end + 1)
            } else {
                None
            };
            let group_end = blank_line.unwrap_or(tabs_end);

            if line >= header_line && line <= group_end {
                let index = if line == header_line {
                    0
                } else if line >= tabs_start && line <= tabs_end && visible_tabs > 0 {
                    line - tabs_start
                } else {
                    visible_tabs
                };
                return Some(DropTarget {
                    group_index,
                    group_id: group.id,
                    index,
                });
            }

            current_line = group_end + 1;
        }

        None
    }

    fn hit_test(&self, position: PhysicalPosition<f64>, size_info: &SizeInfo) -> Option<PanelHit> {
        if !self.is_inside_panel(position) {
            return None;
        }

        let top = size_info.padding_y() as f64;
        if position.y < top {
            return None;
        }

        let line_height = size_info.cell_height() as f64;
        let line = ((position.y - top) / line_height).floor() as usize;
        let layout = self.layout(size_info);

        layout
            .items
            .into_iter()
            .find(|item| item.line == line)
            .map(|item| match item.kind {
                PanelItemKind::GroupHeader { group_index } => PanelHit::Group { group_index },
                PanelItemKind::Tab { tab, .. } => PanelHit::Tab { tab_id: tab.tab_id },
            })
    }

    fn layout(&self, size_info: &SizeInfo) -> PanelLayout {
        let mut items = Vec::new();
        let max_lines = size_info.screen_lines();
        let mut line = 0;

        for (group_index, group) in self.groups.iter().enumerate() {
            if line >= max_lines {
                break;
            }

            items.push(PanelItem {
                line,
                kind: PanelItemKind::GroupHeader { group_index },
            });
            line += 1;

            for tab in &group.tabs {
                if line >= max_lines {
                    break;
                }

                items.push(PanelItem {
                    line,
                        kind: PanelItemKind::Tab { tab: tab.clone(), group_index },
                    });
                line += 1;
            }

            if line < max_lines {
                line += 1;
            }
        }

        PanelLayout { items }
    }
}

impl EditState {
    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        self.cursor -= 1;
        true
    }

    fn move_right(&mut self) -> bool {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return false;
        }

        self.cursor += 1;
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        let len = self.text.chars().count();
        if self.cursor == len {
            return false;
        }

        self.cursor = len;
        true
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        let start = char_to_byte_idx(&self.text, self.cursor - 1);
        let end = char_to_byte_idx(&self.text, self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    fn delete(&mut self) -> bool {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return false;
        }

        let start = char_to_byte_idx(&self.text, self.cursor);
        let end = char_to_byte_idx(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
        true
    }

    fn insert_text(&mut self, text: &str) -> bool {
        let mut filtered = String::new();
        for ch in text.chars() {
            if !ch.is_control() {
                filtered.push(ch);
            }
        }

        if filtered.is_empty() {
            return false;
        }

        let idx = char_to_byte_idx(&self.text, self.cursor);
        self.text.insert_str(idx, &filtered);
        self.cursor += filtered.chars().count();
        true
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct HoverState {
    tab: Option<TabId>,
}

impl HoverState {
    fn from_hit(hit: &Option<PanelHit>) -> Self {
        match hit {
            Some(PanelHit::Tab { tab_id }) => HoverState { tab: Some(*tab_id) },
            Some(PanelHit::Group { .. }) => HoverState::default(),
            None => HoverState::default(),
        }
    }
}

struct DragState {
    tab_id: TabId,
    start_pos: PhysicalPosition<f64>,
    dragging: bool,
}

impl DragState {
    fn new(tab_id: TabId, start_pos: PhysicalPosition<f64>) -> Self {
        Self { tab_id, start_pos, dragging: false }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DropTarget {
    group_index: usize,
    group_id: usize,
    index: usize,
}

struct ResizeState {
    offset: f64,
}

impl ResizeState {
    fn new(width_px: f32, position: PhysicalPosition<f64>) -> Self {
        Self { offset: width_px as f64 - position.x }
    }

    fn width(&self, position: PhysicalPosition<f64>) -> f32 {
        (position.x + self.offset).max(0.0) as f32
    }
}

#[derive(Clone)]
struct PanelItem {
    line: usize,
    kind: PanelItemKind,
}

#[derive(Clone)]
enum PanelItemKind {
    GroupHeader { group_index: usize },
    Tab { tab: TabPanelTab, group_index: usize },
}

struct PanelLayout {
    items: Vec<PanelItem>,
}

#[derive(Clone)]
enum PanelHit {
    Group { group_index: usize },
    Tab { tab_id: TabId },
}

#[derive(Default)]
pub struct TabPanelCursorUpdate {
    pub capture: bool,
    pub needs_redraw: bool,
    pub cursor: Option<CursorIcon>,
    pub resize_width: Option<f32>,
}

#[derive(Default)]
pub struct TabPanelMouseUpdate {
    pub capture: bool,
    pub needs_redraw: bool,
    pub command: Option<TabPanelCommand>,
}

fn render_edit_text(text: &str, cursor: usize) -> String {
    let cursor = cursor.min(text.chars().count());
    let mut output = String::new();
    let mut index = 0;

    for ch in text.chars() {
        if index == cursor {
            output.push('|');
        }
        output.push(ch);
        index += 1;
    }

    if cursor == index {
        output.push('|');
    }

    output
}

fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }

    text.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| text.len())
}

fn truncate_to_columns(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    let mut width = 0;
    let mut output = String::new();

    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max_cols {
            break;
        }
        width += ch_width;
        output.push(ch);
    }

    output
}

struct ActivityIndicator {
    glyph: char,
    color: Rgb,
}

fn tab_activity_indicator(
    tab: &TabPanelTab,
    now: Instant,
    base: Rgb,
    fg: Rgb,
    config: &UiConfig,
) -> Option<ActivityIndicator> {
    let activity = tab.activity.as_ref()?;

    if activity.is_active(now) {
        return Some(ActivityIndicator {
            glyph: ACTIVITY_INDICATOR_FILLED,
            color: config.colors.normal.green,
        });
    }

    if activity.has_unseen_output {
        return Some(ActivityIndicator {
            glyph: ACTIVITY_INDICATOR_FILLED,
            color: config.colors.normal.blue,
        });
    }

    Some(ActivityIndicator {
        glyph: ACTIVITY_INDICATOR_OUTLINE,
        color: mix(fg, base, 0.5),
    })
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let mix_channel = |a: u8, b: u8| -> u8 {
        let a = a as f32;
        let b = b as f32;
        (a + (b - a) * t).round().clamp(0., 255.) as u8
    };

    Rgb::new(mix_channel(a.r, b.r), mix_channel(a.g, b.g), mix_channel(a.b, b.b))
}

const DRAG_THRESHOLD_PX: f64 = 4.0;
