#![windows_subsystem = "windows"]

use std::{
    env, ffi::OsStr, fs::{self, DirEntry}, io, mem, os::windows::ffi::OsStrExt, path::{Path, PathBuf}, ptr
};

use dirs::data_local_dir;
use iced::{
    theme::Theme,
    widget::{
        image,
        pane_grid::{self, Axis},
        Button, Image, PaneGrid, Text,
    },
    window::{self},
    Application, Command, Element, Settings,
};
use windows::{core::PCWSTR, Win32::{self, Foundation::{HANDLE, HINSTANCE, HWND}, System::{Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE}, Registry::HKEY}, UI::Shell::SHELLEXECUTEINFOW}};

pub fn main() -> iced::Result {
    let mut args = env::args().skip(1);
    let folder = args
        .next()
        .and_then(|name| data_local_dir().map(|dir| (dir, name)))
        .map(|(dir, name)| dir.join("kslauncher").join(name));
    let new_item = args.next().map(PathBuf::from);
    if let Some(((folder, new_item), file_name)) = folder
        .clone()
        .zip(new_item.as_ref())
        .zip(new_item.as_ref().and_then(|new_item| new_item.file_name()))
    {
        let new_item = Path::new(&new_item);
        let r = fs::rename(new_item, folder.join(file_name));
        match r {
            Ok(()) => Ok(()),
            Err(e) => Launcher::run(Settings {
                window: window::Settings {
                    decorations: false,
                    ..Default::default()
                },
                flags: LauncherFlags {
                    file_move_error: Some(e),
                    folder: Some(folder),
                },
                ..Default::default()
            }),
        }
    } else {
        Launcher::run(Settings {
            window: window::Settings {
                decorations: false,
                ..Default::default()
            },
            flags: LauncherFlags {
                folder,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

struct Launcher {
    pane_grid_state: pane_grid::State<Option<io::Result<DirEntry>>>,
    flags: LauncherFlags,
}

#[derive(Default)]
struct LauncherFlags {
    file_move_error: Option<io::Error>,
    folder: Option<PathBuf>,
}

#[derive(Debug, Clone)]
enum Message {
    Open(PathBuf),
    Close,
}

impl Application for Launcher {
    type Message = Message;

    type Executor = iced::executor::Default;

    type Theme = iced::Theme;

    type Flags = LauncherFlags;

    fn new(flags: Self::Flags) -> (Self, Command<Message>) {
        let state = init_state(&flags);
        (
            Launcher {
                pane_grid_state: state,
                flags,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        String::from("kslauncher")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Open(file_name) => {
                // Open file
                let file_name_wide = OsStr::new(&file_name)
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<u16>>();
                unsafe {
                    Win32::System::Com::CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).unwrap();
                    let mut shell_info = SHELLEXECUTEINFOW {
                        cbSize: mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                        fMask: 0,
                        hwnd: HWND::default(),
                        lpVerb: PCWSTR::null(),
                        lpFile: PCWSTR(file_name_wide.as_ptr()),
                        lpParameters: PCWSTR::null(),
                        lpDirectory: PCWSTR::null(),
                        nShow: 1,
                        hInstApp: HINSTANCE::default(),
                        lpIDList: ptr::null_mut(),
                        lpClass: PCWSTR::null(),
                        hkeyClass: HKEY::default(),
                        dwHotKey: 0,
                        Anonymous: mem::zeroed(),
                        hProcess: HANDLE::default(),
                    };
                    Win32::UI::Shell::ShellExecuteExW(
                        &mut shell_info
                    ).unwrap();
                };
            }
            Message::Close => {
                // Do nothing
            }
        }
        Command::single(iced_runtime::command::Action::Window(
            iced_runtime::window::Action::Close,
        ))
    }

    fn view(&self) -> Element<Message> {
        let content: Element<Message> = match &self.flags.file_move_error {
            Some(e) => Text::new(format!("Failed to add file to launcher folder: {e}")).into(),
            None => PaneGrid::new(
                &self.pane_grid_state,
                |_pane, entry, _maximized| match entry {
                    Some(Ok(entry)) => {
                        let file_path = entry.path();
                        let file_name = file_path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        Button::new(iced::widget::column!(
                            Image::<image::Handle>::new("bleh"),
                            Text::new(file_name.clone())
                        ))
                        .on_press(Message::Open(file_path))
                        .into()
                    }
                    Some(Err(e)) => Text::new(format!("Failed to read file: {e}")).into(),
                    None => Text::new("Folder is empty").into(),
                },
            )
            .into(),
        };
        let close_button = Button::new(Text::new("Close")).on_press(Message::Close);
        iced::widget::column!(close_button, content).into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

fn init_state(flags: &LauncherFlags) -> pane_grid::State<Option<Result<DirEntry, io::Error>>> {
    match &flags.folder {
        Some(folder) => {
            let _ = fs::create_dir_all(folder);
            match fs::read_dir(folder) {
                Ok(mut read_dir) => {
                    let (mut state, mut last_pane) = pane_grid::State::new(read_dir.next());
                    for entry in read_dir {
                        if let Some((pane, _split)) =
                            state.split(Axis::Horizontal, &last_pane, Some(entry))
                        {
                            last_pane = pane;
                        }
                    }
                    state
                }
                Err(e) => {
                    let (state, _) = pane_grid::State::new(Some(Err(e)));
                    state
                }
            }
        }
        None => {
            let (state, _) = pane_grid::State::new(None);
            state
        }
    }
}
