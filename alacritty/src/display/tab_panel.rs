use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton};
use winit::window::CursorIcon;

use unicode_width::UnicodeWidthChar;

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

#[derive(Default, Clone, Copy)]
pub struct PanelDimensions {
    pub columns: usize,
    pub width: f32,
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
    hover: HoverState,
    drag: Option<DragState>,
    drop_target: Option<usize>,
    last_mouse_pos: Option<PhysicalPosition<f64>>,
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
        true
    }

    pub fn cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
        size_info: &SizeInfo,
    ) -> TabPanelCursorUpdate {
        self.last_mouse_pos = Some(position);

        let capture = self.should_capture(Some(position));
        if !capture {
            let needs_redraw = self.hover != HoverState::default() || self.drop_target.is_some();
            self.hover = HoverState::default();
            self.drop_target = None;
            return TabPanelCursorUpdate { capture: false, needs_redraw, cursor: None };
        }

        let hit = self.hit_test(position, size_info);
        let next_hover = HoverState::from_hit(&hit);
        let drag_started = self.update_drag(position);
        let needs_redraw = drag_started
            || next_hover != self.hover
            || self.update_drop_target(hit.as_ref());
        self.hover = next_hover;

        let cursor = match hit {
            Some(PanelHit::NewTabButton | PanelHit::Tab { .. }) => Some(CursorIcon::Pointer),
            _ => Some(CursorIcon::Default),
        };

        TabPanelCursorUpdate { capture: true, needs_redraw, cursor }
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

        let capture = self.should_capture(Some(position));
        if !capture {
            return TabPanelMouseUpdate::default();
        }

        if button != MouseButton::Left {
            return TabPanelMouseUpdate {
                capture,
                needs_redraw: false,
                command: None,
                create_tab: false,
            };
        }

        let hit = self.hit_test(position, size_info);
        let mut needs_redraw = false;
        let mut command = None;
        let mut create_tab = false;

        match state {
            ElementState::Pressed => {
                if let Some(PanelHit::Tab { tab_id, group_index }) = hit {
                    self.drag = Some(DragState::new(tab_id, group_index, position));
                    needs_redraw = true;
                }
            },
            ElementState::Released => {
                if let Some(drag) = self.drag.take() {
                    if drag.dragging {
                        if let Some(target_group) = self.drop_target {
                            if target_group != drag.origin_group {
                                command = Some(TabPanelCommand::Move {
                                    tab_id: drag.tab_id,
                                    target_group: Some(target_group),
                                });
                            }
                        } else if self.is_inside_panel(position) {
                            command = Some(TabPanelCommand::Move {
                                tab_id: drag.tab_id,
                                target_group: None,
                            });
                        }
                    } else if let Some(PanelHit::Tab { tab_id, .. }) = hit {
                        command = Some(TabPanelCommand::Focus(tab_id));
                    }

                    self.drop_target = None;
                    needs_redraw = true;
                } else if matches!(hit, Some(PanelHit::NewTabButton)) {
                    create_tab = true;
                    needs_redraw = true;
                }
            },
        }

        TabPanelMouseUpdate { capture, needs_redraw, command, create_tab }
    }

    pub fn push_rects(&self, size_info: &SizeInfo, config: &UiConfig, rects: &mut Vec<RenderRect>) {
        if !self.is_enabled() {
            return;
        }

        let layout = self.layout(size_info);
        let base = config.colors.primary.background;
        let fg = config.colors.primary.foreground;
        let panel_bg = mix(base, fg, 0.04);
        let header_bg = mix(base, fg, 0.08);
        let hover_bg = mix(base, fg, 0.12);
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

        let line_height = size_info.cell_height();
        let start_y = size_info.padding_y();

        for item in layout.items {
            let y = start_y + item.line as f32 * line_height;
            let bg = match item.kind {
                PanelItemKind::NewTabButton => {
                    if self.hover.new_tab {
                        hover_bg
                    } else {
                        header_bg
                    }
                },
                PanelItemKind::GroupHeader { group_index } => {
                    if Some(group_index) == self.drop_target {
                        drop_bg
                    } else {
                        header_bg
                    }
                },
                PanelItemKind::Tab { tab, group_index } => {
                    if Some(group_index) == self.drop_target {
                        drop_bg
                    } else if tab.is_active {
                        active_bg
                    } else if self.hover.tab == Some(tab.tab_id) {
                        hover_bg
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

        let layout = self.layout(size_info);
        let panel_size_info = SizeInfo::new(
            size_info.width(),
            size_info.height(),
            size_info.cell_width(),
            size_info.cell_height(),
            0.,
            0.,
            size_info.padding_y(),
            false,
        );

        let base = config.colors.primary.background;
        let fg = config.colors.primary.foreground;
        let panel_bg = mix(base, fg, 0.04);
        let header_bg = mix(base, fg, 0.08);
        let hover_bg = mix(base, fg, 0.12);
        let active_bg = mix(base, fg, 0.18);
        let drop_bg = mix(base, fg, 0.24);
        let header_fg = mix(fg, base, 0.2);

        for item in layout.items {
            match item.kind {
                PanelItemKind::NewTabButton => {
                    let text = "+";
                    let bg = if self.hover.new_tab { hover_bg } else { header_bg };
                    let point = Point::new(item.line, Column(0));
                    renderer.draw_string(
                        point,
                        header_fg,
                        bg,
                        text.chars(),
                        &panel_size_info,
                        glyph_cache,
                    );
                },
                PanelItemKind::GroupHeader { group_index } => {
                    if let Some(group) = self.groups.get(group_index) {
                        let title = format!("{}:", group.label);
                        let text = truncate_to_columns(&title, self.width_cols.saturating_sub(1));
                        let bg = if Some(group_index) == self.drop_target {
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
                    let max_cols = self.width_cols.saturating_sub(indent + 1);
                    let label = format!("{} {}", tab.kind.indicator(), tab.title);
                    let text = truncate_to_columns(&label, max_cols);
                    let bg = if Some(group_index) == self.drop_target {
                        drop_bg
                    } else if Some(tab.tab_id) == self.hover.tab {
                        hover_bg
                    } else if tab.is_active {
                        active_bg
                    } else {
                        panel_bg
                    };
                    let point = Point::new(item.line, Column(indent));
                    renderer.draw_string(
                        point,
                        fg,
                        bg,
                        text.chars(),
                        &panel_size_info,
                        glyph_cache,
                    );
                },
            }
        }
    }

    pub fn should_capture(&self, position: Option<PhysicalPosition<f64>>) -> bool {
        if !self.is_enabled() {
            return false;
        }

        if self.drag.is_some() {
            return true;
        }

        position.is_some_and(|pos| self.is_inside_panel(pos))
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

    fn is_inside_panel(&self, position: PhysicalPosition<f64>) -> bool {
        position.x >= 0.0 && position.x < self.width_px as f64
    }

    fn update_drop_target(&mut self, hit: Option<&PanelHit>) -> bool {
        if self.drag.is_none() {
            if self.drop_target.take().is_some() {
                return true;
            }
            return false;
        }

        let next = hit.and_then(|hit| hit.group_index());
        if next != self.drop_target {
            self.drop_target = next;
            return true;
        }

        false
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
                PanelItemKind::NewTabButton => PanelHit::NewTabButton,
                PanelItemKind::GroupHeader { group_index } => PanelHit::Group { group_index },
                PanelItemKind::Tab { tab, group_index, .. } => {
                    PanelHit::Tab { tab_id: tab.tab_id, group_index }
                },
            })
    }

    fn layout(&self, size_info: &SizeInfo) -> PanelLayout {
        let mut items = Vec::new();
        let max_lines = size_info.screen_lines();
        let mut line = 0;

        if line < max_lines {
            items.push(PanelItem {
                line,
                kind: PanelItemKind::NewTabButton,
            });
            line += 1;
        }

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

#[derive(Clone, Default, PartialEq, Eq)]
struct HoverState {
    tab: Option<TabId>,
    group: Option<usize>,
    new_tab: bool,
}

impl HoverState {
    fn from_hit(hit: &Option<PanelHit>) -> Self {
        match hit {
            Some(PanelHit::Tab { tab_id, group_index }) => {
                HoverState { tab: Some(*tab_id), group: Some(*group_index), new_tab: false }
            }
            Some(PanelHit::Group { group_index }) => {
                HoverState { tab: None, group: Some(*group_index), new_tab: false }
            }
            Some(PanelHit::NewTabButton) => HoverState { tab: None, group: None, new_tab: true },
            None => HoverState::default(),
        }
    }
}

struct DragState {
    tab_id: TabId,
    origin_group: usize,
    start_pos: PhysicalPosition<f64>,
    dragging: bool,
}

impl DragState {
    fn new(tab_id: TabId, origin_group: usize, start_pos: PhysicalPosition<f64>) -> Self {
        Self { tab_id, origin_group, start_pos, dragging: false }
    }
}

#[derive(Clone)]
struct PanelItem {
    line: usize,
    kind: PanelItemKind,
}

#[derive(Clone)]
enum PanelItemKind {
    NewTabButton,
    GroupHeader { group_index: usize },
    Tab { tab: TabPanelTab, group_index: usize },
}

struct PanelLayout {
    items: Vec<PanelItem>,
}

#[derive(Clone)]
enum PanelHit {
    NewTabButton,
    Group { group_index: usize },
    Tab { tab_id: TabId, group_index: usize },
}

impl PanelHit {
    fn group_index(&self) -> Option<usize> {
        match self {
            PanelHit::NewTabButton => None,
            PanelHit::Group { group_index } => Some(*group_index),
            PanelHit::Tab { group_index, .. } => Some(*group_index),
        }
    }
}

#[derive(Default)]
pub struct TabPanelCursorUpdate {
    pub capture: bool,
    pub needs_redraw: bool,
    pub cursor: Option<CursorIcon>,
}

#[derive(Default)]
pub struct TabPanelMouseUpdate {
    pub capture: bool,
    pub needs_redraw: bool,
    pub command: Option<TabPanelCommand>,
    pub create_tab: bool,
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

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let mix_channel = |a: u8, b: u8| -> u8 {
        let a = a as f32;
        let b = b as f32;
        (a + (b - a) * t).round().clamp(0., 255.) as u8
    };

    Rgb::new(mix_channel(a.r, b.r), mix_channel(a.g, b.g), mix_channel(a.b, b.b))
}

const DRAG_THRESHOLD_PX: f64 = 4.0;
