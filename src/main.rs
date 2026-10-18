#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use encoding_rs::{
    BIG5, EUC_KR, Encoding, GB18030, SHIFT_JIS, UTF_8, WINDOWS_1251,
    WINDOWS_1252,
};
use fltk::{
    app::{self, App, MouseWheel, Receiver, Scheme, Sender, version_str},
    browser::MultiBrowser,
    button::Button,
    dialog::{
        alert_default, choice2_default, dir_chooser, file_chooser,
        input_default, message_default,
    },
    draw::{draw_box, pop_clip, push_clip},
    enums::{
        Color, ColorDepth, Cursor as MouseCursor, Event, FrameType, Shortcut,
    },
    frame::Frame,
    group::Flex,
    image::{PngImage, RgbImage, SharedImage},
    input::FileInput,
    menu::{MenuBar, MenuFlag},
    misc::InputChoice,
    prelude::*,
    valuator::HorNiceSlider,
    widget::Widget,
    window::Window,
};
use fltk_theme::{ColorTheme, color_themes::BLACK_THEME};
use fontdue::{Font, FontSettings};
use indexmap::IndexMap;
use phf::{Map, phf_map};
use rayon::prelude::*;
use rpgmad_lib::{
    Engine, EntrySource, VX_RGSS2A_EXT, VXACE_RGSS3A_EXT, XP_RGSSAD_EXT,
    decrypt, encrypt_to_writer,
};
use rpgmasd::{
    DECRYPTED_ASSET_EXTS, Decrypter as AssetDecrypter, ENCRYPTED_ASSET_EXTS,
    FileType, HEADER_LENGTH, M4A_EXT, MV_M4A_EXT, MV_OGG_EXT, MV_PNG_EXT,
    MZ_M4A_EXT, MZ_OGG_EXT, MZ_PNG_EXT, OGG_EXT, PNG_EXT, decrypt_in_place,
    encrypt,
};
use std::{
    borrow::Cow,
    ffi::OsStr,
    fs::{File, create_dir_all, metadata, read, remove_file},
    io::{self, BufWriter, Cursor, Read, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
    ptr::from_mut as ptr_from_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    codecs::audio::{AudioDecoder, AudioDecoderOptions},
    formats::{
        FormatOptions, FormatReader, SeekMode, SeekTo, TrackType, probe::Hint,
    },
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    units::{Time, TimeBase},
};
use sys_locale::get_locale;
use time::OffsetDateTime;
use walkdir::WalkDir;

#[cfg(feature = "gxhash")]
use gxhash::{GxBuildHasher as FastBuildHasher, HashSet};
#[cfg(not(feature = "gxhash"))]
use rapidhash::{
    RapidHashSet as HashSet, fast::RandomState as FastBuildHasher,
};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type IndexMapGx = IndexMap<PathBuf, Option<FileType>, FastBuildHasher>;

const WINDOW_WIDTH: i32 = 1024;
const WINDOW_HEIGHT: i32 = 720;

/// Top-level directories of a game that an `.rgss` archive holds.
const ARCHIVE_DIRS: [&str; 4] = ["Audio", "Graphics", "Data", "Fonts"];

/// Asset extensions besides RPG Maker's encrypted or plain PNG/OGG/M4A: fonts and game data.
const EXTRA_ASSET_EXTS: &[&str] =
    &["ttf", "otf", "rxdata", "rvdata", "rvdata2"];

const MV_ENGINE_LABEL: &str = "MV";
const MZ_ENGINE_LABEL: &str = "MZ";
const XP_ENGINE_LABEL: &str = "XP";
const VX_ENGINE_LABEL: &str = "VX";
const VXACE_ENGINE_LABEL: &str = "VX Ace";

const LOGO_32PX: &[u8] = include_bytes!("../icons/32px.rgba");
const LOGO_256PX: &[u8] = include_bytes!("../icons/256px.rgba");

struct AudioPlayer {
    /// Tags this track's stream messages, so messages from an earlier track are ignored.
    id: u64,
    /// Dropping it stops the stream, and with it the audio callback.
    _stream: cpal::Stream,
    paused: Arc<AtomicBool>,
    seek_pos: Arc<AtomicU64>,
    duration: Time,
    duration_string: String,
}

/// An opened audio file, ready to be decoded.
struct AudioTrack {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: TimeBase,
    duration: Time,
    sample_rate: u32,
    channels: u16,
}

const WELCOME: &str = "Drop files/folders into the window or use `Open \
                        File`/`Open Folder` in the `File` menu.";

enum DisplayMode {
    Image(SharedImage),
    Audio(usize),
    Message(String),
}

/// Encodings the `Encoding` menu offers for file names in `.rgss` archives.
const ARCHIVE_ENCODINGS: &[(&str, &Encoding)] = &[
    ("&Encoding/UTF-8\t", UTF_8),
    ("&Encoding/Shift-JIS\t", SHIFT_JIS),
    ("&Encoding/GB18030\t", GB18030),
    ("&Encoding/Big5\t", BIG5),
    ("&Encoding/EUC-KR\t", EUC_KR),
    ("&Encoding/Windows-1251\t", WINDOWS_1251),
    ("&Encoding/Windows-1252\t", WINDOWS_1252),
];

/// A file of the opened archive. Content is in [`Application::archive_data`], already decrypted.
struct ArchiveFile {
    /// Decoded, with `/` separators.
    path: String,
    data: Range<usize>,
}

/// A file to pack into an archive. Content is read only while writing, so the archive never sits in memory.
struct ArchiveSource {
    /// The path as the archive stores it, already in the archive's encoding.
    path: Vec<u8>,
    file: PathBuf,
    size: usize,
}

impl EntrySource for ArchiveSource {
    fn path(&self) -> &[u8] {
        &self.path
    }

    fn size(&self) -> usize {
        self.size
    }

    fn open(&self) -> io::Result<impl Read> {
        File::open(&self.file)
    }
}

/// The part of `path` from its first component in `anchors` (the last one with `last`).
fn relative_path_from(
    path: &Path,
    anchors: &[&str],
    last: bool,
) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    let is_anchor = |component: &Component| {
        anchors
            .iter()
            .any(|anchor| component.as_os_str() == *anchor)
    };

    let start = if last {
        components.iter().rposition(is_anchor)
    } else {
        components.iter().position(is_anchor)
    }?;

    Some(components[start..].iter().collect())
}

/// Whether `path` stays inside whatever directory it is joined to.
fn is_contained(path: &str) -> bool {
    // `:` would name an NTFS alternate data stream, or a drive.
    !path.is_empty()
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// Whether one of the components of `path` is `dir`.
fn has_dir(path: &Path, dir: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == dir)
}

/// The path of an asset inside its `www` directory, or just its file name when it isn't in one.
fn asset_relative_path(path: &Path) -> PathBuf {
    relative_path_from(path, &["www"], true)
        .or_else(|| path.file_name().map(PathBuf::from))
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    None,
    EncryptAsset,
    DecryptAsset,
    EncryptArchive,
    DecryptArchive,
}

impl State {
    fn is_decrypt_archive(self) -> bool {
        matches!(self, Self::DecryptArchive)
    }

    fn is_decrypt(self) -> bool {
        matches!(self, Self::DecryptArchive | Self::DecryptAsset)
    }
}

static USE_RUSSIAN: AtomicBool = AtomicBool::new(false);

static RUSSIAN: Map<&'static str, &'static str> = phf_map! {
    "Select Output Directory" => "Выбрать выходную директорию",
    "-Select engine for encryption-" => "-Выберите движок для шифровки-",
    "Select All" => "Выбрать всё",
    "Deselect All" => "Снять выделение",
    "Clear" => "Очистить",
    "Remove" => "Удалить",
    "Decrypt" => "Дешифровать",
    "Encrypt" => "Зашифровать",
    "Drop files/folders into the window or use `Open File`/`Open Folder` in the `File` menu." => "Добавьте файлы/папки перетаскиванием, либо используйте пункты «Открыть файл» / «Открыть папку» в меню «Файл».",
    "Play" => "Воспроизвести",
    "Pause" => "Пауза",
    "Stop" => "Стоп",
    "&File/Open File\t" => "&Файл/Открыть файл\t",
    "&File/Open Folder\t" => "&Файл/Открыть папку\t",
    "&File/Exit\t" => "&Файл/Выход\t",
    "&Help/Help\t" => "&Помощь/Помощь\t",
    "&Help/About\t" => "&Помощь/О программе\t",
    "About RPGMDec" => "О RPGMDec",
    "OK" => "ОК",
    "Decryption ended. Decrypted entries are put to the {}" => "Дешифрование завершено. Расшифрованные элементы помещены в `{}`",
    "Output directory is not a directory." => "Папка вывода не является директорией.",
    "Output directory does not exist." => "Папка вывода не существует.",
    "Output directory not specified." => "Папка вывода не указана.",
    "Output engine not specified." => "Модуль вывода не указан.",
    "Asset encryption requires a key. Set it manually or from file?" => "Для шифрования ресурсов требуется ключ. Указать его вручную или загрузить из файла?",
    "Manually" => "Вручную",
    "From File" => "Из файла",
    "Enter the encryption key" => "Введите ключ шифрования",
    "No encryption key entered." => "Ключ шифрования не введён.",
    "Select Encrypted File" => "Выберите зашифрованный файл",
    "Passed path is not a file." => "Указанный путь не является файлом.",
    "Encryption ended. Encrypted entries are put to the {}" => "Шифрование завершено. Зашифрованные элементы помещены в `{}`",
    "Unable to parse PNG data from the file: {}" => "Не удалось разобрать данные PNG из файла: {}",
    "{}: Binary file contents can't be displayed." => "{}: Содержимое двоичного файла не может быть отображено.",
    "{}: Unsupported file extension. If it's something that can be displayed, open an issue in our GitHub repository." => "{}: Неподдерживаемое расширение файла. Если это что-то, что должно отображаться, создайте обращение в нашем репозитории GitHub.",
    "&Language/Русский\t" => "&Язык/Русский\t",
    "&Language/English\t" => "&Язык/English\t",
    "RPGMDec {}\nLicensed under WTFPL\nRepository: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}" => "RPGMDec {}\nЛицензировано под WTFPL\nРепозиторий: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}",
    "Reading file {} failed: {}" => "Чтение файла {} не удалось: {}",
    "Decryption of file {} failed: {}" => "Расшифровка файла {} не удалась: {}",
    "Unable to found playable track" => "Не удалось найти проигрываемую дорожку",
    "Decoding the format failed: {}" => "Расшифровка формата не удалась: {}",
    "Select File" => "Выберите файл",
    "Select Folder" => "Выберите папку",
    "Encrypted Assets/Archives (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,rgss2a,rgss3a})\t" => "Зашифрованные ассеты/архивы (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,rgss2a,rgss3a})\t",
    "Creating directory {} failed: {}" => "Создание директории {} не удалось: {}",
    "Writing file {} failed: {}" => "Запись файла {} не удалась: {}",
    "Aborting decryption: {}" => "Прерываем расшифровку: {}",
    "Parsing key from string failed: {}" => "Парсинг ключа из текста не удался: {}",
    "Several files would be written to {}." => "Несколько файлов были бы записаны в {}.",
    "Aborting encryption: {}" => "Прерываем зашифровку: {}",
    "Decrypting file {} failed: {}" => "Расшифровка файла {} не удалась: {}",
    "No eligible files were found." => "Подходящие файлы не найдены.",
    "Wasn't able to determine processing mode by the first passed file." => "Не удалось определить режим обработки по первому полученному файлу",
    "Font parsing failed: {}" => "Парсинг шрифта не удался: {}",
    "Probing format failed: {}" => "Проверка формата не удалась: {}",
    "Getting default output device failed." => "Не удалось получить стандартное устройство вывода.",
    "Creating audio stream failed: {}" => "Создание аудиопотока не удалось: {}",
    "Playing audio stream failed: {}" => "Проигрываниее аудиопотока не удалось: {}",
    "Encryption of file {} failed: {}" => "Зашифровка файла {} не удалась: {}",
    "No files selected." => "Файлы не выбраны.",
    "Archive entry {} points outside of the output directory." => "Элемент архива {} указывает за пределы выходной директории.",
    "{} is not valid Unicode." => "{} не является корректным Unicode.",
    "{} can't be represented in {}. Pick another encoding in the Encoding menu." => "{} невозможно представить в кодировке {}. Выберите другую кодировку в меню «Кодировка».",
    "{} entries were skipped, their names aren't valid {}. Pick another encoding in the Encoding menu." => "Пропущено элементов: {}, их имена недопустимы в кодировке {}. Выберите другую кодировку в меню «Кодировка».",
    "&Encoding/UTF-8\t" => "&Кодировка/UTF-8\t",
    "&Encoding/Shift-JIS\t" => "&Кодировка/Shift-JIS\t",
    "&Encoding/GB18030\t" => "&Кодировка/GB18030\t",
    "&Encoding/Big5\t" => "&Кодировка/Big5\t",
    "&Encoding/EUC-KR\t" => "&Кодировка/EUC-KR\t",
    "&Encoding/Windows-1251\t" => "&Кодировка/Windows-1251\t",
    "&Encoding/Windows-1252\t" => "&Кодировка/Windows-1252\t",
};

macro_rules! tr {
    ($k:expr) => {
        if USE_RUSSIAN.load(Ordering::Relaxed) {
            RUSSIAN.get(&$k).map(|k| *k).unwrap_or($k)
        } else {
            $k
        }
    };
}

/// Fills the first `{}` of `template`.
fn fmt1(template: &str, arg: &str) -> String {
    template.replacen("{}", arg, 1)
}

/// Fills the first two `{}` of `template`.
fn fmt2(template: &str, first: &str, second: &str) -> String {
    template.replacen("{}", first, 1).replacen("{}", second, 1)
}

/// `template` with the path and the error filled in.
fn path_error(template: &str, path: &Path, err: &impl ToString) -> String {
    fmt2(template, &path.to_string_lossy(), &err.to_string())
}

fn write_error(path: &Path, err: &io::Error) -> String {
    path_error(tr!("Writing file {} failed: {}"), path, err)
}

/// Output extension and type of an asset encrypted for `engine`. `None` for assets that stay as they are.
fn encrypted_asset_kind(
    path: &Path,
    engine: &str,
) -> Option<(&'static str, FileType)> {
    let input_ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    match (input_ext.as_str(), engine) {
        (PNG_EXT, MV_ENGINE_LABEL) => Some((MV_PNG_EXT, FileType::Png)),
        (PNG_EXT, MZ_ENGINE_LABEL) => Some((MZ_PNG_EXT, FileType::Png)),
        (OGG_EXT, MV_ENGINE_LABEL) => Some((MV_OGG_EXT, FileType::Ogg)),
        (OGG_EXT, MZ_ENGINE_LABEL) => Some((MZ_OGG_EXT, FileType::Ogg)),
        (M4A_EXT, MV_ENGINE_LABEL) => Some((MV_M4A_EXT, FileType::M4a)),
        (M4A_EXT, MZ_ENGINE_LABEL) => Some((MZ_M4A_EXT, FileType::M4a)),
        _ => None,
    }
}

/// Whether two of `outputs` are the same file, telling the user if so: parallel writes would clash.
fn has_duplicate_output(
    outputs: impl Iterator<Item = PathBuf>,
    aborting: &str,
) -> bool {
    let mut seen = HashSet::<PathBuf>::default();

    for output in outputs {
        if seen.contains(&output) {
            alert_default(&fmt1(
                aborting,
                &fmt1(
                    tr!("Several files would be written to {}."),
                    &output.to_string_lossy(),
                ),
            ));
            return true;
        }

        seen.insert(output);
    }

    false
}

/// Creates `path` for writing, with its parent directories. The error is ready to show.
fn create_with_parents(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent()
        && let Err(err) = create_dir_all(parent)
    {
        return Err(path_error(
            tr!("Creating directory {} failed: {}"),
            parent,
            &err,
        ));
    }

    File::create(path).map_err(|err| write_error(path, &err))
}

/// Writes `data` to `path`, with its parent directories. The error is ready to show.
fn write_with_parents(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut file = create_with_parents(path)?;
    file.write_all(data).map_err(|err| write_error(path, &err))
}

const MENU_BAR_ITEMS: &[&str] = &[
    "&File/Open File\t",
    "&File/Open Folder\t",
    "&File/Exit\t",
    "&Language/Русский\t",
    "&Language/English\t",
    "&Help/Help\t",
    "&Help/About\t",
];

struct Application {
    /// The opened archive, decrypted in place. [`Application::archive_files`] index into it.
    archive_data: Vec<u8>,
    /// One per line of the file list while an archive is open.
    archive_files: Vec<ArchiveFile>,
    /// Where the open archive came from, to read it again when the encoding changes.
    archive_path: Option<PathBuf>,
    /// Encoding of the file names inside archives, for opening as well as for creating them.
    archive_encoding: &'static Encoding,
    file_list_map: IndexMapGx,
    audio_player: Option<AudioPlayer>,
    /// The audio file last selected, with the list index it was opened for.
    opened_track: Option<(usize, AudioTrack)>,
    next_player_id: u64,
    image_offset_x: i32,
    image_offset_y: i32,
    last_mouse_pos: Option<(i32, i32)>,
    current_image: Option<SharedImage>,
    image_scale_factor: f32,
    progress_slider_locked: bool,
    state: State,

    /// Widgets whose label is the translation of the given key.
    translatable: Vec<(Widget, &'static str)>,

    app: App,
    window: Window,
    trackpos_sender: Sender<(u64, u64)>,
    trackpos_receiver: Receiver<(u64, u64)>,
    trackend_sender: Sender<u64>,
    trackend_receiver: Receiver<u64>,

    menu_bar: MenuBar,

    select_output_dir_button: Button,
    output_dir_input: FileInput,
    output_engine_select: InputChoice,

    file_list: MultiBrowser,

    select_all_button: Button,
    deselect_all_button: Button,

    button_layout: Flex,
    clear_button: Button,
    remove_button: Button,
    process_button: Button,

    right_layout: Flex,

    image_frame: Frame,
    audio_controls: Flex,
    play_button: Button,
    pause_button: Button,
    stop_button: Button,
    progress_label: Frame,
    progress_slider: HorNiceSlider,
    about_window: Option<Window>,
}

impl Application {
    /// A `&mut Self` that outlives the borrow, for the `'static` callbacks FLTK takes.
    ///
    /// Sound because `Application` stays put in [`Application::run`] until the event loop returns, and FLTK calls
    /// callbacks on this thread, one at a time. A modal dialog's nested loop can redraw the window, so
    /// `image_frame_draw` may run while a method holds `&self`: don't read a field after showing a dialog.
    fn alias(&mut self) -> &'static mut Self {
        unsafe { &mut *ptr_from_mut::<Self>(self) }
    }

    fn process_label(&self) -> &'static str {
        if self.state.is_decrypt() {
            tr!("Decrypt")
        } else {
            tr!("Encrypt")
        }
    }

    fn retranslate(&mut self, language: Option<&str>) {
        let russian = language.map_or_else(
            || get_locale().is_some_and(|locale| locale.starts_with("ru")),
            |language| language == "ru",
        );
        USE_RUSSIAN.store(russian, Ordering::Relaxed);

        for (widget, key) in &mut self.translatable {
            widget.set_label(tr!(*key));
        }

        // The frame may show a message about a file instead: only the welcome text is translated.
        let label = self.image_frame.label();

        if label == WELCOME
            || RUSSIAN.get(WELCOME).is_some_and(|ru| label == *ru)
        {
            self.image_frame.set_label(tr!(WELCOME));
        }

        let engine_index = self.output_engine_select.menu_button().value();

        if engine_index == -1 {
            self.output_engine_select
                .set_value(tr!("-Select engine for encryption-"));
        }

        if self.state != State::None {
            self.process_button.set_label(self.process_label());
        }

        self.menu_bar.clear();
        self.add_menubar_entries();
        self.window.redraw();
    }

    fn run() {
        let file_list_map = IndexMapGx::default();
        let audio_player = None;
        let image_offset_x = 0;
        let image_offset_y = 0;
        let last_mouse_pos = None;

        let current_image = None;
        let image_scale_factor = 1.0;
        let progress_slider_locked = false;

        let app = App::default().with_scheme(Scheme::Gleam);
        ColorTheme::new(BLACK_THEME).apply();

        let (trackpos_sender, trackpos_receiver) = app::channel();
        let (trackend_sender, trackend_receiver) = app::channel();

        let mut window = Window::default()
            .with_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .with_label("RPGMDec");
        // SAFETY: `LOGO_32PX` is a `'static` include, so the data outlives the image. fltk checks its length.
        window.set_icon(unsafe {
            RgbImage::from_data(LOGO_32PX, 32, 32, ColorDepth::Rgba8).ok()
        });
        window.make_resizable(true);

        let mut window_layout = Flex::default_fill().column();

        let menu_bar = MenuBar::default().with_size(WINDOW_WIDTH, 30);

        window_layout.fixed(&menu_bar, 24);

        let main_layout = Flex::default().row();

        let mut left_layout = Flex::default()
            .column()
            .with_size(WINDOW_WIDTH / 2, WINDOW_HEIGHT);

        let input_layout = Flex::default().row();

        let select_output_dir_button = Button::default();
        let output_dir_input = FileInput::default();

        let mut output_engine_select = InputChoice::default();
        output_engine_select.input().set_readonly(true);
        output_engine_select.set_value("-Select engine for encryption-");

        left_layout.fixed(&input_layout, 48);
        input_layout.end();

        let mut file_list = MultiBrowser::default();
        file_list.set_selection_color(Color::Blue);
        file_list.set_scrollbar_size(16);

        let select_button_layout = Flex::default().row();
        let select_all_button = Button::default();
        let deselect_all_button = Button::default();
        select_button_layout.end();
        left_layout.fixed(&select_button_layout, 48);

        let button_layout = Flex::default().row();
        let clear_button = Button::default();
        let remove_button = Button::default();
        let mut process_button = Button::default();
        process_button.hide();

        button_layout.end();
        left_layout.fixed(&button_layout, 48);

        left_layout.end();

        let right_layout = Flex::default()
            .column()
            .with_size(WINDOW_WIDTH / 2, WINDOW_HEIGHT);

        let mut image_frame = Frame::default_fill().with_label(WELCOME);
        image_frame.set_frame(FrameType::DownBox);

        let mut audio_controls = Flex::default_fill().column();
        let play_button = Button::default();
        let pause_button = Button::default();
        let stop_button = Button::default();

        let progress_label = Frame::default().with_label("00:00 / 00:00");
        let progress_slider = HorNiceSlider::default();

        audio_controls.end();
        audio_controls.hide();

        right_layout.end();
        main_layout.end();
        window_layout.end();

        window.end();
        window.show();

        let translatable = vec![
            (
                select_output_dir_button.as_base_widget(),
                "Select Output Directory",
            ),
            (select_all_button.as_base_widget(), "Select All"),
            (deselect_all_button.as_base_widget(), "Deselect All"),
            (clear_button.as_base_widget(), "Clear"),
            (remove_button.as_base_widget(), "Remove"),
            (play_button.as_base_widget(), "Play"),
            (pause_button.as_base_widget(), "Pause"),
            (stop_button.as_base_widget(), "Stop"),
        ];

        let mut app = Application {
            translatable,
            archive_data: Vec::new(),
            archive_files: Vec::new(),
            archive_path: None,
            archive_encoding: UTF_8,
            state: State::None,
            file_list_map,
            audio_player,
            opened_track: None,
            next_player_id: 0,
            image_offset_x,
            image_offset_y,
            last_mouse_pos,
            current_image,
            image_scale_factor,
            progress_slider_locked,
            app,
            window,
            trackpos_sender,
            trackpos_receiver,
            trackend_sender,
            trackend_receiver,
            menu_bar,
            select_output_dir_button,
            output_dir_input,
            output_engine_select,
            file_list,
            select_all_button,
            deselect_all_button,
            button_layout,
            clear_button,
            remove_button,
            process_button,
            right_layout,
            image_frame,
            audio_controls,
            play_button,
            pause_button,
            stop_button,
            progress_label,
            progress_slider,
            about_window: None,
        };

        app.retranslate(None);
        app.set_callbacks();

        // `wait` also returns on audio thread messages (`Sender::send` wakes it). An idle callback would keep one core at 100%.
        while app.app.wait() {
            app.poll_audio();
        }
    }

    fn update_display(&mut self, mode: DisplayMode) {
        match mode {
            DisplayMode::Image(image) => {
                self.stop_playback();

                self.current_image = Some(image);
                self.opened_track = None;
                self.image_scale_factor = 1.0;
                self.image_offset_x = 0;
                self.image_offset_y = 0;

                self.image_frame.set_label("");
                self.image_frame.show();
                self.image_frame.redraw();

                self.audio_controls.hide();
                self.audio_controls.redraw();
            }

            DisplayMode::Audio(index) => {
                self.current_image = None;
                self.opened_track = None;

                self.image_frame.set_label("");
                self.image_frame.hide();
                self.image_frame.redraw();

                let track = match self.open_audio(index) {
                    Ok(track) => track,
                    Err(err) => {
                        alert_default(&err);
                        return;
                    }
                };

                self.progress_label.set_label(&format!(
                    "00:00 / {:02}:{:02}",
                    track.duration.as_secs() / 60,
                    track.duration.as_secs() % 60
                ));

                self.audio_controls.show();
                self.audio_controls.redraw();

                // Kept for Play, so that it doesn't read and probe the file again.
                self.opened_track = Some((index, track));
            }

            DisplayMode::Message(msg) => {
                self.stop_playback();

                self.current_image = None;
                self.opened_track = None;

                self.image_frame.set_label(&msg);
                self.image_frame.show();
                self.image_frame.redraw();

                self.audio_controls.hide();
                self.audio_controls.redraw();
            }
        }

        self.right_layout.layout();
    }

    fn clear(&mut self) {
        self.state = State::None;
        self.file_list.clear();
        self.file_list_map.clear();
        self.archive_files.clear();
        self.archive_data = Vec::new();
        self.archive_path = None;
        self.output_engine_select.clear();
        self.output_engine_select
            .set_value(tr!("-Select engine for encryption-"));
        self.process_button.hide();
        self.process_button.redraw();
        self.button_layout.layout();
        self.update_display(DisplayMode::Message(tr!(WELCOME).to_string()));
    }

    fn clear_button_cb(&mut self, _this: &mut Button) {
        self.clear();
    }

    fn stop_playback(&mut self) {
        self.audio_player = None;

        self.progress_label.set_label("00:00 / 00:00");
        self.progress_slider.set_range(0f64, 0f64);
        self.progress_slider.set_value(0f64);
    }

    fn remove_button_cb(&mut self, _this: &mut Button) {
        let selected = self.file_list.selected_items();

        if !selected.is_empty() {
            let mut removed = vec![false; self.file_list.size() as usize];

            for item in selected {
                removed[item as usize - 1] = true;
            }

            // One pass over the entries and one over the list, however many lines are removed.
            let mut index = 0;
            let mut keep = || {
                index += 1;
                !removed[index - 1]
            };

            if self.state.is_decrypt_archive() {
                self.archive_files.retain(|_| keep());
            } else {
                self.file_list_map.retain(|_, _| keep());
            }

            self.refill_file_list();
        }

        self.update_display(DisplayMode::Message(String::new()));
    }

    /// Shows the entries in the list, one line each.
    fn refill_file_list(&mut self) {
        self.file_list.clear();

        if self.state.is_decrypt_archive() {
            for file in &self.archive_files {
                self.file_list.add(&file.path);
            }
        } else {
            for path in self.file_list_map.keys() {
                if let Some(filename) = path.file_name() {
                    self.file_list.add(&filename.to_string_lossy());
                }
            }
        }
    }

    fn add_menubar_entries(&mut self) {
        let mut i = 0;

        let mut_self = self.alias();
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::Ctrl | 'o',
            MenuFlag::Normal,
            move |_| {
                let Some(file) = file_chooser(
                    tr!("Select File"),
                    tr!("Encrypted Assets/Archives \
                         (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,\
                         rgss2a,rgss3a})\t"),
                    dirs::home_dir().unwrap_or_default().join(""),
                    true,
                ) else {
                    return;
                };

                mut_self.parse_files(&file);
            },
        );
        i += 1;

        let mut_self = self.alias();
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::Ctrl | Shortcut::Shift | 'o',
            MenuFlag::Normal,
            move |_| {
                let Some(dir) = dir_chooser(
                    tr!("Select Folder"),
                    &dirs::home_dir()
                        .unwrap_or_default()
                        .join("")
                        .to_string_lossy(),
                    true,
                ) else {
                    return;
                };

                mut_self.parse_files(&dir);
            },
        );
        i += 1;

        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::Ctrl | 'q',
            MenuFlag::Normal,
            {
                let app = self.app;

                move |_| {
                    app.quit();
                }
            },
        );
        i += 1;

        let mut_self = self.alias();
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                mut_self.retranslate(Some("ru"));
            },
        );
        i += 1;

        let mut_self = self.alias();
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                mut_self.retranslate(Some("en"));
            },
        );
        i += 1;

        for &(label, encoding) in ARCHIVE_ENCODINGS {
            let mut_self = self.alias();
            let index = self.menu_bar.add(
                tr!(label),
                Shortcut::None,
                MenuFlag::Radio,
                move |_| {
                    mut_self.set_archive_encoding(encoding);
                },
            );

            if encoding == self.archive_encoding
                && let Some(mut item) = self.menu_bar.at(index)
            {
                item.set();
            }
        }

        self.menu_bar.add(tr!(MENU_BAR_ITEMS[i]), Shortcut::None, MenuFlag::Normal, |_| {
            let _ = if USE_RUSSIAN.load(Ordering::Relaxed) {
                webbrowser::open("https://github.com/rpg-maker-translation-tools/rpgmdec/tree/master/docs/help-ru.md")
            } else {
                webbrowser::open("https://github.com/rpg-maker-translation-tools/rpgmdec/tree/master/docs/help.md")
            };
        });
        i += 1;

        let mut_self = self.alias();
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                mut_self.show_about();
            },
        );
    }

    fn show_about(&mut self) {
        // Each open builds a new window: delete the previous one first.
        if let Some(old) = self.about_window.take() {
            Window::delete(old);
        }

        let mut about_window = Window::default()
            .with_label(tr!("About RPGMDec"))
            .with_size(600, 320);

        let mut layout = Flex::default_fill().column();

        let mut image_frame = Frame::default();
        // SAFETY: `LOGO_256PX` is a `'static` include, so the data outlives the image. fltk checks its length.
        image_frame.set_image(unsafe {
            RgbImage::from_data(LOGO_256PX, 256, 256, ColorDepth::Rgba8).ok()
        });

        let _frame = Frame::default().with_label(&fmt2(tr!("RPGMDec {}\nLicensed under WTFPL\nRepository: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}"), env!("CARGO_PKG_VERSION"), &version_str()));

        let mut ok_button = Button::default().with_label(tr!("OK"));

        layout.fixed(&ok_button, 32);
        layout.end();
        about_window.end();
        about_window.make_modal(true);
        about_window.show();

        self.about_window = Some(about_window.clone());
        ok_button.set_callback(move |_| {
            about_window.hide();
        });
    }

    fn set_archive_encoding(&mut self, encoding: &'static Encoding) {
        self.archive_encoding = encoding;

        // Names of an open archive were decoded with the old one.
        if self.state.is_decrypt_archive()
            && let Some(path) = self.archive_path.clone()
        {
            self.busy(|this| this.parse_archive(&path));
            self.update_display(DisplayMode::Message(String::new()));
        }
    }

    fn set_callbacks(&mut self) {
        self.add_menubar_entries();

        let mut_self = self.alias();
        self.clear_button
            .set_callback(|this| mut_self.clear_button_cb(this));

        let mut_self = self.alias();
        self.remove_button
            .set_callback(|this| mut_self.remove_button_cb(this));

        let mut_self = self.alias();
        self.process_button.set_callback(move |this| {
            let output_dir_text = mut_self.output_dir_input.value();
            let date = OffsetDateTime::now_utc();
            let output_dir = Path::new(&output_dir_text).join(format!(
                "{}-{}_{}-{:02}-{:02}_{:02}-{:02}-{:02}",
                if mut_self.state.is_decrypt_archive() {
                    mut_self
                        .archive_path
                        .as_deref()
                        .and_then(Path::extension)
                        .and_then(OsStr::to_str)
                        .unwrap_or_default()
                } else {
                    "www"
                },
                if mut_self.state.is_decrypt() {
                    "decrypted"
                } else {
                    "encrypted"
                },
                date.year(),
                date.month() as u8,
                date.day(),
                date.hour(),
                date.minute(),
                date.second()
            ));

            if mut_self.state.is_decrypt() {
                mut_self.decrypt_cb(this, &output_dir);
            } else {
                mut_self.encrypt_cb(this, &output_dir);
            }
        });

        let mut_self = self.alias();
        self.window
            .handle(move |_, event| mut_self.window_handle(event));

        let mut_self = self.alias();
        self.file_list
            .set_callback(move |this| mut_self.file_list_cb(this));

        let mut_self = self.alias();
        self.play_button
            .set_callback(move |this| mut_self.play_button_cb(this));

        let mut_self = self.alias();
        self.pause_button
            .set_callback(move |this| mut_self.pause_button_cb(this));

        let mut_self = self.alias();
        self.stop_button
            .set_callback(move |this| mut_self.stop_button_cb(this));

        let mut_self = self.alias();
        self.select_output_dir_button.set_callback(move |this| {
            mut_self.select_output_dir_button_cb(this);
        });

        let mut_self = self.alias();
        self.progress_slider.handle(move |this, event| {
            mut_self.progress_slider_handle(this, event)
        });

        let mut_self = self.alias();
        self.select_all_button
            .set_callback(move |this| mut_self.select_all_button_cb(this));

        let mut_self = self.alias();
        self.deselect_all_button
            .set_callback(move |this| mut_self.deselect_all_button_cb(this));

        let mut_self = self.alias();
        self.output_dir_input.handle(move |this, event| {
            mut_self.output_dir_input_handle(this, event)
        });

        let mut_self = self.alias();
        self.image_frame
            .draw(move |this| mut_self.image_frame_draw(this));

        let mut_self = self.alias();
        self.image_frame.handle(move |this, event| {
            mut_self.image_frame_handle(this, event)
        });
    }

    /// Returns whether everything selected was written.
    fn decrypt_archive(&self, decrypted_dir: &Path) -> bool {
        // Plain references: the widgets in `self` are not `Sync`, these are.
        let archive_data = &self.archive_data;
        let archive_files = &self.archive_files;
        let selected = self.file_list.selected_items();

        // Entries with one name would be written at the same time. Windows names ignore case.
        let outputs = selected.iter().map(|&item| {
            let path = &archive_files[item as usize - 1].path;

            if cfg!(windows) {
                PathBuf::from(path.to_lowercase())
            } else {
                PathBuf::from(path)
            }
        });

        if has_duplicate_output(outputs, tr!("Aborting decryption: {}")) {
            return false;
        }

        let result = selected.into_par_iter().try_for_each(|item| {
            let entry = &archive_files[item as usize - 1];

            // The archive is untrusted: an absolute or `..` path would write outside of `decrypted_dir`.
            if !is_contained(&entry.path) {
                return Err(fmt1(
                    tr!("Archive entry {} points outside of the output \
                         directory."),
                    &entry.path,
                ));
            }

            write_with_parents(
                &decrypted_dir.join(&entry.path),
                &archive_data[entry.data.clone()],
            )
        });

        if let Err(err) = &result {
            alert_default(&fmt1(tr!("Aborting decryption: {}"), err));
        }

        result.is_ok()
    }

    /// Returns whether everything selected was written.
    fn decrypt_assets(&self, decrypted_dir: &Path) -> bool {
        let file_list_map = &self.file_list_map;
        let selected = self.file_list.selected_items();

        let outputs = selected.iter().filter_map(|&item| {
            let (path, file_type) =
                file_list_map.get_index(item as usize - 1).unwrap();

            file_type.map(|file_type| {
                asset_relative_path(path).with_extension(file_type.to_string())
            })
        });

        if has_duplicate_output(outputs, tr!("Aborting decryption: {}")) {
            return false;
        }

        let result = selected.into_par_iter().try_for_each(|item| {
            let index = item as usize - 1;
            let (path, file_type) = file_list_map.get_index(index).unwrap();

            let Some(file_type) = file_type else {
                return Ok(());
            };

            let mut encrypted_data = read(path).map_err(|err| {
                path_error(tr!("Reading file {} failed: {}"), path, &err)
            })?;

            decrypt_in_place(&mut encrypted_data, *file_type).map_err(
                |err| {
                    path_error(
                        tr!("Decryption of file {} failed: {}"),
                        path,
                        &err,
                    )
                },
            )?;

            let relative_path = asset_relative_path(path);

            write_with_parents(
                &decrypted_dir
                    .join(relative_path.with_extension(file_type.to_string())),
                &encrypted_data[HEADER_LENGTH..],
            )
        });

        if let Err(err) = &result {
            alert_default(&fmt1(tr!("Aborting decryption: {}"), err));
        }

        result.is_ok()
    }

    fn decrypt_cb(&mut self, _: &mut Button, decrypted_dir: &Path) {
        if !self.output_dir_valid() || !self.has_selection() {
            return;
        }

        let succeeded = self.busy(|this| match this.state {
            State::DecryptArchive => this.decrypt_archive(decrypted_dir),
            State::DecryptAsset => this.decrypt_assets(decrypted_dir),
            _ => unreachable!(),
        });

        // The failure has been reported already.
        if succeeded {
            message_default(&fmt1(
                tr!("Decryption ended. Decrypted entries are put to the {}"),
                &decrypted_dir.display().to_string(),
            ));
        }
    }

    /// Shows the wait cursor while `work` blocks the event loop.
    fn busy<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> T {
        self.window.set_cursor(MouseCursor::Wait);
        app::flush();
        let result = work(self);
        self.window.set_cursor(MouseCursor::Default);
        result
    }

    fn output_dir_valid(&self) -> bool {
        let output_dir = self.output_dir_input.value();

        let error = if output_dir.is_empty() {
            tr!("Output directory not specified.")
        } else {
            match metadata(&output_dir) {
                Ok(info) if info.is_dir() => return true,
                Ok(_) => tr!("Output directory is not a directory."),
                Err(_) => tr!("Output directory does not exist."),
            }
        };

        alert_default(error);
        false
    }

    /// Returns whether there is something selected, telling the user if there isn't.
    fn has_selection(&self) -> bool {
        let has_selection = !self.file_list.selected_items().is_empty();

        if !has_selection {
            alert_default(tr!("No files selected."));
        }

        has_selection
    }

    /// Returns whether everything selected was written. Backing out of a dialog is not a success.
    fn encrypt_assets(&mut self, encrypted_dir: &Path, engine: &str) -> bool {
        let file_list_map = &self.file_list_map;
        let selected = self.file_list.selected_items();

        let outputs = selected.iter().filter_map(|&item| {
            let (path, file_type) =
                file_list_map.get_index(item as usize - 1).unwrap();

            if file_type.is_some() {
                return None;
            }

            let (output_ext, _) = encrypted_asset_kind(path, engine)?;
            Some(asset_relative_path(path).with_extension(output_ext))
        });

        if has_duplicate_output(outputs, tr!("Aborting encryption: {}")) {
            return false;
        }

        let choice = choice2_default(
            tr!("Asset encryption requires a key. Set it manually or from \
                 file?"),
            tr!("Manually"),
            tr!("From File"),
            "",
        );
        let Some(choice) = choice else { return false };

        let encryption_key = if choice == 0 {
            let Some(key) = input_default(tr!("Enter the encryption key"), "")
            else {
                alert_default(tr!("No encryption key entered."));
                return false;
            };

            if let Err(err) = AssetDecrypter::new().set_key_from_str(&key) {
                alert_default(&fmt1(
                    tr!("Parsing key from string failed: {}"),
                    &err.to_string(),
                ));
                return false;
            }

            key
        } else {
            let Some(file_path) = file_chooser(
                tr!("Select Encrypted File"),
                "*",
                dirs::home_dir().unwrap_or_default().join(""),
                true,
            ) else {
                return false;
            };

            let Some(extension) = Path::new(&file_path).extension() else {
                alert_default(tr!("Passed path is not a file."));
                return false;
            };

            let file_type = match FileType::try_from(extension) {
                Ok(file_type) => file_type,
                Err(err) => {
                    alert_default(&err.to_string());
                    return false;
                }
            };

            let file = match read(&file_path) {
                Ok(file) => file,
                Err(err) => {
                    alert_default(&fmt2(
                        tr!("Reading file {} failed: {}"),
                        &file_path,
                        &err.to_string(),
                    ));
                    return false;
                }
            };

            match AssetDecrypter::new().set_key_from_file(&file, file_type) {
                Ok(key) => key.to_string(),
                Err(err) => {
                    alert_default(&err.to_string());
                    return false;
                }
            }
        };

        let encryption_key = &encryption_key;

        self.window.set_cursor(MouseCursor::Wait);
        app::flush();

        let result = selected.into_par_iter().try_for_each(|item| {
            let index = item as usize - 1;

            let (path, file_type) = file_list_map.get_index(index).unwrap();

            if file_type.is_some() {
                return Ok(());
            }

            let Some((output_ext, file_type)) =
                encrypted_asset_kind(path, engine)
            else {
                return Ok(());
            };

            let decrypted_data = read(path).map_err(|err| {
                path_error(tr!("Reading file {} failed: {}"), path, &err)
            })?;

            let encrypted_data = encrypt(
                &decrypted_data,
                file_type,
                encryption_key,
            )
            .map_err(|err| {
                path_error(tr!("Encryption of file {} failed: {}"), path, &err)
            })?;

            let relative_path = asset_relative_path(path);

            write_with_parents(
                &encrypted_dir.join(relative_path).with_extension(output_ext),
                &encrypted_data,
            )
        });

        self.window.set_cursor(MouseCursor::Default);

        if let Err(err) = &result {
            alert_default(&fmt1(tr!("Aborting encryption: {}"), err));
        }

        result.is_ok()
    }

    /// Returns whether the archive was written.
    fn encrypt_archive(&mut self, encrypted_dir: &Path, engine: &str) -> bool {
        let file_list_map = &self.file_list_map;
        let encoding = self.archive_encoding;

        let sources: Result<Vec<ArchiveSource>, String> = self
            .file_list
            .selected_items()
            .into_par_iter()
            .map(|item| {
                let index = item as usize - 1;
                let (path, _) = file_list_map.get_index(index).unwrap();

                // Listed files all passed `should_include_file`, so a directory matches. Take the last one, so a game folder named like one (`Data\MyGame\...`) isn't picked.
                let relative_path =
                    relative_path_from(path, &ARCHIVE_DIRS, true).unwrap();

                // Archives hold the path from the top-level directory down, with backslashes.
                let Some(relative_path) = relative_path.to_str() else {
                    return Err(fmt1(
                        tr!("{} is not valid Unicode."),
                        &path.to_string_lossy(),
                    ));
                };

                let relative_path = relative_path.replace('/', "\\");
                let (archive_path, _, unmappable) =
                    encoding.encode(&relative_path);

                // Anything else would silently write `&#1234;` into the name.
                if unmappable {
                    return Err(fmt2(
                        tr!("{} can't be represented in {}. Pick another \
                             encoding in the Encoding menu."),
                        &path.to_string_lossy(),
                        encoding.name(),
                    ));
                }

                // Only the size is needed here: the content is read while the archive is written.
                let len = metadata(path)
                    .map_err(|err| {
                        path_error(
                            tr!("Reading file {} failed: {}"),
                            path,
                            &err,
                        )
                    })?
                    .len();

                Ok(ArchiveSource {
                    path: archive_path.into_owned(),
                    file: path.clone(),
                    size: len as usize,
                })
            })
            .collect();

        let sources = match sources {
            Ok(sources) => sources,
            Err(err) => {
                alert_default(&fmt1(tr!("Aborting encryption: {}"), &err));
                return false;
            }
        };

        let engine_kind = match engine {
            VXACE_ENGINE_LABEL => Engine::Vxace,
            _ => Engine::Older,
        };

        let output_path =
            encrypted_dir.join("Game").with_extension(match engine {
                XP_ENGINE_LABEL => XP_RGSSAD_EXT,
                VX_ENGINE_LABEL => VX_RGSS2A_EXT,
                VXACE_ENGINE_LABEL => VXACE_RGSS3A_EXT,
                _ => unreachable!(),
            });

        let mut writer = match create_with_parents(&output_path) {
            Ok(file) => BufWriter::new(file),
            Err(err) => {
                alert_default(&err);
                return false;
            }
        };

        let written = encrypt_to_writer(&sources, engine_kind, &mut writer)
            .map_err(|err| err.to_string())
            .and_then(|()| writer.flush().map_err(|err| err.to_string()));

        if let Err(err) = written {
            // A half-written archive must not be left behind looking complete.
            drop(writer);
            let _ = remove_file(&output_path);
            alert_default(&fmt1(tr!("Aborting encryption: {}"), &err));
            return false;
        }

        true
    }

    fn encrypt_cb(&mut self, _: &mut Button, encrypted_dir: &Path) {
        if !self.output_dir_valid() || !self.has_selection() {
            return;
        }

        if self.output_engine_select.menu_button().value() == -1 {
            alert_default(tr!("Output engine not specified."));
            return;
        }

        // The menu has an item selected, so there is a value.
        let engine = self.output_engine_select.value().unwrap();

        let succeeded = match self.state {
            State::EncryptAsset => self.encrypt_assets(encrypted_dir, &engine),
            State::EncryptArchive => {
                self.busy(|this| this.encrypt_archive(encrypted_dir, &engine))
            }
            _ => unreachable!(),
        };

        // The failure has been reported already.
        if succeeded {
            message_default(&fmt1(
                tr!("Encryption ended. Encrypted entries are put to the {}"),
                &encrypted_dir.display().to_string(),
            ));
        }
    }

    fn parse_archive(&mut self, path: &Path) {
        // A failure below must not leave the previous archive behind an empty list.
        self.file_list.clear();
        self.archive_files.clear();
        self.archive_data = Vec::new();
        self.archive_path = None;

        let mut archive_data = match read(path) {
            Ok(archive_data) => archive_data,
            Err(err) => {
                alert_default(&path_error(
                    tr!("Reading file {} failed: {}"),
                    path,
                    &err,
                ));
                return;
            }
        };

        // The entries borrow the buffer, which has to be stored along with them, so keep offsets into it instead.
        let base = archive_data.as_ptr() as usize;
        let mut archive_files = Vec::new();
        let mut skipped: usize = 0;

        let decrypted = decrypt(&mut archive_data);

        if let Err(err) = decrypted {
            alert_default(&path_error(
                tr!("Decrypting file {} failed: {}"),
                path,
                &err,
            ));
            return;
        }

        for entry in decrypted.unwrap() {
            // Never show a mangled name: it could match another entry's and silently overwrite it.
            let Some(name) = self
                .archive_encoding
                .decode_without_bom_handling_and_without_replacement(
                    entry.path,
                )
            else {
                skipped += 1;
                continue;
            };

            // Entries are slices of `archive_data`, so the offset is in bounds.
            let start = entry.data.as_ptr() as usize - base;

            archive_files.push(ArchiveFile {
                path: name.replace('\\', "/"),
                data: start..start + entry.data.len(),
            });
        }

        if skipped > 0 {
            alert_default(&fmt2(
                tr!("{} entries were skipped, their names aren't valid {}. \
                     Pick another encoding in the Encoding menu."),
                &skipped.to_string(),
                self.archive_encoding.name(),
            ));
        }

        self.archive_data = archive_data;
        self.archive_files = archive_files;
        self.archive_path = Some(path.to_path_buf());

        self.refill_file_list();
    }

    fn parse_asset(&mut self, file_path: &Path) -> Option<FileType> {
        let extension = file_path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)?;

        let ext = extension.as_str();

        let is_allowed = ENCRYPTED_ASSET_EXTS.contains(&ext)
            || DECRYPTED_ASSET_EXTS.contains(&ext)
            || EXTRA_ASSET_EXTS.contains(&ext);

        if !is_allowed {
            return None;
        }

        if self.file_list_map.contains_key(file_path) {
            return None;
        }

        let filename = file_path.file_name()?.to_string_lossy();
        let file_type = FileType::try_from(ext).ok();

        self.file_list.add(&filename);
        self.file_list_map
            .insert(file_path.to_path_buf(), file_type);

        file_type
    }

    fn detect_state_from_path(path: &Path) -> Option<State> {
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)?;

        let ext = extension.as_str();

        if matches!(ext, XP_RGSSAD_EXT | VX_RGSS2A_EXT | VXACE_RGSS3A_EXT) {
            return Some(State::DecryptArchive);
        }

        if ENCRYPTED_ASSET_EXTS.contains(&ext) {
            return Some(State::DecryptAsset);
        }

        if DECRYPTED_ASSET_EXTS.contains(&ext)
            || EXTRA_ASSET_EXTS.contains(&ext)
        {
            if has_dir(path, "www") {
                return Some(State::EncryptAsset);
            }

            if ARCHIVE_DIRS.iter().any(|dir| has_dir(path, dir)) {
                return Some(State::EncryptArchive);
            }
        }

        None
    }

    fn should_include_file(path: &Path, target_state: State) -> bool {
        let Some(extension) = path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)
        else {
            return false;
        };

        let ext = extension.as_str();

        match target_state {
            State::DecryptArchive => {
                matches!(ext, XP_RGSSAD_EXT | VX_RGSS2A_EXT | VXACE_RGSS3A_EXT)
            }
            State::DecryptAsset => ENCRYPTED_ASSET_EXTS.contains(&ext),
            // RPG Maker MV/MZ encrypts only images and audio. Fonts and the like stay as they are.
            State::EncryptAsset => {
                DECRYPTED_ASSET_EXTS.contains(&ext) && has_dir(path, "www")
            }
            State::EncryptArchive => {
                (DECRYPTED_ASSET_EXTS.contains(&ext)
                    || EXTRA_ASSET_EXTS.contains(&ext))
                    && ARCHIVE_DIRS.iter().any(|&dir| has_dir(path, dir))
            }
            State::None => false,
        }
    }

    fn parse_files(&mut self, paths: &str) {
        self.busy(|this| this.parse_files_blocking(paths));
    }

    fn parse_files_blocking(&mut self, paths: &str) {
        let mut files = Vec::new();

        for path_str in paths.lines() {
            let path = Path::new(path_str);

            if path.is_dir() {
                files.extend(
                    WalkDir::new(path)
                        .into_iter()
                        .flatten()
                        .filter(|entry| entry.file_type().is_file())
                        .map(walkdir::DirEntry::into_path),
                );
            } else {
                files.push(path.to_path_buf());
            }
        }

        // A folder has files that say nothing, e.g. `Game.exe` next to `Game.rgss3a`: the first one that does decides.
        // The current list stays untouched when none does.
        let Some(state) = files
            .iter()
            .find_map(|path| Self::detect_state_from_path(path))
        else {
            message_default(tr!("Wasn't able to determine processing mode \
                                 by the first passed file."));
            return;
        };

        self.clear();
        self.state = state;

        let mut eligible = files
            .iter()
            .filter(|path| Self::should_include_file(path, state));

        if state.is_decrypt_archive() {
            if let Some(archive) = eligible.next() {
                self.parse_archive(archive);
            }
        } else {
            for asset in eligible {
                self.parse_asset(asset);
            }
        }

        if self.file_list.size() == 0 {
            message_default(tr!("No eligible files were found."));
        }

        match self.state {
            State::EncryptAsset => {
                self.output_engine_select.clear();
                self.output_engine_select.add(MV_ENGINE_LABEL);
                self.output_engine_select.add(MZ_ENGINE_LABEL);
            }

            State::EncryptArchive => {
                self.output_engine_select.clear();
                self.output_engine_select.add(XP_ENGINE_LABEL);
                self.output_engine_select.add(VX_ENGINE_LABEL);
                self.output_engine_select.add(VXACE_ENGINE_LABEL);
            }

            State::DecryptArchive | State::DecryptAsset | State::None => {}
        }

        self.process_button.set_label(self.process_label());

        self.process_button.show();
        self.process_button.redraw();
        self.button_layout.layout();
    }

    fn window_handle(&mut self, event: Event) -> bool {
        match event {
            Event::DndEnter | Event::DndDrag | Event::DndRelease => true,
            Event::Paste => {
                let paths = app::event_text();
                self.parse_files(&paths);
                true
            }
            _ => false,
        }
    }

    /// Index of the selected line, which is also its index into the archive's or map's entries.
    fn selected_index(&self) -> Option<usize> {
        usize::try_from(self.file_list.value()).ok()?.checked_sub(1)
    }

    /// The content of the file at `index`, decrypted if it is encrypted. The error is ready to show.
    ///
    /// Each file has its own key: one kept from an earlier file would be wrong for the next game.
    fn file_data(&self, index: usize) -> Result<Cow<'_, [u8]>, String> {
        if self.state.is_decrypt_archive() {
            let file = &self.archive_files[index];
            return Ok(Cow::Borrowed(&self.archive_data[file.data.clone()]));
        }

        let (path, file_type) = self.file_list_map.get_index(index).unwrap();

        let mut data = read(path).map_err(|err| {
            path_error(tr!("Reading file {} failed: {}"), path, &err)
        })?;

        let Some(file_type) = file_type else {
            return Ok(Cow::Owned(data));
        };

        decrypt_in_place(&mut data, *file_type).map_err(|err| {
            path_error(tr!("Decryption of file {} failed: {}"), path, &err)
        })?;

        data.drain(..HEADER_LENGTH);
        Ok(Cow::Owned(data))
    }

    fn file_list_cb(&mut self, this: &mut MultiBrowser) {
        let (Some(filename), Some(index)) =
            (this.selected_text(), self.selected_index())
        else {
            return;
        };

        // An archive entry may have no extension.
        let extension = filename
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension)
            .to_ascii_lowercase();

        self.stop_playback();

        match extension.as_str() {
            MV_PNG_EXT | MZ_PNG_EXT | PNG_EXT => {
                let png_data = match self.file_data(index) {
                    Ok(png_data) => png_data,
                    Err(err) => {
                        alert_default(&err);
                        return;
                    }
                };

                let image = PngImage::from_data(&png_data)
                    .and_then(|image| SharedImage::from_image(&image));

                let image = match image {
                    Ok(image) => image,
                    Err(err) => {
                        alert_default(&fmt1(
                            tr!("Unable to parse PNG data from the file: {}"),
                            &err.to_string(),
                        ));
                        return;
                    }
                };

                self.update_display(DisplayMode::Image(image));
            }
            MV_OGG_EXT | MZ_OGG_EXT | MV_M4A_EXT | MZ_M4A_EXT | OGG_EXT
            | M4A_EXT => {
                self.update_display(DisplayMode::Audio(index));
            }
            "rxdata" | "rvdata" | "rvdata2" => {
                self.update_display(DisplayMode::Message(fmt1(
                    tr!("{}: Binary file contents can't be displayed."),
                    &extension,
                )));
            }
            "ttf" | "otf" => {
                let font_data = match self.file_data(index) {
                    Ok(font_data) => font_data,
                    Err(err) => {
                        alert_default(&err);
                        return;
                    }
                };

                let font = match Font::from_bytes(
                    &*font_data,
                    FontSettings::default(),
                ) {
                    Ok(font) => font,
                    Err(err) => {
                        alert_default(&fmt1(
                            tr!("Font parsing failed: {}"),
                            err,
                        ));
                        return;
                    }
                };

                let text = "The quick brown fox jumps over the lazy dog";
                let px = 24.0;

                let Some(line_metrics) = font.horizontal_line_metrics(px)
                else {
                    alert_default(&fmt1(
                        tr!("Font parsing failed: {}"),
                        "no horizontal line metrics",
                    ));
                    return;
                };

                let glyphs: Vec<_> =
                    text.chars().map(|ch| font.rasterize(ch, px)).collect();
                let total_width: usize = glyphs
                    .iter()
                    .map(|(metrics, _)| metrics.advance_width as usize)
                    .sum();

                let ascent = line_metrics.ascent.ceil() as isize;
                let descent = line_metrics.descent.floor() as isize;
                let total_height = (ascent - descent) as usize;

                let mut rgb_buf = vec![0u8; total_width * total_height * 3];
                let mut x_offset = 0;

                for (metrics, bitmap) in &glyphs {
                    for y in 0..metrics.height {
                        for x in 0..metrics.width {
                            let g = bitmap[y * metrics.width + x];
                            let dx = x_offset + x;
                            // `ymin` is the glyph's bottom relative to the baseline, negative for descenders.
                            let dy = (ascent
                                - metrics.height as isize
                                - metrics.ymin as isize
                                + y as isize)
                                as usize;

                            if dy < total_height && dx < total_width {
                                let i = (dy * total_width + dx) * 3;
                                rgb_buf[i] = g;
                                rgb_buf[i + 1] = g;
                                rgb_buf[i + 2] = g;
                            }
                        }
                    }
                    x_offset += metrics.advance_width as usize;
                }

                // Fails for a font with nothing to draw, e.g. one lacking the text's glyphs.
                let image = RgbImage::new(
                    &rgb_buf,
                    total_width as i32,
                    total_height as i32,
                    ColorDepth::Rgb8,
                )
                .and_then(|image| SharedImage::from_image(&image));

                match image {
                    Ok(image) => {
                        self.update_display(DisplayMode::Image(image));
                    }
                    Err(err) => {
                        alert_default(&fmt1(
                            tr!("Font parsing failed: {}"),
                            &err.to_string(),
                        ));
                    }
                }
            }
            _ => {
                self.update_display(DisplayMode::Message(fmt1(
                    tr!("{}: Unsupported file extension. If it's something \
                         that can be displayed, open an issue in our GitHub \
                         repository."),
                    &extension,
                )));
            }
        }
    }

    /// Decodes nothing yet: opens the container of `audio_data` and finds its track.
    fn open_track(
        audio_data: Vec<u8>,
    ) -> Result<Box<dyn FormatReader + 'static>, String> {
        let mut hint = Hint::new();

        if audio_data.starts_with(b"OggS") {
            hint.with_extension("ogg");
        } else {
            hint.with_extension("mp4");
        }

        let src = Box::new(Cursor::new(audio_data));
        let mss =
            MediaSourceStream::new(src, MediaSourceStreamOptions::default());

        symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|err| {
                fmt1(tr!("Probing format failed: {}"), &err.to_string())
            })
    }

    /// Opens the audio file at `index` of the list, ready to be decoded. The error is ready to show.
    fn open_audio(&self, index: usize) -> Result<AudioTrack, String> {
        let audio_data = self.file_data(index)?.into_owned();
        let format = Self::open_track(audio_data)?;

        let unplayable = || tr!("Unable to found playable track").to_string();

        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(unplayable)?;
        let track_id = track.id;
        let parameters = track
            .codec_params
            .as_ref()
            .and_then(|parameters| parameters.audio())
            .ok_or_else(unplayable)?;

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(parameters, &AudioDecoderOptions::default())
            .map_err(|err| {
                fmt1(tr!("Decoding the format failed: {}"), &err.to_string())
            })?;

        // Not every stream declares its timing or format.
        let (Some(time_base), Some(sample_rate), Some(channels)) = (
            track.time_base,
            parameters.sample_rate,
            parameters.channels.as_ref(),
        ) else {
            return Err(unplayable());
        };
        let Some(duration) = track
            .duration
            .and_then(|duration| time_base.calc_duration(duration))
        else {
            return Err(unplayable());
        };
        let channels = channels.count() as u16;

        Ok(AudioTrack {
            duration,
            channels,
            format,
            decoder,
            track_id,
            time_base,
            sample_rate,
        })
    }

    fn play_button_cb(&mut self, _: &mut Button) {
        // Playing or paused already: Play only resumes.
        if let Some(player) = &self.audio_player {
            player.paused.store(false, Ordering::Release);
            return;
        }

        let Some(index) = self.selected_index() else {
            return;
        };

        let AudioTrack {
            mut format,
            mut decoder,
            track_id,
            time_base,
            duration,
            sample_rate,
            channels: track_channels,
        } = match self.opened_track.take() {
            Some((opened_index, track)) if opened_index == index => track,
            _ => match self.open_audio(index) {
                Ok(track) => track,
                Err(err) => {
                    alert_default(&err);
                    return;
                }
            },
        };

        let Some(output_device) = cpal::default_host().default_output_device()
        else {
            alert_default(tr!("Getting default output device failed."));
            return;
        };

        // The samples keep the track's own rate and channels. WASAPI converts them to the device's format.
        let config = cpal::StreamConfig {
            channels: track_channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let channels = track_channels as usize;

        let paused = Arc::new(AtomicBool::new(false));
        let seek_pos = Arc::new(AtomicU64::new(u64::MAX));
        self.next_player_id += 1;
        let player_id = self.next_player_id;
        let mut last_playback_second = 0u64;

        let mut pcm_buffer: Vec<f32> = Vec::new();
        let mut pcm_offset = 0;

        // The callback runs on the audio thread: it mustn't block or print. The Ogg reader still allocates one buffer per packet, which is accepted.
        let stream = match output_device.build_output_stream(
            config,
            {
                let paused = paused.clone();
                let seek_pos = seek_pos.clone();
                let trackpos_sender = self.trackpos_sender;
                let trackend_sender = self.trackend_sender;

                move |data, _| {
                    if paused.load(Ordering::Acquire) {
                        data.fill(0f32);
                        return;
                    }

                    let needed_frames = data.len() / channels;
                    let mut written_frames = 0;

                    let seek_second = seek_pos.load(Ordering::Acquire);

                    if seek_second != u64::MAX {
                        // `seek_second` is a second of the slider's range, so it always fits an `i64`.
                        if let Some(time) = Time::try_new(seek_second as i64, 0)
                        {
                            let _ = format.seek(
                                SeekMode::Coarse,
                                SeekTo::Time {
                                    time,
                                    track_id: Some(track_id),
                                },
                            );
                        }

                        // What was decoded before the jump belongs to the old position.
                        decoder.reset();
                        pcm_buffer.clear();
                        pcm_offset = 0;

                        seek_pos.store(u64::MAX, Ordering::Release);
                    }

                    while written_frames < needed_frames {
                        if pcm_offset >= pcm_buffer.len() {
                            let Ok(Some(packet)) = format.next_packet() else {
                                // End of the track. What is left of the buffer is silence.
                                trackend_sender.send(player_id);
                                break;
                            };

                            if packet.track_id != track_id {
                                pcm_buffer.clear();
                                pcm_offset = 0;
                                break;
                            }

                            // A packet that fails to decode is skipped, the next one may be fine.
                            let Ok(decoded_buf) = decoder.decode(&packet)
                            else {
                                break;
                            };

                            pcm_buffer.clear();
                            pcm_buffer
                                .resize(decoded_buf.samples_interleaved(), 0.0);
                            decoded_buf
                                .copy_to_slice_interleaved(&mut pcm_buffer);
                            pcm_offset = 0;

                            let second = time_base
                                .calc_time(packet.pts)
                                .map_or(0, |pos| pos.as_secs().max(0) as u64);

                            if second != last_playback_second {
                                trackpos_sender.send((player_id, second));
                                last_playback_second = second;
                            }
                        }

                        let available_samples = pcm_buffer.len() - pcm_offset;
                        let remaining_frames = needed_frames - written_frames;
                        let remaining_samples = remaining_frames * channels;
                        let samples_to_copy =
                            available_samples.min(remaining_samples);

                        let start = written_frames * channels;
                        let end = start + samples_to_copy;
                        data[start..end].copy_from_slice(
                            &pcm_buffer
                                [pcm_offset..pcm_offset + samples_to_copy],
                        );

                        pcm_offset += samples_to_copy;
                        written_frames += samples_to_copy / channels;
                    }

                    data[written_frames * channels..].fill(0.0);
                }
            },
            move |_| {},
            Some(Duration::ZERO),
        ) {
            Ok(stream) => stream,
            Err(err) => {
                alert_default(&fmt1(
                    tr!("Creating audio stream failed: {}"),
                    &err.to_string(),
                ));
                return;
            }
        };

        if let Err(err) = stream.play() {
            alert_default(&fmt1(
                tr!("Playing audio stream failed: {}"),
                &err.to_string(),
            ));
            return;
        }

        self.audio_player = Some(AudioPlayer {
            id: player_id,
            _stream: stream,
            paused,
            seek_pos,
            duration,
            duration_string: format!(
                "{:02}:{:02}",
                duration.as_secs() / 60,
                duration.as_secs() % 60
            ),
        });

        self.trackpos_sender.send((player_id, 0));
    }

    fn pause_button_cb(&mut self, _: &mut Button) {
        if let Some(player) = &self.audio_player {
            player.paused.store(true, Ordering::Release);
        }
    }

    fn stop_button_cb(&mut self, _: &mut Button) {
        self.stop_playback();
    }

    fn select_output_dir_button_cb(&mut self, _: &mut Button) {
        let Some(dir) = dir_chooser(
            tr!("Select Output Directory"),
            &dirs::home_dir()
                .unwrap_or_default()
                .join("")
                .to_string_lossy(),
            true,
        ) else {
            return;
        };

        self.output_dir_input.set_value(&dir);
    }

    fn progress_slider_handle(
        &mut self,
        this: &mut HorNiceSlider,
        event: Event,
    ) -> bool {
        let Some(player) = &self.audio_player else {
            return false;
        };

        // Always `false`: the slider must handle the mouse itself to follow it. The lock only stops `poll_audio` moving it while held.
        match event {
            Event::Push => self.progress_slider_locked = true,
            Event::Released if self.progress_slider_locked => {
                self.progress_slider_locked = false;
                player
                    .seek_pos
                    .store(this.value() as u64, Ordering::Release);
            }
            _ => {}
        }

        false
    }

    fn select_all_button_cb(&mut self, _: &mut Button) {
        for item in 1..=self.file_list.size() {
            self.file_list.select(item);
        }
    }

    fn deselect_all_button_cb(&mut self, _: &mut Button) {
        for item in self.file_list.selected_items() {
            self.file_list.deselect(item);
        }
    }

    fn output_dir_input_handle(
        &mut self,
        _: &mut FileInput,
        event: Event,
    ) -> bool {
        // Dropped files arrive as a paste of their paths. Treat it as a drop unless it is a single folder: the output directory the user means.
        if event != Event::Paste {
            return false;
        }

        let text = app::event_text();
        let is_drop = !text.is_empty()
            && text.lines().all(|line| Path::new(line).exists())
            && (text.lines().nth(1).is_some() || Path::new(&text).is_file());

        if is_drop {
            self.parse_files(&text);
        }

        is_drop
    }

    fn image_frame_draw(&mut self, this: &mut Frame) {
        if this.label() != "" {
            // A labelled frame is drawn by FLTK.
            return;
        }

        draw_box(
            self.image_frame.frame(),
            self.image_frame.x(),
            self.image_frame.y(),
            self.image_frame.w(),
            self.image_frame.h(),
            self.image_frame.color(),
        );

        let Some(image) = &mut self.current_image else {
            return;
        };

        push_clip(
            self.image_frame.x(),
            self.image_frame.y(),
            self.image_frame.width(),
            self.image_frame.height(),
        );

        // Zoom is a factor of the picture's own size, so 100% is its real size. `draw` only clips: the image has to be scaled itself.
        // `data_w`/`data_h` stay the real size, so repeated scaling doesn't compound.
        let zoomed_width =
            ((image.data_w() as f32 * self.image_scale_factor) as i32).max(1);
        let zoomed_height =
            ((image.data_h() as f32 * self.image_scale_factor) as i32).max(1);

        image.scale(zoomed_width, zoomed_height, false, true);
        image.draw(
            self.image_frame.x() + self.image_offset_x,
            self.image_frame.y() + self.image_offset_y,
            zoomed_width,
            zoomed_height,
        );

        pop_clip();
    }

    fn image_frame_handle(&mut self, _: &mut Frame, event: Event) -> bool {
        match event {
            Event::MouseWheel => {
                let direction = app::event_dy();
                let (mut mouse_x, mut mouse_y) = app::event_coords();
                mouse_x -= self.image_frame.x();
                mouse_y -= self.image_frame.y();

                let old_scale = self.image_scale_factor;

                if direction == MouseWheel::Up {
                    self.image_scale_factor *= 1.25;
                } else if direction == MouseWheel::Down {
                    self.image_scale_factor /= 1.25;
                }

                // Unbounded, zooming out reaches a zero-sized image and zooming in a gigantic one.
                self.image_scale_factor =
                    self.image_scale_factor.clamp(0.1, 16.0);

                if (0.99..=1.01).contains(&self.image_scale_factor) {
                    self.image_scale_factor = 1.0;
                }

                self.image_offset_x = (mouse_x as f32
                    - (mouse_x - self.image_offset_x) as f32
                        * (self.image_scale_factor / old_scale))
                    as i32;
                self.image_offset_y = (mouse_y as f32
                    - (mouse_y - self.image_offset_y) as f32
                        * (self.image_scale_factor / old_scale))
                    as i32;

                self.image_frame.redraw();
                true
            }
            Event::Push => {
                if self.current_image.is_some() {
                    self.last_mouse_pos = Some(app::event_coords());
                    true
                } else {
                    false
                }
            }
            Event::Drag => {
                if self.current_image.is_some() {
                    if let Some((last_x, last_y)) = self.last_mouse_pos {
                        let (event_x, event_y) = app::event_coords();
                        let dx = event_x - last_x;
                        let dy = event_y - last_y;

                        self.image_offset_x += dx;
                        self.image_offset_y += dy;

                        self.last_mouse_pos = Some((event_x, event_y));
                        self.image_frame.redraw();
                    }
                    true
                } else {
                    false
                }
            }
            Event::Released => {
                self.last_mouse_pos = None;
                true
            }
            _ => false,
        }
    }

    /// Applies what the audio thread reported: the playback position, and that the track has ended.
    fn poll_audio(&mut self) {
        // A report can outlive its track: it may arrive after Stop or after loading another track.
        while let Some((player_id, playback_second)) =
            self.trackpos_receiver.recv()
        {
            let Some(audio_player) = self
                .audio_player
                .as_ref()
                .filter(|player| player.id == player_id)
            else {
                continue;
            };

            self.progress_label.set_label(&format!(
                "{:02}:{:02} / {}",
                (playback_second as u32) / 60,
                (playback_second as u32) % 60,
                audio_player.duration_string
            ));

            if !self.progress_slider_locked {
                self.progress_slider
                    .set_range(0f64, audio_player.duration.as_secs() as f64);
                self.progress_slider.set_value(playback_second as f64);
            }
        }

        while let Some(player_id) = self.trackend_receiver.recv() {
            if self
                .audio_player
                .as_ref()
                .is_some_and(|player| player.id == player_id)
            {
                self.stop_playback();
            }
        }
    }
}

fn main() {
    Application::run();
}
