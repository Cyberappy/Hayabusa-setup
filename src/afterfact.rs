use crate::detections::configs;
use crate::detections::print;
use crate::detections::print::AlertMessage;
use crate::detections::utils;
use chrono::{DateTime, Local, TimeZone, Utc};
use csv::QuoteStyle;
use hashbrown::HashMap;
use serde::Serialize;
use std::error::Error;
use std::fs::File;
use std::io;
use std::io::Write;
use std::process;
use termcolor::{BufferWriter, Color, ColorChoice, WriteColor};

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CsvFormat<'a> {
    timestamp: &'a str,
    computer: &'a str,
    channel: &'a str,
    event_i_d: &'a str,
    level: &'a str,
    mitre_attack: &'a str,
    rule_title: &'a str,
    details: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_information: Option<&'a str>,
    rule_path: &'a str,
    file_path: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayFormat<'a> {
    timestamp: &'a str,
    pub computer: &'a str,
    pub channel: &'a str,
    pub event_i_d: &'a str,
    pub level: &'a str,
    pub rule_title: &'a str,
    pub details: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_information: Option<&'a str>,
}

pub fn after_fact() {
    let fn_emit_csv_err = |err: Box<dyn Error>| {
        AlertMessage::alert(
            &mut BufWriter::new(std::io::stderr().lock()),
            &format!("Failed to write CSV. {}", err),
        )
        .ok();
        process::exit(1);
    };

    let mut displayflag = false;
    let mut target: Box<dyn io::Write> =
        if let Some(csv_path) = configs::CONFIG.read().unwrap().args.value_of("output") {
            // ファイル出力する場合
            match File::create(csv_path) {
                Ok(file) => Box::new(BufWriter::new(file)),
                Err(err) => {
                    AlertMessage::alert(
                        &mut BufWriter::new(std::io::stderr().lock()),
                        &format!("Failed to open file. {}", err),
                    )
                    .ok();
                    process::exit(1);
                }
            }
        } else {
            displayflag = true;
            // 標準出力に出力する場合
            Box::new(BufWriter::new(io::stdout()))
        };
    if let Err(err) = emit_csv(&mut target, displayflag) {
        fn_emit_csv_err(Box::new(err));
    }
}

fn emit_csv<W: std::io::Write>(writer: &mut W, displayflag: bool) -> io::Result<()> {
    let mut wtr = if displayflag {
        csv::WriterBuilder::new()
            .double_quote(false)
            .quote_style(QuoteStyle::Never)
            .delimiter(b'|')
            .from_writer(writer)
    } else {
        csv::WriterBuilder::new().from_writer(writer)
    };

    let messages = print::MESSAGES.lock().unwrap();
    // levelの区分が"Critical","High","Medium","Low","Informational","Undefined"の6つであるため
    let mut total_detect_counts_by_level: Vec<u128> = vec![0; 6];
    let mut unique_detect_counts_by_level: Vec<u128> = vec![0; 6];
    let mut detected_rule_files: Vec<String> = Vec::new();

    for (time, detect_infos) in messages.iter() {
        for detect_info in detect_infos {
            let mut level = detect_info.level.to_string();
            if level == "informational" {
                level = "info".to_string();
            }
            if displayflag {
                let color = _get_output_color(&detect_info.level);

                let recinfo = detect_info
                    .record_information
                    .as_ref()
                    .map(|recinfo| _format_cellpos(ColPos::Last, recinfo));
                let details = detect_info
                    .detail
                    .chars()
                    .filter(|&c| !c.is_control())
                    .collect::<String>();

                let dispformat = DisplayFormat {
                    timestamp: &_format_cellpos(&format_time(time), ColPos::First),
                    level: &_format_cellpos(&level, ColPos::Other),
                    computer: &_format_cellpos(&detect_info.computername, ColPos::Other),
                    event_i_d: &_format_cellpos(&detect_info.eventid, ColPos::Other),
                    channel: &_format_cellpos(&detect_info.channel, ColPos::Other),
                    rule_title: &_format_cellpos(&detect_info.alert, ColPos::Other),
                    details: &_format_cellpos(&details, ColPos::Other),
                    record_information: recinfo.as_deref(),
                };
                wtr.serialize(dispformat)?;
            } else {
                // csv出力時フォーマット
                wtr.serialize(CsvFormat {
                    timestamp: &format_time(time),
                    level: &level,
                    computer: &detect_info.computername,
                    event_i_d: &detect_info.eventid,
                    channel: &detect_info.channel,
                    mitre_attack: &detect_info.tag_info,
                    rule_title: &detect_info.alert,
                    details: &detect_info.detail,
                    record_information: detect_info.record_information.as_deref(),
                    file_path: &detect_info.filepath,
                    rule_path: &detect_info.rulepath,
                })?;
            }
            let level_suffix = *configs::LEVELMAP
                .get(&detect_info.level.to_uppercase())
                .unwrap_or(&0) as usize;
            if !detected_rule_files.contains(&detect_info.rulepath) {
                detected_rule_files.push(detect_info.rulepath.clone());
                unique_detect_counts_by_level[level_suffix] += 1;
            }
            total_detect_counts_by_level[level_suffix] += 1;
        }
    }
    println!();

    wtr.flush()?;
    println!();
    _print_unique_results(
        total_detect_counts_by_level,
        "Total".to_string(),
        "detections".to_string(),
        &color_map,
    );
    _print_unique_results(
        unique_detect_counts_by_level,
        "Unique".to_string(),
        "detections".to_string(),
        &color_map,
    );
    Ok(())
}

/// columnt position. in cell
/// First: |<str> |
/// Last: | <str>|
/// Othre: | <str> |
enum ColPos {
    First, // 先頭
    Last,  // 最後
    Other, // それ以外
}

/// return str position in output file
fn _format_cellpos(colval: &str, column: ColPos) -> String {
    return match column {
        ColPos::First => format!("{} ", colval),
        ColPos::Last => format!(" {}", colval),
        ColPos::Other => format!(" {} ", colval),
    };
}

/// 与えられたユニークな検知数と全体の検知数の情報(レベル別と総計)を元に結果文を標準出力に表示する関数
fn _print_unique_results(
    mut counts_by_level: Vec<u128>,
    head_word: String,
    tail_word: String,
    color_map: &Option<HashMap<String, Vec<u8>>>,
) {
    let mut wtr = BufWriter::new(io::stdout());
    let levels = Vec::from([
        "critical",
        "high",
        "medium",
        "low",
        "informational",
        "undefined",
    ]);

    // configsの登録順番と表示をさせたいlevelの順番が逆であるため
    counts_by_level.reverse();

    // 全体の集計(levelの記載がないためformatの第二引数は空の文字列)
    writeln!(
        wtr,
        "{} {}: {}",
        head_word,
        tail_word,
        counts_by_level.iter().sum::<u128>()
    )
    .ok();
    for (i, level_name) in levels.iter().enumerate() {
        let output_raw_str = format!(
            "{} {} {}: {}",
            head_word, level_name, tail_word, counts_by_level[i]
        );
        let output_str = if color_map.is_none() {
            output_raw_str
        } else {
            let output_color = _get_output_color(level_name);

            output_raw_str
                .truecolor(output_color[0], output_color[1], output_color[2])
                .to_string()
        };
        writeln!(wtr, "{}", output_str).ok();
    }
    wtr.flush().ok();
}

/// return termcolor by supported level
fn _get_output_color(level: &str) -> Option<termcolor::Color> {
    // return white no supported color
    let support_color: HashMap<String, termcolor::Color> = HashMap::from([
        ("CRITICAL".to_string(), termcolor::Color::Red),
        ("HIGH".to_string(), termcolor::Color::Yellow),
        ("MEDIUM".to_string(), termcolor::Color::Cyan),
        ("LOW".to_string(), termcolor::Color::Green),
    ]);
    support_color.get(level.to_uppercase())
}

fn format_time(time: &DateTime<Utc>) -> String {
    if configs::CONFIG.read().unwrap().args.is_present("utc") {
        format_rfc(time)
    } else {
        format_rfc(&time.with_timezone(&Local))
    }
}

fn format_rfc<Tz: TimeZone>(time: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if configs::CONFIG.read().unwrap().args.is_present("rfc-2822") {
        time.to_rfc2822()
    } else if configs::CONFIG.read().unwrap().args.is_present("rfc-3339") {
        time.to_rfc3339()
    } else {
        time.format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::afterfact::emit_csv;
    use crate::detections::print;
    use crate::detections::print::DetectInfo;
    use crate::detections::print::CH_CONFIG;
    use chrono::{Local, TimeZone, Utc};
    use serde_json::Value;
    use std::fs::File;
    use std::fs::{read_to_string, remove_file};
    use std::io;

    #[test]
    fn test_emit_csv() {
        //テストの並列処理によって読み込みの順序が担保できずstatic変数の内容が担保が取れない為、このテストはシーケンシャルで行う
        test_emit_csv_output();
        test_emit_csv_output();
    }

    fn test_emit_csv_output() {
        let test_filepath: &str = "test.evtx";
        let test_rulepath: &str = "test-rule.yml";
        let test_title = "test_title";
        let test_level = "high";
        let test_computername = "testcomputer";
        let test_eventid = "1111";
        let test_channel = "Sec";
        let output = "pokepoke";
        let test_attack = "execution/txxxx.yyy";
        let test_recinfo = "record_infoinfo11";
        {
            let mut messages = print::MESSAGES.lock().unwrap();
            messages.clear();
            let val = r##"
                {
                    "Event": {
                        "EventData": {
                            "CommandRLine": "hoge"
                        },
                        "System": {
                            "TimeCreated_attributes": {
                                "SystemTime": "1996-02-27T01:05:01Z"
                            }
                        }
                    }
                }
            "##;
            let event: Value = serde_json::from_str(val).unwrap();
            messages.insert(
                &event,
                output.to_string(),
                DetectInfo {
                    filepath: test_filepath.to_string(),
                    rulepath: test_rulepath.to_string(),
                    level: test_level.to_string(),
                    computername: test_computername.to_string(),
                    eventid: test_eventid.to_string(),
                    channel: CH_CONFIG
                        .get("Security")
                        .unwrap_or(&String::default())
                        .to_string(),
                    alert: test_title.to_string(),
                    detail: String::default(),
                    tag_info: test_attack.to_string(),
                    record_information: Option::Some(test_recinfo.to_string()),
                },
            );
        }
        let expect_time = Utc
            .datetime_from_str("1996-02-27T01:05:01Z", "%Y-%m-%dT%H:%M:%SZ")
            .unwrap();
        let expect_tz = expect_time.with_timezone(&Local);
        let expect =
            "Timestamp,Computer,Channel,EventID,Level,MitreAttack,RuleTitle,Details,RecordInformation,RulePath,FilePath\n"
                .to_string()
                + &expect_tz
                    .clone()
                    .format("%Y-%m-%d %H:%M:%S%.3f %:z")
                    .to_string()
                + ","
                + test_computername
                + ","
                + test_channel
                + ","
                + test_eventid
                + ","
                + test_level
                + ","
                + test_attack
                + ","
                + test_title
                + ","
                + output
                + ","
                + test_recinfo
                + ","
                + test_rulepath
                + ","
                + test_filepath
                + "\n";
        let mut file: Box<dyn io::Write> = Box::new(File::create("./test_emit_csv.csv").unwrap());
        assert!(emit_csv(&mut file, false, None).is_ok());
        match read_to_string("./test_emit_csv.csv") {
            Err(_) => panic!("Failed to open file."),
            Ok(s) => {
                assert_eq!(s, expect);
            }
        };
        assert!(remove_file("./test_emit_csv.csv").is_ok());
        check_emit_csv_display();
    }

    fn check_emit_csv_display() {
        let test_filepath: &str = "test2.evtx";
        let test_rulepath: &str = "test-rule2.yml";
        let test_title = "test_title2";
        let test_level = "medium";
        let test_computername = "testcomputer2";
        let test_eventid = "2222";
        let expect_channel = "Sysmon";
        let output = "displaytest";
        let test_attack = "execution/txxxx.zzz";
        {
            let mut messages = print::MESSAGES.lock().unwrap();
            messages.clear();
            let val = r##"
                {
                    "Event": {
                        "EventData": {
                            "CommandRLine": "hoge"
                        },
                        "System": {
                            "TimeCreated_attributes": {
                                "SystemTime": "1996-02-27T01:05:01Z"
                            }
                        }
                    }
                }
            "##;
            let event: Value = serde_json::from_str(val).unwrap();
            messages.insert(
                &event,
                output.to_string(),
                DetectInfo {
                    filepath: test_filepath.to_string(),
                    rulepath: test_rulepath.to_string(),
                    level: test_level.to_string(),
                    computername: test_computername.to_string(),
                    eventid: test_eventid.to_string(),
                    channel: CH_CONFIG
                        .get("Microsoft-Windows-Sysmon/Operational")
                        .unwrap_or(&String::default())
                        .to_string(),
                    alert: test_title.to_string(),
                    detail: String::default(),
                    tag_info: test_attack.to_string(),
                    record_information: Option::Some(String::default()),
                },
            );
            messages.debug();
        }
        let expect_time = Utc
            .datetime_from_str("1996-02-27T01:05:01Z", "%Y-%m-%dT%H:%M:%SZ")
            .unwrap();
        let expect_tz = expect_time.with_timezone(&Local);
        let expect_header =
            "Timestamp|Computer|Channel|EventID|Level|RuleTitle|Details|RecordInformation\n";
        let expect_colored = expect_header.to_string()
            + &get_white_color_string(
                &expect_tz
                    .clone()
                    .format("%Y-%m-%d %H:%M:%S%.3f %:z")
                    .to_string(),
            )
            + " | "
            + &get_white_color_string(test_computername)
            + " | "
            + &get_white_color_string(expect_channel)
            + " | "
            + &get_white_color_string(test_eventid)
            + " | "
            + &get_white_color_string(test_level)
            + " | "
            + &get_white_color_string(test_title)
            + " | "
            + &get_white_color_string(output)
            + " | "
            + &get_white_color_string("")
            + "\n";
        let expect_nocoloed = expect_header.to_string()
            + &expect_tz
                .clone()
                .format("%Y-%m-%d %H:%M:%S%.3f %:z")
                .to_string()
            + " | "
            + test_computername
            + " | "
            + expect_channel
            + " | "
            + test_eventid
            + " | "
            + test_level
            + " | "
            + test_title
            + " | "
            + output
            + " | "
            + ""
            + "\n";

        let mut file: Box<dyn io::Write> =
            Box::new(File::create("./test_emit_csv_display.txt").unwrap());
        assert!(emit_csv(&mut file, true, None).is_ok());
        match read_to_string("./test_emit_csv_display.txt") {
            Err(_) => panic!("Failed to open file."),
            Ok(s) => {
                assert!(s == expect_colored || s == expect_nocoloed);
            }
        };
        assert!(remove_file("./test_emit_csv_display.txt").is_ok());
    }

    fn get_white_color_string(target: &str) -> String {
        let white_color_header = "\u{1b}[38;2;255;255;255m";
        let white_color_footer = "\u{1b}[0m";

        white_color_header.to_owned() + target + white_color_footer
    }
}
