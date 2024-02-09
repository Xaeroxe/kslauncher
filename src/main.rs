#![windows_subsystem = "windows"]

use std::{
    convert::Infallible,
    env,
    ffi::OsStr,
    fs,
    hash::Hasher,
    io, mem,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process, ptr,
};

use dirs::data_local_dir;
use iced::{
    alignment::{Horizontal, Vertical},
    futures::{channel::mpsc::Sender, future, stream, SinkExt, StreamExt},
    subscription,
    theme::{self, Palette, Theme},
    widget::{image, Button, Container, Image, Space, Text},
    window, Application, Color, Command, Element, Length, Settings, Subscription,
};
use iced_runtime::futures::subscription::Recipe;
use notify::event::{ModifyKind, RenameMode};
use windows::{
    core::PCWSTR,
    Win32::{
        self,
        Foundation::{HANDLE, HINSTANCE, HWND},
        Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        System::{
            Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
            Registry::HKEY,
        },
        UI::{
            Controls::{IImageList, ILD_TRANSPARENT},
            Shell::{
                SHGetFileInfoW, SHGetImageList, SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_SYSICONINDEX,
                SHIL_EXTRALARGE,
            },
            WindowsAndMessaging::{DestroyIcon, HICON},
        },
    },
};

const GRID_WIDTH: usize = 6;

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
                window: window::Settings::default(),
                flags: LauncherFlags {
                    file_move_error: Some(e),
                    folder: Some(folder),
                },
                ..Default::default()
            }),
        }
    } else {
        Launcher::run(Settings {
            window: window::Settings::default(),
            flags: LauncherFlags {
                folder,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

struct Launcher {
    folder_state: Vec<io::Result<(PathBuf, image::Handle)>>,
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
    NewEntry(PathBuf),
    EntryModified,
    RemoveEntry(PathBuf),
    OpenFolder,
    FileDropped(PathBuf),
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
                folder_state: state,
                flags,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        format!(
            "kslauncher - {}",
            self.flags
                .folder
                .clone()
                .unwrap_or_default()
                .file_name()
                .unwrap()
                .to_string_lossy()
        )
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
                    Win32::UI::Shell::ShellExecuteExW(&mut shell_info).unwrap();
                };
                return Command::single(iced_runtime::command::Action::Window(
                    iced_runtime::window::Action::Close,
                ));
            }
            Message::OpenFolder => {
                if let Some(folder) = &self.flags.folder {
                    process::Command::new("explorer.exe")
                        .arg(folder.display().to_string())
                        .spawn()
                        .unwrap();
                }
            }
            Message::NewEntry(file_path) => {
                let icon = get_icon(&file_path);
                self.folder_state.push(Ok((file_path, icon)));
            }
            Message::RemoveEntry(file_path) => self.folder_state.retain(|e| match e {
                Ok((path, _handle)) => path != &file_path,
                Err(_) => true,
            }),
            Message::EntryModified => {}
            Message::FileDropped(path) => {
                if let Some((folder, file_name)) = self.flags.folder.as_ref().zip(path.file_name())
                {
                    let _ = fs::rename(&path, folder.join(file_name));
                }
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<Message> {
        if self.folder_state.is_empty() {
            return Text::new("This folder is empty.").into();
        }
        let content: Element<Message> = match &self.flags.file_move_error {
            Some(e) => Text::new(format!("Failed to add file to launcher folder: {e}")).into(),
            None => iced::widget::Column::with_children(
                self.folder_state
                    .chunks(GRID_WIDTH)
                    .map(|row| {
                        let empty = (0..(GRID_WIDTH - row.len()))
                            .map(|_| Space::new(Length::FillPortion(1), Length::Shrink).into());
                        iced::widget::Row::with_children(
                            row.iter()
                                .map(|entry| match entry {
                                    Ok((file_path, image_handle)) => {
                                        let file_name = file_path
                                            .file_stem()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string();
                                        Container::new(
                                            Button::new(
                                                iced::widget::column!(
                                                    Image::<image::Handle>::new(
                                                        image_handle.clone()
                                                    )
                                                    .content_fit(iced::ContentFit::Contain)
                                                    .height(Length::Fixed(48.0))
                                                    .width(Length::Fill),
                                                    Text::new(file_name.clone())
                                                        .vertical_alignment(
                                                            iced::alignment::Vertical::Center
                                                        )
                                                        .horizontal_alignment(
                                                            iced::alignment::Horizontal::Center
                                                        )
                                                        .height(Length::FillPortion(1))
                                                        .width(Length::Fill)
                                                )
                                                .align_items(iced::Alignment::Center),
                                            )
                                            .on_press(Message::Open(file_path.clone()))
                                            .width(Length::Fill)
                                            .height(Length::Fill),
                                        )
                                        .width(Length::FillPortion(1))
                                        .height(Length::Fill)
                                        .align_x(Horizontal::Center)
                                        .align_y(Vertical::Center)
                                        .padding(2.0)
                                        .into()
                                    }
                                    Err(e) => Text::new(format!("Failed to read file: {e}")).into(),
                                })
                                .chain(empty)
                                .collect::<Vec<_>>(),
                        )
                        .height(Length::FillPortion(1))
                        .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .into(),
        };
        let open_folder = Button::new(
            Text::new("Open Folder in Explorer")
                .horizontal_alignment(iced::alignment::Horizontal::Center)
                .width(Length::Fill),
        )
        .on_press(Message::OpenFolder)
        .width(Length::Fill);
        iced::widget::column!(open_folder, content).into()
    }

    fn theme(&self) -> Theme {
        Theme::Custom(Box::new(theme::Custom::new(Palette {
            primary: Color::from_rgb8(0x38, 0x38, 0x43),
            ..Palette::DARK
        })))
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        struct RecipeDragNDrop;
        impl Recipe for RecipeDragNDrop {
            type Output = PathBuf;

            fn hash(&self, state: &mut iced_runtime::core::Hasher) {
                state.write(b"DragNDrop");
            }

            fn stream(
                self: Box<Self>,
                input: iced_runtime::futures::subscription::EventStream,
            ) -> iced_runtime::futures::BoxStream<Self::Output> {
                Box::pin(input.filter_map(|(e, _status)| async move {
                    if let iced::Event::Window(window::Event::FileDropped(path)) = e {
                        Some(path)
                    } else {
                        None
                    }
                }))
            }
        }
        let folder = self.flags.folder.clone();
        Subscription::batch([
            Subscription::from_recipe(RecipeDragNDrop).map(Message::FileDropped),
            subscription::channel(0, 16, move |sender| background(sender, folder)),
        ])
    }
}

fn get_icon(file_path: &Path) -> image::Handle {
    unsafe {
        Win32::System::Com::CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .unwrap();
        let mut psfi = SHFILEINFOW::default();
        let file_path_wide = file_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>();
        let system_image_list = SHGetFileInfoW(
            PCWSTR(file_path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut psfi),
            mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        );
        let extra_large_image_list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).unwrap();
        if system_image_list != 0 {
            let icon = extra_large_image_list
                .GetIcon(psfi.iIcon, ILD_TRANSPARENT.0)
                .unwrap();
            let image = icon_to_rgba_image(icon);
            DestroyIcon(icon).unwrap();
            image
        } else {
            unimplemented!()
        }
    }
}

unsafe fn icon_to_rgba_image(icon: HICON) -> image::Handle {
    use std::{mem::MaybeUninit, ptr::addr_of_mut};
    use windows::Win32::{
        Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS, HDC,
        },
        UI::WindowsAndMessaging::GetIconInfo,
    };

    let bitmap_size = i32::try_from(mem::size_of::<BITMAP>()).unwrap();
    let biheader_size = u32::try_from(mem::size_of::<BITMAPINFOHEADER>()).unwrap();

    let mut info = MaybeUninit::uninit();
    GetIconInfo(icon, info.as_mut_ptr()).unwrap();
    let info = info.assume_init_ref();
    DeleteObject(info.hbmMask).unwrap();

    let mut bitmap: MaybeUninit<BITMAP> = MaybeUninit::uninit();
    let result = GetObjectW(info.hbmColor, bitmap_size, Some(bitmap.as_mut_ptr().cast()));
    assert!(result == bitmap_size);
    let bitmap = bitmap.assume_init_ref();

    let width = u32::try_from(bitmap.bmWidth).unwrap();
    let height = u32::try_from(bitmap.bmHeight).unwrap();
    let w = usize::try_from(bitmap.bmWidth).unwrap();
    let h = usize::try_from(bitmap.bmHeight).unwrap();

    let buf_size = w
        .checked_mul(h)
        .and_then(|size| size.checked_mul(4))
        .unwrap();
    let mut buf: Vec<u8> = Vec::with_capacity(buf_size);

    let dc = GetDC(HWND(0));
    assert!(dc != HDC(0));

    let mut bitmap_info = BITMAPINFOHEADER {
        biSize: biheader_size,
        biWidth: bitmap.bmWidth,
        biHeight: -bitmap.bmHeight,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let result = GetDIBits(
        dc,
        info.hbmColor,
        0,
        height,
        Some(buf.as_mut_ptr().cast()),
        addr_of_mut!(bitmap_info).cast(),
        DIB_RGB_COLORS,
    );
    assert_ne!(result, 0);
    buf.set_len(buf.capacity());

    let result = ReleaseDC(HWND(0), dc);
    assert!(result == 1);
    DeleteObject(info.hbmColor).unwrap();

    for chunk in buf.chunks_exact_mut(4) {
        let [b, _, r, _] = chunk else { unreachable!() };
        mem::swap(b, r);
    }

    image::Handle::from_pixels(width, height, buf)
}

fn init_state(flags: &LauncherFlags) -> Vec<Result<(PathBuf, image::Handle), io::Error>> {
    match &flags.folder {
        Some(folder) => {
            let _ = fs::create_dir_all(folder);
            match fs::read_dir(folder) {
                Ok(read_dir) => read_dir
                    .map(|r| {
                        r.map(|e| {
                            let path = e.path();
                            let icon = get_icon(&path);
                            (path, icon)
                        })
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    vec![Err(e)]
                }
            }
        }
        None => {
            vec![]
        }
    }
}

async fn background(sender: Sender<Message>, folder_to_monitor: Option<PathBuf>) -> Infallible {
    use notify::{event::EventKind, RecursiveMode, Watcher};

    struct FolderEventHandler {
        sender: Sender<Message>,
    }
    impl notify::EventHandler for FolderEventHandler {
        fn handle_event(&mut self, event: notify::Result<notify::Event>) {
            if let Ok(event) = event {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                        let mut sender = self.sender.clone();
                        smol::spawn(async move {
                            let mut s = stream::iter(
                                event.paths.into_iter().map(Message::NewEntry).map(Ok),
                            );
                            sender.send_all(&mut s).await.unwrap();
                        })
                        .detach();
                    }
                    EventKind::Remove(_)
                    | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                        let mut sender = self.sender.clone();
                        smol::spawn(async move {
                            let mut s = stream::iter(
                                event.paths.into_iter().map(Message::RemoveEntry).map(Ok),
                            );
                            sender.send_all(&mut s).await.unwrap();
                        })
                        .detach();
                    }
                    EventKind::Modify(_) => {
                        let mut sender = self.sender.clone();
                        smol::spawn(async move {
                            sender.send(Message::EntryModified).await.unwrap();
                        })
                        .detach();
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(folder) = folder_to_monitor {
        let event_handler = FolderEventHandler {
            sender: sender.clone(),
        };
        let mut watcher = notify::recommended_watcher(event_handler).unwrap();
        watcher.watch(&folder, RecursiveMode::Recursive).unwrap();
        future::pending().await
    }
    future::pending().await
}
