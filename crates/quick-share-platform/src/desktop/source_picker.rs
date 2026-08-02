use super::{DesktopError, SourceKind};
use cosmic_text::{
    Align, Attrs, Buffer as TextBuffer, Color as TextColor, Ellipsize, EllipsizeHeightLimit,
    Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use softbuffer::{Context, Surface};
use std::{num::NonZeroU32, sync::Arc};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect as SkiaRect, Stroke,
    Transform,
};
use winit::{
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{
        CursorIcon, Icon as WindowIcon, Theme, UserAttentionType, Window, WindowButtons, WindowId,
    },
};

const WINDOW_WIDTH: f64 = 520.0;
const WINDOW_HEIGHT: f64 = 320.0;
const FILE_RECT: UiRect = UiRect::new(32.0, 112.0, 218.0, 122.0);
const FOLDER_RECT: UiRect = UiRect::new(270.0, 112.0, 218.0, 122.0);
const CANCEL_RECT: UiRect = UiRect::new(398.0, 264.0, 90.0, 36.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PickerAction {
    Pick(SourceKind),
    Cancel,
    RenderFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Files,
    Folder,
    Cancel,
}

impl Control {
    const fn action(self) -> PickerAction {
        match self {
            Self::Files => PickerAction::Pick(SourceKind::Files),
            Self::Folder => PickerAction::Pick(SourceKind::Folder),
            Self::Cancel => PickerAction::Cancel,
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Files => Self::Folder,
            Self::Folder => Self::Cancel,
            Self::Cancel => Self::Files,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Files => Self::Cancel,
            Self::Folder => Self::Files,
            Self::Cancel => Self::Folder,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct UiRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl UiRect {
    const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn contains(self, position: PhysicalPosition<f64>, scale: f64) -> bool {
        let x = position.x / scale;
        let y = position.y / scale;
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    fn scaled(self, scale: f32) -> DrawRect {
        DrawRect {
            x: self.x as f32 * scale,
            y: self.y as f32 * scale,
            width: self.width as f32 * scale,
            height: self.height as f32 * scale,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DrawRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

pub(super) struct SourcePicker {
    surface: Surface<Arc<Window>, Arc<Window>>,
    _context: Context<Arc<Window>>,
    window: Arc<Window>,
    renderer: PickerRenderer,
    requester_name: String,
    hovered: Option<Control>,
    focused: Control,
    theme: Theme,
}

impl std::fmt::Debug for SourcePicker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourcePicker")
            .field("window_id", &self.window.id())
            .field("requester_name", &"REDACTED")
            .field("hovered", &self.hovered)
            .field("focused", &self.focused)
            .field("theme", &self.theme)
            .finish_non_exhaustive()
    }
}

impl SourcePicker {
    pub(super) fn new(
        event_loop: &ActiveEventLoop,
        requester_name: String,
    ) -> Result<Self, DesktopError> {
        let size = LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT);
        let window_icon = quick_share_window_icon()?;
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Quick Share - 选择发送内容")
                        .with_inner_size(size)
                        .with_min_inner_size(size)
                        .with_max_inner_size(size)
                        .with_resizable(false)
                        .with_enabled_buttons(WindowButtons::CLOSE)
                        .with_window_icon(Some(window_icon))
                        .with_visible(false)
                        .with_active(true),
                )
                .map_err(|_| DesktopError::Unavailable)?,
        );
        center_on_monitor(&window);
        let context = Context::new(Arc::clone(&window)).map_err(|_| DesktopError::Backend)?;
        let surface =
            Surface::new(&context, Arc::clone(&window)).map_err(|_| DesktopError::Backend)?;
        let theme = window.theme().unwrap_or(Theme::Light);
        let picker = Self {
            surface,
            _context: context,
            window,
            renderer: PickerRenderer::new(),
            requester_name,
            hovered: None,
            focused: Control::Files,
            theme,
        };
        picker.window.set_visible(true);
        picker.window.focus_window();
        picker
            .window
            .request_user_attention(Some(UserAttentionType::Critical));
        picker.window.request_redraw();
        Ok(picker)
    }

    pub(super) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(super) fn handle_event(&mut self, event: &WindowEvent) -> Option<PickerAction> {
        match event {
            WindowEvent::CloseRequested => Some(PickerAction::Cancel),
            WindowEvent::ThemeChanged(theme) => {
                self.theme = *theme;
                self.window.request_redraw();
                None
            }
            WindowEvent::CursorMoved { position, .. } => {
                let hovered = hit_test(*position, self.window.scale_factor());
                if hovered != self.hovered {
                    self.hovered = hovered;
                    self.window.set_cursor(if hovered.is_some() {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    });
                    self.window.request_redraw();
                }
                None
            }
            WindowEvent::CursorLeft { .. } => {
                if self.hovered.take().is_some() {
                    self.window.set_cursor(CursorIcon::Default);
                    self.window.request_redraw();
                }
                None
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.hovered.map(Control::action),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(PickerAction::Cancel),
                    Key::Named(NamedKey::Enter | NamedKey::Space) => Some(self.focused.action()),
                    Key::Named(NamedKey::Tab | NamedKey::ArrowRight | NamedKey::ArrowDown) => {
                        self.focused = self.focused.next();
                        self.window.request_redraw();
                        None
                    }
                    Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => {
                        self.focused = self.focused.previous();
                        self.window.request_redraw();
                        None
                    }
                    _ => None,
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.window.request_redraw();
                None
            }
            WindowEvent::RedrawRequested => {
                match self.renderer.draw(
                    &mut self.surface,
                    &self.window,
                    &self.requester_name,
                    self.theme,
                    self.hovered,
                    self.focused,
                ) {
                    Ok(()) => None,
                    Err(()) => Some(PickerAction::RenderFailed),
                }
            }
            _ => None,
        }
    }
}

fn quick_share_window_icon() -> Result<WindowIcon, DesktopError> {
    const SIZE: u32 = 32;
    let rgba = super::icon::quick_share_icon_rgba(SIZE);
    WindowIcon::from_rgba(rgba, SIZE, SIZE).map_err(|_| DesktopError::Backend)
}

fn center_on_monitor(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let window_size = window.outer_size();
    let x = monitor_position.x
        + i32::try_from(monitor_size.width.saturating_sub(window_size.width) / 2)
            .unwrap_or_default();
    let y = monitor_position.y
        + i32::try_from(monitor_size.height.saturating_sub(window_size.height) / 2)
            .unwrap_or_default();
    window.set_outer_position(PhysicalPosition::new(x, y));
}

fn hit_test(position: PhysicalPosition<f64>, scale: f64) -> Option<Control> {
    if FILE_RECT.contains(position, scale) {
        Some(Control::Files)
    } else if FOLDER_RECT.contains(position, scale) {
        Some(Control::Folder)
    } else if CANCEL_RECT.contains(position, scale) {
        Some(Control::Cancel)
    } else {
        None
    }
}

struct PickerRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl PickerRenderer {
    fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn draw(
        &mut self,
        surface: &mut Surface<Arc<Window>, Arc<Window>>,
        window: &Window,
        requester_name: &str,
        theme: Theme,
        hovered: Option<Control>,
        focused: Control,
    ) -> Result<(), ()> {
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let scale = window.scale_factor() as f32;
        let palette = Palette::for_theme(theme);
        let mut pixmap = Pixmap::new(width.get(), height.get()).ok_or(())?;
        pixmap.fill(palette.background);

        draw_brand(&mut pixmap, scale, palette);
        self.draw_text(
            &mut pixmap,
            "选择要发送的内容",
            DrawRect {
                x: 72.0 * scale,
                y: 27.0 * scale,
                width: 416.0 * scale,
                height: 30.0 * scale,
            },
            20.0 * scale,
            28.0 * scale,
            Weight::SEMIBOLD,
            Align::Left,
            palette.text,
        );
        let requester = format!("来自 {requester_name} 的请求");
        self.draw_text(
            &mut pixmap,
            &requester,
            DrawRect {
                x: 32.0 * scale,
                y: 72.0 * scale,
                width: 456.0 * scale,
                height: 24.0 * scale,
            },
            14.0 * scale,
            21.0 * scale,
            Weight::NORMAL,
            Align::Left,
            palette.secondary_text,
        );

        draw_choice(
            &mut pixmap,
            FILE_RECT.scaled(scale),
            Control::Files,
            hovered,
            focused,
            palette,
            scale,
        );
        draw_file_icon(
            &mut pixmap,
            141.0 * scale,
            151.0 * scale,
            scale,
            palette.file,
        );
        self.draw_text(
            &mut pixmap,
            "选择文件",
            DrawRect {
                x: 48.0 * scale,
                y: 194.0 * scale,
                width: 186.0 * scale,
                height: 25.0 * scale,
            },
            15.0 * scale,
            22.0 * scale,
            Weight::SEMIBOLD,
            Align::Center,
            palette.text,
        );

        draw_choice(
            &mut pixmap,
            FOLDER_RECT.scaled(scale),
            Control::Folder,
            hovered,
            focused,
            palette,
            scale,
        );
        draw_folder_icon(
            &mut pixmap,
            379.0 * scale,
            151.0 * scale,
            scale,
            palette.folder,
        );
        self.draw_text(
            &mut pixmap,
            "选择文件夹",
            DrawRect {
                x: 286.0 * scale,
                y: 194.0 * scale,
                width: 186.0 * scale,
                height: 25.0 * scale,
            },
            15.0 * scale,
            22.0 * scale,
            Weight::SEMIBOLD,
            Align::Center,
            palette.text,
        );

        draw_cancel(
            &mut pixmap,
            CANCEL_RECT.scaled(scale),
            hovered == Some(Control::Cancel),
            focused == Control::Cancel,
            palette,
            scale,
        );
        self.draw_text(
            &mut pixmap,
            "取消",
            DrawRect {
                x: 398.0 * scale,
                y: 271.0 * scale,
                width: 90.0 * scale,
                height: 22.0 * scale,
            },
            14.0 * scale,
            20.0 * scale,
            Weight::NORMAL,
            Align::Center,
            palette.text,
        );

        surface.resize(width, height).map_err(|_| ())?;
        let mut buffer = surface.buffer_mut().map_err(|_| ())?;
        for (target, pixel) in buffer.iter_mut().zip(pixmap.pixels()) {
            *target = u32::from(pixel.blue())
                | (u32::from(pixel.green()) << 8)
                | (u32::from(pixel.red()) << 16);
        }
        buffer.present().map_err(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        rect: DrawRect,
        font_size: f32,
        line_height: f32,
        weight: Weight,
        align: Align,
        color: Color,
    ) {
        let mut buffer =
            TextBuffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        let mut buffer = buffer.borrow_with(&mut self.font_system);
        buffer.set_size(Some(rect.width), Some(rect.height));
        buffer.set_wrap(Wrap::None);
        buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif).weight(weight),
            Shaping::Advanced,
            Some(align),
        );
        let text_color = TextColor::rgb(
            (color.red() * 255.0).round() as u8,
            (color.green() * 255.0).round() as u8,
            (color.blue() * 255.0).round() as u8,
        );
        buffer.draw(
            &mut self.swash_cache,
            text_color,
            |x, y, width, height, color| {
                let Some(glyph_rect) = SkiaRect::from_xywh(
                    rect.x + x as f32,
                    rect.y + y as f32,
                    width as f32,
                    height as f32,
                ) else {
                    return;
                };
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r(), color.g(), color.b(), color.a());
                pixmap.fill_rect(glyph_rect, &paint, Transform::identity(), None);
            },
        );
    }
}

#[derive(Clone, Copy)]
struct Palette {
    background: Color,
    surface: Color,
    hover: Color,
    border: Color,
    text: Color,
    secondary_text: Color,
    accent: Color,
    file: Color,
    folder: Color,
}

impl Palette {
    fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self {
                background: rgb(31, 32, 35),
                surface: rgb(43, 45, 49),
                hover: rgb(51, 55, 61),
                border: rgb(75, 79, 87),
                text: rgb(243, 244, 246),
                secondary_text: rgb(177, 182, 190),
                accent: rgb(96, 165, 250),
                file: rgb(96, 165, 250),
                folder: rgb(72, 201, 176),
            },
            Theme::Light => Self {
                background: rgb(247, 248, 250),
                surface: rgb(255, 255, 255),
                hover: rgb(241, 245, 249),
                border: rgb(215, 220, 227),
                text: rgb(31, 35, 40),
                secondary_text: rgb(95, 103, 114),
                accent: rgb(40, 120, 235),
                file: rgb(40, 120, 235),
                folder: rgb(22, 133, 107),
            },
        }
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::from_rgba8(red, green, blue, 255)
}

fn draw_brand(pixmap: &mut Pixmap, scale: f32, palette: Palette) {
    let rect = DrawRect {
        x: 32.0 * scale,
        y: 26.0 * scale,
        width: 28.0 * scale,
        height: 28.0 * scale,
    };
    fill_round_rect(pixmap, rect, 7.0 * scale, palette.accent);
    let stroke = Stroke {
        width: 2.0 * scale,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let mut paint = Paint::default();
    paint.set_color(rgb(255, 255, 255));
    let mut path = PathBuilder::new();
    path.move_to(39.0 * scale, 36.0 * scale);
    path.line_to(52.0 * scale, 36.0 * scale);
    path.line_to(48.0 * scale, 32.0 * scale);
    path.move_to(53.0 * scale, 44.0 * scale);
    path.line_to(40.0 * scale, 44.0 * scale);
    path.line_to(44.0 * scale, 48.0 * scale);
    if let Some(path) = path.finish() {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn draw_choice(
    pixmap: &mut Pixmap,
    rect: DrawRect,
    control: Control,
    hovered: Option<Control>,
    focused: Control,
    palette: Palette,
    scale: f32,
) {
    fill_round_rect(
        pixmap,
        rect,
        8.0 * scale,
        if hovered == Some(control) {
            palette.hover
        } else {
            palette.surface
        },
    );
    stroke_round_rect(
        pixmap,
        rect,
        8.0 * scale,
        if focused == control {
            palette.accent
        } else {
            palette.border
        },
        if focused == control { 2.0 } else { 1.0 } * scale,
    );
}

fn draw_cancel(
    pixmap: &mut Pixmap,
    rect: DrawRect,
    hovered: bool,
    focused: bool,
    palette: Palette,
    scale: f32,
) {
    fill_round_rect(
        pixmap,
        rect,
        6.0 * scale,
        if hovered {
            palette.hover
        } else {
            palette.surface
        },
    );
    stroke_round_rect(
        pixmap,
        rect,
        6.0 * scale,
        if focused {
            palette.accent
        } else {
            palette.border
        },
        if focused { 2.0 } else { 1.0 } * scale,
    );
}

fn draw_file_icon(pixmap: &mut Pixmap, center_x: f32, center_y: f32, scale: f32, color: Color) {
    let mut path = PathBuilder::new();
    path.move_to(center_x - 12.0 * scale, center_y - 18.0 * scale);
    path.line_to(center_x + 4.0 * scale, center_y - 18.0 * scale);
    path.line_to(center_x + 12.0 * scale, center_y - 10.0 * scale);
    path.line_to(center_x + 12.0 * scale, center_y + 18.0 * scale);
    path.line_to(center_x - 12.0 * scale, center_y + 18.0 * scale);
    path.close();
    path.move_to(center_x + 4.0 * scale, center_y - 18.0 * scale);
    path.line_to(center_x + 4.0 * scale, center_y - 10.0 * scale);
    path.line_to(center_x + 12.0 * scale, center_y - 10.0 * scale);
    stroke_icon_path(pixmap, path.finish(), color, scale);
}

fn draw_folder_icon(pixmap: &mut Pixmap, center_x: f32, center_y: f32, scale: f32, color: Color) {
    let mut path = PathBuilder::new();
    path.move_to(center_x - 18.0 * scale, center_y - 12.0 * scale);
    path.line_to(center_x - 4.0 * scale, center_y - 12.0 * scale);
    path.line_to(center_x + 1.0 * scale, center_y - 7.0 * scale);
    path.line_to(center_x + 18.0 * scale, center_y - 7.0 * scale);
    path.line_to(center_x + 18.0 * scale, center_y + 15.0 * scale);
    path.line_to(center_x - 18.0 * scale, center_y + 15.0 * scale);
    path.close();
    stroke_icon_path(pixmap, path.finish(), color, scale);
}

fn stroke_icon_path(pixmap: &mut Pixmap, path: Option<Path>, color: Color, scale: f32) {
    let Some(path) = path else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    let stroke = Stroke {
        width: 2.2 * scale,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn fill_round_rect(pixmap: &mut Pixmap, rect: DrawRect, radius: f32, color: Color) {
    let Some(path) = round_rect_path(rect, radius) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn stroke_round_rect(pixmap: &mut Pixmap, rect: DrawRect, radius: f32, color: Color, width: f32) {
    let inset = width / 2.0;
    let Some(path) = round_rect_path(
        DrawRect {
            x: rect.x + inset,
            y: rect.y + inset,
            width: (rect.width - width).max(0.0),
            height: (rect.height - width).max(0.0),
        },
        (radius - inset).max(0.0),
    ) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn round_rect_path(rect: DrawRect, radius: f32) -> Option<Path> {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let radius = radius.min(rect.width / 2.0).min(rect.height / 2.0);
    let right = rect.x + rect.width;
    let bottom = rect.y + rect.height;
    let mut path = PathBuilder::new();
    path.move_to(rect.x + radius, rect.y);
    path.line_to(right - radius, rect.y);
    path.quad_to(right, rect.y, right, rect.y + radius);
    path.line_to(right, bottom - radius);
    path.quad_to(right, bottom, right - radius, bottom);
    path.line_to(rect.x + radius, bottom);
    path.quad_to(rect.x, bottom, rect.x, bottom - radius);
    path.line_to(rect.x, rect.y + radius);
    path.quad_to(rect.x, rect.y, rect.x + radius, rect.y);
    path.close();
    path.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_targets_do_not_overlap_and_include_cancel() {
        let scale = 1.5;
        assert_eq!(
            hit_test(PhysicalPosition::new(100.0 * scale, 160.0 * scale), scale),
            Some(Control::Files)
        );
        assert_eq!(
            hit_test(PhysicalPosition::new(350.0 * scale, 160.0 * scale), scale),
            Some(Control::Folder)
        );
        assert_eq!(
            hit_test(PhysicalPosition::new(440.0 * scale, 280.0 * scale), scale),
            Some(Control::Cancel)
        );
        assert_eq!(
            hit_test(PhysicalPosition::new(260.0 * scale, 160.0 * scale), scale),
            None
        );
    }

    #[test]
    fn keyboard_focus_cycles_in_both_directions() {
        assert_eq!(Control::Files.next(), Control::Folder);
        assert_eq!(Control::Folder.next(), Control::Cancel);
        assert_eq!(Control::Cancel.next(), Control::Files);
        assert_eq!(Control::Files.previous(), Control::Cancel);
    }
}
