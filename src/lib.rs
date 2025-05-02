use reqwest;
use scraper::{selector::ToCss, ElementRef, Html, Selector};
use std::io::Write;
use std::thread;
use std::time::Duration;
use std::vec::Vec;
use time::macros::format_description;
use time::{self, ext::NumericalDuration, OffsetDateTime, Time};
use time_tz::{timezones, OffsetDateTimeExt};

// TODO: Improve error handling (are strings that bad?).
// TODO: Add multiple student numbers support.

const BASE_URL: &str = "https://hub.ucd.ie/usis/";
const GYM_URL: &str = "https://hub.ucd.ie/usis/W_HU_MENU.P_PUBLISH?p_tag=GYMBOOK";
const CLASSES_URL: &str = "https://hub.ucd.ie/usis/W_HU_MENU.P_PUBLISH?p_tag=GYMKIOSK";

#[macro_export]
macro_rules! current_time_dublin {
    () => {{
        let now_utc = OffsetDateTime::now_utc();

        let dublin_tz =
            timezones::get_by_name("Europe/Dublin").expect("Failed to find Dublin timezone");

        now_utc.to_timezone(dublin_tz)
    }};
}

macro_rules! is_ok_for_book {
    ($block:block) => {
        if option_env!("KAPAKOULAK").is_some() {
            $block
        }
    };
    ($block:block else $else_block:block) => {
        if option_env!("KAPAKOULAK").is_some() {
            $block
        } else {
            $else_block
        }
    };
}

#[derive(Debug)]
pub enum BookType {
    Gym,
    Classes,
}

impl BookType {
    fn get_url(&self) -> &'static str {
        match self {
            BookType::Gym => GYM_URL,
            BookType::Classes => CLASSES_URL,
        }
    }

    fn open_time(&self) -> time::Duration {
        match self {
            BookType::Gym => 2.hours(),
            BookType::Classes => 15.minutes(),
        }
    }
}

#[derive(Debug)]
pub struct BookingTable {
    pub table_css_selector: Selector,
    pub html: Option<Html>,
    pub book_type: BookType,
    pub book_class: Option<String>,
}

impl BookingTable {
    pub fn new(book_class: Option<&str>) -> Self {
        // If None is passed BookType::Gym is assumed.
        let book_type = match book_class {
            Some(_) => BookType::Classes,
            None => BookType::Gym,
        };
        Self {
            table_css_selector: Selector::parse("#SW300-1Q > tbody:nth-child(2) tr").unwrap(),
            html: None,
            book_type: book_type,
            book_class: book_class.map(|x| x.to_string()),
        }
    }

    pub fn load_html(&mut self) -> Result<(), String> {
        let body = get_html_responce(self.book_type.get_url())?;
        self.html = Some(Html::parse_document(&body));
        Ok(())
    }

    pub fn reload_html(&mut self) -> Result<(), String> {
        self.load_html()
    }

    pub fn is_valid_timeslot(&self, time: &str, classname: Option<&str>) -> bool {
        let html = self
            .html
            .as_ref()
            .expect("Html is not load. Call `BookingTable::load_html`.");
        let mut book_row_iter = BookingRowIterator::new(html.select(&self.table_css_selector));
        match self.book_type {
            BookType::Gym => book_row_iter.any(|row| row.time == time),
            BookType::Classes => book_row_iter.any(|row| {
                row.class == classname.expect("Expected Some(&str) but found `None`")
                    && row.time == time
            }),
        }
    }

    pub fn is_valid_class(&self, classname: &str) -> bool {
        let html = self
            .html
            .as_ref()
            .expect("Html is not load. Call `BookingTable::load_html`.");
        let mut book_row_iter = BookingRowIterator::new(html.select(&self.table_css_selector));
        book_row_iter.any(|row| &row.class == classname && row.class_type != "Member")
    }

    pub fn get_valid_timeslots(&self, classname: Option<&str>) -> Vec<String> {
        let html = self
            .html
            .as_ref()
            .expect("Html is not load. Call `BookingTable::load_html`.");
        let book_row_iter = BookingRowIterator::new(html.select(&self.table_css_selector));
        book_row_iter
            .filter_map(|row| match &self.book_type {
                BookType::Gym => Some(row.time),
                BookType::Classes => {
                    if row.class == classname.expect("Expected  Some(&str) but found None") {
                        Some(row.time)
                    } else {
                        None
                    }
                }
            })
            .collect()
    }

    pub fn get_valid_classes(&self) -> Vec<String> {
        let html = self
            .html
            .as_ref()
            .expect("Html is not load. Call `BookingTable::load_html`.");
        let book_row_iter = BookingRowIterator::new(html.select(&self.table_css_selector));
        let mut classes = book_row_iter
            .filter_map(|row| {
                if self.is_valid_class(&row.class) {
                    Some(row.class)
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();
        classes.sort();
        classes.dedup();
        classes
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct BookingRow {
    pub time: String,
    pub class: String,
    pub duration: String,
    pub location: String,
    pub class_type: String,
    pub book_text: String, // Can be empty string
    pub book_url: String,  // Can be empty string
}

pub struct BookingRowIterator<'a> {
    pub element: scraper::html::Select<'a, 'a>,
}

impl<'a> BookingRowIterator<'a> {
    pub fn new(element: scraper::html::Select<'a, 'a>) -> Self {
        Self { element: element }
    }
}

fn format_duration(duration: &time::Duration) -> String {
    let total_seconds = duration.whole_seconds();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{:02} hrs {:02} mins {:02} secs", hours, minutes, seconds)
}

impl<'a> Iterator for BookingRowIterator<'a> {
    type Item = BookingRow;

    fn next(&mut self) -> Option<Self::Item> {
        let a_selector = Selector::parse("a").unwrap();
        let row = self.element.next()?;
        let columns = row.child_elements().collect::<Vec<_>>();
        Some(BookingRow {
            time: columns[0].text().collect::<String>(),
            class: columns[1].text().collect::<String>(),
            duration: columns[2].text().collect::<String>(),
            location: columns[3].text().collect::<String>(),
            class_type: columns[4].text().collect::<String>(),
            book_text: columns[5].text().collect::<String>(),
            book_url: get_href_url(columns[5], &a_selector).unwrap_or("".to_string()),
        })
    }
}

fn get_html_responce(url: &str) -> Result<String, String> {
    let body = reqwest::blocking::get(url)
        .map_err(|_| format!("{} did not respond", url))?
        .text()
        .map_err(|_| format!("Could not read response body from {}", url));
    body
}

fn get_href_url(element: ElementRef, selector: &Selector) -> Result<String, String> {
    Ok(format!(
        "{}{}",
        BASE_URL,
        element
            .select(selector)
            .next()
            .ok_or(format!(
                "Cannot find element with css selector '{}'",
                selector.to_css_string()
            ))?
            .attr("href")
            .ok_or(format!(
                "href is missing from element with selector '{}'",
                selector.to_css_string()
            ))?
    ))
}

fn fill_id_and_redirect(url: &str, id: &str, mut out: impl Write) -> Result<String, String> {
    let html =
        Html::parse_document(&get_html_responce(url).expect("Unable to get booking reponse"));

    let sel = Selector::parse(".panel-body > form:nth-child(1) > input:nth-child(3)").unwrap();

    let p_parameters = html
        .select(&sel)
        .next()
        .expect("p_parameters element does not exist.")
        .attr("value")
        .expect("Value of p_parameters does not exist");

    // To emulate filling the text box we create get request with the parameters from the document and
    // member id.
    let data = format!(
        "p_query=SW-GYMANON&p_confirmed=Y&p_parameters={}&MEMBER_NO={}",
        p_parameters, id
    );

    let post_url = format!("{BASE_URL}!W_HU_REPORTING.P_RUN_SQL?{data}");
    writeln!(out, "Sending member ID...").expect("Unable to write to output buffer");
    let res = get_html_responce(&post_url)?;

    let mut redirect_url = None;
    for line in res.lines() {
        let pat = "<meta http-equiv=\"refresh\" content=\"0;url=";
        let pat_len = pat.len();
        if let Some(_) = line.find(pat) {
            redirect_url = Some(
                (&line[pat_len..])
                    .chars()
                    .filter(|&c| !"\">".contains(c))
                    .collect::<String>(),
            );
            break;
        }
    }

    let redirect_url = redirect_url.ok_or("Could not find redirect URL.")?;
    let post_url = format!("{BASE_URL}{redirect_url}");
    let res = get_html_responce(&post_url)?;
    Ok(res)
}

fn confirm_booking(response: &str, mut out: impl Write) -> Result<(), String> {
    // Find the link from the html responce.
    let html = Html::parse_document(response);
    let selector = Selector::parse("a.menubutton:nth-child(9)").unwrap();
    let confirm_url = get_href_url(html.root_element(), &selector)
        .map_err(|_| "Cound not find 'Confirm Booking' button. Aborting.")?;
    is_ok_for_book!({
        let _ = get_html_responce(&confirm_url)?;
    } else {
        return Err("Booking failed.".to_string());
    });

    writeln!(out, "Booked!").expect("Unable to write to output buffer");
    writeln!(
        out,
        concat!(
            "Remember to snort your creatine and pay your respects to the One, ",
            "kapakoulak! Or he will be waiting for you!"
        )
    )
    .expect("Unable to write to output buffer");
    Ok(())
}

fn book_gym(url: &str, id: &str, mut out: impl Write) -> Result<(), String> {
    let res = fill_id_and_redirect(url, id, out.by_ref())?;
    confirm_booking(&res, out.by_ref())
}

pub fn sleep_until_book_time(
    book_table: &BookingTable,
    timeslot: Time,
    mut out: impl Write,
) -> Result<(), String> {
    let format = format_description!("[hour]:[minute]");
    // let sleeper = SpinSleeper::new(500_000);
    let small_offset = 1.milliseconds();

    let dt =
        timeslot - current_time_dublin!().time() - book_table.book_type.open_time() + small_offset;
    if dt > 0.seconds() {
        writeln!(
            out,
            "Current time is {}. Sleeping for {}. Do not close this window.",
            current_time_dublin!().time().format(format).unwrap(),
            format_duration(&dt)
        )
        .expect("Unable to write to output buffer");

        // Hybrid sleep
        let now = current_time_dublin!();
        let target_time =
            now.replace_time(timeslot) - book_table.book_type.open_time() + small_offset;
        loop {
            let now = current_time_dublin!();
            if now >= target_time {
                break;
            }
            let remaining = target_time - current_time_dublin!();
            let remaining = if remaining > time::Duration::minutes(30) {
                time::Duration::minutes(5)
            } else if remaining > time::Duration::minutes(10) {
                time::Duration::minutes(1)
            } else {
                (target_time - current_time_dublin!()).min(1.milliseconds())
            };

            // DEBUG print
            if cfg!(debug_assertions) {
                let total_seconds = remaining.whole_seconds();
                let minutes = (total_seconds % 3600) / 60;
                let seconds = total_seconds % 60;
                let fmt_dur = if total_seconds == 0 {
                    format!("{:.2} ms", remaining.whole_microseconds() as f64 * 1e-3)
                } else {
                    format!("{:02} mins {:02} secs", minutes, seconds)
                };
                println!("**DEBUG** Sleep loop: {}", fmt_dur);
            }

            thread::sleep(Duration::from_secs_f64(remaining.as_seconds_f64())); // Low accuracy but it's fine.
        }
    }
    Ok(())
}

pub fn main_loop(
    time: &str,
    id: &str,
    classname: Option<&str>,
    mut out: impl Write,
) -> Result<(), String> {
    // Format time
    let format = format_description!("[hour]:[minute]");
    let timeslot =
        Time::parse(time, format).map_err(|_| format!("Cannot parse time '{}'.", time))?;

    let mut book_table = BookingTable::new(classname);
    book_table.load_html()?;

    // Check if class if valid
    if let Some(classname) = &book_table.book_class {
        if !book_table.is_valid_class(classname) {
            let valid_classes = book_table.get_valid_classes();
            let err_message = format!("Class '{}' is not a valid class.\n", classname);
            let err_message = if valid_classes.len() > 0 {
                format!("{err_message}Valid classes: {}", valid_classes.join(", "))
            } else {
                format!("{err_message}Too late idiot! No more available classes today.")
            };
            return Err(err_message);
        }
    }

    // Check if time is valid
    if !book_table.is_valid_timeslot(time, classname) {
        let valid_times = book_table.get_valid_timeslots(book_table.book_class.as_deref());
        let err_message = format!("Time '{}' is not a valid time slot.\n", time);
        let err_message = if valid_times.len() > 0 {
            format!("{err_message}Valid time slots: {}", valid_times.join(", "))
        } else {
            format!("{err_message}Too late idiot! No more available time slots today.")
        };
        return Err(err_message);
    }

    sleep_until_book_time(&book_table, timeslot, out.by_ref())?;

    'outer: loop {
        book_table.reload_html()?;

        let html = book_table
            .html
            .as_ref()
            .expect("Html is not load. Call `BookingTable::load_html`.");
        let book_row_iter = BookingRowIterator::new(html.select(&book_table.table_css_selector));
        let rows = book_row_iter.collect::<Vec<_>>();

        if rows.len() == 0 {
            return Err("Unreachable (or not)! No rows found in book tabl!".to_string());
        }

        for row in rows {
            if row.time != time {
                continue;
            }
            if row.book_text == "Full" {
                return Err(
                    "The requested timeslot is Full! Next time pray to kapakoulak!".to_string(),
                );
            }
            if row.book_text == "" || row.book_url == "" {
                writeln!(out, "Booking link not available. Trying again...")
                    .expect("Unable to write to output buffer");
                continue 'outer;
            }
            writeln!(out, "Found time slot.\nTrying to book...")
                .expect("Unable to write to output buffer");
            book_gym(&row.book_url, id, out.by_ref())?;
            break 'outer;
        }
    }
    Ok(())
}
