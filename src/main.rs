extern crate bytesize;
extern crate downcast_rs;
extern crate maxminddb;
extern crate serde;
extern crate serde_derive;

use bytesize::ByteSize;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, Utc};
use clap::Command;
use evtx::{EvtxParser, ParserSettings};
use hashbrown::{HashMap, HashSet};
use hayabusa::debug::checkpoint_process_timer::CHECKPOINT;
use hayabusa::detections::configs::{
    load_pivot_keywords, Action, ConfigReader, EventInfoConfig, EventKeyAliasConfig, StoredStatic,
    TargetEventIds, TargetEventTime, CURRENT_EXE_PATH, STORED_EKEY_ALIAS, STORED_STATIC,
};
use hayabusa::detections::detection::{self, EvtxRecordInfo};
use hayabusa::detections::message::{AlertMessage, ERROR_LOG_STACK};
use hayabusa::detections::rule::{get_detection_keys, RuleNode};
use hayabusa::detections::utils::{
    check_setting_path, get_writable_color, output_and_data_stack_for_html, output_profile_name,
};
use hayabusa::options;
use hayabusa::options::htmlreport::{self, HTML_REPORTER};
use hayabusa::options::pivot::create_output;
use hayabusa::options::pivot::PIVOT_KEYWORD;
use hayabusa::options::profile::set_default_profile;
use hayabusa::options::{level_tuning::LevelTuning, update::Update};
use hayabusa::{afterfact::after_fact, detections::utils};
use hayabusa::{detections::configs, timeline::timelines::Timeline};
use hayabusa::{detections::utils::write_color_buffer, filter};
use hhmmss::Hhmmss;
use itertools::Itertools;
use libmimalloc_sys::mi_stats_print_out;
use mimalloc::MiMalloc;
use nested::Nested;
use pbr::ProgressBar;
use serde_json::{Map, Value};
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fmt::Write as _;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;
use std::{
    env,
    fs::{self, File},
    path::PathBuf,
    vec,
};
use termcolor::{BufferWriter, Color, ColorChoice};
use tokio::runtime::Runtime;
use tokio::spawn;
use tokio::task::JoinHandle;

#[cfg(target_os = "windows")]
use is_elevated::is_elevated;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// 一度にtimelineやdetectionを実行する行数
const MAX_DETECT_RECORDS: usize = 5000;

fn main() {
    let mut config_reader = ConfigReader::new();
    // コマンドのパース情報を作成してstatic変数に格納する
    let mut stored_static = StoredStatic::create_static_data(config_reader.config);
    config_reader.config = None;
    let mut app = App::new(stored_static.thread_number);
    app.exec(&mut config_reader.app, &mut stored_static);
    app.rt.shutdown_background();
}

pub struct App {
    rt: Runtime,
    rule_keys: Nested<String>,
}

impl App {
    pub fn new(thread_number: Option<usize>) -> App {
        App {
            rt: utils::create_tokio_runtime(thread_number),
            rule_keys: Nested::<String>::new(),
        }
    }

    fn exec(&mut self, app: &mut Command, stored_static: &mut StoredStatic) {
        if stored_static.profiles.is_none() {
            return;
        }

        let analysis_start_time: DateTime<Local> = Local::now();
        if stored_static.html_report_flag {
            let mut output_data = Nested::<String>::new();
            output_data.extend(vec![
                format!("- Command line: {}", std::env::args().join(" ")),
                format!(
                    "- Start time: {}",
                    analysis_start_time.format("%Y/%m/%d %H:%M")
                ),
            ]);
            htmlreport::add_md_data("General Overview {#general_overview}", output_data);
        }

        // 引数がなかった時にhelpを出力するためのサブコマンド出力。引数がなくても動作するサブコマンドはhelpを出力しない
        let subcommand_name = Action::get_action_name(stored_static.config.action.as_ref());
        if stored_static.config.action.is_some()
            && !self.check_is_valid_args_num(stored_static.config.action.as_ref())
        {
            if !stored_static.common_options.quiet {
                self.output_logo(stored_static);
                write_color_buffer(&BufferWriter::stdout(ColorChoice::Always), None, "", true).ok();
            }
            app.find_subcommand(subcommand_name)
                .unwrap()
                .clone()
                .print_help()
                .ok();
            println!();
            return;
        }

        // Show usage when no arguments.
        if stored_static.config.action.is_none() {
            if !stored_static.common_options.quiet {
                self.output_logo(stored_static);
                write_color_buffer(&BufferWriter::stdout(ColorChoice::Always), None, "", true).ok();
            }
            app.print_help().ok();
            println!();
            return;
        }
        if !stored_static.common_options.quiet {
            self.output_logo(stored_static);
            write_color_buffer(&BufferWriter::stdout(ColorChoice::Always), None, "", true).ok();
            self.output_eggs(&format!(
                "{:02}/{:02}",
                &analysis_start_time.month(),
                &analysis_start_time.day()
            ));
        }
        if !self.is_matched_architecture_and_binary() {
            AlertMessage::alert(
                "The hayabusa version you ran does not match your PC architecture.\nPlease use the correct architecture. (Binary ending in -x64.exe for 64-bit and -x86.exe for 32-bit.)",
            )
            .ok();
            println!();
            return;
        }

        // 実行時のexeファイルのパスをベースに変更する必要があるためデフォルトの値であった場合はそのexeファイルと同一階層を探すようにする
        if !CURRENT_EXE_PATH.join("config").exists() && !Path::new("./config").exists() {
            AlertMessage::alert(
                "Hayabusa could not find the config directory.\nPlease make sure that it is in the same directory as the hayabusa executable."
            )
            .ok();
            return;
        }
        // カレントディレクトリ以外からの実行の際にrules-configオプションの指定がないとエラーが発生することを防ぐための処理
        if stored_static.config_path == Path::new("./rules/config") {
            stored_static.config_path =
                utils::check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), "rules/config", true)
                    .unwrap();
        }

        let time_filter = TargetEventTime::new(stored_static);
        if !time_filter.is_parse_success() {
            return;
        }

        if stored_static.metrics_flag {
            write_color_buffer(
                &BufferWriter::stdout(ColorChoice::Always),
                None,
                "Generating Event ID Metrics",
                true,
            )
            .ok();
            println!();
        }
        if stored_static.logon_summary_flag {
            write_color_buffer(
                &BufferWriter::stdout(ColorChoice::Always),
                None,
                "Generating Logon Summary",
                true,
            )
            .ok();
            println!();
        }
        if stored_static.search_flag {
            write_color_buffer(
                &BufferWriter::stdout(ColorChoice::Always),
                None,
                "Searching...",
                true,
            )
            .ok();
            println!();
        }

        write_color_buffer(
            &BufferWriter::stdout(ColorChoice::Always),
            None,
            &format!(
                "Start time: {}\n",
                analysis_start_time.format("%Y/%m/%d %H:%M")
            ),
            true,
        )
        .ok();
        CHECKPOINT
            .lock()
            .as_mut()
            .unwrap()
            .set_checkpoint(analysis_start_time);
        let target_extensions = if stored_static.output_option.is_some() {
            configs::get_target_extensions(
                stored_static
                    .output_option
                    .as_ref()
                    .unwrap()
                    .detect_common_options
                    .evtx_file_ext
                    .as_ref(),
                stored_static.json_input_flag,
            )
        } else {
            HashSet::default()
        };

        let output_saved_file = |output_path: &Option<PathBuf>, message: &str| {
            if let Some(path) = output_path {
                if let Ok(metadata) = fs::metadata(path) {
                    let output_saved_str = format!(
                        "{message}: {} ({})",
                        path.display(),
                        ByteSize::b(metadata.len()).to_string_as(false)
                    );
                    output_and_data_stack_for_html(
                        &output_saved_str,
                        "General Overview {#general_overview}",
                        stored_static.html_report_flag,
                    );
                }
            }
        };

        match &stored_static.config.action.as_ref().unwrap() {
            Action::CsvTimeline(_) | Action::JsonTimeline(_) => {
                // カレントディレクトリ以外からの実行の際にrulesオプションの指定がないとエラーが発生することを防ぐための処理
                if stored_static.output_option.as_ref().unwrap().rules == Path::new("./rules") {
                    stored_static.output_option.as_mut().unwrap().rules =
                        utils::check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), "rules", true)
                            .unwrap();
                }
                // rule configのフォルダ、ファイルを確認してエラーがあった場合は終了とする
                if let Err(e) = utils::check_rule_config(&stored_static.config_path) {
                    AlertMessage::alert(&e).ok();
                    return;
                }

                if stored_static.profiles.is_none() {
                    return;
                }
                if let Some(html_path) = &stored_static.output_option.as_ref().unwrap().html_report
                {
                    // if already exists same html report file. output alert message and exit
                    if !(stored_static.output_option.as_ref().unwrap().clobber)
                        && utils::check_file_expect_not_exist(
                            html_path.as_path(),
                            format!(
                                " The file {} already exists. Please specify a different filename.\n",
                                html_path.to_str().unwrap()
                            ),
                        )
                    {
                        return;
                    }
                }
                if let Some(path) = &stored_static.output_path {
                    if !(stored_static.output_option.as_ref().unwrap().clobber)
                        && utils::check_file_expect_not_exist(
                            path.as_path(),
                            format!(
                                " The file {} already exists. Please specify a different filename.\n",
                                path.as_os_str().to_str().unwrap()
                            ),
                        )
                    {
                        return;
                    }
                }
                self.analysis_start(&target_extensions, &time_filter, stored_static);

                output_profile_name(&stored_static.output_option, false);
                output_saved_file(&stored_static.output_path, "Saved file");
                println!();
                if stored_static.html_report_flag {
                    let html_str = HTML_REPORTER.read().unwrap().to_owned().create_html();
                    htmlreport::create_html_file(
                        html_str,
                        stored_static
                            .output_option
                            .as_ref()
                            .unwrap()
                            .html_report
                            .as_ref()
                            .unwrap()
                            .to_str()
                            .unwrap_or(""),
                    )
                }
            }
            Action::ListContributors(_) => {
                self.print_contributors();
                return;
            }
            Action::LogonSummary(_) => {
                let mut target_output_path = Nested::<String>::new();
                if let Some(path) = &stored_static.output_path {
                    for suffix in &["-successful.csv", "-failed.csv"] {
                        let output_file = format!("{}{suffix}", path.to_str().unwrap());
                        if !(stored_static.output_option.as_ref().unwrap().clobber)
                            && utils::check_file_expect_not_exist(
                                Path::new(output_file.as_str()),
                                format!(
                                " The files with a base name of {} already exist. Please specify a different base filename.\n",
                                path.as_os_str().to_str().unwrap()
                            ),
                            )
                        {
                            return;
                        }
                        target_output_path.push(output_file);
                    }
                }
                self.analysis_start(&target_extensions, &time_filter, stored_static);
                for target_path in target_output_path.iter() {
                    let mut msg = "";
                    if target_path.ends_with("-successful.csv") {
                        msg = "Successful logon results:"
                    }
                    if target_path.ends_with("-failed.csv") {
                        msg = "Failed logon results:"
                    }
                    output_saved_file(&Some(Path::new(target_path).to_path_buf()), msg);
                }
                println!();
            }
            Action::Metrics(_) | Action::Search(_) => {
                if let Some(path) = &stored_static.output_path {
                    if !(stored_static.output_option.as_ref().unwrap().clobber)
                        && utils::check_file_expect_not_exist(
                            path.as_path(),
                            format!(
                                " The file {} already exists. Please specify a different filename.\n",
                                path.as_os_str().to_str().unwrap()
                            ),
                        )
                    {
                        return;
                    }
                }
                self.analysis_start(&target_extensions, &time_filter, stored_static);
                match &stored_static.config.action.as_ref().unwrap() {
                    Action::Search(_) => {
                        output_saved_file(&stored_static.output_path, "Saved file");
                    }
                    _ => {
                        // SearchでなければMetricsの結果となるため
                        output_saved_file(&stored_static.output_path, "Metrics results");
                    }
                }

                println!();
            }
            Action::PivotKeywordsList(_) => {
                // pivot 機能でファイルを出力する際に同名ファイルが既に存在していた場合はエラー文を出して終了する。
                if let Some(csv_path) = &stored_static.output_path {
                    let mut error_flag = false;
                    let pivot_key_unions = PIVOT_KEYWORD.read().unwrap();
                    pivot_key_unions.iter().for_each(|(key, _)| {
                        let keywords_file_name =
                            csv_path.as_path().display().to_string() + "-" + key + ".txt";
                        if utils::check_file_expect_not_exist(
                            Path::new(&keywords_file_name),
                            format!(
                                " The file {} already exists. Please specify a different filename.\n",
                                &keywords_file_name
                            ),
                        ) {
                            error_flag = true
                        };
                    });
                    if error_flag {
                        return;
                    }
                }
                load_pivot_keywords(
                    utils::check_setting_path(
                        &CURRENT_EXE_PATH.to_path_buf(),
                        "rules/config/pivot_keywords.txt",
                        true,
                    )
                    .unwrap()
                    .to_str()
                    .unwrap(),
                );

                self.analysis_start(&target_extensions, &time_filter, stored_static);

                let pivot_key_unions = PIVOT_KEYWORD.read().unwrap();
                if let Some(pivot_file) = &stored_static.output_path {
                    //ファイル出力の場合
                    pivot_key_unions.iter().for_each(|(key, pivot_keyword)| {
                        let mut f = BufWriter::new(
                            fs::File::create(
                                pivot_file.as_path().display().to_string() + "-" + key + ".txt",
                            )
                            .unwrap(),
                        );
                        f.write_all(
                            create_output(
                                String::default(),
                                key,
                                pivot_keyword,
                                "file",
                                stored_static,
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    });
                    let mut output =
                        "Pivot keyword results were saved to the following files:\n".to_string();

                    pivot_key_unions.iter().for_each(|(key, _)| {
                        writeln!(
                            output,
                            "{}",
                            &(pivot_file.as_path().display().to_string() + "-" + key + ".txt")
                        )
                        .ok();
                    });
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        None,
                        &output,
                        true,
                    )
                    .ok();
                } else {
                    //標準出力の場合
                    let output = "\nThe following pivot keywords were found:\n";
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        None,
                        output,
                        true,
                    )
                    .ok();

                    pivot_key_unions.iter().for_each(|(key, pivot_keyword)| {
                        create_output(
                            String::default(),
                            key,
                            pivot_keyword,
                            "standard",
                            stored_static,
                        );

                        if pivot_keyword.keywords.is_empty() {
                            write_color_buffer(
                                &BufferWriter::stdout(ColorChoice::Always),
                                get_writable_color(
                                    Some(Color::Red),
                                    stored_static.common_options.no_color,
                                ),
                                "No keywords found\n",
                                true,
                            )
                            .ok();
                        }
                    });
                }
            }
            Action::UpdateRules(_) => {
                let update_target = match &stored_static.config.action.as_ref().unwrap() {
                    Action::UpdateRules(option) => Some(option.rules.to_owned()),
                    _ => None,
                };
                // エラーが出た場合はインターネット接続がそもそもできないなどの問題点もあるためエラー等の出力は行わない
                let latest_version_data = if let Ok(data) = Update::get_latest_hayabusa_version() {
                    data
                } else {
                    None
                };
                let now_version = &format!("v{}", env!("CARGO_PKG_VERSION"));

                match Update::update_rules(update_target.unwrap().to_str().unwrap(), stored_static)
                {
                    Ok(output) => {
                        if output != "You currently have the latest rules." {
                            write_color_buffer(
                                &BufferWriter::stdout(ColorChoice::Always),
                                None,
                                "Rules updated successfully.",
                                true,
                            )
                            .ok();
                        }
                    }
                    Err(e) => {
                        if e.message().is_empty() {
                            AlertMessage::alert("Failed to update rules.").ok();
                        } else {
                            AlertMessage::alert(&format!("Failed to update rules. {e:?}  ")).ok();
                        }
                    }
                }
                println!();
                let split_now_version = &now_version
                    .replace("-dev", "")
                    .split('.')
                    .filter_map(|x| x.parse().ok())
                    .collect::<Vec<i8>>();
                let split_latest_version = &latest_version_data
                    .as_ref()
                    .unwrap_or(now_version)
                    .replace('"', "")
                    .split('.')
                    .filter_map(|x| x.parse().ok())
                    .collect::<Vec<i8>>();
                if split_latest_version > split_now_version {
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        None,
                        &format!(
                            "There is a new version of Hayabusa: {}",
                            latest_version_data.unwrap().replace('\"', "")
                        ),
                        true,
                    )
                    .ok();
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        None,
                        "You can download it at https://github.com/Yamato-Security/hayabusa/releases",
                        true,
                    )
                    .ok();
                    println!();
                }
                return;
            }
            Action::LevelTuning(option) => {
                let level_tuning_config_path = if option.level_tuning.to_str().unwrap()
                    != "./rules/config/level_tuning.txt"
                {
                    utils::check_setting_path(
                        option
                            .level_tuning
                            .parent()
                            .unwrap_or_else(|| Path::new("")),
                        option
                            .level_tuning
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap_or_default(),
                        false,
                    )
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/level_tuning.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .display()
                    .to_string()
                } else {
                    utils::check_setting_path(&stored_static.config_path, "level_tuning.txt", false)
                        .unwrap_or_else(|| {
                            utils::check_setting_path(
                                &CURRENT_EXE_PATH.to_path_buf(),
                                "rules/config/level_tuning.txt",
                                true,
                            )
                            .unwrap()
                        })
                        .display()
                        .to_string()
                };

                let rules_path = if stored_static.output_option.as_ref().is_some() {
                    stored_static
                        .output_option
                        .as_ref()
                        .unwrap()
                        .rules
                        .as_os_str()
                        .to_str()
                        .unwrap()
                } else {
                    "./rules"
                };

                if Path::new(&level_tuning_config_path).exists() {
                    if let Err(err) =
                        LevelTuning::run(&level_tuning_config_path, rules_path, stored_static)
                    {
                        AlertMessage::alert(&err).ok();
                    }
                } else {
                    AlertMessage::alert(
                        "Need rule_levels.txt file to use --level-tuning option [default: ./rules/config/level_tuning.txt]",
                    )
                    .ok();
                }
                return;
            }
            Action::SetDefaultProfile(_) => {
                if let Err(e) = set_default_profile(
                    check_setting_path(
                        &CURRENT_EXE_PATH.to_path_buf(),
                        "config/default_profile.yaml",
                        true,
                    )
                    .unwrap()
                    .to_str()
                    .unwrap(),
                    check_setting_path(
                        &CURRENT_EXE_PATH.to_path_buf(),
                        "config/profiles.yaml",
                        true,
                    )
                    .unwrap()
                    .to_str()
                    .unwrap(),
                    stored_static,
                ) {
                    AlertMessage::alert(&e).ok();
                } else {
                    println!("Successfully updated the default profile.");
                }
                return;
            }
            Action::ListProfiles(_) => {
                let profile_list =
                    options::profile::get_profile_list("config/profiles.yaml", stored_static);
                write_color_buffer(
                    &BufferWriter::stdout(ColorChoice::Always),
                    None,
                    "List of available profiles:",
                    true,
                )
                .ok();
                for profile in profile_list.iter() {
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        Some(Color::Green),
                        &format!("- {:<25}", &format!("{}:", profile[0])),
                        false,
                    )
                    .ok();
                    write_color_buffer(
                        &BufferWriter::stdout(ColorChoice::Always),
                        None,
                        &profile[1],
                        true,
                    )
                    .ok();
                }
                println!();
                return;
            }
        }

        // 処理時間の出力
        let analysis_end_time: DateTime<Local> = Local::now();
        let analysis_duration = analysis_end_time.signed_duration_since(analysis_start_time);
        let elapsed_output_str = format!("Elapsed time: {}", &analysis_duration.hhmmssxxx());
        output_and_data_stack_for_html(
            &elapsed_output_str,
            "General Overview {#general_overview}",
            stored_static.html_report_flag,
        );

        // Qオプションを付けた場合もしくはパースのエラーがない場合はerrorのstackが0となるのでエラーログファイル自体が生成されない。
        if ERROR_LOG_STACK.lock().unwrap().len() > 0 {
            AlertMessage::create_error_log(stored_static.quiet_errors_flag);
        }

        // Debugフラグをつけていた時にはメモリ利用情報などの統計情報を画面に出力する
        if stored_static.config.debug {
            CHECKPOINT.lock().as_ref().unwrap().output_stocked_result();
            println!();
            println!("Memory usage stats:");
            unsafe {
                mi_stats_print_out(None, null_mut());
            }
        }
        println!();
    }

    fn analysis_start(
        &mut self,
        target_extensions: &HashSet<String>,
        time_filter: &TargetEventTime,
        stored_static: &StoredStatic,
    ) {
        if stored_static.output_option.is_none() {
        } else if stored_static
            .output_option
            .as_ref()
            .unwrap()
            .input_args
            .live_analysis
        {
            let live_analysis_list =
                self.collect_liveanalysis_files(target_extensions, stored_static);
            if live_analysis_list.is_none() {
                return;
            }
            self.analysis_files(
                live_analysis_list.unwrap(),
                time_filter,
                &stored_static.event_timeline_config,
                &stored_static.target_eventids,
                stored_static,
            );
        } else if let Some(directory) = &stored_static
            .output_option
            .as_ref()
            .unwrap()
            .input_args
            .directory
        {
            let evtx_files = Self::collect_evtxfiles(
                directory.as_os_str().to_str().unwrap(),
                target_extensions,
                stored_static,
            );
            if evtx_files.is_empty() {
                AlertMessage::alert("No .evtx files were found.").ok();
                return;
            }
            self.analysis_files(
                evtx_files,
                time_filter,
                &stored_static.event_timeline_config,
                &stored_static.target_eventids,
                stored_static,
            );
        } else {
            // directory, live_analysis以外はfilepathの指定の場合
            if let Some(filepath) = &stored_static
                .output_option
                .as_ref()
                .unwrap()
                .input_args
                .filepath
            {
                let mut replaced_filepath = filepath.display().to_string();
                if replaced_filepath.starts_with('"') {
                    replaced_filepath.remove(0);
                }
                if replaced_filepath.ends_with('"') {
                    replaced_filepath.remove(replaced_filepath.len() - 1);
                }
                let check_path = Path::new(&replaced_filepath);
                if !check_path.exists() {
                    AlertMessage::alert(&format!(
                        " The file {} does not exist. Please specify a valid file path.",
                        filepath.as_os_str().to_str().unwrap()
                    ))
                    .ok();
                    return;
                }
                if !target_extensions.contains(
                    check_path
                        .extension()
                        .unwrap_or_else(|| OsStr::new("."))
                        .to_str()
                        .unwrap(),
                ) || check_path
                    .file_stem()
                    .unwrap_or_else(|| OsStr::new("."))
                    .to_str()
                    .unwrap()
                    .trim()
                    .starts_with('.')
                {
                    AlertMessage::alert(
                        "--filepath only accepts .evtx files. Hidden files are ignored.",
                    )
                    .ok();
                    return;
                }
                self.analysis_files(
                    vec![check_path.to_path_buf()],
                    time_filter,
                    &stored_static.event_timeline_config,
                    &stored_static.target_eventids,
                    stored_static,
                );
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn collect_liveanalysis_files(
        &self,
        _target_extensions: &HashSet<String>,
        _stored_static: &StoredStatic,
    ) -> Option<Vec<PathBuf>> {
        AlertMessage::alert("-l / --liveanalysis needs to be run as Administrator on Windows.")
            .ok();
        println!();
        None
    }

    #[cfg(target_os = "windows")]
    fn collect_liveanalysis_files(
        &self,
        target_extensions: &HashSet<String>,
        stored_static: &StoredStatic,
    ) -> Option<Vec<PathBuf>> {
        if is_elevated() {
            let log_dir = env::var("windir").expect("windir is not found");
            let evtx_files = Self::collect_evtxfiles(
                &[log_dir, "System32\\winevt\\Logs".to_string()].join("/"),
                target_extensions,
                stored_static,
            );
            if evtx_files.is_empty() {
                AlertMessage::alert("No .evtx files were found.").ok();
                return None;
            }
            Some(evtx_files)
        } else {
            AlertMessage::alert("-l / --liveanalysis needs to be run as Administrator on Windows.")
                .ok();
            println!();
            None
        }
    }

    fn collect_evtxfiles(
        dir_path: &str,
        target_extensions: &HashSet<String>,
        stored_static: &StoredStatic,
    ) -> Vec<PathBuf> {
        let mut dirpath = dir_path.to_string();
        if dirpath.starts_with('"') {
            dirpath.remove(0);
        }
        if dirpath.ends_with('"') {
            dirpath.remove(dirpath.len() - 1);
        }
        let entries = fs::read_dir(dirpath);
        if entries.is_err() {
            let errmsg = format!("{}", entries.unwrap_err());
            if stored_static.verbose_flag {
                AlertMessage::alert(&errmsg).ok();
            }
            if !stored_static.quiet_errors_flag {
                ERROR_LOG_STACK
                    .lock()
                    .unwrap()
                    .push(format!("[ERROR] {errmsg}"));
            }
            return vec![];
        }

        let mut ret = vec![];
        for e in entries.unwrap() {
            if e.is_err() {
                continue;
            }

            let path = e.unwrap().path();
            if path.is_dir() {
                path.to_str().map(|path_str| {
                    let subdir_ret =
                        Self::collect_evtxfiles(path_str, target_extensions, stored_static);
                    ret.extend(subdir_ret);
                    Option::Some(())
                });
            } else if target_extensions.contains(
                path.extension()
                    .unwrap_or_else(|| OsStr::new(""))
                    .to_str()
                    .unwrap(),
            ) && !path
                .file_stem()
                .unwrap_or_else(|| OsStr::new("."))
                .to_str()
                .unwrap()
                .starts_with('.')
            {
                ret.push(path);
            }
        }

        ret
    }

    fn print_contributors(&self) {
        match fs::read_to_string(
            utils::check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), "contributors.txt", true)
                .unwrap(),
        ) {
            Ok(contents) => {
                write_color_buffer(
                    &BufferWriter::stdout(ColorChoice::Always),
                    None,
                    &contents,
                    true,
                )
                .ok();
            }
            Err(err) => {
                AlertMessage::alert(&format!("{err}")).ok();
            }
        }
    }

    fn analysis_files(
        &mut self,
        evtx_files: Vec<PathBuf>,
        time_filter: &TargetEventTime,
        event_timeline_config: &EventInfoConfig,
        target_event_ids: &TargetEventIds,
        stored_static: &StoredStatic,
    ) {
        let level = stored_static
            .output_option
            .as_ref()
            .unwrap()
            .min_level
            .to_uppercase();
        let target_level = stored_static
            .output_option
            .as_ref()
            .unwrap()
            .exact_level
            .as_ref()
            .unwrap_or(&String::default())
            .to_uppercase();
        write_color_buffer(
            &BufferWriter::stdout(ColorChoice::Always),
            None,
            &format!("Total event log files: {:?}", evtx_files.len()),
            true,
        )
        .ok();

        let mut total_file_size = ByteSize::b(0);
        for file_path in &evtx_files {
            let file_size = match fs::metadata(file_path) {
                Ok(res) => res.len(),
                Err(err) => {
                    if stored_static.verbose_flag {
                        AlertMessage::warn(&err.to_string()).ok();
                    }
                    if !stored_static.quiet_errors_flag {
                        ERROR_LOG_STACK
                            .lock()
                            .unwrap()
                            .push(format!("[WARN] {err}"));
                    }
                    0
                }
            };
            total_file_size += ByteSize::b(file_size);
        }
        let total_size_output = format!("Total file size: {}", total_file_size.to_string_as(false));
        println!("{total_size_output}");
        println!();
        if !(stored_static.metrics_flag
            || stored_static.logon_summary_flag
            || stored_static.search_flag)
        {
            println!("Loading detections rules. Please wait.");
            println!();
        }

        if stored_static.html_report_flag {
            let mut output_data = Nested::<String>::new();
            output_data.extend(vec![
                format!("- Analyzed event files: {}", evtx_files.len()),
                format!("- {total_size_output}"),
            ]);
            htmlreport::add_md_data("General Overview #{general_overview}", output_data);
        }

        let rule_files = detection::Detection::parse_rule_files(
            &level,
            &target_level,
            &stored_static.output_option.as_ref().unwrap().rules,
            &filter::exclude_ids(stored_static),
            stored_static,
        );
        CHECKPOINT
            .lock()
            .as_mut()
            .unwrap()
            .rap_check_point("Rule Parse Processing Time");

        if rule_files.is_empty() {
            AlertMessage::alert(
                "No rules were loaded. Please download the latest rules with the update-rules command.\r\n",
            )
            .ok();
            return;
        }

        let mut pb = ProgressBar::new(evtx_files.len() as u64);
        pb.show_speed = false;
        self.rule_keys = self.get_all_keys(&rule_files);
        let mut detection = detection::Detection::new(rule_files);
        let mut total_records: usize = 0;
        let mut tl = Timeline::new();

        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        for evtx_file in evtx_files {
            if stored_static.verbose_flag {
                println!("Checking target evtx FilePath: {:?}", &evtx_file);
            }
            let cnt_tmp: usize;
            (detection, cnt_tmp, tl) = if evtx_file.extension().unwrap() == "json" {
                self.analysis_json_file(
                    evtx_file,
                    detection,
                    time_filter,
                    tl.to_owned(),
                    target_event_ids,
                    stored_static,
                )
            } else {
                self.analysis_file(
                    evtx_file,
                    detection,
                    time_filter,
                    tl.to_owned(),
                    target_event_ids,
                    stored_static,
                )
            };
            total_records += cnt_tmp;
            pb.inc();
        }
        CHECKPOINT
            .lock()
            .as_mut()
            .unwrap()
            .rap_check_point("Analysis Processing Time");
        if stored_static.metrics_flag {
            tl.tm_stats_dsp_msg(event_timeline_config, stored_static);
        } else if stored_static.logon_summary_flag {
            tl.tm_logon_stats_dsp_msg(stored_static);
        } else if stored_static.search_flag {
            tl.search_dsp_msg(event_timeline_config, stored_static);
        }
        if stored_static.output_path.is_some() {
            println!("\n\nScanning finished. Please wait while the results are being saved.");
        }
        println!();
        detection.add_aggcondition_msges(&self.rt, stored_static);
        if !(stored_static.metrics_flag
            || stored_static.logon_summary_flag
            || stored_static.search_flag
            || stored_static.pivot_keyword_list_flag)
        {
            after_fact(
                total_records,
                &stored_static.output_path,
                stored_static.common_options.no_color,
                stored_static,
                tl,
            );
        }
        CHECKPOINT
            .lock()
            .as_mut()
            .unwrap()
            .rap_check_point("Output Processing Time");
    }

    // Windowsイベントログファイルを1ファイル分解析する。
    fn analysis_file(
        &self,
        evtx_filepath: PathBuf,
        mut detection: detection::Detection,
        time_filter: &TargetEventTime,
        mut tl: Timeline,
        target_event_ids: &TargetEventIds,
        stored_static: &StoredStatic,
    ) -> (detection::Detection, usize, Timeline) {
        let path = evtx_filepath.display();
        let parser = self.evtx_to_jsons(&evtx_filepath);
        let mut record_cnt = 0;
        if parser.is_none() {
            return (detection, record_cnt, tl);
        }

        let mut parser = parser.unwrap();
        let mut records = parser.records_json_value();

        let verbose_flag = stored_static.verbose_flag;
        let quiet_errors_flag = stored_static.quiet_errors_flag;
        loop {
            let mut records_per_detect = vec![];
            while records_per_detect.len() < MAX_DETECT_RECORDS {
                // パースに失敗している場合、エラーメッセージを出力
                let next_rec = records.next();
                if next_rec.is_none() {
                    break;
                }
                record_cnt += 1;

                let record_result = next_rec.unwrap();
                if record_result.is_err() {
                    let evtx_filepath = &path;
                    let errmsg = format!(
                        "Failed to parse event file. EventFile:{} Error:{}",
                        evtx_filepath,
                        record_result.unwrap_err()
                    );
                    if verbose_flag {
                        AlertMessage::alert(&errmsg).ok();
                    }
                    if !quiet_errors_flag {
                        ERROR_LOG_STACK
                            .lock()
                            .unwrap()
                            .push(format!("[ERROR] {errmsg}"));
                    }
                    continue;
                }

                let data = &record_result.as_ref().unwrap().data;
                // Searchならすべてのフィルタを無視
                if !stored_static.search_flag {
                    // channelがnullである場合とEventID Filter optionが指定されていない場合は、target_eventids.txtでイベントIDベースでフィルタする。
                    if !self._is_valid_channel(
                        data,
                        &stored_static.eventkey_alias,
                        "Event.System.Channel",
                    ) || (stored_static.output_option.as_ref().unwrap().eid_filter
                        && !self._is_target_event_id(
                            data,
                            target_event_ids,
                            &stored_static.eventkey_alias,
                        ))
                    {
                        continue;
                    }

                    // EventID側の条件との条件の混同を防ぐため時間でのフィルタリングの条件分岐を分離した
                    let timestamp = record_result.as_ref().unwrap().timestamp;
                    if !time_filter.is_target(&Some(timestamp)) {
                        continue;
                    }
                }

                records_per_detect.push(data.to_owned());
            }
            if records_per_detect.is_empty() {
                break;
            }

            let records_per_detect = self.rt.block_on(App::create_rec_infos(
                records_per_detect,
                &path,
                self.rule_keys.to_owned(),
            ));

            // timeline機能の実行
            tl.start(&records_per_detect, stored_static);

            if !(stored_static.metrics_flag
                || stored_static.logon_summary_flag
                || stored_static.search_flag)
            {
                // ruleファイルの検知
                detection = detection.start(&self.rt, records_per_detect);
            }
        }

        (detection, record_cnt, tl)
    }

    // JSON形式のイベントログファイルを1ファイル分解析する。
    fn analysis_json_file(
        &self,
        filepath: PathBuf,
        mut detection: detection::Detection,
        time_filter: &TargetEventTime,
        mut tl: Timeline,
        target_event_ids: &TargetEventIds,
        stored_static: &StoredStatic,
    ) -> (detection::Detection, usize, Timeline) {
        let path = filepath.display();
        let mut record_cnt = 0;
        let filename = filepath.to_str().unwrap_or_default();
        let filepath = if filename.starts_with("./") {
            check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), filename, true)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        } else {
            filename.to_string()
        };
        let jsonl_value_iter = utils::read_jsonl_to_value(&filepath);
        let mut records = match jsonl_value_iter {
            // JSONL形式の場合
            Ok(values) => values,
            // JSONL形式以外(JSON(Array or jq)形式)の場合
            Err(_) => {
                let json_value_iter = utils::read_json_to_value(&filepath);
                match json_value_iter {
                    Ok(values) => values,
                    Err(e) => {
                        AlertMessage::alert(&e).ok();
                        return (detection, record_cnt, tl);
                    }
                }
            }
        };

        loop {
            let mut records_per_detect = vec![];
            while records_per_detect.len() < MAX_DETECT_RECORDS {
                // パースに失敗している場合、エラーメッセージを出力
                let next_rec = records.next();
                if next_rec.is_none() {
                    break;
                }
                record_cnt += 1;

                let mut data = next_rec.unwrap();
                // ChannelなどのデータはEvent -> Systemに存在する必要があるが、他処理のことも考え、Event -> EventDataのデータをそのまま投入する形にした。cloneを利用しているのはCopy trait実装がserde_json::Valueにないため
                data["Event"]["System"] = data["Event"]["EventData"].clone();
                data["Event"]["System"]
                    .as_object_mut()
                    .unwrap()
                    .insert("EventRecordID".to_string(), Value::from(1));
                data["Event"]["System"].as_object_mut().unwrap().insert(
                    "Provider_attributes".to_string(),
                    Value::Object(Map::from_iter(vec![("Name".to_string(), Value::from(1))])),
                );

                data["Event"]["System"]["EventRecordID"] =
                    data["Event"]["EventData"]["RecordNumber"].clone();
                data["Event"]["System"]["Provider_attributes"]["Name"] =
                    data["Event"]["EventData"]["SourceName"].clone();
                data["Event"]["UserData"] = data["Event"]["EventData"].clone();
                // Computer名に対応する内容はHostnameであることがわかったためデータをクローンして投入
                data["Event"]["System"]["Computer"] =
                    data["Event"]["EventData"]["Hostname"].clone();
                // channelがnullである場合とEventID Filter optionが指定されていない場合は、target_eventids.txtでイベントIDベースでフィルタする。
                if !self._is_valid_channel(
                    &data,
                    &stored_static.eventkey_alias,
                    "Event.EventData.Channel",
                ) || (stored_static.output_option.as_ref().unwrap().eid_filter
                    && !self._is_target_event_id(
                        &data,
                        target_event_ids,
                        &stored_static.eventkey_alias,
                    ))
                {
                    continue;
                }
                let target_timestamp = if data["Event"]["EventData"]["@timestamp"].is_null() {
                    &data["Event"]["EventData"]["TimeGenerated"]
                } else {
                    &data["Event"]["EventData"]["@timestamp"]
                };
                // EventID側の条件との条件の混同を防ぐため時間でのフィルタリングの条件分岐を分離した
                let timestamp = match NaiveDateTime::parse_from_str(
                    &target_timestamp
                        .to_string()
                        .replace("\\\"", "")
                        .replace('"', ""),
                    "%Y-%m-%dT%H:%M:%S%.3fZ",
                ) {
                    Ok(without_timezone_datetime) => {
                        Some(DateTime::<Utc>::from_utc(without_timezone_datetime, Utc))
                    }
                    Err(e) => {
                        AlertMessage::alert(&format!(
                            "timestamp parse error. filepath:{},{} {}",
                            path,
                            &data["Event"]["EventData"]["@timestamp"]
                                .to_string()
                                .replace("\\\"", "")
                                .replace('"', ""),
                            e
                        ))
                        .ok();
                        None
                    }
                };
                if !time_filter.is_target(&timestamp) {
                    continue;
                }

                records_per_detect.push(data.to_owned());
            }
            if records_per_detect.is_empty() {
                break;
            }

            let records_per_detect = self.rt.block_on(App::create_rec_infos(
                records_per_detect,
                &path,
                self.rule_keys.to_owned(),
            ));

            // timeline機能の実行
            tl.start(&records_per_detect, stored_static);

            // 以下のコマンドの際にはルールにかけない
            if !(stored_static.metrics_flag
                || stored_static.logon_summary_flag
                || stored_static.search_flag)
            {
                // ruleファイルの検知
                detection = detection.start(&self.rt, records_per_detect);
            }
        }

        (detection, record_cnt, tl)
    }

    async fn create_rec_infos(
        records_per_detect: Vec<Value>,
        path: &dyn Display,
        rule_keys: Nested<String>,
    ) -> Vec<EvtxRecordInfo> {
        let path = Arc::new(path.to_string());
        let rule_keys = Arc::new(rule_keys);
        let threads: Vec<JoinHandle<EvtxRecordInfo>> = {
            let this = records_per_detect
                .into_iter()
                .map(|rec| -> JoinHandle<EvtxRecordInfo> {
                    let arc_rule_keys = Arc::clone(&rule_keys);
                    let arc_path = Arc::clone(&path);
                    spawn(async move {
                        utils::create_rec_info(rec, arc_path.to_string(), &arc_rule_keys)
                    })
                });
            FromIterator::from_iter(this)
        };

        let mut ret = vec![];
        for thread in threads.into_iter() {
            ret.push(thread.await.unwrap());
        }

        ret
    }

    fn get_all_keys(&self, rules: &[RuleNode]) -> Nested<String> {
        let mut key_set = HashSet::new();
        for rule in rules {
            let keys = get_detection_keys(rule);
            key_set.extend(keys.iter().map(|x| x.to_string()));
        }

        key_set.into_iter().collect::<Nested<String>>()
    }

    /// target_eventids.txtの設定を元にフィルタする。 trueであれば検知確認対象のEventIDであることを意味する。
    fn _is_target_event_id(
        &self,
        data: &Value,
        target_event_ids: &TargetEventIds,
        eventkey_alias: &EventKeyAliasConfig,
    ) -> bool {
        let eventid = utils::get_event_value(&utils::get_event_id_key(), data, eventkey_alias);
        if eventid.is_none() {
            return true;
        }

        match eventid.unwrap() {
            Value::String(s) => target_event_ids.is_target(&s.replace('\"', "")),
            Value::Number(n) => target_event_ids.is_target(&n.to_string().replace('\"', "")),
            _ => true, // レコードからEventIdが取得できない場合は、特にフィルタしない
        }
    }

    /// レコードのチャンネルの値が正しい(Stringの形でありnullでないもの)ことを判定する関数
    fn _is_valid_channel(
        &self,
        data: &Value,
        eventkey_alias: &EventKeyAliasConfig,
        channel_key: &str,
    ) -> bool {
        let channel = utils::get_event_value(channel_key, data, eventkey_alias);
        if channel.is_none() {
            return false;
        }
        match channel.unwrap() {
            Value::String(s) => s != "null",
            _ => false, // channelの値は文字列を想定しているため、それ以外のデータが来た場合はfalseを返す
        }
    }

    fn evtx_to_jsons(&self, evtx_filepath: &PathBuf) -> Option<EvtxParser<File>> {
        match EvtxParser::from_path(evtx_filepath) {
            Ok(evtx_parser) => {
                // parserのデフォルト設定を変更
                let mut parse_config = ParserSettings::default();
                parse_config = parse_config.separate_json_attributes(true); // XMLのattributeをJSONに変換する時のルールを設定
                parse_config = parse_config.num_threads(0); // 設定しないと遅かったので、設定しておく。

                let evtx_parser = evtx_parser.with_configuration(parse_config);
                Option::Some(evtx_parser)
            }
            Err(e) => {
                eprintln!("{e}");
                Option::None
            }
        }
    }

    /// output logo
    fn output_logo(&self, stored_static: &StoredStatic) {
        let fp = utils::check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), "art/logo.txt", true)
            .unwrap();
        let content = fs::read_to_string(fp).unwrap_or_default();
        let output_color = if stored_static.common_options.no_color {
            None
        } else {
            Some(Color::Green)
        };
        write_color_buffer(
            &BufferWriter::stdout(ColorChoice::Always),
            output_color,
            &content,
            true,
        )
        .ok();
    }

    /// output easter egg arts
    fn output_eggs(&self, exec_datestr: &str) {
        let mut eggs: HashMap<&str, (&str, Color)> = HashMap::new();
        eggs.insert("01/01", ("art/happynewyear.txt", Color::Rgb(255, 0, 0))); // Red
        eggs.insert("02/22", ("art/ninja.txt", Color::Rgb(0, 171, 240))); // Cerulean
        eggs.insert("08/08", ("art/takoyaki.txt", Color::Rgb(181, 101, 29))); // Light Brown
        eggs.insert("12/24", ("art/christmas.txt", Color::Rgb(70, 192, 22))); // Green
        eggs.insert("12/25", ("art/christmas.txt", Color::Rgb(70, 192, 22))); // Green

        match eggs.get(exec_datestr) {
            None => {}
            Some((path, color)) => {
                let egg_path =
                    utils::check_setting_path(&CURRENT_EXE_PATH.to_path_buf(), path, true).unwrap();
                let content = fs::read_to_string(egg_path).unwrap_or_default();
                write_color_buffer(
                    &BufferWriter::stdout(ColorChoice::Always),
                    Some(color.to_owned()),
                    &content,
                    true,
                )
                .ok();
            }
        }
    }

    /// check architecture
    fn is_matched_architecture_and_binary(&self) -> bool {
        if cfg!(target_os = "windows") {
            let is_processor_arch_32bit = env::var_os("PROCESSOR_ARCHITECTURE")
                .unwrap_or_default()
                .eq("x86");
            // PROCESSOR_ARCHITEW6432は32bit環境には存在しないため、環境変数存在しなかった場合は32bit環境であると判断する
            let not_wow_flag = env::var_os("PROCESSOR_ARCHITEW6432")
                .unwrap_or_else(|| OsString::from("x86"))
                .eq("x86");
            return (cfg!(target_pointer_width = "64") && !is_processor_arch_32bit)
                || (cfg!(target_pointer_width = "32") && is_processor_arch_32bit && not_wow_flag);
        }
        true
    }

    fn check_is_valid_args_num(&self, action: Option<&Action>) -> bool {
        match action.as_ref().unwrap() {
            Action::CsvTimeline(_)
            | Action::JsonTimeline(_)
            | Action::LogonSummary(_)
            | Action::Metrics(_)
            | Action::PivotKeywordsList(_)
            | Action::SetDefaultProfile(_) => std::env::args().len() != 2,
            Action::Search(opt) => {
                std::env::args().len() != 2 && (opt.keywords.is_some() ^ opt.regex.is_some())
                // key word and regex are conflict
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, remove_file, File},
        path::Path,
    };

    use crate::App;
    use chrono::Local;
    use hashbrown::HashSet;
    use hayabusa::{
        detections::{
            configs::{
                Action, CommonOptions, Config, ConfigReader, CsvOutputOption, DetectCommonOption,
                InputOption, JSONOutputOption, LogonSummaryOption, MetricsOption, OutputOption,
                StoredStatic, TargetEventIds, TargetEventTime, STORED_EKEY_ALIAS, STORED_STATIC,
            },
            detection,
            message::{MESSAGEKEYS, MESSAGES},
            rule::create_rule,
        },
        options::htmlreport::HTML_REPORTER,
        timeline::timelines::Timeline,
    };
    use itertools::Itertools;
    use yaml_rust::YamlLoader;

    fn create_dummy_stored_static() -> StoredStatic {
        StoredStatic::create_static_data(Some(Config {
            action: Some(Action::CsvTimeline(CsvOutputOption {
                output_options: OutputOption {
                    input_args: InputOption {
                        directory: None,
                        filepath: None,
                        live_analysis: false,
                    },
                    profile: None,
                    enable_deprecated_rules: false,
                    exclude_status: None,
                    min_level: "informational".to_string(),
                    exact_level: None,
                    enable_noisy_rules: false,
                    end_timeline: None,
                    start_timeline: None,
                    eid_filter: false,
                    european_time: false,
                    iso_8601: false,
                    rfc_2822: false,
                    rfc_3339: false,
                    us_military_time: false,
                    us_time: false,
                    utc: false,
                    visualize_timeline: false,
                    rules: Path::new("./rules").to_path_buf(),
                    html_report: None,
                    no_summary: false,
                    common_options: CommonOptions {
                        no_color: false,
                        quiet: false,
                    },
                    detect_common_options: DetectCommonOption {
                        evtx_file_ext: None,
                        thread_number: None,
                        quiet_errors: false,
                        config: Path::new("./rules/config").to_path_buf(),
                        verbose: false,
                        json_input: true,
                    },
                    enable_unsupported_rules: false,
                    clobber: false,
                },
                geo_ip: None,
                output: None,
                multiline: false,
            })),
            debug: false,
        }))
    }

    #[test]
    fn test_collect_evtxfiles() {
        let files = App::collect_evtxfiles(
            "test_files/evtx",
            &HashSet::from(["evtx".to_string()]),
            &create_dummy_stored_static(),
        );
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

    #[test]
    fn test_exec_none_storedstatic() {
        let mut app = App::new(None);
        let mut config_reader = ConfigReader::new();
        let mut stored_static = StoredStatic::create_static_data(config_reader.config);
        config_reader.config = None;
        stored_static.profiles = None;
        app.exec(&mut config_reader.app, &mut stored_static);
    }

    #[test]
    fn test_exec_general_html_output() {
        let mut app = App::new(None);
        let mut config_reader = ConfigReader::new();
        let mut stored_static = StoredStatic::create_static_data(config_reader.config);
        config_reader.config = None;
        stored_static.config.action = None;
        stored_static.html_report_flag = true;
        app.exec(&mut config_reader.app, &mut stored_static);
        let expect_general_contents = vec![
            format!("- Command line: {}", std::env::args().join(" ")),
            format!("- Start time: {}", Local::now().format("%Y/%m/%d %H:%M")),
        ];

        let actual = &HTML_REPORTER.read().unwrap().md_datas;
        let general_contents = actual.get("General Overview {#general_overview}").unwrap();
        assert_eq!(expect_general_contents.len(), general_contents.len());

        for actual_general_contents in general_contents.iter() {
            assert!(expect_general_contents.contains(&actual_general_contents.to_string()));
        }
    }

    #[test]
    fn test_analysis_json_file() {
        let mut app = App::new(None);
        let stored_static = create_dummy_stored_static();
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());

        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'Microsoft-Windows-Sysmon/Operational'
            condition: selection1
        details: testdata
        "#;
        let mut rule_yaml = YamlLoader::load_from_str(rule_str).unwrap().into_iter();
        let test_yaml_data = rule_yaml.next().unwrap();
        let mut rule = create_rule("testpath".to_string(), test_yaml_data);
        let rule_init = rule.init(&stored_static);
        assert!(rule_init.is_ok());
        let rule_files = vec![rule];
        app.rule_keys = app.get_all_keys(&rule_files);
        let detection = detection::Detection::new(rule_files);
        let target_time_filter = TargetEventTime::new(&stored_static);
        let tl = Timeline::default();
        let target_event_ids = TargetEventIds::default();

        let actual = app.analysis_json_file(
            Path::new("test_files/evtx/test.jsonl").to_path_buf(),
            detection,
            &target_time_filter,
            tl,
            &target_event_ids,
            &stored_static,
        );
        assert_eq!(actual.1, 2);
        assert_eq!(MESSAGES.len(), 2);
    }

    #[test]
    fn test_same_file_output_csv_exit() {
        MESSAGES.clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite.csv").ok();
        let action = Action::CsvTimeline(CsvOutputOption {
            output_options: OutputOption {
                input_args: InputOption {
                    directory: None,
                    filepath: Some(Path::new("test_files/evtx/test.json").to_path_buf()),
                    live_analysis: false,
                },
                profile: None,
                enable_deprecated_rules: false,
                exclude_status: None,
                min_level: "informational".to_string(),
                exact_level: None,
                enable_noisy_rules: false,
                end_timeline: None,
                start_timeline: None,
                eid_filter: false,
                european_time: false,
                iso_8601: false,
                rfc_2822: false,
                rfc_3339: false,
                us_military_time: false,
                us_time: false,
                utc: false,
                visualize_timeline: false,
                rules: Path::new("./test_files/rules/yaml/test_json_detect.yml").to_path_buf(),
                html_report: None,
                no_summary: true,
                common_options: CommonOptions {
                    no_color: false,
                    quiet: false,
                },
                detect_common_options: DetectCommonOption {
                    evtx_file_ext: None,
                    thread_number: None,
                    quiet_errors: false,
                    config: Path::new("./rules/config").to_path_buf(),
                    verbose: false,
                    json_input: true,
                },
                enable_unsupported_rules: false,
                clobber: false,
            },
            geo_ip: None,
            output: Some(Path::new("overwrite.csv").to_path_buf()),
            multiline: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        assert_eq!(MESSAGES.len(), 0);

        // テストファイルの作成
        remove_file("overwrite.csv").ok();
    }

    #[test]
    fn test_overwrite_csv() {
        MESSAGES.clear();
        MESSAGEKEYS.lock().unwrap().clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite.csv").ok();
        let action = Action::CsvTimeline(CsvOutputOption {
            output_options: OutputOption {
                input_args: InputOption {
                    directory: None,
                    filepath: Some(Path::new("test_files/evtx/test.json").to_path_buf()),
                    live_analysis: false,
                },
                profile: None,
                enable_deprecated_rules: false,
                exclude_status: None,
                min_level: "informational".to_string(),
                exact_level: None,
                enable_noisy_rules: false,
                end_timeline: None,
                start_timeline: None,
                eid_filter: false,
                european_time: false,
                iso_8601: false,
                rfc_2822: false,
                rfc_3339: false,
                us_military_time: false,
                us_time: false,
                utc: false,
                visualize_timeline: false,
                rules: Path::new("test_files/rules/yaml/test_json_detect.yml").to_path_buf(),
                html_report: None,
                no_summary: true,
                common_options: CommonOptions {
                    no_color: false,
                    quiet: false,
                },
                detect_common_options: DetectCommonOption {
                    evtx_file_ext: None,
                    thread_number: None,
                    quiet_errors: false,
                    config: Path::new("./rules/config").to_path_buf(),
                    verbose: false,
                    json_input: true,
                },
                enable_unsupported_rules: false,
                clobber: true,
            },
            geo_ip: None,
            output: Some(Path::new("overwrite.csv").to_path_buf()),
            multiline: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        assert_ne!(MESSAGES.len(), 0);
        // テストファイルの作成
        remove_file("overwrite.csv").ok();
    }

    #[test]
    fn test_same_file_output_json_exit() {
        MESSAGES.clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite.json").ok();
        let action = Action::JsonTimeline(JSONOutputOption {
            output_options: OutputOption {
                input_args: InputOption {
                    directory: None,
                    filepath: Some(Path::new("test_files/evtx/test.json").to_path_buf()),
                    live_analysis: false,
                },
                profile: None,
                enable_deprecated_rules: false,
                exclude_status: None,
                min_level: "informational".to_string(),
                exact_level: None,
                enable_noisy_rules: false,
                end_timeline: None,
                start_timeline: None,
                eid_filter: false,
                european_time: false,
                iso_8601: false,
                rfc_2822: false,
                rfc_3339: false,
                us_military_time: false,
                us_time: false,
                utc: false,
                visualize_timeline: false,
                rules: Path::new("./test_files/rules/yaml/test_json_detect.yml").to_path_buf(),
                html_report: None,
                no_summary: true,
                common_options: CommonOptions {
                    no_color: false,
                    quiet: false,
                },
                detect_common_options: DetectCommonOption {
                    evtx_file_ext: None,
                    thread_number: None,
                    quiet_errors: false,
                    config: Path::new("./rules/config").to_path_buf(),
                    verbose: false,
                    json_input: true,
                },
                enable_unsupported_rules: false,
                clobber: false,
            },
            geo_ip: None,
            output: Some(Path::new("overwrite.json").to_path_buf()),
            jsonl_timeline: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        assert_eq!(MESSAGES.len(), 0);

        // テストファイルの作成
        remove_file("overwrite.json").ok();
    }

    #[test]
    fn test_overwrite_json() {
        MESSAGES.clear();
        MESSAGEKEYS.lock().unwrap().clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite.csv").ok();
        let action = Action::JsonTimeline(JSONOutputOption {
            output_options: OutputOption {
                input_args: InputOption {
                    directory: None,
                    filepath: Some(Path::new("test_files/evtx/test.json").to_path_buf()),
                    live_analysis: false,
                },
                profile: None,
                enable_deprecated_rules: false,
                exclude_status: None,
                min_level: "informational".to_string(),
                exact_level: None,
                enable_noisy_rules: false,
                end_timeline: None,
                start_timeline: None,
                eid_filter: false,
                european_time: false,
                iso_8601: false,
                rfc_2822: false,
                rfc_3339: false,
                us_military_time: false,
                us_time: false,
                utc: false,
                visualize_timeline: false,
                rules: Path::new("test_files/rules/yaml/test_json_detect.yml").to_path_buf(),
                html_report: None,
                no_summary: true,
                common_options: CommonOptions {
                    no_color: false,
                    quiet: false,
                },
                detect_common_options: DetectCommonOption {
                    evtx_file_ext: None,
                    thread_number: None,
                    quiet_errors: false,
                    config: Path::new("./rules/config").to_path_buf(),
                    verbose: false,
                    json_input: true,
                },
                enable_unsupported_rules: false,
                clobber: true,
            },
            geo_ip: None,
            output: Some(Path::new("overwrite.json").to_path_buf()),
            jsonl_timeline: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        assert_ne!(MESSAGES.len(), 0);
        // テストファイルの削除
        remove_file("overwrite.json").ok();
    }

    #[test]
    fn test_same_file_output_metric_csv_exit() {
        MESSAGES.clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite-metric.csv").ok();
        let action = Action::Metrics(MetricsOption {
            output: Some(Path::new("overwrite-metric.csv").to_path_buf()),
            input_args: InputOption {
                directory: None,
                filepath: Some(Path::new("test_files/evtx/test_metrics.json").to_path_buf()),
                live_analysis: false,
            },
            common_options: CommonOptions {
                no_color: false,
                quiet: false,
            },
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: true,
            },
            european_time: false,
            iso_8601: false,
            rfc_2822: false,
            rfc_3339: false,
            us_military_time: false,
            us_time: false,
            utc: false,
            clobber: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        let meta = fs::metadata("overwrite-metric.csv").unwrap();
        assert_eq!(meta.len(), 0);

        // テストファイルの削除
        remove_file("overwrite-metric.csv").ok();
    }

    #[test]
    fn test_same_file_output_metric_csv() {
        MESSAGES.clear();
        MESSAGEKEYS.lock().unwrap().clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite-metric.csv").ok();
        let action = Action::Metrics(MetricsOption {
            output: Some(Path::new("overwrite-metric.csv").to_path_buf()),
            input_args: InputOption {
                directory: None,
                filepath: Some(Path::new("test_files/evtx/test_metrics.json").to_path_buf()),
                live_analysis: false,
            },
            common_options: CommonOptions {
                no_color: false,
                quiet: false,
            },
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: true,
            },
            european_time: false,
            iso_8601: false,
            rfc_2822: false,
            rfc_3339: false,
            us_military_time: false,
            us_time: false,
            utc: false,
            clobber: true,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        let meta = fs::metadata("overwrite-metric.csv").unwrap();
        assert_ne!(meta.len(), 0);
        // テストファイルの削除
        remove_file("overwrite-metric.csv").ok();
    }

    #[test]
    fn test_same_file_output_logon_summary_csv_exit() {
        MESSAGES.clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite-metric-successful.csv").ok();
        let action = Action::LogonSummary(LogonSummaryOption {
            output: Some(Path::new("overwrite-metric").to_path_buf()),
            input_args: InputOption {
                directory: None,
                filepath: Some(Path::new("test_files/evtx/test_metrics.json").to_path_buf()),
                live_analysis: false,
            },
            common_options: CommonOptions {
                no_color: false,
                quiet: false,
            },
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: true,
            },
            european_time: false,
            iso_8601: false,
            rfc_2822: false,
            rfc_3339: false,
            us_military_time: false,
            us_time: false,
            utc: false,
            clobber: false,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        let meta = fs::metadata("overwrite-metric-successful.csv").unwrap();
        assert_eq!(meta.len(), 0);

        // テストファイルの削除
        remove_file("overwrite-metric-successful.csv").ok();
    }

    #[test]
    fn test_same_file_output_logon_summary_csv() {
        MESSAGES.clear();
        MESSAGEKEYS.lock().unwrap().clear();
        // 先に空ファイルを作成する
        let mut app = App::new(None);
        File::create("overwrite-metric-successful.csv").ok();
        let action = Action::LogonSummary(LogonSummaryOption {
            output: Some(Path::new("overwrite-metric").to_path_buf()),
            input_args: InputOption {
                directory: None,
                filepath: Some(Path::new("test_files/evtx/test_metrics.json").to_path_buf()),
                live_analysis: false,
            },
            common_options: CommonOptions {
                no_color: false,
                quiet: false,
            },
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: true,
            },
            european_time: false,
            iso_8601: false,
            rfc_2822: false,
            rfc_3339: false,
            us_military_time: false,
            us_time: false,
            utc: false,
            clobber: true,
        });
        let config = Some(Config {
            action: Some(action),
            debug: false,
        });
        let mut stored_static = StoredStatic::create_static_data(config);
        *STORED_EKEY_ALIAS.write().unwrap() = Some(stored_static.eventkey_alias.clone());
        *STORED_STATIC.write().unwrap() = Some(stored_static.clone());
        let mut config_reader = ConfigReader::new();
        app.exec(&mut config_reader.app, &mut stored_static);
        let meta = fs::metadata("overwrite-metric-successful.csv").unwrap();
        assert_ne!(meta.len(), 0);
        // テストファイルの削除
        remove_file("overwrite-metric-successful.csv").ok();
    }
}
