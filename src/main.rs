extern crate serde;

use evtx::EvtxParser;
use quick_xml::de::DeError;
use std::{fs, path::PathBuf, process};
use yamato_event_analyzer::detections::configs;
use yamato_event_analyzer::detections::detection;
use yamato_event_analyzer::omikuji::Omikuji;

fn main() -> Result<(), DeError> {
    if let Some(filepath) = configs::singleton().args.value_of("filepath"){
        parse_file(&filepath);
    }

    Ok(())
}

fn parse_file(filepath: &str) {
    let fp = PathBuf::from(filepath);
    let parser = match EvtxParser::from_path(fp) {
        Ok(pointer) => pointer,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    };

    let mut detection = detection::Detection::new();
    &detection.start(parser);
}

fn output_with_omikuji(omikuji: Omikuji) {
    let fp = &format!("art/omikuji/{}", omikuji);
    let content = fs::read_to_string(fp).unwrap();
    println!("{}", content);
}
