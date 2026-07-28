use super::{
    AuthorizationChoice, AuthorizationDialog, ConflictAction, ConflictChoice, ConflictDialog,
    ConflictScope, DesktopError, DesktopInteraction, DesktopNotification, DirectoryChoice,
    ReceiveDirectoryDialog, SourceChoice, SourceDialog,
};
use std::{env, fs, io, path::Path, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageResult {
    Primary,
    Secondary,
    Tertiary,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageRequest {
    pub title: String,
    pub description: String,
    pub primary: String,
    pub secondary: Option<String>,
    pub tertiary: Option<String>,
}

pub(crate) trait NativeDialogs: Send + Sync {
    fn show_message(&self, request: &MessageRequest) -> Result<MessageResult, DesktopError>;

    fn pick_files(
        &self,
        title: &str,
        initial_directory: &Path,
    ) -> Result<Option<Vec<PathBuf>>, DesktopError>;

    fn pick_folder(
        &self,
        title: &str,
        initial_directory: &Path,
    ) -> Result<Option<PathBuf>, DesktopError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Files,
    Folder,
}

#[derive(Debug)]
pub(crate) struct DialogMapper<D> {
    dialogs: D,
}

impl<D> DialogMapper<D> {
    pub(crate) const fn new(dialogs: D) -> Self {
        Self { dialogs }
    }
}

impl<D: NativeDialogs> DialogMapper<D> {
    pub(crate) fn pick_send_source(
        &self,
        request: &SourceDialog,
        kind: SourceKind,
    ) -> Result<SourceChoice, DesktopError> {
        let initial = if request.initial_directory.is_dir() {
            request.initial_directory.clone()
        } else {
            env::current_dir().map_err(|_| DesktopError::Backend)?
        };
        match kind {
            SourceKind::Files => match self.dialogs.pick_files("选择一个或多个文件", &initial)?
            {
                Some(paths) if paths.is_empty() => Err(DesktopError::InvalidSelection),
                Some(paths) => Ok(SourceChoice::Files(paths)),
                None => Ok(SourceChoice::Cancelled),
            },
            SourceKind::Folder => match self.dialogs.pick_folder("选择文件夹", &initial)? {
                Some(path) if path.as_os_str().is_empty() => Err(DesktopError::InvalidSelection),
                Some(path) => Ok(SourceChoice::Folder(path)),
                None => Ok(SourceChoice::Cancelled),
            },
        }
    }

    fn choose_scope(&self) -> Result<Option<ConflictScope>, DesktopError> {
        let result = self.dialogs.show_message(&MessageRequest {
            title: "应用冲突处理".to_owned(),
            description: "仅处理此项，还是应用到后续同类冲突？".to_owned(),
            primary: "仅此项".to_owned(),
            secondary: Some("应用到全部".to_owned()),
            tertiary: Some("取消传输".to_owned()),
        })?;
        Ok(match result {
            MessageResult::Primary => Some(ConflictScope::ThisEntry),
            MessageResult::Secondary => Some(ConflictScope::AllRemaining),
            MessageResult::Tertiary | MessageResult::Closed => None,
        })
    }
}

impl<D: NativeDialogs> DesktopInteraction for DialogMapper<D> {
    fn authorize_peer(
        &self,
        request: &AuthorizationDialog,
    ) -> Result<AuthorizationChoice, DesktopError> {
        if request.identity_changed {
            return Ok(
                match self.dialogs.show_message(&MessageRequest {
                    title: "设备身份已变化".to_owned(),
                    description: format!(
                        "拒绝设备 {}（{}）：其认证身份已经变化。验证码：{}",
                        request.device_name, request.device_id, request.sas
                    ),
                    primary: "拒绝".to_owned(),
                    secondary: Some("取消".to_owned()),
                    tertiary: None,
                })? {
                    MessageResult::Primary => AuthorizationChoice::Reject,
                    MessageResult::Secondary | MessageResult::Tertiary | MessageResult::Closed => {
                        AuthorizationChoice::Cancelled
                    }
                },
            );
        }

        Ok(
            match self.dialogs.show_message(&MessageRequest {
                title: "授权设备".to_owned(),
                description: format!(
                    "是否允许设备 {}（{}）？验证码：{}",
                    request.device_name, request.device_id, request.sas
                ),
                primary: "仅允许一次".to_owned(),
                secondary: Some("允许并信任".to_owned()),
                tertiary: Some("拒绝".to_owned()),
            })? {
                MessageResult::Primary => AuthorizationChoice::AcceptOnce,
                MessageResult::Secondary => AuthorizationChoice::AcceptAndTrust,
                MessageResult::Tertiary => AuthorizationChoice::Reject,
                MessageResult::Closed => AuthorizationChoice::Cancelled,
            },
        )
    }

    fn choose_send_source(&self, request: &SourceDialog) -> Result<SourceChoice, DesktopError> {
        let result = self.dialogs.show_message(&MessageRequest {
            title: "选择要发送的内容".to_owned(),
            description: format!("设备 {} 请求从此设备选择内容。", request.requester_name),
            primary: "选择文件".to_owned(),
            secondary: Some("选择文件夹".to_owned()),
            tertiary: Some("取消".to_owned()),
        })?;
        match result {
            MessageResult::Primary => self.pick_send_source(request, SourceKind::Files),
            MessageResult::Secondary => self.pick_send_source(request, SourceKind::Folder),
            MessageResult::Tertiary | MessageResult::Closed => Ok(SourceChoice::Cancelled),
        }
    }

    fn confirm_receive_directory(
        &self,
        request: &ReceiveDirectoryDialog,
    ) -> Result<DirectoryChoice, DesktopError> {
        let result = self.dialogs.show_message(&MessageRequest {
            title: "选择保存目录".to_owned(),
            description: format!(
                "是否接收来自 {} 的内容并保存到以下目录？\n{}",
                request.sender_name,
                request.current_directory.display()
            ),
            primary: "接收".to_owned(),
            secondary: Some("更改目录".to_owned()),
            tertiary: Some("取消".to_owned()),
        })?;
        let selected = match result {
            MessageResult::Primary => request.current_directory.clone(),
            MessageResult::Secondary => match self
                .dialogs
                .pick_folder("选择保存目录", &request.current_directory)?
            {
                Some(path) => path,
                None => return Ok(DirectoryChoice::Cancelled),
            },
            MessageResult::Tertiary | MessageResult::Closed => {
                return Ok(DirectoryChoice::Cancelled);
            }
        };
        validate_receive_directory(&selected)?;
        Ok(DirectoryChoice::Confirm(selected))
    }

    fn resolve_conflict(&self, request: &ConflictDialog) -> Result<ConflictChoice, DesktopError> {
        let kind = if request.directory {
            "目录"
        } else {
            "文件"
        };
        let result = self.dialogs.show_message(&MessageRequest {
            title: "处理接收冲突".to_owned(),
            description: format!("目标位置 {} 已存在同名{kind}。", request.relative_path),
            primary: "覆盖".to_owned(),
            secondary: Some("跳过".to_owned()),
            tertiary: Some("重命名".to_owned()),
        })?;
        let action = match result {
            MessageResult::Primary => ConflictAction::Overwrite,
            MessageResult::Secondary => ConflictAction::Skip,
            MessageResult::Tertiary => ConflictAction::Rename,
            MessageResult::Closed => return Ok(ConflictChoice::Cancelled),
        };
        let Some(scope) = self.choose_scope()? else {
            return Ok(ConflictChoice::Cancelled);
        };
        Ok(ConflictChoice::Decision { action, scope })
    }

    fn notify(&self, notification: &DesktopNotification) -> Result<(), DesktopError> {
        let _ = self.dialogs.show_message(&MessageRequest {
            title: notification.title.clone(),
            description: notification.message.clone(),
            primary: "OK".to_owned(),
            secondary: None,
            tertiary: None,
        })?;
        Ok(())
    }
}

fn validate_receive_directory(path: &Path) -> Result<(), DesktopError> {
    let metadata = fs::metadata(path).map_err(|error| match error.kind() {
        io::ErrorKind::PermissionDenied => DesktopError::DirectoryNotWritable,
        _ => DesktopError::InvalidDirectory,
    })?;
    if !metadata.is_dir() {
        return Err(DesktopError::InvalidDirectory);
    }
    tempfile::NamedTempFile::new_in(path)
        .map(drop)
        .map_err(|_| DesktopError::DirectoryNotWritable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Debug, Default)]
    struct FakeDialogs {
        messages: Mutex<VecDeque<MessageResult>>,
        files: Mutex<VecDeque<Option<Vec<PathBuf>>>>,
        folders: Mutex<VecDeque<Option<PathBuf>>>,
        seen: Mutex<Vec<MessageRequest>>,
    }

    impl FakeDialogs {
        fn with_messages(results: impl IntoIterator<Item = MessageResult>) -> Self {
            Self {
                messages: Mutex::new(results.into_iter().collect()),
                ..Self::default()
            }
        }
    }

    impl NativeDialogs for FakeDialogs {
        fn show_message(&self, request: &MessageRequest) -> Result<MessageResult, DesktopError> {
            self.seen.lock().expect("seen lock").push(request.clone());
            self.messages
                .lock()
                .expect("message lock")
                .pop_front()
                .ok_or(DesktopError::Backend)
        }

        fn pick_files(
            &self,
            _title: &str,
            _initial_directory: &Path,
        ) -> Result<Option<Vec<PathBuf>>, DesktopError> {
            self.files
                .lock()
                .expect("files lock")
                .pop_front()
                .ok_or(DesktopError::Backend)
        }

        fn pick_folder(
            &self,
            _title: &str,
            _initial_directory: &Path,
        ) -> Result<Option<PathBuf>, DesktopError> {
            self.folders
                .lock()
                .expect("folders lock")
                .pop_front()
                .ok_or(DesktopError::Backend)
        }
    }

    fn authorization(identity_changed: bool) -> AuthorizationDialog {
        AuthorizationDialog {
            device_name: "peer".to_owned(),
            device_id: "qs_device".to_owned(),
            sas: "123456".to_owned(),
            identity_changed,
        }
    }

    #[test]
    fn authorization_maps_every_button_and_close_without_implicit_trust() {
        for (raw, expected) in [
            (MessageResult::Primary, AuthorizationChoice::AcceptOnce),
            (
                MessageResult::Secondary,
                AuthorizationChoice::AcceptAndTrust,
            ),
            (MessageResult::Tertiary, AuthorizationChoice::Reject),
            (MessageResult::Closed, AuthorizationChoice::Cancelled),
        ] {
            let mapper = DialogMapper::new(FakeDialogs::with_messages([raw]));
            assert_eq!(mapper.authorize_peer(&authorization(false)), Ok(expected));
        }
        for (raw, expected) in [
            (MessageResult::Primary, AuthorizationChoice::Reject),
            (MessageResult::Secondary, AuthorizationChoice::Cancelled),
            (MessageResult::Closed, AuthorizationChoice::Cancelled),
        ] {
            let mapper = DialogMapper::new(FakeDialogs::with_messages([raw]));
            assert_eq!(mapper.authorize_peer(&authorization(true)), Ok(expected));
        }
    }

    #[test]
    fn source_mapping_supports_multi_file_folder_empty_and_close() {
        let dialogs = FakeDialogs::with_messages([MessageResult::Primary]);
        dialogs
            .files
            .lock()
            .expect("files lock")
            .push_back(Some(vec![PathBuf::from("a"), PathBuf::from("b")]));
        let mapper = DialogMapper::new(dialogs);
        assert!(matches!(
            mapper.choose_send_source(&SourceDialog {
                requester_name: "peer".to_owned(),
                initial_directory: env::current_dir().expect("current directory"),
            }),
            Ok(SourceChoice::Files(paths)) if paths.len() == 2
        ));
        let seen = mapper.dialogs.seen.lock().expect("seen lock");
        assert_eq!(seen[0].primary, "选择文件");
        assert_eq!(seen[0].secondary.as_deref(), Some("选择文件夹"));
        assert_eq!(seen[0].tertiary.as_deref(), Some("取消"));
        drop(seen);

        let dialogs = FakeDialogs::with_messages([MessageResult::Secondary]);
        dialogs
            .folders
            .lock()
            .expect("folders lock")
            .push_back(Some(PathBuf::from("folder")));
        assert!(matches!(
            DialogMapper::new(dialogs).choose_send_source(&SourceDialog {
                requester_name: "peer".to_owned(),
                initial_directory: env::current_dir().expect("current directory"),
            }),
            Ok(SourceChoice::Folder(_))
        ));

        let dialogs = FakeDialogs::with_messages([MessageResult::Primary]);
        dialogs
            .files
            .lock()
            .expect("files lock")
            .push_back(Some(Vec::new()));
        assert_eq!(
            DialogMapper::new(dialogs).choose_send_source(&SourceDialog {
                requester_name: "peer".to_owned(),
                initial_directory: env::current_dir().expect("current directory"),
            }),
            Err(DesktopError::InvalidSelection)
        );
        assert_eq!(
            DialogMapper::new(FakeDialogs::with_messages([MessageResult::Closed]))
                .choose_send_source(&SourceDialog {
                    requester_name: "peer".to_owned(),
                    initial_directory: env::current_dir().expect("current directory"),
                }),
            Ok(SourceChoice::Cancelled)
        );
    }

    #[test]
    fn receive_directory_validates_confirm_change_cancel_and_invalid_paths() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let request = ReceiveDirectoryDialog {
            sender_name: "peer".to_owned(),
            current_directory: directory.path().to_path_buf(),
        };
        let mapper = DialogMapper::new(FakeDialogs::with_messages([MessageResult::Primary]));
        assert!(matches!(
            mapper.confirm_receive_directory(&request),
            Ok(DirectoryChoice::Confirm(_))
        ));

        let changed = tempfile::tempdir().expect("changed directory");
        let dialogs = FakeDialogs::with_messages([MessageResult::Secondary]);
        dialogs
            .folders
            .lock()
            .expect("folders lock")
            .push_back(Some(changed.path().to_path_buf()));
        assert_eq!(
            DialogMapper::new(dialogs).confirm_receive_directory(&request),
            Ok(DirectoryChoice::Confirm(changed.path().to_path_buf()))
        );

        let cancelled = FakeDialogs::with_messages([MessageResult::Secondary]);
        cancelled
            .folders
            .lock()
            .expect("folders lock")
            .push_back(None);
        assert_eq!(
            DialogMapper::new(cancelled).confirm_receive_directory(&request),
            Ok(DirectoryChoice::Cancelled)
        );

        let file = directory.path().join("not-a-directory");
        fs::write(&file, b"x").expect("write fixture");
        let invalid = ReceiveDirectoryDialog {
            sender_name: "peer".to_owned(),
            current_directory: file,
        };
        assert_eq!(
            DialogMapper::new(FakeDialogs::with_messages([MessageResult::Primary]))
                .confirm_receive_directory(&invalid),
            Err(DesktopError::InvalidDirectory)
        );

        assert_eq!(
            DialogMapper::new(FakeDialogs::with_messages([MessageResult::Closed]))
                .confirm_receive_directory(&request),
            Ok(DirectoryChoice::Cancelled)
        );
    }

    #[test]
    fn conflict_uses_two_steps_and_close_on_either_step_cancels() {
        for (raw, action) in [
            (MessageResult::Primary, ConflictAction::Overwrite),
            (MessageResult::Secondary, ConflictAction::Skip),
            (MessageResult::Tertiary, ConflictAction::Rename),
        ] {
            let mapper =
                DialogMapper::new(FakeDialogs::with_messages([raw, MessageResult::Secondary]));
            assert_eq!(
                mapper.resolve_conflict(&ConflictDialog {
                    relative_path: "entry".to_owned(),
                    directory: false,
                }),
                Ok(ConflictChoice::Decision {
                    action,
                    scope: ConflictScope::AllRemaining,
                })
            );
        }
        for results in [
            vec![MessageResult::Closed],
            vec![MessageResult::Primary, MessageResult::Closed],
        ] {
            assert_eq!(
                DialogMapper::new(FakeDialogs::with_messages(results)).resolve_conflict(
                    &ConflictDialog {
                        relative_path: "entry".to_owned(),
                        directory: true,
                    }
                ),
                Ok(ConflictChoice::Cancelled)
            );
        }
    }
}
