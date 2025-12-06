#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::needless_doctest_main)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::deref_addrof)]
#![allow(clippy::cast_precision_loss)]
#![allow(static_mut_refs)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
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
    ArchiveEntry, Engine, VX_RGSS2A_EXT, VXACE_RGSS3A_EXT, XP_RGSSAD_EXT,
    decrypt_archive, encrypt_archive,
};
use rpgmasd::{
    DECRYPTED_ASSETS_EXTS, Decrypter, ENCRYPTED_ASSET_EXTS, FileType,
    HEADER_LENGTH, M4A_EXT, MV_M4A_EXT, MV_OGG_EXT, MV_PNG_EXT, MZ_M4A_EXT,
    MZ_OGG_EXT, MZ_PNG_EXT, OGG_EXT, PNG_EXT, decrypt_in_place, encrypt,
};
use std::{
    borrow::Cow,
    cell::RefCell,
    ffi::OsStr,
    fs::{create_dir_all, exists, metadata, read, write},
    io::Cursor,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::DecoderOptions,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
    units::Time,
};
use sys_locale::get_locale;
use time::OffsetDateTime;
use walkdir::WalkDir;

type IndexMapGx = IndexMap<String, Option<FileType>, GxBuildHasher>;

const WINDOW_WIDTH: i32 = 1024;
const WINDOW_HEIGHT: i32 = 720;

const MV_ENGINE_LABEL: &str = "MV";
const MZ_ENGINE_LABEL: &str = "MZ";
const XP_ENGINE_LABEL: &str = "XP";
const VX_ENGINE_LABEL: &str = "VX";
const VXACE_ENGINE_LABEL: &str = "VX Ace";

const LOGO_32PX: &[u8] = include_bytes!("../icons/32px.rgba");
const LOGO_256PX: &[u8] = include_bytes!("../icons/256px.rgba");

struct AudioPlayer {
    stream: Option<cpal::Stream>,
    state: Arc<AtomicU8>,
    seek_pos: Arc<AtomicU64>,
    duration: Time,
    duration_string: Arc<String>,
}

impl AudioPlayer {
    fn stop(&mut self) {
        self.state.store(2, Ordering::Release);
        self.stream.take();
    }
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
        WIDGET_MAP.with_borrow(|m| unsafe {
            &*std::ptr::from_ref::<str>(
                m.get(&(self.as_widget_ptr() as usize)).unwrap_or(&""),
            )
        })
    }
}

enum DisplayMode {
    Image(SharedImage),
    Audio((&'static str, &'static Option<FileType>)),
    Message(String),
}

// Quick & dirty translation scratch, only supports two languages.
// If more languages is about to be added to the app, that needs to be cleanly rewritten.
static mut TRANSLATION: Map<&'static str, &'static str> = phf_map! {};

const fn init_ru() {
    unsafe {
        TRANSLATION = phf_map! {
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
            "Writing file {} failed: {}" => "Чтение файла {} не удалось: {}",
            "Aborting decryption: {}" => "Прерываем расшифровку: ",
            "Parsing key from string failed: {}" => "Парсинг ключа из текста не удался: {}",
            "Aborting encryption: {}" => "Прерываем зашифровку: {}",
            "Decrypting file {} failed: {}" => "Расшифровка файла {} не удалась: {}",
            "No eligible files were found." => "Подходящие файлы не найдены.",
            "Font parsing failed: {}" => "Парсинг шрифта не удался: {}",
            "Probing format failed: {}" => "Проверка формата не удалась: {}",
            "Getting default output device failed." => "Не удалось получить стандартное устройство вывода.",
            "Creating audio stream failed: {}" => "Создание аудиопотока не удалось: {}",
            "Playing audio stream failed: {}" => "Проигрываниее аудиопотока не удалось: {}",

        }
    }
}

const fn init_en() {
    unsafe { TRANSLATION = phf_map! {} }
}

macro_rules! tr {
    ($k:expr) => {
        unsafe { TRANSLATION.get(&$k).map(|k| *k).unwrap_or($k) }
    };
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

#[derive(Clone, Copy, PartialEq)]
enum State {
    None,
    EncryptAsset,
    DecryptAsset,
    EncryptArchive,
    DecryptArchive,
}

enum Language {
    English,
    Russian,
}

struct Application {
    decrypted_archive_entries: Vec<ArchiveEntry>,
    encrypted_archive_extension: String,
    file_list_map: IndexMapGx,
    audio_player: Option<AudioPlayer>,
    decrypter: Decrypter,
    output_dir: String,
    image_offset_x: i32,
    image_offset_y: i32,
    last_mouse_pos: Option<(i32, i32)>,
    current_image: Option<SharedImage>,
    image_scale_factor: f32,
    progress_slider_locked: bool,
    state: State,
    language: Language,

    app: App,
    window: Window,
    trackpos_sender: Sender<u64>,
    trackpos_receiver: Receiver<u64>,

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
    fn retranslate_widgets(root: &Group) {
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

        Self::retranslate_widgets(&self.window.as_group().unwrap());

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
        let decrypter = Decrypter::new();
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

        let mut image_frame = Frame::default_fill().with_tr_id("Drop files/folders into the window or use `Add File`/`Add Folder` in the `File` menu.");
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
            decrypted_archive_entries: Vec::new(),
            encrypted_archive_extension: String::new(),
            state: State::None,
            language: Language::English,
            file_list_map,
            audio_player,
            decrypter,
            output_dir,
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
        app.app.run().expect("Application failed to run");
    }

    fn update_display(&mut self, mode: DisplayMode) {
        match mode {
            DisplayMode::Image(image) => {
                self.stop_playback();

                self.current_image = Some(image);

                self.image_frame.set_label("");
                self.image_frame.show();
                self.image_frame.redraw();

                self.audio_controls.hide();
                self.audio_controls.redraw();
            }

            DisplayMode::Audio((path, file_type)) => {
                self.current_image = None;

                self.image_frame.set_label("");
                self.image_frame.hide();
                self.image_frame.redraw();

                let data = match read(path) {
                    Ok(data) => data,
                    Err(err) => {
                        alert_default(
                            &tr!("Reading file {} failed: {}")
                                .replacen("{}", path, 1)
                                .replacen("{}", &err.to_string(), 1),
                        );
                        return;
                    }
                };

                let audio_data = if let Some(file_type) = file_type {
                    match self.decrypter.decrypt(&data, *file_type) {
                        Ok(audio_data) => audio_data,
                        Err(err) => {
                            alert_default(
                                &tr!("Decryption of file {} failed: {}")
                                    .replacen("{}", path, 1)
                                    .replacen("{}", &err.to_string(), 1),
                            );
                            return;
                        }
                    }
                } else {
                    data
                };

                let format = match Self::open_track(audio_data) {
                    Ok(format) => format,
                    Err(err) => {
                        alert_default(&err);
                        return;
                    }
                };

                let Some(track) = format.default_track() else {
                    alert_default(tr!("Unable to found playable track"));
                    return;
                };

                let decoder_options = DecoderOptions::default();
                let decoder =
                    match symphonia::default::get_codecs()
                        .make(&track.codec_params, &decoder_options)
                    {
                        Ok(decoder) => decoder,
                        Err(err) => {
                            alert_default(
                                &tr!("Decoding the format failed: {}")
                                    .replacen("{}", &err.to_string(), 1),
                            );
                            return;
                        }
                    };

                let codec_parameters = decoder.codec_params();
                let time_base = codec_parameters.time_base.unwrap();
                let duration =
                    time_base.calc_time(codec_parameters.n_frames.unwrap());

                self.progress_label.set_label(&format!(
                    "00:00 / {:02}:{:02}",
                    duration.seconds / 60,
                    duration.seconds % 60
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
        self.decrypted_archive_entries.clear();
        self.encrypted_archive_extension = String::new();
        self.output_engine_select.clear();
        self.output_engine_select
            .set_value(tr!("-Select engine for encryption-"));
        self.process_button.hide();
        self.process_button.redraw();
        self.button_layout.layout();
        self.update_display(DisplayMode::Message(tr!("Drop files/folders into the window or use `Add File`/`Add Folder` in the `File` menu.").to_string()));
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
        for item in self.file_list.selected_items().into_iter().rev() {
            if self.state == State::DecryptArchive {
                self.decrypted_archive_entries.remove(item as usize - 1);
            } else {
                self.file_list_map.shift_remove_index(item as usize - 1);
                self.file_list.remove(item);
            }
        }

        self.update_display(DisplayMode::Message(String::new()));
    }

    fn add_menubar_entries(&mut self) {
        let mut i = 0;

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.menu_bar.add(tr!(MENU_BAR_ITEMS[i]), Shortcut::Ctrl | 'o', MenuFlag::Normal, move |_| {
            let Some(file) = file_chooser(tr!("Select File"), tr!("Encrypted Assets/Archives (*.{rpgmvp,rpgmvo,rpgmvm,png_,ogg_,m4a_,rgssad,rgss2a,rgss3a})\t"), dirs::home_dir().unwrap().join(""), true) else {
                return;
            };

            mut_self.parse_files(&file);
        });
        i += 1;

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
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

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                mut_self.retranslate(Some("ru"));
            },
        );
        i += 1;

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.menu_bar.add(
            tr!(MENU_BAR_ITEMS[i]),
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                mut_self.retranslate(Some("en"));
            },
        );
        i += 1;

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
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

    fn set_callbacks(&mut self) {
        self.add_menubar_entries();

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.clear_button
            .set_callback(|this| mut_self.clear_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.remove_button
            .set_callback(|this| mut_self.remove_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.process_button.set_callback(move |this| {
            let date = OffsetDateTime::now_utc();
            let output_dir = Path::new(&mut_self.output_dir).join(format!(
                "{}-{}_{}-{:02}-{:02}_{:02}-{:02}-{:02}",
                if mut_self.decrypted_archive_entries.is_empty() {
                    "www"
                } else {
                    &mut_self.encrypted_archive_extension
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

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.window
            .handle(move |_, event| mut_self.window_handle(event));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.file_list
            .set_callback(move |this| mut_self.file_list_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.play_button
            .set_callback(move |this| mut_self.play_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.pause_button
            .set_callback(move |this| mut_self.pause_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.stop_button
            .set_callback(move |this| mut_self.stop_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.select_output_dir_button.set_callback(move |this| {
            mut_self.select_output_dir_button_cb(this);
        });

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.progress_slider.handle(move |this, event| {
            mut_self.progress_slider_handle(this, event)
        });

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.select_all_button
            .set_callback(move |this| mut_self.select_all_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.deselect_all_button
            .set_callback(move |this| mut_self.deselect_all_button_cb(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.output_dir_input.handle(move |this, event| {
            mut_self.output_dir_input_handle(this, event)
        });

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.image_frame
            .draw(move |this| mut_self.image_frame_draw(this));

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        self.image_frame.handle(move |this, event| {
            mut_self.image_frame_handle(this, event)
        });

        let mut_self = unsafe { &mut *std::ptr::from_mut::<Self>(self) };
        app::add_idle3(move |handle| mut_self.idle_cb(handle));
    }

    fn decrypt_archive(&self, decrypted_dir: &Path) {
        let decrypted_dir = Arc::new(decrypted_dir);
        let decrypted_archive_entries =
            Arc::new(&self.decrypted_archive_entries);

        let result = self
            .file_list
            .selected_items()
            .into_par_iter()
            .try_for_each(move |item| {
                let index = item as usize - 1;
                let entry = &decrypted_archive_entries[index];

                let output_path = decrypted_dir.join(
                    String::from_utf8_lossy(entry.path.as_ref()).into_owned(),
                );

                let parent_dir = output_path.parent().unwrap();

                if let Err(err) = create_dir_all(parent_dir) {
                    return Err(tr!("Creating directory {} failed: {}")
                        .replacen("{}", &parent_dir.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                if let Err(err) = write(&output_path, &entry.data) {
                    return Err(tr!("Writing file {} failed: {}")
                        .replacen("{}", &output_path.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                Ok(())
            });

        if let Err(err) = result {
            alert_default(
                &tr!("Aborting decryption: {}").replacen("{}", &err, 1),
            );
        }
    }

    fn decrypt_assets(&self, decrypted_dir: &Path) {
        let file_list_map = Arc::new(&self.file_list_map);
        let decrypted_dir = Arc::new(decrypted_dir);

        let result = self
            .file_list
            .selected_items()
            .into_par_iter()
            .try_for_each(move |item| {
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

                let decrypted_data = &encrypted_data[HEADER_LENGTH..];

                let path = Path::new(path);
                let path_components = path.components();
                let mut relative_path = PathBuf::default();
                let mut collecting_relative_path = false;

                for component in path_components {
                    if component.as_os_str() == "www" {
                        collecting_relative_path = true;
                        relative_path = PathBuf::default();
                    }

                    if collecting_relative_path {
                        relative_path.push(component.as_os_str());
                    }
                }

                let output_path = Path::new(decrypted_dir.as_ref())
                    .join(relative_path.with_extension(file_type.to_string()));

                let parent_dir = output_path.parent().unwrap();

                if let Err(err) = create_dir_all(parent_dir) {
                    return Err(tr!("Creating directory {} failed: {}")
                        .replacen("{}", &parent_dir.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                if let Err(err) = write(&output_path, decrypted_data) {
                    return Err(tr!("Writing file {} failed: {}")
                        .replacen("{}", &output_path.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                Ok(())
            });

        if let Err(err) = result {
            alert_default(
                &tr!("Aborting decryption: {}").replacen("{}", &err, 1),
            );
        }
    }

    fn decrypt_cb(&mut self, _: &mut Button, decrypted_dir: &Path) {
        match self.state {
            State::DecryptArchive | State::DecryptAsset => {
                if !self.output_dir_valid() {
                    return;
                }

                match self.state {
                    State::DecryptArchive => {
                        self.decrypt_archive(decrypted_dir);
                    }

                    State::DecryptAsset => {
                        self.decrypt_assets(decrypted_dir);
                    }

                    _ => unreachable!(),
                }

                message_default(
                    &tr!(
                        "Decryption ended. Decrypted entries are put to the {}"
                    )
                    .replacen(
                        "{}",
                        &decrypted_dir.display().to_string(),
                        1,
                    ),
                );
            }

            _ => unreachable!(),
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

    fn encrypt_assets(&mut self, encrypted_dir: &Path, engine: &str) {
        let choice = choice2_default(
            tr!(
                "Asset encryption requires a key. Set it manually or from file?"
            ),
            tr!("Manually"),
            tr!("From File"),
            "",
        );
        let Some(choice) = choice else { return };

        let encryption_key = if choice == 0 {
            let Some(key) = input_default(tr!("Enter the encryption key"), "")
            else {
                alert_default(tr!("No encryption key entered."));
                return;
            };

            if let Err(err) = self.decrypter.set_key_from_str(&key) {
                alert_default(
                    &tr!("Parsing key from string failed: {}").replacen(
                        "{}",
                        &err.to_string(),
                        1,
                    ),
                );
                return;
            }

            key
        } else {
            let Some(file_path) = file_chooser(
                tr!("Select Encrypted File"),
                "*",
                dirs::home_dir().unwrap().join(""),
                true,
            ) else {
                return;
            };

            let Some(extension) = Path::new(&file_path).extension() else {
                alert_default(tr!("Passed path is not a file."));
                return;
            };

            let file_type = match FileType::try_from(extension) {
                Ok(file_type) => file_type,
                Err(err) => {
                    alert_default(err);
                    return;
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
                    return;
                }
            };

            let key = match self.decrypter.set_key_from_file(&file, file_type) {
                Ok(key) => key,
                Err(err) => {
                    alert_default(&err.to_string());
                    return;
                }
            };

            key.to_string()
        };

        let file_list_map = Arc::new(&self.file_list_map);
        let output_dir = Arc::new(encrypted_dir);
        let encryption_key = Arc::new(&encryption_key);
        let engine = Arc::new(engine);

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

                let decrypted_data = match read(path) {
                    Ok(decrypted_data) => decrypted_data,
                    Err(err) => {
                        return Err(tr!("Reading file {} failed: {}")
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1));
                    }
                };

                let encrypted_data =
                    match encrypt(&decrypted_data, &encryption_key) {
                        Ok(encrypted_data) => encrypted_data,
                        Err(err) => {
                            return Err(tr!(
                                "Encryption of file {} failed: {}"
                            )
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1));
                        }
                    };

                let path = Path::new(path);
                let path_components = path.components();
                let mut relative_path = PathBuf::default();
                let mut collecting_relative_path = false;

                for component in path_components {
                    if component.as_os_str() == "www" {
                        collecting_relative_path = true;
                        relative_path = PathBuf::default();
                    }

                    if collecting_relative_path {
                        relative_path.push(component.as_os_str());
                    }
                }

                let input_ext =
                    relative_path.extension().and_then(OsStr::to_str).unwrap();

                let output_ext = match (input_ext, *engine) {
                    (PNG_EXT, MV_ENGINE_LABEL) => MV_PNG_EXT,
                    (PNG_EXT, MZ_ENGINE_LABEL) => MZ_PNG_EXT,
                    (OGG_EXT, MV_ENGINE_LABEL) => MV_OGG_EXT,
                    (OGG_EXT, MZ_ENGINE_LABEL) => MZ_OGG_EXT,
                    (M4A_EXT, MV_ENGINE_LABEL) => MV_M4A_EXT,
                    (M4A_EXT, MZ_ENGINE_LABEL) => MZ_M4A_EXT,
                    _ => unreachable!(),
                };

                let output_path = Path::new(output_dir.as_ref())
                    .join(relative_path)
                    .with_extension(output_ext);

                let parent_dir = output_path.parent().unwrap();

                if let Err(err) = create_dir_all(parent_dir) {
                    return Err(tr!("Creating directory {} failed: {}")
                        .replacen("{}", &parent_dir.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                if let Err(err) = write(&output_path, encrypted_data) {
                    return Err(tr!("Writing file {} failed: {}")
                        .replacen("{}", &output_path.to_string_lossy(), 1)
                        .replacen("{}", &err.to_string(), 1));
                }

                Ok(())
            });

        if let Err(err) = result {
            alert_default(
                &tr!("Aborting decryption: {}").replacen("{}", &err, 1),
            );
        }
    }

    fn encrypt_archive(&mut self, encrypted_dir: &Path, engine: &str) {
        let file_list_map = Arc::new(&self.file_list_map);

        let archive_entries: Result<Vec<ArchiveEntry>, String> = self
            .file_list
            .selected_items()
            .into_par_iter()
            .map(|item| {
                let index = item as usize - 1;
                let (path, _) = file_list_map.get_index(index).unwrap();

                let mut relative_path = PathBuf::default();
                let mut collecting_path = false;

                for component in Path::new(path).components() {
                    let component_str = component.as_os_str();

                    for candidate in ["Audio", "Graphics", "Data", "Fonts"] {
                        if component_str == candidate {
                            collecting_path = true;
                            break;
                        }
                    }

                    if collecting_path {
                        relative_path.push(component_str.to_str().unwrap());
                    }
                }

                let data = match read(path) {
                    Ok(data) => data,
                    Err(err) => {
                        return Err(tr!("Reading file {} failed: {}")
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1));
                    }
                };

                Ok(ArchiveEntry {
                    path: Cow::Owned(
                        relative_path.into_os_string().into_encoded_bytes(),
                    ),
                    data,
                })
            })
            .collect();

        let archive_entries = match archive_entries {
            Ok(entries) => entries,
            Err(err) => {
                alert_default(
                    &tr!("Aborting encryption: {}").replacen("{}", &err, 1),
                );
                return;
            }
        };

        let encrypted = encrypt_archive(
            &archive_entries,
            match engine {
                VXACE_ENGINE_LABEL => Engine::VXAce,
                _ => Engine::Older,
            },
        );

        let output_path = Path::new(encrypted_dir).join("Game").with_extension(
            match engine {
                XP_ENGINE_LABEL => XP_RGSSAD_EXT,
                VX_ENGINE_LABEL => VX_RGSS2A_EXT,
                VXACE_ENGINE_LABEL => VXACE_RGSS3A_EXT,
                _ => unreachable!(),
            },
        );

        if let Err(err) = create_dir_all(encrypted_dir) {
            alert_default(
                &tr!("Creating directory {} failed: {}")
                    .replacen("{}", &encrypted_dir.to_string_lossy(), 1)
                    .replacen("{}", &err.to_string(), 1),
            );
            return;
        }

        if let Err(err) = write(&output_path, encrypted) {
            alert_default(
                &tr!("Writing file {} failed: {}")
                    .replacen("{}", &output_path.to_string_lossy(), 1)
                    .replacen("{}", &err.to_string(), 1),
            );
        }
    }

    fn encrypt_cb(&mut self, _: &mut Button, encrypted_dir: &Path) {
        match self.state {
            State::EncryptAsset | State::EncryptArchive => {
                if !self.output_dir_valid() {
                    return;
                }

                if self.output_engine_select.menu_button().value() == -1 {
                    alert_default(tr!("Output engine not specified."));
                    return;
                }

                let engine = self.output_engine_select.value().unwrap();

                match self.state {
                    State::EncryptAsset => {
                        self.encrypt_assets(encrypted_dir, &engine);
                    }

                    State::EncryptArchive => {
                        self.encrypt_archive(encrypted_dir, &engine);
                    }

                    _ => unreachable!(),
                }

                message_default(
                    &tr!(
                        "Encryption ended. Encrypted entries are put to the {}"
                    )
                    .replacen(
                        "{}",
                        &encrypted_dir.display().to_string(),
                        1,
                    ),
                );
            }

            _ => unreachable!(),
        }
    }

    fn parse_archive(&mut self, path: &Path) {
        self.file_list.clear();
        let path_str = &path.to_string_lossy();

        let archive_data = match read(path) {
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

        let decrypted_archive_entries = match decrypt_archive(&archive_data) {
            Ok(decrypted_archive_entries) => decrypted_archive_entries,
            Err(err) => {
                alert_default(
                    &tr!("Decrypting file {} failed: {}")
                        .replacen("{}", path_str, 1)
                        .replacen("{}", &err.to_string(), 1),
                );
                return;
            }
        };

        self.decrypted_archive_entries = decrypted_archive_entries;
        self.encrypted_archive_extension = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap()
            .to_string();

        for entry in &self.decrypted_archive_entries {
            let path = String::from_utf8_lossy(entry.path.as_ref());
            self.file_list.add(&path);
        }
    }

    fn parse_asset(
        &mut self,
        file_path: &Path,
        required_component: &mut Option<String>,
    ) -> ControlFlow<String> {
        let extension = file_path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);

        let Some(ext) = extension else {
            return ControlFlow::Continue(());
        };

        let ext = ext.as_str();

        let is_allowed_extension = ENCRYPTED_ASSET_EXTS.contains(&ext)
            || DECRYPTED_ASSETS_EXTS.contains(&ext)
            || matches!(ext, "ttf" | "otf" | "rxdata" | "rvdata" | "rvdata2");

        let entry_already_exists =
            self.file_list_map.contains_key(file_path.to_str().unwrap());

        if !is_allowed_extension || entry_already_exists {
            return ControlFlow::Continue(());
        }

        let mut found_component: Option<&str> = None;

        if DECRYPTED_ASSETS_EXTS.contains(&ext) {
            let mut path_components = file_path.components().rev();

            if path_components.any(|c| c.as_os_str() == "www") {
                found_component = Some("www");
            } else {
                for candidate in ["Audio", "Graphics", "Data", "Fonts"] {
                    let mut path_components = file_path.components().rev();

                    if path_components.any(|c| c.as_os_str() == candidate) {
                        found_component = Some(candidate);
                        break;
                    }
                }
            }

            if found_component.is_none() {
                return ControlFlow::Break(tr!("Unable to determine the type of the passed assets from the passed path {}. If you want to encrypt an archive, your assets should be arranged in `Audio`, `Graphics`, `Data` or/and `Fonts` directories. If you want to encrypt assets, the path the the asset should contain `www` directory.").replacen("{}", &file_path.display().to_string(), 1));
            }

            if let Some(component) = required_component {
                if component == "www" {
                    if component != found_component.unwrap() {
                        return ControlFlow::Break(tr!("Component mismatch when parsing files. Detected `www` directory for asset encryption, but it's missing in the path {}.").replacen("{}", &file_path.display().to_string(), 1));
                    }
                } else {
                    let mut is_required_component = false;

                    for candidate in ["Audio", "Graphics", "Data", "Fonts"] {
                        if component == candidate {
                            is_required_component = true;
                        }
                    }

                    if !is_required_component {
                        return ControlFlow::Break(tr!("Component mismatch when parsing files. Detected Audio/Graphics/Data/Fonts directories for archive encryption, but it's missing in the path {}.").replacen("{}", &file_path.display().to_string(), 1));
                    }
                }
            } else {
                *required_component =
                    Some(found_component.unwrap().to_string());
            }
        }

        let filename = file_path.file_name().and_then(OsStr::to_str).unwrap();
        let file_type = FileType::try_from(ext).ok();

        self.file_list.add(filename);
        self.file_list_map
            .insert(file_path.to_string_lossy().to_string(), file_type);

        ControlFlow::Continue(())
    }

    fn parse_assets(&mut self, paths: &str) {
        self.clear();

        let mut required_component: Option<String> = None;

        for path_str in paths.lines() {
            let path = Path::new(path_str);

            if path.is_dir() {
                for entry in WalkDir::new(path)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                {
                    let file_path = entry.path();

                    if let ControlFlow::Break(err) =
                        self.parse_asset(file_path, &mut required_component)
                    {
                        alert_default(&err);
                        self.clear();
                        return;
                    }
                }

                continue;
            }

            if let ControlFlow::Break(err) =
                self.parse_asset(path, &mut required_component)
            {
                alert_default(&err);
                self.clear();
                return;
            }
        }

        if self.file_list.size() == 0 {
            message_default(tr!("No eligible files were found."));
        }

        if let Some(required_component) = required_component {
            self.output_engine_select.clear();

            if required_component == "www" {
                self.state = State::EncryptAsset;

                self.output_engine_select.add(MV_ENGINE_LABEL);
                self.output_engine_select.add(MZ_ENGINE_LABEL);
            } else {
                self.state = State::EncryptArchive;

                self.output_engine_select.add(XP_ENGINE_LABEL);
                self.output_engine_select.add(VX_ENGINE_LABEL);
                self.output_engine_select.add(VXACE_ENGINE_LABEL);
            }

            self.process_button.set_label(tr!("Encrypt"));
        } else {
            self.state = State::DecryptAsset;
            self.process_button.set_label(tr!("Decrypt"));
        }

        self.process_button.show();
        self.process_button.redraw();

        self.button_layout.layout();
    }

    fn parse_files(&mut self, paths: &str) {
        for path_str in paths.lines() {
            let path = Path::new(path_str);
            if let Some(ext) = path.extension()
                && (ext == XP_RGSSAD_EXT
                    || ext == VX_RGSS2A_EXT
                    || ext == VXACE_RGSS3A_EXT)
            {
                self.parse_archive(path);

                self.state = State::DecryptArchive;

                self.process_button.set_label(tr!("Decrypt"));

                self.process_button.show();
                self.process_button.redraw();

                self.button_layout.layout();

                return;
            }
        }

        self.parse_assets(paths);
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

    fn file_list_cb(&mut self, this: &mut MultiBrowser) {
        let Some(filename) = this.selected_text() else {
            return;
        };

        let index = this.value() as usize - 1;
        let extension =
            filename.rsplit_once('.').unwrap().1.to_ascii_lowercase();

        self.stop_playback();

        match extension.as_str() {
            MV_PNG_EXT | MZ_PNG_EXT | PNG_EXT => {
                let mut encrypted_data: Vec<u8>;

                let png_slice = if self.decrypted_archive_entries.is_empty() {
                    let (path, file_type) =
                        self.file_list_map.get_index(index).unwrap();

                    encrypted_data = match read(path) {
                        Ok(encrypted_data) => encrypted_data,
                        Err(err) => {
                            alert_default(
                                &tr!("Reading file {} failed: {}")
                                    .replacen("{}", path, 1)
                                    .replacen("{}", &err.to_string(), 1),
                            );
                            return;
                        }
                    };

                    if extension == PNG_EXT {
                        &encrypted_data
                    } else {
                        match self.decrypter.decrypt_in_place(
                            &mut encrypted_data,
                            file_type.unwrap(),
                        ) {
                            Ok(slice) => slice,
                            Err(err) => {
                                alert_default(
                                    &tr!("Decryption of file {} failed: {}")
                                        .replacen("{}", path, 1)
                                        .replacen("{}", &err.to_string(), 1),
                                );
                                return;
                            }
                        }
                    }
                } else {
                    &self.decrypted_archive_entries[index].data
                };

                let image = match PngImage::from_data(png_slice) {
                    Ok(image) => image,
                    Err(err) => {
                        alert_default(
                            &tr!("Unable to parse PNG data from the file: {}")
                                .replacen("{}", &err.to_string(), 1),
                        );
                        return;
                    }
                };

                self.update_display(DisplayMode::Image(
                    SharedImage::from_image(&image).unwrap(),
                ));
            }
            MV_OGG_EXT | MZ_OGG_EXT | MV_M4A_EXT | MZ_M4A_EXT | OGG_EXT
            | M4A_EXT => {
                let (path, file_type) =
                    self.file_list_map.get_index(index).unwrap();

                self.update_display(DisplayMode::Audio((
                    unsafe { &*std::ptr::from_ref::<String>(path) },
                    unsafe {
                        &*std::ptr::from_ref::<Option<FileType>>(file_type)
                    },
                )));
            }
            "rxdata" | "rvdata" | "rvdata2" => {
                self.update_display(DisplayMode::Message(
                    tr!("{}: Binary file contents can't be displayed.")
                        .replacen("{}", &extension, 1),
                ));
            }
            "ttf" | "otf" => {
                let font_data: Vec<u8>;

                let font_data = if self.state == State::DecryptArchive {
                    &self.decrypted_archive_entries[index].data
                } else {
                    let (path, _) =
                        self.file_list_map.get_index(index).unwrap();

                    font_data = match read(path) {
                        Ok(font_data) => font_data,
                        Err(err) => {
                            alert_default(
                                &tr!("Reading file {} failed: {}")
                                    .replacen("{}", path, 1)
                                    .replacen("{}", &err.to_string(), 1),
                            );
                            return;
                        }
                    };

                    &font_data
                };

                let font = match Font::from_bytes(
                    font_data.as_ref(),
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

                let metrics = font.horizontal_line_metrics(px).unwrap();
                let mut total_width = 0;

                for c in text.chars() {
                    let (m, _) = font.rasterize(c, px);
                    total_width += m.advance_width as usize;
                }

                let ascent = metrics.ascent.ceil() as isize;
                let descent = metrics.descent.floor() as isize;
                let total_height = (ascent - descent) as usize;

                let mut rgb_buf = vec![0u8; total_width * total_height * 3];
                let mut x_offset = 0;

                for ch in text.chars() {
                    let (metrics, bitmap) = font.rasterize(ch, px);

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

                self.update_display(DisplayMode::Image(
                    SharedImage::from_image(
                        &RgbImage::new(
                            &rgb_buf,
                            total_width as i32,
                            total_height as i32,
                            ColorDepth::Rgb8,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ));
            }
            _ => {
                self.update_display(DisplayMode::Message(tr!("{}: Unsupported file extension. If it's something that can be displayed, open an issue in our GitHub repository.").replacen("{}", &extension, 1)));
            }
        }
    }

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

        let probed = match symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) {
            Ok(probed) => probed,
            Err(err) => {
                return Err(tr!("Probing format failed: {}").replacen(
                    "{}",
                    &err.to_string(),
                    1,
                ));
            }
        };

        let format = probed.format;
        Ok(format)
    }

    fn play_button_cb(&mut self, _: &mut Button) {
        if let Some(player) = &mut self.audio_player {
            let state = player.state.load(Ordering::Acquire);

            if state == 2 {
                player.stop();
            } else if state == 1 {
                player.state.store(0, Ordering::Release);
                return;
            }

            player.state.store(0, Ordering::Release);
        }

        let index = self.file_list.value() as usize - 1;
        let (path, file_type) = self.file_list_map.get_index(index).unwrap();

        let encrypted_data = match read(path) {
            Ok(encrypted_data) => encrypted_data,
            Err(err) => {
                alert_default(
                    &tr!("Reading file {} failed: {}")
                        .replacen("{}", path, 1)
                        .replacen("{}", &err.to_string(), 1),
                );
                return;
            }
        };

        let audio_data = if let Some(file_type) = file_type {
            match self.decrypter.decrypt(&encrypted_data, *file_type) {
                Ok(audio_data) => audio_data,
                Err(err) => {
                    alert_default(
                        &tr!("Decryption of file {} failed: {}")
                            .replacen("{}", path, 1)
                            .replacen("{}", &err.to_string(), 1),
                    );
                    return;
                }
            }
        } else {
            encrypted_data
        };

        let mut format = match Self::open_track(audio_data) {
            Ok(format) => format,
            Err(err) => {
                alert_default(&err);
                return;
            }
        };

        let Some(track) = format.default_track() else {
            alert_default(tr!("Unable to found playable track"));
            return;
        };

        let decoder_options = DecoderOptions::default();
        let mut decoder = match symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_options)
        {
            Ok(decoder) => decoder,
            Err(err) => {
                alert_default(&tr!("Decoding the format failed: {}").replacen(
                    "{}",
                    &err.to_string(),
                    1,
                ));
                return;
            }
        };

        let track_id = track.id;

        let codec_parameters = decoder.codec_params();
        let time_base = codec_parameters.time_base.unwrap();
        let duration = time_base.calc_time(codec_parameters.n_frames.unwrap());

        let Some(output_device) = cpal::default_host().default_output_device()
        else {
            alert_default(tr!("Getting default output device failed."));
            return;
        };

        let config = output_device.default_output_config().unwrap().config();
        let channels = config.channels as usize;

        let state = Arc::new(AtomicU8::new(0));
        let seek_pos = Arc::new(AtomicU64::new(u64::MAX));
        let last_playback_second = Arc::new(AtomicU64::new(0));

        let mut pcm_buffer = Vec::new();
        let mut pcm_offset = 0;

        let stream = match output_device.build_output_stream(
            &config,
            {
                let state = state.clone();
                let seek_pos = seek_pos.clone();
                let sender = self.trackpos_sender;

                move |data, _| {
                    if state.load(Ordering::Acquire) == 1 {
                        data.fill(0f32);
                        return;
                    }

                    let needed_frames = data.len() / channels;
                    let mut written_frames = 0;

                    let seek_second = seek_pos.load(Ordering::Acquire);

                    if seek_second != u64::MAX {
                        let _ = format.seek(
                            SeekMode::Coarse,
                            SeekTo::Time {
                                time: Time::new(seek_second, 0.0),
                                track_id: Some(track_id),
                            },
                        );

                        seek_pos.store(u64::MAX, Ordering::Release);
                    }

                    while written_frames < needed_frames {
                        if pcm_offset >= pcm_buffer.len() {
                            let packet = match format.next_packet() {
                                Ok(packet) => packet,
                                Err(err) => {
                                    println!("Packets ended: {err}.");
                                    break;
                                }
                            };

                            if packet.track_id() != track_id {
                                pcm_buffer.clear();
                                pcm_offset = 0;
                                break;
                            }

                            let decoded_buf = match decoder.decode(&packet) {
                                Ok(decoded_buf) => decoded_buf,
                                Err(err) => {
                                    println!("Decoding failed: {err}.");
                                    break;
                                }
                            };

                            let mut sample_buf = SampleBuffer::<f32>::new(
                                decoded_buf.capacity() as u64,
                                *decoded_buf.spec(),
                            );
                            sample_buf.copy_interleaved_ref(decoded_buf);
                            pcm_buffer = sample_buf.samples().to_vec();
                            pcm_offset = 0;

                            let pos = time_base.calc_time(packet.ts());

                            if pos.seconds
                                != last_playback_second.load(Ordering::Acquire)
                            {
                                sender.send(pos.seconds);
                                last_playback_second
                                    .store(pos.seconds, Ordering::Release);
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
            state,
            seek_pos,
            duration,
            duration_string: Arc::new(format!(
                "{:02}:{:02}",
                duration.seconds / 60,
                duration.seconds % 60
            )),
        });

        self.trackpos_sender.send(0);
    }

    fn pause_button_cb(&mut self, _: &mut Button) {
        if let Some(player) = &self.audio_player {
            player.state.store(1, Ordering::Release);
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
        this: &mut FileInput,
        event: Event,
    ) -> bool {
        match event {
            Event::Paste => {
                this.set_value(&self.output_dir);
                true
            }
            _ => false,
        }
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
        let mut scaled_image: SharedImage;

        #[allow(clippy::float_cmp)]
        if self.image_scale_factor != 1.0 {
            scaled_image = img.copy();
            scaled_image.scale(scaled_width, scaled_height, true, true);
            img = &mut scaled_image;
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

    fn idle_cb(&mut self, _: app::TimeoutHandle) {
        if let Some(playback_second) = self.trackpos_receiver.recv() {
            let audio_player = self.audio_player.as_ref().unwrap();

            self.progress_label.set_label(&format!(
                "{:02}:{:02} / {}",
                (playback_second as u32) / 60,
                (playback_second as u32) % 60,
                audio_player.duration_string
            ));

            if !self.progress_slider_locked {
                self.progress_slider
                    .set_range(0f64, audio_player.duration.seconds as f64);
                self.progress_slider.set_value(playback_second as f64);
            }
        }
    }
}

fn main() {
    Application::run();
}
