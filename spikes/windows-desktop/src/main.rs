#![forbid(unsafe_code)]

#[cfg(not(windows))]
fn main() {
    eprintln!("qs-windows-desktop-spike only exercises native UI on Windows");
}

#[cfg(windows)]
mod windows_app {
    use qs_windows_desktop_spike::UiGate;
    use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    use std::{
        error::Error,
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };
    use tray_icon_win::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuId, MenuItem},
    };
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{UserAttentionType, Window, WindowId},
    };

    #[derive(Debug)]
    enum UserEvent {
        DrainUi,
        Menu(MenuId),
    }

    #[derive(Debug, Clone, Copy)]
    enum UiRequest {
        ShowPicker,
    }

    struct DesktopProbe {
        proxy: EventLoopProxy<UserEvent>,
        requests: mpsc::Receiver<UiRequest>,
        gate: Arc<UiGate>,
        window: Option<Window>,
        tray: Option<TrayIcon>,
        show_id: Option<MenuId>,
        exit_id: Option<MenuId>,
        event_handler_installed: bool,
    }

    impl DesktopProbe {
        fn new(proxy: EventLoopProxy<UserEvent>, requests: mpsc::Receiver<UiRequest>) -> Self {
            Self {
                proxy,
                requests,
                gate: Arc::new(UiGate::default()),
                window: None,
                tray: None,
                show_id: None,
                exit_id: None,
                event_handler_installed: false,
            }
        }

        fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
            let window = event_loop.create_window(
                Window::default_attributes()
                    .with_title("Quick Share desktop probe")
                    .with_visible(false),
            )?;
            let menu = Menu::new();
            let show = MenuItem::new("打开选择器", true, None);
            let exit = MenuItem::new("退出", true, None);
            menu.append_items(&[&show, &exit])?;
            self.show_id = Some(show.id().clone());
            self.exit_id = Some(exit.id().clone());
            let icon = probe_icon()?;
            self.tray = Some(
                TrayIconBuilder::new()
                    .with_tooltip("Quick Share Windows desktop probe")
                    .with_icon(icon)
                    .with_menu(Box::new(menu))
                    .build()?,
            );
            if !self.event_handler_installed {
                let proxy = self.proxy.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    let _ = proxy.send_event(UserEvent::Menu(event.id));
                }));
                self.event_handler_installed = true;
            }
            self.window = Some(window);
            Ok(())
        }

        fn show_picker(&self) {
            let Ok(_lease) = self.gate.try_acquire() else {
                return;
            };
            let Some(window) = self.window.as_ref() else {
                return;
            };
            window.request_user_attention(Some(UserAttentionType::Informational));
            let result = MessageDialog::new()
                .set_parent(window)
                .set_level(MessageLevel::Info)
                .set_title("Quick Share")
                .set_description("请选择要发送的内容类型。关闭窗口将取消操作。")
                .set_buttons(MessageButtons::YesNoCancelCustom(
                    "选择文件".to_owned(),
                    "选择文件夹".to_owned(),
                    "取消".to_owned(),
                ))
                .show();
            match result {
                MessageDialogResult::Custom(label) if label == "选择文件" => {
                    let files = FileDialog::new()
                        .set_parent(window)
                        .set_title("选择一个或多个文件")
                        .set_directory(initial_directory())
                        .pick_files();
                    show_summary(
                        window,
                        files.as_ref().map_or("已取消文件选择".to_owned(), |items| {
                            format!("已选择 {} 个文件。", items.len())
                        }),
                    );
                }
                MessageDialogResult::Custom(label) if label == "选择文件夹" => {
                    let folder = FileDialog::new()
                        .set_parent(window)
                        .set_title("选择文件夹")
                        .set_directory(initial_directory())
                        .pick_folder();
                    show_summary(
                        window,
                        folder
                            .as_ref()
                            .map_or("已取消文件夹选择".to_owned(), |path| {
                                format!("已选择文件夹：{}", path.display())
                            }),
                    );
                }
                _ => show_summary(window, "已取消。".to_owned()),
            }
        }
    }

    impl ApplicationHandler<UserEvent> for DesktopProbe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none()
                && let Err(error) = self.initialize(event_loop)
            {
                eprintln!("desktop initialization failed: {error}");
                event_loop.exit();
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
            match event {
                UserEvent::DrainUi => {
                    while let Ok(request) = self.requests.try_recv() {
                        match request {
                            UiRequest::ShowPicker => self.show_picker(),
                        }
                    }
                }
                UserEvent::Menu(id) if self.show_id.as_ref() == Some(&id) => self.show_picker(),
                UserEvent::Menu(id) if self.exit_id.as_ref() == Some(&id) => event_loop.exit(),
                UserEvent::Menu(_) => {}
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            if event == WindowEvent::CloseRequested {
                event_loop.exit();
            }
        }
    }

    fn initial_directory() -> std::path::PathBuf {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    }

    fn show_summary(window: &Window, description: String) {
        let _ = MessageDialog::new()
            .set_parent(window)
            .set_level(MessageLevel::Info)
            .set_title("Quick Share probe result")
            .set_description(description)
            .set_buttons(MessageButtons::Ok)
            .show();
    }

    fn probe_icon() -> Result<Icon, Box<dyn Error>> {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16 {
            for x in 0..16 {
                let accent = (x + y) % 4 < 2;
                rgba.extend_from_slice(if accent {
                    &[40, 120, 235, 255]
                } else {
                    &[235, 245, 255, 255]
                });
            }
        }
        Ok(Icon::from_rgba(rgba, 16, 16)?)
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let worker_proxy = proxy.clone();
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_time()
                .build()
                .expect("build probe Tokio runtime");
            runtime.block_on(async move {
                tokio::time::sleep(Duration::from_millis(750)).await;
                if request_tx.try_send(UiRequest::ShowPicker).is_ok() {
                    let _ = worker_proxy.send_event(UserEvent::DrainUi);
                }
            });
        });
        let mut app = DesktopProbe::new(proxy, request_rx);
        event_loop.run_app(&mut app)?;
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_app::run() {
        eprintln!("Windows desktop probe failed: {error}");
        std::process::exit(1);
    }
}
