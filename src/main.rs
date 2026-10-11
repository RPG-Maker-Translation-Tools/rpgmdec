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
    enums::{Color, ColorDepth, Event, FrameType, Shortcut},
    frame::Frame,
    group::{Flex, Group},
    image::{PngImage, RgbImage, SharedImage},
    input::{FileInput, Input},
    menu::{MenuBar, MenuFlag},
    misc::InputChoice,
    prelude::*,
    valuator::HorNiceSlider,
    window::Window,
};
use fltk_theme::{ColorTheme, color_themes::BLACK_THEME};
use fontdue::{Font, FontSettings};
use gxhash::{GxBuildHasher, HashMap};
use indexmap::IndexMap;
use phf::{Map, phf_map};
use rayon::prelude::*;
use rpgmad_lib::{
    ArchiveEntry, Decrypter as ArchiveDecrypter, Engine, VX_RGSS2A_EXT,
    VXACE_RGSS3A_EXT, XP_RGSSAD_EXT,
};
use rpgmasd::{
    DECRYPTED_ASSETS_EXTS, Decrypter as AssetDecrypter, ENCRYPTED_ASSET_EXTS,
    FileType, HEADER_LENGTH, M4A_EXT, MV_M4A_EXT, MV_OGG_EXT, MV_PNG_EXT,
    MZ_M4A_EXT, MZ_OGG_EXT, MZ_PNG_EXT, OGG_EXT, PNG_EXT, decrypt_in_place,
    encrypt,
};
use std::{
    borrow::Cow,
    cell::RefCell,
    ffi::OsStr,
    fs::{create_dir_all, exists, metadata, read, write},
    io::Cursor,
    ops::Range,
    path::{Component, Path, PathBuf},
    ptr::from_mut as ptr_from_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};
use strum_macros::EnumIs;
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

type IndexMapGx = IndexMap<String, Option<FileType>, GxBuildHasher>;

const WINDOW_WIDTH: i32 = 1024;
const WINDOW_HEIGHT: i32 = 720;

/// Top-level directories of a game that an `.rgss` archive holds.
const ARCHIVE_DIRS: [&str; 4] = ["Audio", "Graphics", "Data", "Fonts"];

const MV_ENGINE_LABEL: &str = "MV";
const MZ_ENGINE_LABEL: &str = "MZ";
const XP_ENGINE_LABEL: &str = "XP";
const VX_ENGINE_LABEL: &str = "VX";
const VXACE_ENGINE_LABEL: &str = "VX Ace";

const LOGO_32PX: &[u8] = include_bytes!("../icons/32px.rgba");
const LOGO_256PX: &[u8] = include_bytes!("../icons/256px.rgba");

struct AudioPlayer {
    stream: Option<cpal::Stream>,
    playback_state: Arc<AtomicU8>,
    seek_pos: Arc<AtomicU64>,
    duration: Time,
    duration_string: Arc<String>,
}

impl AudioPlayer {
    fn stop(&mut self) {
        self.playback_state.store(2, Ordering::Release);
        self.stream.take();
    }
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

thread_local! {
    static WIDGET_MAP: RefCell<HashMap<usize, &'static str>> =
        RefCell::new(HashMap::default());
}

trait TrId<W>
where
    W: WidgetExt,
{
    fn set_tr_id(&self, id: &'static str);
    fn with_tr_id(self, id: &'static str) -> Self;
    fn tr_id(&self) -> &'static str;
}

impl<W> TrId<W> for W
where
    W: WidgetExt + Clone + 'static,
{
    fn set_tr_id(&self, id: &'static str) {
        WIDGET_MAP
            .with_borrow_mut(|m| m.insert(self.as_widget_ptr() as usize, id));
    }

    fn with_tr_id(self, id: &'static str) -> Self {
        self.set_tr_id(id);
        self
    }

    fn tr_id(&self) -> &'static str {
        WIDGET_MAP.with_borrow(|m| {
            m.get(&(self.as_widget_ptr() as usize))
                .copied()
                .unwrap_or("")
        })
    }
}

enum DisplayMode {
    Image(SharedImage),
    /// Index of the file in the list.
    Audio(usize),
    Message(String),
}

/// Encodings of the file names inside `.rgss` archives that the `Encoding` menu offers.
const ARCHIVE_ENCODINGS: &[(&str, &Encoding)] = &[
    ("&Encoding/UTF-8\t", UTF_8),
    ("&Encoding/Shift-JIS\t", SHIFT_JIS),
    ("&Encoding/GB18030\t", GB18030),
    ("&Encoding/Big5\t", BIG5),
    ("&Encoding/EUC-KR\t", EUC_KR),
    ("&Encoding/Windows-1251\t", WINDOWS_1251),
    ("&Encoding/Windows-1252\t", WINDOWS_1252),
];

/// A file of the opened archive. Its content lives in [`Application::archive_data`], already decrypted.
struct ArchiveFile {
    /// Decoded, with `/` separators.
    path: String,
    data: Range<usize>,
}

/// A file to put into an archive: its path as the archive stores it, and its content.
type EncodedFile = (Vec<u8>, Vec<u8>);

/// The part of `path` starting at the first (or, with `last`, the last) component that is one of `anchors`.
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
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[derive(EnumIs, Clone, Copy, PartialEq)]
enum State {
    None,
    EncryptAsset,
    DecryptAsset,
    EncryptArchive,
    DecryptArchive,
}

#[derive(EnumIs)]
enum Language {
    English,
    Russian,
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
    "Drop files/folders into the window or use `Add File`/`Add Folder` in the `File` menu." => "Добавьте файлы/папки перетаскиванием, либо используйте пункты «Добавить файл» / «Добавить папку» в меню «Файл».",
    "Play" => "Воспроизвести",
    "Pause" => "Пауза",
    "Stop" => "Стоп",
    "&File/Add File\t" => "&Файл/Добавить файл\t",
    "&File/Add Folder\t" => "&Файл/Добавить папку\t",
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
    "Unable to determine the type of the passed assets from the passed path {}. If you want to encrypt an archive, your assets should be arranged in `Audio`, `Graphics`, `Data` or/and `Fonts` directories. If you want to encrypt assets, the path the the asset should contain `www` directory." => "Не удалось определить тип загруженных ассетов из пути {}. Если вы хотите зашифровать архив, ваши ассеты должны быть распределены по директориям Audio, Graphics, Data и/или Fonts. Если вы хотите зашифровать ассеты, путь к ассетам должен содержать директорию www",
    "Component mismatch when parsing files. Detected `www` directory for asset encryption, but it's missing in the path {}." => "Несовпадение компонента при парсинге файлов. Определена директория www для зашифровки ассетов, но она отсутствует в пути {}.",
    "Component mismatch when parsing files. Detected `Audio/Graphics/Data/Fonts` directories for archive encryption, but it's missing in the path {}." => "Несовпадение компонента при парсинге файлов. Определены директории Audio/Graphics/Data/Fonts для зашифровки архива, но они отсутствуют в пути {}.",
    "RPGMDec {}\nLicensed under WTFPL\nRepository: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}" => "RPGMDec {}\nЛицензировано под WTFPL\nРепозиторий: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}",
    "Reading file {} failed: {}" => "Чтение файла {} не удалось: {}",
    "Decryption of file {} failed: {}" => "Расшифровка файла {} не удалась: {}",
    "Unable to found playable track" => "Не удалось найти проигрываемую дорожку",
    "Decoding the format failed: {}" => "Расшифровка формата не удалась: {}",
    "Select File" => "Выберите файл",
    "Encrypted Assets/Archives (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,rgss2a,rgss3a}\t" => "Зашифрованные ассеты/архивы (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,rgss2a,rgss3a}\t",
    "Creating directory {} failed: {}" => "Создание директории {} не удалось: {}",
    "Writing file {} failed: {}" => "Запись файла {} не удалась: {}",
    "Aborting decryption: {}" => "Прерываем расшифровку: {}",
    "Parsing key from string failed: {}" => "Парсинг ключа из текста не удался: {}",
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

fn init_ru() {
    USE_RUSSIAN.store(true, Ordering::Relaxed);
}

fn init_en() {
    USE_RUSSIAN.store(false, Ordering::Relaxed);
}

macro_rules! tr {
    ($k:expr) => {
        if USE_RUSSIAN.load(Ordering::Relaxed) {
            RUSSIAN.get(&$k).map(|k| *k).unwrap_or($k)
        } else {
            $k
        }
    };
}

/// Writes `data` to `path`, creating the directories above it. The error is ready to show.
fn write_with_parents(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && let Err(err) = create_dir_all(parent)
    {
        return Err(tr!("Creating directory {} failed: {}")
            .replacen("{}", &parent.to_string_lossy(), 1)
            .replacen("{}", &err.to_string(), 1));
    }

    write(path, data).map_err(|err| {
        tr!("Writing file {} failed: {}")
            .replacen("{}", &path.to_string_lossy(), 1)
            .replacen("{}", &err.to_string(), 1)
    })
}

const MENU_BAR_ITEMS: &[&str] = &[
    "&File/Add File\t",
    "&File/Add Folder\t",
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
    encrypted_archive_extension: String,
    file_list_map: IndexMapGx,
    audio_player: Option<AudioPlayer>,
    /// Copied from `output_dir_input` when processing starts, it's what the user typed or picked.
    output_dir: String,
    image_offset_x: i32,
    image_offset_y: i32,
    last_mouse_pos: Option<(i32, i32)>,
    current_image: Option<SharedImage>,
    /// `current_image` scaled by the factor it's paired with.
    scaled_image: Option<(f32, SharedImage)>,
    image_scale_factor: f32,
    progress_slider_locked: bool,
    state: State,
    language: Language,

    app: App,
    window: Window,
    trackpos_sender: Sender<u64>,
    trackpos_receiver: Receiver<u64>,
    trackend_sender: Sender<bool>,
    trackend_receiver: Receiver<bool>,

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
}

impl Application {
    /// A `&mut Self` that outlives the borrow, for the `'static` callbacks FLTK takes.
    ///
    /// The invariant that makes it sound: `Application` is a local of [`Application::run`] that stays put until the
    /// event loop returns, and FLTK runs every callback registered on its widgets on this thread from inside that
    /// loop, one at a time. The only nested loops are modal dialogs, which block the other widgets' callbacks until
    /// they close.
    fn alias(&mut self) -> &'static mut Self {
        unsafe { &mut *ptr_from_mut::<Self>(self) }
    }

    fn retranslate_widgets(&self, root: &Group) {
        let mut stack = Vec::new();
        stack.push(root.clone());

        while let Some(group) = stack.pop() {
            for i in 0..group.children() {
                let mut widget = group.child(i).unwrap();

                if let Some(subgroup) = widget.as_group() {
                    stack.push(subgroup);
                    continue;
                }

                if let Some(mut input) = Input::from_dyn_widget(&widget) {
                    let tr_id = input.tr_id();

                    if tr_id.is_empty() {
                        continue;
                    }

                    input.set_value(tr!(&tr_id));
                } else {
                    let tr_id = widget.tr_id();

                    if tr_id.is_empty() {
                        continue;
                    }

                    if self.current_image.is_some()
                        && widget == self.image_frame.as_base_widget()
                    {
                        continue;
                    }

                    widget.set_label(tr!(&tr_id));
                }
            }
        }
    }

    fn retranslate(&mut self, language: Option<&str>) {
        if let Some(language) = language {
            if language == "ru" {
                self.language = Language::Russian;
                init_ru();
            } else {
                self.language = Language::English;
                init_en();
            }
        } else if let Some(locale) = get_locale()
            && locale.starts_with("ru")
        {
            self.language = Language::Russian;
            init_ru();
        } else {
            self.language = Language::English;
            init_en();
        }

        self.retranslate_widgets(&self.window.as_group().unwrap());

        let engine_index = self.output_engine_select.menu_button().value();

        if engine_index == -1 {
            self.output_engine_select
                .set_value(tr!("-Select engine for encryption-"));
        }

        if matches!(self.state, State::DecryptArchive | State::DecryptAsset) {
            self.process_button.set_label(tr!("Decrypt"));
        } else if matches!(
            self.state,
            State::EncryptArchive | State::EncryptAsset
        ) {
            self.process_button.set_label(tr!("Encrypt"));
        }

        self.menu_bar.clear();
        self.add_menubar_entries();
        self.window.redraw();
    }

    fn run() {
        let file_list_map = IndexMapGx::default();
        let audio_player = None;
        let output_dir = String::new();
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

        let select_output_dir_button =
            Button::default().with_tr_id("Select Output Directory");
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
        let select_all_button = Button::default().with_tr_id("Select All");
        let deselect_all_button = Button::default().with_tr_id("Deselect All");
        select_button_layout.end();
        left_layout.fixed(&select_button_layout, 48);

        let button_layout = Flex::default().row();
        let clear_button = Button::default().with_tr_id("Clear");
        let remove_button = Button::default().with_tr_id("Remove");
        let mut process_button = Button::default();
        process_button.hide();

        button_layout.end();
        left_layout.fixed(&button_layout, 48);

        left_layout.end();

        let right_layout = Flex::default()
            .column()
            .with_size(WINDOW_WIDTH / 2, WINDOW_HEIGHT);

        let mut image_frame = Frame::default_fill().with_tr_id(
            "Drop files/folders into the window or use `Add File`/`Add \
             Folder` in the `File` menu.",
        );
        image_frame.set_frame(FrameType::DownBox);

        let mut audio_controls = Flex::default_fill().column();
        let play_button = Button::default().with_tr_id("Play");
        let pause_button = Button::default().with_tr_id("Pause");
        let stop_button = Button::default().with_tr_id("Stop");

        let progress_label = Frame::default().with_label("00:00 / 00:00");
        let progress_slider = HorNiceSlider::default();

        audio_controls.end();
        audio_controls.hide();

        right_layout.end();
        main_layout.end();
        window_layout.end();

        window.end();
        window.show();

        let mut app = Application {
            archive_data: Vec::new(),
            archive_files: Vec::new(),
            archive_path: None,
            archive_encoding: UTF_8,
            encrypted_archive_extension: String::new(),
            state: State::None,
            language: Language::English,
            file_list_map,
            audio_player,
            output_dir,
            image_offset_x,
            image_offset_y,
            last_mouse_pos,
            current_image,
            scaled_image: None,
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
        };

        app.retranslate(None);
        app.set_callbacks();

        // `wait` sleeps until an event or a message from the audio thread (`Sender::send` wakes the loop). An idle
        // callback would do the same polling, but keeps FLTK from ever sleeping, i.e. one core at 100% all the time.
        while app.app.wait() {
            app.poll_audio();
        }
    }

    fn update_display(&mut self, mode: DisplayMode) {
        match mode {
            DisplayMode::Image(image) => {
                self.stop_playback();

                self.current_image = Some(image);
                self.scaled_image = None;

                self.image_frame.set_label("");
                self.image_frame.show();
                self.image_frame.redraw();

                self.audio_controls.hide();
                self.audio_controls.redraw();
            }

            DisplayMode::Audio(index) => {
                self.current_image = None;

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
            }

            DisplayMode::Message(msg) => {
                self.stop_playback();

                self.current_image = None;

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
        self.encrypted_archive_extension = String::new();
        self.output_engine_select.clear();
        self.output_engine_select
            .set_value(tr!("-Select engine for encryption-"));
        self.process_button.hide();
        self.process_button.redraw();
        self.button_layout.layout();
        self.update_display(DisplayMode::Message(
            tr!("Drop files/folders into the window or use `Add File`/`Add \
                 Folder` in the `File` menu.")
            .to_string(),
        ));
    }

    fn clear_button_cb(&mut self, _this: &mut Button) {
        self.clear();
    }

    fn stop_playback(&mut self) {
        if let Some(player) = &mut self.audio_player {
            player.stop();
        }

        self.progress_label.set_label("00:00 / 00:00");
        self.progress_slider.set_range(0f64, 0f64);
        self.progress_slider.set_value(0f64);
    }

    fn remove_button_cb(&mut self, _this: &mut Button) {
        // Reversed, so that removing a line doesn't shift the ones still to go.
        for item in self.file_list.selected_items().into_iter().rev() {
            if self.state.is_decrypt_archive() {
                self.archive_files.remove(item as usize - 1);
            } else {
                self.file_list_map.shift_remove_index(item as usize - 1);
            }

            self.file_list.remove(item);
        }

        self.update_display(DisplayMode::Message(String::new()));
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
                    dirs::home_dir().unwrap().join(""),
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
                    tr!("Select File"),
                    dirs::home_dir()
                        .unwrap()
                        .join("")
                        .as_os_str()
                        .to_str()
                        .unwrap(),
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

        let mut_self = self.alias();
        self.menu_bar.add(tr!(MENU_BAR_ITEMS[i]), Shortcut::None, MenuFlag::Normal, move |_| {
            let _ = match mut_self.language {
                Language::Russian => webbrowser::open("https://github.com/rpg-maker-translation-tools/rpgmdec/tree/master/docs/help-ru.md"),
                Language::English => webbrowser::open("https://github.com/rpg-maker-translation-tools/rpgmdec/tree/master/docs/help.md"),
            };
        });
        i += 1;

        self.menu_bar.add(tr!(MENU_BAR_ITEMS[i]), Shortcut::None, MenuFlag::Normal, |_| {
            let mut about_window = Window::default().with_label(tr!("About RPGMDec")).with_size(600, 320);

            let mut layout = Flex::default_fill().column();

            let mut image_frame = Frame::default();
            image_frame.set_image(unsafe {RgbImage::from_data(LOGO_256PX, 256, 256, ColorDepth::Rgba8).ok()});

            let _frame = Frame::default().with_label(&tr!("RPGMDec {}\nLicensed under WTFPL\nRepository: https://github.com/rpg-maker-translation-tools/rpgmdec\nFLTK {}").replacen("{}", env!("CARGO_PKG_VERSION"), 1).replacen("{}", &version_str(), 1));

            let mut ok_button = Button::default().with_label(tr!("OK"));

            layout.fixed(&ok_button, 32);
            layout.end();
            about_window.end();
            about_window.make_modal(true);
            about_window.show();

            ok_button.set_callback(move |_| {
                about_window.hide();
            });
        });
    }

    fn set_archive_encoding(&mut self, encoding: &'static Encoding) {
        self.archive_encoding = encoding;

        // Names of an open archive were decoded with the old one.
        if self.state.is_decrypt_archive()
            && let Some(path) = self.archive_path.clone()
        {
            self.parse_archive(&path);
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
            // The field can be typed into, not only filled by the button.
            mut_self.output_dir = mut_self.output_dir_input.value();

            let date = OffsetDateTime::now_utc();
            let output_dir = Path::new(&mut_self.output_dir).join(format!(
                "{}-{}_{}-{:02}-{:02}_{:02}-{:02}-{:02}",
                if mut_self.state.is_decrypt_archive() {
                    &mut_self.encrypted_archive_extension
                } else {
                    "www"
                },
                if matches!(
                    mut_self.state,
                    State::DecryptArchive | State::DecryptAsset
                ) {
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

            if matches!(
                mut_self.state,
                State::DecryptArchive | State::DecryptAsset
            ) {
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

        let result = self
            .file_list
            .selected_items()
            .into_par_iter()
            .try_for_each(|item| {
                let entry = &archive_files[item as usize - 1];

                // The archive is untrusted: an absolute or `..` path would write outside of `decrypted_dir`.
                if !is_contained(&entry.path) {
                    return Err(tr!("Archive entry {} points outside of the \
                                    output directory.")
                    .replacen("{}", &entry.path, 1));
                }

                write_with_parents(
                    &decrypted_dir.join(&entry.path),
                    &archive_data[entry.data.clone()],
                )
            });

        if let Err(err) = &result {
            alert_default(
                &tr!("Aborting decryption: {}").replacen("{}", err, 1),
            );
        }

        result.is_ok()
    }

    /// Returns whether everything selected was written.
    fn decrypt_assets(&self, decrypted_dir: &Path) -> bool {
        let file_list_map = &self.file_list_map;

        let result = self
            .file_list
            .selected_items()
            .into_par_iter()
            .try_for_each(|item| {
                let index = item as usize - 1;
                let (path, file_type) = file_list_map.get_index(index).unwrap();

                let Some(file_type) = file_type else {
                    return Ok(());
                };

                let mut encrypted_data = match read(path) {
                    Ok(encrypted_data) => encrypted_data,
                    Err(err) => {
                        return Err(tr!("Reading file {} failed: {}")
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1));
                    }
                };

                if let Err(err) =
                    decrypt_in_place(&mut encrypted_data, *file_type)
                {
                    return Err(tr!("Decryption of file {} failed: {}")
                        .replacen("{}", path, 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                let path = Path::new(path);
                let relative_path = relative_path_from(path, &["www"], true)
                    .or_else(|| path.file_name().map(PathBuf::from))
                    .unwrap_or_default();

                write_with_parents(
                    &decrypted_dir.join(
                        relative_path.with_extension(file_type.to_string()),
                    ),
                    &encrypted_data[HEADER_LENGTH..],
                )
            });

        if let Err(err) = &result {
            alert_default(
                &tr!("Aborting decryption: {}").replacen("{}", err, 1),
            );
        }

        result.is_ok()
    }

    fn decrypt_cb(&mut self, _: &mut Button, decrypted_dir: &Path) {
        if !self.output_dir_valid() || !self.has_selection() {
            return;
        }

        let succeeded = match self.state {
            State::DecryptArchive => self.decrypt_archive(decrypted_dir),
            State::DecryptAsset => self.decrypt_assets(decrypted_dir),
            _ => unreachable!(),
        };

        // The failure has been reported already.
        if succeeded {
            message_default(
                &tr!("Decryption ended. Decrypted entries are put to the {}")
                    .replacen("{}", &decrypted_dir.display().to_string(), 1),
            );
        }
    }

    fn output_dir_valid(&self) -> bool {
        let mut output_dir_set = !self.output_dir.is_empty();

        if output_dir_set {
            output_dir_set = exists(&self.output_dir).unwrap_or_default();

            if output_dir_set {
                output_dir_set =
                    metadata(&self.output_dir).is_ok_and(|m| m.is_dir());

                if !output_dir_set {
                    alert_default(tr!("Output directory is not a directory."));
                }
            } else {
                alert_default(tr!("Output directory does not exist."));
            }
        } else {
            alert_default(tr!("Output directory not specified."));
        }

        output_dir_set
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
                alert_default(
                    &tr!("Parsing key from string failed: {}").replacen(
                        "{}",
                        &err.to_string(),
                        1,
                    ),
                );
                return false;
            }

            key
        } else {
            let Some(file_path) = file_chooser(
                tr!("Select Encrypted File"),
                "*",
                dirs::home_dir().unwrap().join(""),
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
                    alert_default(err);
                    return false;
                }
            };

            let file = match read(&file_path) {
                Ok(file) => file,
                Err(err) => {
                    alert_default(
                        &tr!("Reading file {} failed: {}")
                            .replacen("{}", &file_path, 1)
                            .replacen("{}", &err.to_string(), 1),
                    );
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

        let file_list_map = &self.file_list_map;
        let encryption_key = &encryption_key;

        let result = self
            .file_list
            .selected_items()
            .into_par_iter()
            .try_for_each(|item| {
                let index = item as usize - 1;

                let (path, file_type) = file_list_map.get_index(index).unwrap();

                if file_type.is_some() {
                    return Ok(());
                }

                let path = Path::new(path);
                let input_ext = path
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();

                // Only these are encrypted by RPG Maker, a game's fonts and the like stay as they are.
                let output_ext = match (input_ext.as_str(), engine) {
                    (PNG_EXT, MV_ENGINE_LABEL) => MV_PNG_EXT,
                    (PNG_EXT, MZ_ENGINE_LABEL) => MZ_PNG_EXT,
                    (OGG_EXT, MV_ENGINE_LABEL) => MV_OGG_EXT,
                    (OGG_EXT, MZ_ENGINE_LABEL) => MZ_OGG_EXT,
                    (M4A_EXT, MV_ENGINE_LABEL) => MV_M4A_EXT,
                    (M4A_EXT, MZ_ENGINE_LABEL) => MZ_M4A_EXT,
                    _ => return Ok(()),
                };

                let decrypted_data = match read(path) {
                    Ok(decrypted_data) => decrypted_data,
                    Err(err) => {
                        return Err(tr!("Reading file {} failed: {}")
                            .replacen("{}", &path.to_string_lossy(), 1)
                            .replacen("{}", &err.to_string(), 1));
                    }
                };

                let encrypted_data =
                    match encrypt(&decrypted_data, encryption_key) {
                        Ok(encrypted_data) => encrypted_data,
                        Err(err) => {
                            return Err(
                                tr!("Encryption of file {} failed: {}")
                                    .replacen("{}", &path.to_string_lossy(), 1)
                                    .replacen("{}", &err.to_string(), 1),
                            );
                        }
                    };

                let relative_path = relative_path_from(path, &["www"], true)
                    .or_else(|| path.file_name().map(PathBuf::from))
                    .unwrap_or_default();

                write_with_parents(
                    &encrypted_dir
                        .join(relative_path)
                        .with_extension(output_ext),
                    &encrypted_data,
                )
            });

        if let Err(err) = &result {
            alert_default(
                &tr!("Aborting encryption: {}").replacen("{}", err, 1),
            );
        }

        result.is_ok()
    }

    /// Returns whether the archive was written.
    fn encrypt_archive(&mut self, encrypted_dir: &Path, engine: &str) -> bool {
        let file_list_map = &self.file_list_map;
        let encoding = self.archive_encoding;

        let files: Result<Vec<EncodedFile>, String> = self
            .file_list
            .selected_items()
            .into_par_iter()
            .map(|item| {
                let index = item as usize - 1;
                let (path, _) = file_list_map.get_index(index).unwrap();

                // Listed files all passed `should_include_file`, so one of these directories is in the path.
                let relative_path =
                    relative_path_from(Path::new(path), &ARCHIVE_DIRS, false)
                        .unwrap();

                // Archives hold the path from the top-level directory down, with backslashes.
                let Some(relative_path) = relative_path.to_str() else {
                    return Err(
                        tr!("{} is not valid Unicode.").replacen("{}", path, 1)
                    );
                };

                let relative_path = relative_path.replace('/', "\\");
                let (archive_path, _, unmappable) =
                    encoding.encode(&relative_path);

                // Anything else would silently write `&#1234;` into the name.
                if unmappable {
                    return Err(tr!("{} can't be represented in {}. Pick \
                                    another encoding in the Encoding menu.")
                    .replacen("{}", path, 1)
                    .replacen("{}", encoding.name(), 1));
                }

                let data = match read(path) {
                    Ok(data) => data,
                    Err(err) => {
                        return Err(tr!("Reading file {} failed: {}")
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1));
                    }
                };

                Ok((archive_path.into_owned(), data))
            })
            .collect();

        let files = match files {
            Ok(files) => files,
            Err(err) => {
                alert_default(
                    &tr!("Aborting encryption: {}").replacen("{}", &err, 1),
                );
                return false;
            }
        };

        let archive_entries: Vec<ArchiveEntry> = files
            .iter()
            .map(|(path, data)| ArchiveEntry { path, data })
            .collect();

        let engine_kind = match engine {
            VXACE_ENGINE_LABEL => Engine::VXAce,
            _ => Engine::Older,
        };

        // Zeroed allocation, so no pass over a buffer that is about to be overwritten anyway.
        let mut archive = vec![
            0u8;
            ArchiveDecrypter::encrypted_buffer_size(
                &archive_entries,
                engine_kind
            )
        ];

        if let Err(err) = ArchiveDecrypter::new().encrypt(
            &archive_entries,
            engine_kind,
            &mut archive,
        ) {
            alert_default(&tr!("Aborting encryption: {}").replacen(
                "{}",
                &err.to_string(),
                1,
            ));
            return false;
        }

        let output_path =
            encrypted_dir.join("Game").with_extension(match engine {
                XP_ENGINE_LABEL => XP_RGSSAD_EXT,
                VX_ENGINE_LABEL => VX_RGSS2A_EXT,
                VXACE_ENGINE_LABEL => VXACE_RGSS3A_EXT,
                _ => unreachable!(),
            });

        if let Err(err) = write_with_parents(&output_path, &archive) {
            alert_default(&err);
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
                self.encrypt_archive(encrypted_dir, &engine)
            }
            _ => unreachable!(),
        };

        // The failure has been reported already.
        if succeeded {
            message_default(
                &tr!("Encryption ended. Encrypted entries are put to the {}")
                    .replacen("{}", &encrypted_dir.display().to_string(), 1),
            );
        }
    }

    fn parse_archive(&mut self, path: &Path) {
        self.file_list.clear();
        let path_str = &path.to_string_lossy();

        let mut archive_data = match read(path) {
            Ok(archive_data) => archive_data,
            Err(err) => {
                alert_default(
                    &tr!("Reading file {} failed: {}")
                        .replacen("{}", path_str, 1)
                        .replacen("{}", &err.to_string(), 1),
                );
                return;
            }
        };

        // The entries borrow the buffer, which has to be stored along with them, so keep offsets into it instead.
        let base = archive_data.as_ptr() as usize;
        let mut archive_files = Vec::new();
        let mut skipped: usize = 0;

        let mut decrypter = ArchiveDecrypter::new();

        if let Err(err) = decrypter.start(&mut archive_data) {
            alert_default(
                &tr!("Decrypting file {} failed: {}")
                    .replacen("{}", path_str, 1)
                    .replacen("{}", &err.to_string(), 1),
            );
            return;
        }

        while let Some(entry) = decrypter.next_entry() {
            // Never show a mangled name: it could be the same as another entry's, and silently overwrite it.
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
                // Archives store paths with backslashes.
                path: name.replace('\\', "/"),
                data: start..start + entry.data.len(),
            });
        }

        if skipped > 0 {
            alert_default(
                &tr!("{} entries were skipped, their names aren't valid {}. \
                      Pick another encoding in the Encoding menu.")
                .replacen("{}", &skipped.to_string(), 1)
                .replacen("{}", self.archive_encoding.name(), 1),
            );
        }

        self.archive_data = archive_data;
        self.archive_files = archive_files;
        self.archive_path = Some(path.to_path_buf());
        self.encrypted_archive_extension = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();

        for file in &self.archive_files {
            self.file_list.add(&file.path);
        }
    }

    fn parse_asset(&mut self, file_path: &Path) -> Option<FileType> {
        let extension = file_path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)?;

        let ext = extension.as_str();

        let is_allowed = ENCRYPTED_ASSET_EXTS.contains(&ext)
            || DECRYPTED_ASSETS_EXTS.contains(&ext)
            || matches!(ext, "ttf" | "otf" | "rxdata" | "rvdata" | "rvdata2");

        if !is_allowed {
            return None;
        }

        let path = file_path.to_string_lossy();

        if self.file_list_map.contains_key(&*path) {
            return None;
        }

        let filename = file_path.file_name()?.to_string_lossy();
        let file_type = FileType::try_from(ext).ok();

        self.file_list.add(&filename);
        self.file_list_map.insert(path.into_owned(), file_type);

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

        if DECRYPTED_ASSETS_EXTS.contains(&ext)
            || matches!(ext, "ttf" | "otf" | "rxdata" | "rvdata" | "rvdata2")
        {
            if path.components().any(|c| c.as_os_str() == "www") {
                return Some(State::EncryptAsset);
            }

            if ARCHIVE_DIRS
                .iter()
                .any(|dir| path.components().any(|c| c.as_os_str() == *dir))
            {
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
                DECRYPTED_ASSETS_EXTS.contains(&ext)
                    && path.components().any(|c| c.as_os_str() == "www")
            }
            State::EncryptArchive => {
                let is_valid_ext = DECRYPTED_ASSETS_EXTS.contains(&ext)
                    || matches!(
                        ext,
                        "ttf" | "otf" | "rxdata" | "rvdata" | "rvdata2"
                    );

                if !is_valid_ext {
                    return false;
                }

                ARCHIVE_DIRS.iter().any(|&candidate| {
                    path.components().any(|c| c.as_os_str() == candidate)
                })
            }
            State::None => false,
        }
    }

    fn parse_files(&mut self, paths: &str) {
        self.clear();

        let mut detected_state: Option<State> = None;

        for path_str in paths.lines() {
            let path = Path::new(path_str);

            if path.is_dir() {
                for entry in WalkDir::new(path)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                {
                    let file_path = entry.path();

                    if detected_state.is_none() {
                        // A folder has files that say nothing, e.g. `Game.exe` next to `Game.rgss3a`. Keep looking.
                        let Some(state) =
                            Self::detect_state_from_path(file_path)
                        else {
                            continue;
                        };

                        detected_state = Some(state);
                        self.state = state;
                    }

                    if let Some(target_state) = detected_state
                        && Self::should_include_file(file_path, target_state)
                    {
                        if target_state.is_decrypt_archive() {
                            self.parse_archive(file_path);
                            break;
                        }

                        self.parse_asset(file_path);
                    }
                }
            } else {
                if detected_state.is_none() {
                    let Some(state) = Self::detect_state_from_path(path) else {
                        continue;
                    };

                    detected_state = Some(state);
                    self.state = state;
                }

                if let Some(target_state) = detected_state
                    && Self::should_include_file(path, target_state)
                {
                    if target_state.is_decrypt_archive() {
                        self.parse_archive(path);
                        break;
                    }

                    self.parse_asset(path);
                }
            }
        }

        if detected_state.is_none() {
            message_default(tr!("Wasn't able to determine processing mode \
                                 by the first passed file."));
            return;
        }

        if self.file_list.size() == 0 {
            message_default(tr!("No eligible files were found."));
        }

        match self.state {
            State::DecryptArchive | State::DecryptAsset => {
                self.process_button.set_label(tr!("Decrypt"));
            }

            State::EncryptAsset => {
                self.output_engine_select.clear();
                self.output_engine_select.add(MV_ENGINE_LABEL);
                self.output_engine_select.add(MZ_ENGINE_LABEL);
                self.process_button.set_label(tr!("Encrypt"));
            }

            State::EncryptArchive => {
                self.output_engine_select.clear();
                self.output_engine_select.add(XP_ENGINE_LABEL);
                self.output_engine_select.add(VX_ENGINE_LABEL);
                self.output_engine_select.add(VXACE_ENGINE_LABEL);
                self.process_button.set_label(tr!("Encrypt"));
            }

            State::None => {}
        }

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

    /// Index of the selected line in the file list, which is also its index in the archive's or the map's entries.
    fn selected_index(&self) -> Option<usize> {
        usize::try_from(self.file_list.value()).ok()?.checked_sub(1)
    }

    /// The content of the file at `index`, decrypted if it is encrypted. The error is ready to show.
    ///
    /// Each file is decrypted with the key it carries: a key kept from an earlier file would be wrong for the next
    /// game, and for an encrypted file of another key within one.
    fn file_data(&self, index: usize) -> Result<Cow<'_, [u8]>, String> {
        if self.state.is_decrypt_archive() {
            let file = &self.archive_files[index];
            return Ok(Cow::Borrowed(&self.archive_data[file.data.clone()]));
        }

        let (path, file_type) = self.file_list_map.get_index(index).unwrap();

        let mut data = read(path).map_err(|err| {
            tr!("Reading file {} failed: {}")
                .replacen("{}", path, 1)
                .replacen("{}", &err.to_string(), 1)
        })?;

        let Some(file_type) = file_type else {
            return Ok(Cow::Owned(data));
        };

        decrypt_in_place(&mut data, *file_type).map_err(|err| {
            tr!("Decryption of file {} failed: {}")
                .replacen("{}", path, 1)
                .replacen("{}", &err.to_string(), 1)
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
                        alert_default(
                            &tr!("Unable to parse PNG data from the file: {}")
                                .replacen("{}", &err.to_string(), 1),
                        );
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
                self.update_display(DisplayMode::Message(
                    tr!("{}: Binary file contents can't be displayed.")
                        .replacen("{}", &extension, 1),
                ));
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
                        alert_default(
                            &tr!("Font parsing failed: {}")
                                .replacen("{}", err, 1),
                        );
                        return;
                    }
                };

                let text = "The quick brown fox jumps over the lazy dog";
                let px = 24.0;

                let Some(line_metrics) = font.horizontal_line_metrics(px)
                else {
                    alert_default(&tr!("Font parsing failed: {}").replacen(
                        "{}",
                        "no horizontal line metrics",
                        1,
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
                            let dy = (total_height as isize
                                - metrics.height as isize
                                + y as isize
                                + metrics.ymin as isize
                                - 2)
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

                // Fails for a font with nothing to draw, e.g. one without the glyphs of the text.
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
                        alert_default(
                            &tr!("Font parsing failed: {}").replacen(
                                "{}",
                                &err.to_string(),
                                1,
                            ),
                        );
                    }
                }
            }
            _ => {
                self.update_display(DisplayMode::Message(
                    tr!("{}: Unsupported file extension. If it's something \
                         that can be displayed, open an issue in our GitHub \
                         repository.")
                    .replacen("{}", &extension, 1),
                ));
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
                tr!("Probing format failed: {}").replacen(
                    "{}",
                    &err.to_string(),
                    1,
                )
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
                tr!("Decoding the format failed: {}").replacen(
                    "{}",
                    &err.to_string(),
                    1,
                )
            })?;

        // Not every stream says how long it is, or what it sounds like.
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

    /// The stream configuration to play a track of `sample_rate` and `channels` with.
    ///
    /// The samples are copied to the stream as they are, so it has to match the track: a different rate would play
    /// at the wrong speed, a different channel count would mix the channels up. The device's default is only the
    /// fallback for when it has no such configuration, the OS converts what the device doesn't take natively.
    fn output_config(
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
    ) -> Option<cpal::StreamConfig> {
        let matching = device
            .supported_output_configs()
            .ok()?
            .filter(|range| {
                range.channels() == channels
                    && range.sample_format() == cpal::SampleFormat::F32
            })
            .find_map(|range| range.try_with_sample_rate(sample_rate));

        matching
            .or_else(|| device.default_output_config().ok())
            .map(|config| config.config())
    }

    fn play_button_cb(&mut self, _: &mut Button) {
        if let Some(player) = &mut self.audio_player {
            let state = player.playback_state.load(Ordering::Acquire);

            if state == 2 {
                player.stop();
            } else if state == 1 {
                player.playback_state.store(0, Ordering::Release);
                return;
            }

            player.playback_state.store(0, Ordering::Release);
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
        } = match self.open_audio(index) {
            Ok(track) => track,
            Err(err) => {
                alert_default(&err);
                return;
            }
        };

        let Some(output_device) = cpal::default_host().default_output_device()
        else {
            alert_default(tr!("Getting default output device failed."));
            return;
        };

        let Some(config) =
            Self::output_config(&output_device, sample_rate, track_channels)
        else {
            alert_default(tr!("Getting default output device failed."));
            return;
        };

        let channels = config.channels as usize;

        let playback_state = Arc::new(AtomicU8::new(0));
        let seek_pos = Arc::new(AtomicU64::new(u64::MAX));
        let last_playback_second = Arc::new(AtomicU64::new(0));

        let mut pcm_buffer: Vec<f32> = Vec::new();
        let mut pcm_offset = 0;

        // The callback runs on the audio thread: it mustn't block or print, and once warmed up it doesn't allocate.
        let stream = match output_device.build_output_stream(
            config,
            {
                let state = playback_state.clone();
                let seek_pos = seek_pos.clone();
                let trackpos_sender = self.trackpos_sender;
                let trackend_sender = self.trackend_sender;

                move |data, _| {
                    let state_value = state.load(Ordering::Acquire);

                    if state_value == 1 {
                        data.fill(0f32);
                        return;
                    } else if state_value == 2 {
                        data.fill(0f32);
                        trackend_sender.send(true);
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
                                trackend_sender.send(true);
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

                            if second
                                != last_playback_second.load(Ordering::Acquire)
                            {
                                trackpos_sender.send(second);
                                last_playback_second
                                    .store(second, Ordering::Release);
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
                alert_default(
                    &tr!("Creating audio stream failed: {}").replacen(
                        "{}",
                        &err.to_string(),
                        1,
                    ),
                );
                return;
            }
        };

        if let Err(err) = stream.play() {
            alert_default(&tr!("Playing audio stream failed: {}").replacen(
                "{}",
                &err.to_string(),
                1,
            ));
            return;
        }

        self.audio_player = Some(AudioPlayer {
            stream: Some(stream),
            playback_state,
            seek_pos,
            duration,
            duration_string: Arc::new(format!(
                "{:02}:{:02}",
                duration.as_secs() / 60,
                duration.as_secs() % 60
            )),
        });

        self.trackpos_sender.send(0);
    }

    fn pause_button_cb(&mut self, _: &mut Button) {
        if let Some(player) = &self.audio_player {
            player.playback_state.store(1, Ordering::Release);
        }
    }

    fn stop_button_cb(&mut self, _: &mut Button) {
        self.stop_playback();
    }

    fn select_output_dir_button_cb(&mut self, _: &mut Button) {
        let Some(dir) = dir_chooser(
            tr!("Select Output Directory"),
            dirs::home_dir()
                .unwrap()
                .join("")
                .as_os_str()
                .to_str()
                .unwrap(),
            true,
        ) else {
            return;
        };

        self.output_dir_input.set_value(&dir);
        self.output_dir = dir;
    }

    fn progress_slider_handle(
        &mut self,
        this: &mut HorNiceSlider,
        event: Event,
    ) -> bool {
        let Some(player) = &self.audio_player else {
            return false;
        };

        match event {
            Event::Push => {
                self.progress_slider_locked = true;
                true
            }
            Event::Drag if self.progress_slider_locked => true,
            Event::Released => {
                self.progress_slider_locked = false;
                player
                    .seek_pos
                    .store(this.value() as u64, Ordering::Release);
                true
            }
            _ => false,
        }
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
        // Files dropped onto the field arrive as a paste of their paths, which would turn them into the output
        // directory. Dropped files and several folders are told from a pasted path by being files or more than one
        // line; a single folder dropped here is the output directory the user means.
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
            // FLTK inners will handle it
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

        let scaled_width =
            (self.image_frame.w() as f32 * self.image_scale_factor) as i32;
        let scaled_height =
            (self.image_frame.h() as f32 * self.image_scale_factor) as i32;

        push_clip(
            self.image_frame.x(),
            self.image_frame.y(),
            self.image_frame.width(),
            self.image_frame.height(),
        );

        let mut img: &mut SharedImage = image;

        // Redraws happen on every step of a drag. Scaling is what costs, so do it only when the factor changed.
        #[allow(clippy::float_cmp)]
        if self.image_scale_factor != 1.0 {
            if self
                .scaled_image
                .as_ref()
                .is_none_or(|(factor, _)| *factor != self.image_scale_factor)
            {
                let mut scaled_image = img.copy();
                scaled_image.scale(scaled_width, scaled_height, true, true);
                self.scaled_image =
                    Some((self.image_scale_factor, scaled_image));
            }

            if let Some((_, scaled_image)) = &mut self.scaled_image {
                img = scaled_image;
            }
        }

        img.draw(
            self.image_frame.x() + self.image_offset_x,
            self.image_frame.y() + self.image_offset_y,
            img.width(),
            img.height(),
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

                // Unbounded, zooming out ends in a zero-sized image and zooming in in a gigantic one.
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
        // A report can outlive its stream: it may arrive after Stop or after loading another track.
        let Some(audio_player) = self.audio_player.as_mut() else {
            return;
        };

        while let Some(playback_second) = self.trackpos_receiver.recv() {
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

        while let Some(track_end) = self.trackend_receiver.recv() {
            if track_end {
                audio_player.stop();
            }
        }
    }
}

fn main() {
    Application::run();
}
