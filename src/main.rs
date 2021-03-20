extern crate serde;
extern crate serde_derive;

use std::{fs, path::PathBuf};
use yamato_event_analyzer::afterfact::after_fact;
use yamato_event_analyzer::detections::configs;
use yamato_event_analyzer::detections::detection;
use yamato_event_analyzer::detections::print::AlertMessage;
use yamato_event_analyzer::omikuji::Omikuji;

fn main() {
    if let Some(filepath) = configs::CONFIG.read().unwrap().args.value_of("filepath") {
        detect_files(vec![PathBuf::from(filepath)]);
    } else if let Some(directory) = configs::CONFIG.read().unwrap().args.value_of("directory") {
        let evtx_files = collect_evtxfiles(&directory);
        detect_files(evtx_files);
    } else if configs::CONFIG.read().unwrap().args.is_present("credits") {
        print_credits();
    }
}

fn collect_evtxfiles(dirpath: &str) -> Vec<PathBuf> {
    let entries = fs::read_dir(dirpath);
    if entries.is_err() {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        AlertMessage::alert(&mut stdout, format!("{}", entries.unwrap_err())).ok();
        return vec![];
    }

    let mut ret = vec![];
    for e in entries.unwrap() {
        if e.is_err() {
            continue;
        }

        let path = e.unwrap().path();
        if path.is_dir() {
            path.to_str().and_then(|path_str| {
                let subdir_ret = collect_evtxfiles(path_str);
                ret.extend(subdir_ret);
                return Option::Some(());
            });
        } else {
            let path_str = path.to_str().unwrap_or("");
            if path_str.ends_with(".evtx") {
                ret.push(path);
            }
        }
    }

    return ret;
}

fn print_credits() {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    match fs::read_to_string("./credits.txt") {
        Ok(contents) => println!("{}", contents),
        Err(err) => {
            AlertMessage::alert(&mut stdout, format!("{}", err)).ok();
        }
    }
}

fn detect_files(evtx_files: Vec<PathBuf>) {
    let mut detection = detection::Detection::new();
    &detection.start(evtx_files);

    after_fact();
}

fn _output_with_omikuji(omikuji: Omikuji) {
    let fp = &format!("art/omikuji/{}", omikuji);
    let content = fs::read_to_string(fp).unwrap();
    println!("{}", content);
}

#[cfg(test)]
mod tests {
    use crate::collect_evtxfiles;

    #[test]
    fn test_collect_evtxfiles() {
        let files = collect_evtxfiles("test_files/evtx");
        assert_eq!(3, files.len());

        files.iter().for_each(|file| {
            let is_contains = &vec!["test1.evtx", "test2.evtx", "testtest4.evtx"]
                .into_iter()
                .any(|filepath_str| {
                    return file.file_name().unwrap().to_str().unwrap_or("") == filepath_str;
                });
            assert_eq!(is_contains, &true);
        })
    }
}
