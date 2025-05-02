use clap::Parser;
use std::fmt::Debug;
use std::io;
use std::process::ExitCode;

use gymbooking::*;

// TODO: Add more than one student numbers

/// Program that books the poolside gym at the requested time
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The disired time slot
    #[arg(short, long)]
    time: String,

    /// The member id (student number)
    #[arg(short, long)]
    id: String,

    /// The name of the class to be booked. If omitted the poolside gym is booked
    /// instead.
    #[arg(short, long)]
    class: Option<String>,
}

fn main() -> ExitCode {
    let args = Args::parse();

    if !args.id.chars().all(|c| c.is_digit(10)) {
        eprintln!("Student number is not a number!");
        return ExitCode::FAILURE;
    }

    // Logging info
    let selected_timeslot = &args.time;
    let student_number = &args.id;
    if let Some(classname) = &args.class {
        println!("Booking {classname} class:");
    } else {
        println!("Booking Poolside gym:");
    }
    println!("    Time: {selected_timeslot}");
    println!("    Student number: {student_number}");

    if let Err(e) = main_loop(&args.time, &args.id, args.class.as_deref(), io::stdout()) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
