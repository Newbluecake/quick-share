//! Windows desktop owner-window, dialog, tray, and event-loop adapter.

use super::{
    DesktopBroker, DesktopBrokerReceiver, DesktopError, DialogMapper, MessageRequest,
    MessageResult, NativeDialogs, PendingSourceRequest, desktop_broker,
    source_picker::{PickerAction, SourcePicker},
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};
use tray_icon_win::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

/// Non-modal tray actions consumed by the background agent runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    Exit,
}

/// Main-thread Windows desktop runtime. `run` blocks until tray exit or initialization failure.
pub struct WindowsDesktopRuntime {
    event_loop: EventLoop<UserEvent>,
    application: WindowsApplication,
}

/// Thread-safe control used by agent shutdown paths to stop the desktop event loop.
#[derive(Debug, Clone)]
pub struct WindowsDesktopControl {
    proxy: EventLoopProxy<UserEvent>,
}

impl WindowsDesktopControl {
    pub fn exit(&self) -> Result<(), DesktopError> {
        self.proxy
            .send_event(UserEvent::Exit)
            .map_err(|_| DesktopError::EventLoopExited)
    }
}

impl std::fmt::Debug for WindowsDesktopRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsDesktopRuntime")
            .finish_non_exhaustive()
    }
}

/// Creates the Windows desktop broker, bounded tray event stream, and main-thread runtime.
pub fn windows_desktop(
    deadline: Duration,
) -> Result<
    (
        DesktopBroker,
        WindowsDesktopControl,
        mpsc::Receiver<TrayAction>,
        WindowsDesktopRuntime,
    ),
    DesktopError,
> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|_| DesktopError::Unavailable)?;
    let proxy = event_loop.create_proxy();
    let wake_proxy = proxy.clone();
    let wake = Arc::new(move || {
        wake_proxy
            .send_event(UserEvent::Drain)
            .map_err(|_| DesktopError::EventLoopExited)
    });
    let (broker, requests) = desktop_broker(1, deadline, wake);
    let (tray_sender, tray_receiver) = mpsc::sync_channel(4);
    let control = WindowsDesktopControl {
        proxy: proxy.clone(),
    };
    let application = WindowsApplication::new(proxy, requests, tray_sender);
    Ok((
        broker,
        control,
        tray_receiver,
        WindowsDesktopRuntime {
            event_loop,
            application,
        },
    ))
}

impl WindowsDesktopRuntime {
    /// Runs native dialogs and tray events on the creating thread.
    pub fn run(mut self) -> Result<(), DesktopError> {
        self.event_loop
            .run_app(&mut self.application)
            .map_err(|_| DesktopError::Backend)?;
        if self.application.initialization_failed {
            Err(DesktopError::Unavailable)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
enum UserEvent {
    Drain,
    Exit,
    Menu(MenuId),
}

struct WindowsApplication {
    proxy: EventLoopProxy<UserEvent>,
    requests: DesktopBrokerReceiver,
    tray_sender: mpsc::SyncSender<TrayAction>,
    owner: Option<Arc<Window>>,
    backend: Option<DialogMapper<WindowsDialogs>>,
    source_picker: Option<SourcePicker>,
    pending_source: Option<PendingSourceRequest>,
    tray: Option<TrayIcon>,
    open_id: Option<MenuId>,
    exit_id: Option<MenuId>,
    menu_handler_installed: bool,
    initialization_failed: bool,
}

impl WindowsApplication {
    fn new(
        proxy: EventLoopProxy<UserEvent>,
        requests: DesktopBrokerReceiver,
        tray_sender: mpsc::SyncSender<TrayAction>,
    ) -> Self {
        Self {
            proxy,
            requests,
            tray_sender,
            owner: None,
            backend: None,
            source_picker: None,
            pending_source: None,
            tray: None,
            open_id: None,
            exit_id: None,
            menu_handler_installed: false,
            initialization_failed: false,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), DesktopError> {
        let owner = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Quick Share")
                        .with_visible(false),
                )
                .map_err(|_| DesktopError::Unavailable)?,
        );
        let menu = Menu::new();
        let open = MenuItem::new("Open Quick Share", true, None);
        let exit = MenuItem::new("Exit", true, None);
        menu.append_items(&[&open, &exit])
            .map_err(|_| DesktopError::Backend)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Quick Share")
            .with_icon(quick_share_icon().map_err(|_| DesktopError::Backend)?)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|_| DesktopError::Unavailable)?;
        self.open_id = Some(open.id().clone());
        self.exit_id = Some(exit.id().clone());
        if !self.menu_handler_installed {
            let proxy = self.proxy.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let _ = proxy.send_event(UserEvent::Menu(event.id));
            }));
            self.menu_handler_installed = true;
        }
        self.backend = Some(DialogMapper::new(WindowsDialogs));
        self.owner = Some(owner);
        self.tray = Some(tray);
        Ok(())
    }

    fn drain(&mut self, event_loop: &ActiveEventLoop) {
        if self.pending_source.is_some() {
            return;
        }
        let pending = self
            .backend
            .as_ref()
            .and_then(|backend| self.requests.drain_for_source_picker(backend));
        let Some(pending) = pending else {
            return;
        };
        match SourcePicker::new(event_loop, pending.request().requester_name.clone()) {
            Ok(picker) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(pending.expires_at()));
                self.pending_source = Some(pending);
                self.source_picker = Some(picker);
            }
            Err(error) => pending.respond(Err(error)),
        }
    }

    fn finish_source_picker(&mut self, action: PickerAction, event_loop: &ActiveEventLoop) {
        self.source_picker = None;
        let Some(pending) = self.pending_source.take() else {
            return;
        };
        event_loop.set_control_flow(ControlFlow::Wait);
        let response = match action {
            PickerAction::Pick(kind) => self
                .backend
                .as_ref()
                .ok_or(DesktopError::Unavailable)
                .and_then(|backend| backend.pick_send_source(pending.request(), kind)),
            PickerAction::Cancel => Ok(super::SourceChoice::Cancelled),
            PickerAction::RenderFailed => Err(DesktopError::Backend),
        };
        pending.respond(response);
    }

    fn exit(&mut self, event_loop: &ActiveEventLoop) {
        self.source_picker = None;
        if let Some(pending) = self.pending_source.take() {
            pending.respond(Err(DesktopError::EventLoopExited));
        }
        let _ = self.tray_sender.try_send(TrayAction::Exit);
        self.requests.shutdown();
        event_loop.exit();
    }
}

impl Drop for WindowsApplication {
    fn drop(&mut self) {
        self.requests.shutdown();
        if self.menu_handler_installed {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
            self.menu_handler_installed = false;
        }
    }
}

impl ApplicationHandler<UserEvent> for WindowsApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.owner.is_none() {
            if self.initialize(event_loop).is_err() {
                self.initialization_failed = true;
                self.requests.shutdown();
                event_loop.exit();
                return;
            }
            self.drain(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Drain => self.drain(event_loop),
            UserEvent::Exit => self.exit(event_loop),
            UserEvent::Menu(id) if self.open_id.as_ref() == Some(&id) => {
                let _ = self.tray_sender.try_send(TrayAction::Open);
            }
            UserEvent::Menu(id) if self.exit_id.as_ref() == Some(&id) => self.exit(event_loop),
            UserEvent::Menu(_) => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let action = self
            .source_picker
            .as_mut()
            .filter(|picker| picker.window_id() == window_id)
            .and_then(|picker| picker.handle_event(&event));
        if let Some(action) = action {
            self.finish_source_picker(action, event_loop);
        }
        // The owner is deliberately hidden and is not an application-close surface.
        // Only tray Exit or the explicit control channel may stop the resident agent.
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .pending_source
            .as_ref()
            .is_some_and(|pending| std::time::Instant::now() >= pending.expires_at())
        {
            self.finish_source_picker(PickerAction::Cancel, event_loop);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.requests.shutdown();
        if self.menu_handler_installed {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
            self.menu_handler_installed = false;
        }
        self.tray = None;
        self.source_picker = None;
        self.pending_source = None;
        self.backend = None;
        self.owner = None;
    }
}

#[derive(Debug)]
struct WindowsDialogs;

impl NativeDialogs for WindowsDialogs {
    fn show_message(&self, request: &MessageRequest) -> Result<MessageResult, DesktopError> {
        let buttons = match (&request.secondary, &request.tertiary) {
            (None, None) => MessageButtons::OkCustom(request.primary.clone()),
            (Some(secondary), None) => {
                MessageButtons::OkCancelCustom(request.primary.clone(), secondary.clone())
            }
            (Some(secondary), Some(tertiary)) => MessageButtons::YesNoCancelCustom(
                request.primary.clone(),
                secondary.clone(),
                tertiary.clone(),
            ),
            (None, Some(_)) => return Err(DesktopError::Backend),
        };
        let result = MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title(&request.title)
            .set_description(&request.description)
            .set_buttons(buttons)
            .show();
        Ok(match result {
            MessageDialogResult::Custom(label) if label == request.primary => {
                MessageResult::Primary
            }
            MessageDialogResult::Custom(label)
                if request.secondary.as_deref() == Some(label.as_str()) =>
            {
                MessageResult::Secondary
            }
            MessageDialogResult::Custom(label)
                if request.tertiary.as_deref() == Some(label.as_str()) =>
            {
                MessageResult::Tertiary
            }
            MessageDialogResult::Ok if request.secondary.is_none() => MessageResult::Primary,
            MessageDialogResult::Cancel
            | MessageDialogResult::Ok
            | MessageDialogResult::Yes
            | MessageDialogResult::No
            | MessageDialogResult::Custom(_) => MessageResult::Closed,
        })
    }

    fn pick_files(
        &self,
        title: &str,
        initial_directory: &Path,
    ) -> Result<Option<Vec<PathBuf>>, DesktopError> {
        Ok(FileDialog::new()
            .set_title(title)
            .set_directory(initial_directory)
            .pick_files())
    }

    fn pick_folder(
        &self,
        title: &str,
        initial_directory: &Path,
    ) -> Result<Option<PathBuf>, DesktopError> {
        Ok(FileDialog::new()
            .set_title(title)
            .set_directory(initial_directory)
            .pick_folder())
    }
}

fn quick_share_icon() -> Result<Icon, tray_icon_win::BadIcon> {
    const SIZE: u32 = 32;
    let rgba = super::icon::quick_share_icon_rgba(SIZE);
    Icon::from_rgba(rgba, SIZE, SIZE)
}
