#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::{Deserialize, Serialize};
use serde_json;
use std::cell::RefCell;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;

use slint::{Model, ModelRc, SharedString, VecModel};

use gymbooking::*;

slint::include_modules!();

// TODO: Logger: Add dump to file method
// TODO: Logger: Add clear method
// TODO: Add a button to clear the log
// TODO: Save log to tmp file.
// TODO: Add a button to open the log file
// TODO: Save last 10 used student numbers and create a drop down menu for the student number input field

const BOOKED_SOUNDS: [&[u8]; 2] = [
    include_bytes!("../assets/kapakoulak_booked.mp3"),
    include_bytes!("../assets/Ronnie_booked.mp3"),
];

const APPNAME: &str = "Gymbooking";

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
enum Background {
    Grizzly = 0,
    Ronnie = 1,
}

#[derive(Serialize, Deserialize, Debug)]
struct Settings {
    backround: Background,
    audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backround: Background::Grizzly,
            audio: false,
        }
    }
}

impl Settings {
    fn new() -> Self {
        Settings::default()
    }

    fn save(&self) -> Result<(), String> {
        let mut path = find_or_create_config_dir()?;
        path.push("settings.json");
        let settings_str =
            serde_json::to_string(self).expect("Could not serialize Settings struct");
        std::fs::write(path, settings_str)
            .map_err(|_| "Could not create settings.json file.".to_string())
    }

    fn load_from_config(&mut self) -> Result<(), String> {
        let mut path = find_or_create_config_dir()?;
        path.push("settings.json");
        if !path.exists() {
            return Ok(());
        }
        let settings_str = fs::read_to_string(path).map_err(|_| "Could not read settings.json")?;
        *self = serde_json::from_str(&settings_str)
            .map_err(|_| "Could not deserialize settings.json")?;
        Ok(())
    }

    fn set_background(&mut self, background: &str) {
        self.backround = match background {
            "Grizzly" => Background::Grizzly,
            "Ronnie" => Background::Ronnie,
            _ => Background::Grizzly,
        };
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BookedData {
    student_number: String,
}

impl BookedData {
    fn new() -> Self {
        Self {
            student_number: String::new(),
        }
    }

    fn save(&self) -> Result<(), String> {
        let mut path = find_or_create_config_dir()?;
        path.push("cache.json");
        let booked_str =
            serde_json::to_string(self).expect("Could not serialize BookedData struct");
        std::fs::write(path, booked_str)
            .map_err(|_| "Could not create booked.json file.".to_string())
    }

    fn load_from_config(&mut self) -> Result<(), String> {
        let mut path = find_or_create_config_dir()?;
        path.push("cache.json");
        if !path.exists() {
            return Ok(());
        }
        let booked_str = fs::read_to_string(path).map_err(|_| "Could not read booked.json")?;
        *self =
            serde_json::from_str(&booked_str).map_err(|_| "Could not deserialize booked.json")?;
        Ok(())
    }
}

#[derive(Clone)]
struct Logger {
    shared_output: Arc<Mutex<String>>,
}

impl Logger {
    pub fn new() -> Self {
        Self {
            shared_output: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn read(&self) -> String {
        self.shared_output.lock().unwrap().clone()
    }
}

impl Write for Logger {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut buffer = self.shared_output.lock().unwrap();
        let input = String::from_utf8_lossy(buf);
        buffer.push_str(&input);
        Ok(buf.len())
    }

    /// Flushes the log buffer (no-op for this implementation).
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn find_or_create_config_dir() -> Result<PathBuf, String> {
    let mut path = dirs::config_local_dir()
        .or_else(|| dirs::config_dir())
        .ok_or_else(|| "Could not find config dir.".to_string())?;
    path.push(APPNAME);
    if !path.exists() {
        fs::create_dir_all(&path)
            .map_err(|_| format!("Cound not create directory '{}'", path.to_str().unwrap()))?;
    }
    Ok(path)
}

fn play_booked_sound(mut logger: Logger, background: Background) {
    if let Ok((_stream, stream_handle)) = OutputStream::try_default() {
        let sink = Sink::try_new(&stream_handle).unwrap();

        // Decode the embedded bytes
        let cursor = match background {
            Background::Grizzly => Cursor::new(BOOKED_SOUNDS[0]),
            Background::Ronnie => Cursor::new(BOOKED_SOUNDS[1]),
        };
        if let Ok(source) = Decoder::new(cursor) {
            sink.append(source);
            sink.set_volume(0.5);
            sink.sleep_until_end();
        } else {
            writeln!(logger, "WARNING: Failed to decode booked sound").unwrap();
        }
    } else {
        writeln!(logger, "WARNING: Failed to initialize audio output").unwrap();
    }
}

fn sumbit_handle(
    selected_timeslot: String,
    student_number: String,
    classname: Option<String>,
    mut logger: Logger,
    is_running: Arc<std::sync::atomic::AtomicBool>,
    play_sound: bool,
    background: Background,
) {
    if is_running.load(Ordering::SeqCst) {
        writeln!(logger, "You already submitted!").unwrap();
        return;
    }

    let trimmed_sn = student_number.trim();
    if trimmed_sn.is_empty() {
        writeln!(logger, "ERROR: Student number is empty!").unwrap();
        return;
    }
    if !trimmed_sn.chars().all(|c| c.is_digit(10)) {
        writeln!(logger, "ERROR: Student number is not a number!").unwrap();
        return;
    }

    // Logging info
    let mut logger_for_main_loop = logger.clone();
    if let Some(classname) = &classname {
        writeln!(logger, "Booking {} class:", classname).unwrap();
    } else {
        writeln!(logger, "Booking Poolside gym:").unwrap();
    }
    writeln!(logger, "    Time: {selected_timeslot}").unwrap();
    writeln!(logger, "    Student number: {student_number}").unwrap();

    is_running.store(true, Ordering::SeqCst);
    let is_running = is_running.clone();
    thread::spawn(move || {
        if let Err(e) = main_loop(
            &selected_timeslot,
            &student_number,
            classname.as_deref(),
            logger_for_main_loop.clone(),
        ) {
            writeln!(logger_for_main_loop, "ERROR: {e}").unwrap();
            if play_sound {}
        } else if play_sound {
            play_booked_sound(logger.clone(), background);
        }
        is_running.clone().store(false, Ordering::SeqCst);
    });
}

fn update_timeslots(classname: Option<&str>, book_table: MutexGuard<'_, BookingTable>, app: &App) {
    let timeslots: Vec<SharedString> = book_table
        .get_valid_timeslots(classname)
        .into_iter()
        .map(Into::into)
        .collect();
    let curr_time: &str = timeslots.get(0).map(|s| s.as_str()).unwrap_or_else(|| "");
    app.set_curr_time(curr_time.into());
    let timeslots_rc = app.get_timeslots();
    let timeslots_rc = timeslots_rc
        .as_any()
        .downcast_ref::<VecModel<SharedString>>()
        .expect("We know we set a timeslots VecModel earlier");
    timeslots_rc.set_vec(timeslots.clone());
}

fn update_class_names(book_table: MutexGuard<'_, BookingTable>, app: &App) {
    let classnames: Vec<SharedString> = book_table
        .get_valid_classes()
        .into_iter()
        .map(Into::into)
        .collect();
    let curr_class = classnames.get(0).map(|s| s.as_str()).unwrap_or_else(|| "");
    app.set_curr_class(curr_class.into());
    let classnames_rc = app.get_class_names();
    let classnames_rc = classnames_rc
        .as_any()
        .downcast_ref::<VecModel<SharedString>>()
        .expect("We know we set a classnames VecModel earlier");
    classnames_rc.set_vec(classnames.clone());
    update_timeslots(Some(curr_class), book_table, &app);
}

fn main() -> ExitCode {
    let settings = Rc::new(RefCell::new(Settings::new()));
    if let Err(e) = settings.borrow_mut().load_from_config() {
        eprintln!("ERROR: {e}");
        return ExitCode::FAILURE;
    }

    let booked_data = Rc::new(RefCell::new(BookedData::new()));
    if let Err(e) = booked_data.borrow_mut().load_from_config() {
        eprintln!("ERROR: {e}");
        return ExitCode::FAILURE;
    }

    let app = App::new().unwrap();
    let settings_window = SettingsWindow::new().unwrap();
    settings_window.hide().unwrap();

    settings_window.set_background_idx(settings.borrow().backround as i32);
    settings_window.set_enable_audio(settings.borrow().audio);
    app.set_background_idx(settings.borrow().backround as i32);
    app.set_student_id(booked_data.borrow().student_number.clone().into());

    // Creating the logger
    let mut logger = Logger::new();

    // Create the timeslots and class names vectors
    let timeslots: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    let classnames: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    app.set_timeslots(ModelRc::from(timeslots));
    app.set_class_names(ModelRc::from(classnames));

    // Default settings
    let book_table = Arc::new(Mutex::new(BookingTable::new(None)));
    if let Err(e) = book_table.lock().unwrap().load_html() {
        writeln!(logger, "{e}").unwrap();
    }
    update_timeslots(None, book_table.clone().lock().unwrap(), &app);

    // Thread that periodically reads the logger to the UI
    let logger_for_ui = logger.clone();
    let app_weak = app.as_weak();
    app.on_update_text(move || {
        let log_content = logger_for_ui.read();
        app_weak.unwrap().set_log(SharedString::from(log_content));
    });

    // Update drop menus when type is selected
    let app_weak = app.as_weak();
    let book_clone = book_table.clone();
    let mut logger_clone = logger.clone();
    app.on_type_selected(move |selected_type| {
        let app = app_weak.unwrap();
        let mut book_table = book_clone.lock().unwrap();
        if selected_type == "Classes" {
            book_table.book_type = BookType::Classes;
            if let Err(e) = book_table.reload_html() {
                writeln!(logger_clone, "{e}").unwrap();
            }
            app.set_class_box_enabled(true);
            update_class_names(book_table, &app);
        } else {
            book_table.book_type = BookType::Gym;
            if let Err(e) = book_table.reload_html() {
                writeln!(logger_clone, "{e}").unwrap();
            }
            app.set_curr_class("".into());
            app.set_class_box_enabled(false);
            update_timeslots(None, book_table, &app);
        }
    });

    // Update drop menus when class is selected
    let app_weak = app.as_weak();
    let book_clone = book_table.clone();
    app.on_class_selected(move |selected_class| {
        let app = app_weak.unwrap();
        let book_table = book_clone.lock().unwrap();
        update_timeslots(Some(&selected_class), book_table, &app);
    });

    // Thread management
    let is_running = Arc::new(AtomicBool::new(false));
    // Submit button/Thread spawn
    let mut logger_for_submit = logger.clone();
    let is_running_clone = is_running.clone();
    let play_sound = settings.borrow().audio;
    let background = settings.borrow().backround.clone();
    let book_data_clone = booked_data.clone();
    app.on_submit(
        move |selected_timeslot, student_number, is_class, classname| {
            let is_class = is_class != 0;
            let classname = if is_class {
                Some(classname.to_string())
            } else {
                None
            };
            let mut book_data = book_data_clone.borrow_mut();
            book_data.student_number = student_number.to_string();
            if let Err(e) = book_data.save() {
                writeln!(logger_for_submit, "ERROR: {e}").expect("Could not write to logger");
            }

            sumbit_handle(
                selected_timeslot.to_string(),
                student_number.to_string(),
                classname,
                logger_for_submit.clone(),
                is_running_clone.clone(),
                play_sound,
                background,
            );
        },
    );

    // Settings button
    let settings_window_weak = settings_window.as_weak();
    let setting_clone = settings.clone();
    app.on_show_settings(move || {
        let settings_window = settings_window_weak.unwrap();
        let settings = setting_clone.borrow();
        settings_window.set_background_idx(settings.backround as i32);
        settings_window.set_enable_audio(settings.audio);
        settings_window.show().unwrap();
    });

    // Settings window
    // Settings window close button
    let settings_window_weak = settings_window.as_weak();
    let setting_clone = settings.clone();
    settings_window.on_close_settings(move || {
        let settings = setting_clone.borrow();
        let settings_window = settings_window_weak.unwrap();
        settings_window.set_background_idx(settings.backround as i32);
        settings_window.set_enable_audio(settings.audio);
        settings_window.hide().unwrap();
    });

    // Settings window OK button
    let app_weak = app.as_weak();
    let settings_window_weak = settings_window.as_weak();
    let mut logger_clone = logger.clone();
    let settings_clone = settings.clone();
    settings_window.on_ok_settings(move |background, audio| {
        let mut settings = settings_clone.borrow_mut();
        let app = app_weak.unwrap();
        let settings_window = settings_window_weak.unwrap();
        settings.set_background(background.as_str());
        settings.audio = audio;
        if let Err(e) = settings.save() {
            writeln!(logger_clone, "ERROR: {e}").expect("Could not write to logger");
        }
        app.set_background_idx(settings.backround as i32);
        settings_window.hide().unwrap();
    });

    // Settings window apply button
    let app_weak = app.as_weak();
    let mut logger_clone = logger.clone();
    let settings_clone = settings.clone();
    settings_window.on_apply_settings(move |background, audio| {
        let mut settings = settings_clone.borrow_mut();
        let app = app_weak.unwrap();
        settings.set_background(background.as_str());
        settings.audio = audio;
        if let Err(e) = settings.save() {
            writeln!(logger_clone, "ERROR: {e}").expect("Could not write to logger");
        }
        app.set_background_idx(settings.backround as i32);
    });

    app.run().unwrap();

    ExitCode::SUCCESS
}
