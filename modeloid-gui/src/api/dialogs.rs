//! Native file and folder dialogs, opened on the server machine (the server
//! runs locally; browsers can't hand out real paths). Each kind of dialog
//! starts in the folder it was last used in, and is brought to the front.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// What a dialog is for; each purpose remembers its own last folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogPurpose {
    /// Creating or loading a workspace.
    Workspace,
    /// Importing a dataset folder.
    DatasetFolder,
    /// Importing a dataset zip.
    DatasetZip,
}

impl DialogPurpose {
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    fn key(self) -> &'static str {
        match self {
            DialogPurpose::Workspace => "workspace",
            DialogPurpose::DatasetFolder => "dataset-folder",
            DialogPurpose::DatasetZip => "dataset-zip",
        }
    }
}

/// Opens a native folder picker. `None` if the user cancels.
#[post("/api/fs/pick-folder")]
pub async fn pick_folder(title: String, purpose: DialogPurpose) -> Result<Option<String>, ServerFnError> {
    let dir = backend::pick_folder(title, purpose).await?;
    Ok(dir.map(|d| d.display().to_string()))
}

/// Opens a native file picker limited to `extensions` (e.g. `["zip"]`). `None` if cancelled.
#[post("/api/fs/pick-file")]
pub async fn pick_file(
    title: String,
    purpose: DialogPurpose,
    filter_name: String,
    extensions: Vec<String>,
) -> Result<Option<String>, ServerFnError> {
    let file = backend::pick_file(title, purpose, filter_name, extensions).await?;
    Ok(file.map(|f| f.display().to_string()))
}

#[cfg(feature = "server")]
pub use backend::pick_folder as pick_folder_path;

#[cfg(feature = "server")]
mod backend {
    use std::path::{Path, PathBuf};

    use dioxus::prelude::ServerFnError;
    use modeloid::Settings;
    use rfd::FileDialog;

    use super::{front, DialogPurpose};

    pub async fn pick_folder(title: String, purpose: DialogPurpose) -> Result<Option<PathBuf>, ServerFnError> {
        // Next time, start next to the chosen folder so its siblings show.
        open(purpose, move |d| d.set_title(title).pick_folder(), |dir| dir.parent().unwrap_or(dir).to_path_buf()).await
    }

    pub async fn pick_file(
        title: String,
        purpose: DialogPurpose,
        filter_name: String,
        extensions: Vec<String>,
    ) -> Result<Option<PathBuf>, ServerFnError> {
        let show = move |dialog: FileDialog| {
            let dialog = dialog.set_title(title);
            let dialog = if extensions.is_empty() { dialog } else { dialog.add_filter(filter_name, &extensions) };
            dialog.pick_file()
        };
        open(purpose, show, |file| file.parent().unwrap_or(file).to_path_buf()).await
    }

    /// Shows a dialog off the async runtime (on the main thread on macOS, which
    /// needs it there), starting in the purpose's last folder, and remembers
    /// where the user went (`remember` maps the pick to a folder).
    async fn open(
        purpose: DialogPurpose,
        show: impl FnOnce(FileDialog) -> Option<PathBuf> + Send + 'static,
        remember: impl FnOnce(&Path) -> PathBuf + Send + 'static,
    ) -> Result<Option<PathBuf>, ServerFnError> {
        crate::main_thread::run(move || {
            let mut dialog = FileDialog::new();
            if let Some(dir) = Settings::load_default().last_folder(purpose.key()) {
                dialog = dialog.set_directory(dir);
            }
            // Owner window in front, so the dialog doesn't open behind the browser.
            let owner = front::Owner::new();
            if let Some(owner) = &owner {
                dialog = dialog.set_parent(owner);
            }
            let picked = show(dialog);
            drop(owner);

            if let Some(path) = &picked {
                // Re-read just before saving so settings changed meanwhile aren't lost.
                if let Some(settings_path) = Settings::default_path() {
                    if let Ok(mut settings) = Settings::load(&settings_path) {
                        settings.set_last_folder(purpose.key(), remember(path));
                        let _ = settings.save(&settings_path);
                    }
                }
            }
            picked
        })
        .await
        .map_err(ServerFnError::new)
    }
}

/// A tiny always-on-top window that owns the dialog. The server is a
/// background process, which Windows won't let raise windows on its own; the
/// owner borrows the focus of the current foreground window (the browser) and
/// the dialog opens above it, centred on screen. Other systems: no owner.
#[cfg(feature = "server")]
mod front {
    #[cfg(windows)]
    pub use imp::Owner;

    #[cfg(not(windows))]
    pub struct Owner;

    #[cfg(not(windows))]
    impl Owner {
        pub fn new() -> Option<Owner> {
            None
        }
    }

    #[cfg(not(windows))]
    impl raw_window_handle::HasWindowHandle for Owner {
        fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::NotSupported)
        }
    }

    #[cfg(not(windows))]
    impl raw_window_handle::HasDisplayHandle for Owner {
        fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::NotSupported)
        }
    }

    #[cfg(windows)]
    mod imp {
        use std::num::NonZeroIsize;
        use std::ptr::{null, null_mut};

        use raw_window_handle::{
            DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
        };
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, CreateWindowExW, DestroyWindow, GetForegroundWindow, GetSystemMetrics,
            GetWindowThreadProcessId, SetForegroundWindow, ShowWindow, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNA,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        };

        pub struct Owner(HWND);

        impl Owner {
            pub fn new() -> Option<Owner> {
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                // SAFETY: plain Win32 calls on a window this thread creates and owns.
                unsafe {
                    let (x, y) = (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2);
                    let hwnd = CreateWindowExW(
                        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                        class.as_ptr(),
                        null(),
                        WS_POPUP,
                        x,
                        y,
                        1,
                        1,
                        null_mut(),
                        null_mut(),
                        null_mut(),
                        null(),
                    );
                    if hwnd.is_null() {
                        return None;
                    }

                    // Windows only lets the foreground thread hand out focus: join its input queue briefly.
                    let me = GetCurrentThreadId();
                    let foreground = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
                    let attached = foreground != 0 && foreground != me && AttachThreadInput(me, foreground, 1) != 0;
                    ShowWindow(hwnd, SW_SHOWNA);
                    BringWindowToTop(hwnd);
                    SetForegroundWindow(hwnd);
                    if attached {
                        AttachThreadInput(me, foreground, 0);
                    }
                    Some(Owner(hwnd))
                }
            }
        }

        impl Drop for Owner {
            fn drop(&mut self) {
                // SAFETY: the window was created by this thread and is destroyed once.
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }

        impl HasWindowHandle for Owner {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                let hwnd = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
                // SAFETY: the window lives as long as `self`.
                Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd))) })
            }
        }

        impl HasDisplayHandle for Owner {
            fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
                Ok(DisplayHandle::windows())
            }
        }
    }
}
