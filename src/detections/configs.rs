use crate::detections::message::AlertMessage;
use crate::detections::pivot::{PivotKeyword, PIVOT_KEYWORD};
use crate::detections::utils;
use crate::options::geoip_search::GeoIPSearch;
use crate::options::htmlreport;
use crate::options::profile::{load_profile, Profile};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use chrono::{DateTime, Utc};
use clap::{ArgGroup, Args, ColorChoice, Command, CommandFactory, Parser, Subcommand};
use compact_str::CompactString;
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use lazy_static::lazy_static;
use nested::Nested;
use regex::Regex;
use std::env::current_exe;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::{fs, process};
use terminal_size::{terminal_size, Width};
use yaml_rust::{Yaml, YamlLoader};

use super::message::{create_output_filter_config, LEVEL_ABBR_MAP};
use super::utils::check_setting_path;

lazy_static! {
    pub static ref STORED_STATIC: RwLock<Option<StoredStatic>> = RwLock::new(None);
    pub static ref STORED_EKEY_ALIAS: RwLock<Option<EventKeyAliasConfig>> = RwLock::new(None);
    pub static ref GEOIP_DB_PARSER: RwLock<Option<GeoIPSearch>> = RwLock::new(None);
    pub static ref GEOIP_DB_YAML: RwLock<Option<HashMap<CompactString, Yaml>>> = RwLock::new(None);
    pub static ref GEOIP_FILTER: RwLock<Option<Vec<Yaml>>> = RwLock::new(None);
    pub static ref CURRENT_EXE_PATH: PathBuf =
        current_exe().unwrap().parent().unwrap().to_path_buf();
    pub static ref IDS_REGEX: Regex =
        Regex::new(r"^[0-9a-z]{8}-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{12}$").unwrap();
}

pub struct ConfigReader {
    pub app: Command,
    pub config: Option<Config>,
}

impl Default for ConfigReader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct StoredStatic {
    pub config: Config,
    pub config_path: PathBuf,
    pub eventkey_alias: EventKeyAliasConfig,
    pub ch_config: HashMap<CompactString, CompactString>,
    pub disp_abbr_generic: AhoCorasick,
    pub disp_abbr_general_values: Vec<CompactString>,
    pub provider_abbr_config: HashMap<CompactString, CompactString>,
    pub quiet_errors_flag: bool,
    pub verbose_flag: bool,
    pub metrics_flag: bool,
    pub logon_summary_flag: bool,
    pub output_option: Option<OutputOption>,
    pub pivot_keyword_list_flag: bool,
    pub default_details: HashMap<CompactString, CompactString>,
    pub html_report_flag: bool,
    pub profiles: Option<Vec<(CompactString, Profile)>>,
    pub event_timeline_config: EventInfoConfig,
    pub target_eventids: TargetEventIds,
    pub thread_number: Option<usize>,
    pub json_input_flag: bool,
    pub output_path: Option<PathBuf>,
    pub common_options: CommonOptions,
    pub multiline_flag: bool,
}
impl StoredStatic {
    /// main.rsでパースした情報からデータを格納する関数
    pub fn create_static_data(input_config: Option<Config>) -> StoredStatic {
        let action_id = Action::to_usize(input_config.as_ref().unwrap().action.as_ref());
        let quiet_errors_flag = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.output_options.detect_common_options.quiet_errors,
            Some(Action::JsonTimeline(opt)) => {
                opt.output_options.detect_common_options.quiet_errors
            }
            Some(Action::LogonSummary(opt)) => opt.detect_common_options.quiet_errors,
            Some(Action::Metrics(opt)) => opt.detect_common_options.quiet_errors,
            Some(Action::PivotKeywordsList(opt)) => opt.detect_common_options.quiet_errors,
            _ => false,
        };
        let common_options = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.output_options.common_options,
            Some(Action::JsonTimeline(opt)) => opt.output_options.common_options,
            Some(Action::LevelTuning(opt)) => opt.common_options,
            Some(Action::LogonSummary(opt)) => opt.common_options,
            Some(Action::Metrics(opt)) => opt.common_options,
            Some(Action::PivotKeywordsList(opt)) => opt.common_options,
            Some(Action::SetDefaultProfile(opt)) => opt.common_options,
            Some(Action::ListContributors(opt)) | Some(Action::ListProfiles(opt)) => *opt,
            Some(Action::UpdateRules(opt)) => opt.common_options,
            Some(Action::AlertElastic(opt)) => opt.common_options,
            None => CommonOptions {
                no_color: false,
                quiet: false,
            },
        };
        let binding = Path::new("./rules/config").to_path_buf();
        let config_path = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => &opt.output_options.detect_common_options.config,
            Some(Action::JsonTimeline(opt)) => &opt.output_options.detect_common_options.config,
            Some(Action::LogonSummary(opt)) => &opt.detect_common_options.config,
            Some(Action::Metrics(opt)) => &opt.detect_common_options.config,
            Some(Action::PivotKeywordsList(opt)) => &opt.detect_common_options.config,
            _ => &binding,
        };
        let verbose_flag = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.output_options.detect_common_options.verbose,
            Some(Action::JsonTimeline(opt)) => opt.output_options.detect_common_options.verbose,
            Some(Action::LogonSummary(opt)) => opt.detect_common_options.verbose,
            Some(Action::Metrics(opt)) => opt.detect_common_options.verbose,
            Some(Action::PivotKeywordsList(opt)) => opt.detect_common_options.verbose,
            _ => false,
        };
        let json_input_flag = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.output_options.detect_common_options.json_input,
            Some(Action::JsonTimeline(opt)) => opt.output_options.detect_common_options.json_input,
            Some(Action::LogonSummary(opt)) => opt.detect_common_options.json_input,
            Some(Action::Metrics(opt)) => opt.detect_common_options.json_input,
            Some(Action::PivotKeywordsList(opt)) => opt.detect_common_options.json_input,
            _ => false,
        };
        let is_valid_min_level = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => LEVEL_ABBR_MAP
                .keys()
                .any(|level| &opt.output_options.min_level.to_lowercase() == level),
            Some(Action::JsonTimeline(opt)) => LEVEL_ABBR_MAP
                .keys()
                .any(|level| &opt.output_options.min_level.to_lowercase() == level),
            Some(Action::PivotKeywordsList(opt)) => LEVEL_ABBR_MAP
                .keys()
                .any(|level| &opt.min_level.to_lowercase() == level),
            _ => true,
        };
        let is_valid_exact_level = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => {
                opt.output_options.exact_level.is_none()
                    || LEVEL_ABBR_MAP.keys().any(|level| {
                        &opt.output_options
                            .exact_level
                            .as_ref()
                            .unwrap()
                            .to_lowercase()
                            == level
                    })
            }
            Some(Action::JsonTimeline(opt)) => {
                opt.output_options.exact_level.is_none()
                    || LEVEL_ABBR_MAP.keys().any(|level| {
                        &opt.output_options
                            .exact_level
                            .as_ref()
                            .unwrap()
                            .to_lowercase()
                            == level
                    })
            }
            Some(Action::PivotKeywordsList(opt)) => {
                opt.exact_level.is_none()
                    || LEVEL_ABBR_MAP
                        .keys()
                        .any(|level| &opt.exact_level.as_ref().unwrap().to_lowercase() == level)
            }
            _ => true,
        };
        if !is_valid_min_level || !is_valid_exact_level {
            AlertMessage::alert(" You specified an invalid level. Please specify informational, low, medium, high or critical.").ok();
            process::exit(1);
        }

        let geo_ip_db_result = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => GeoIPSearch::check_exist_geo_ip_files(
                &opt.geo_ip,
                vec![
                    "GeoLite2-ASN.mmdb",
                    "GeoLite2-Country.mmdb",
                    "GeoLite2-City.mmdb",
                ],
            ),
            Some(Action::JsonTimeline(opt)) => GeoIPSearch::check_exist_geo_ip_files(
                &opt.geo_ip,
                vec![
                    "GeoLite2-ASN.mmdb",
                    "GeoLite2-Country.mmdb",
                    "GeoLite2-City.mmdb",
                ],
            ),
            _ => Ok(None),
        };
        if let Err(err_msg) = geo_ip_db_result {
            AlertMessage::alert(&err_msg).ok();
            process::exit(1);
        }
        if let Some(geo_ip_db_path) = geo_ip_db_result.unwrap() {
            *GEOIP_DB_PARSER.write().unwrap() = Some(GeoIPSearch::new(
                &geo_ip_db_path,
                vec![
                    "GeoLite2-ASN.mmdb",
                    "GeoLite2-Country.mmdb",
                    "GeoLite2-City.mmdb",
                ],
            ));
            let geo_ip_file_path =
                utils::check_setting_path(config_path, "geoip_field_mapping", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/geoip_field_mapping.yaml",
                            true,
                        )
                        .unwrap()
                    });
            if !geo_ip_file_path.exists() {
                AlertMessage::alert(
                    "Could not find the geoip_field_mapping.yaml config file. Please run update-rules."
                )
                .ok();
                process::exit(1);
            }
            let geo_ip_mapping = if let Ok(loaded_yaml) =
                YamlLoader::load_from_str(&fs::read_to_string(geo_ip_file_path).unwrap())
            {
                loaded_yaml
            } else {
                AlertMessage::alert("Parse error in geoip_field_mapping.yaml.").ok();
                YamlLoader::load_from_str("").unwrap()
            };
            let target_map = &geo_ip_mapping[0];
            let empty_yaml_vec: Vec<Yaml> = vec![];
            *GEOIP_FILTER.write().unwrap() = Some(
                target_map["Filter"]
                    .as_vec()
                    .unwrap_or(&empty_yaml_vec)
                    .to_owned(),
            );
            let mut static_geoip_conf = HashMap::new();
            let check_target_map = vec!["SrcIP", "TgtIP"];
            for check_key in check_target_map {
                if !target_map[check_key].is_badvalue()
                    && !target_map[check_key]
                        .as_vec()
                        .unwrap_or(&empty_yaml_vec)
                        .is_empty()
                {
                    static_geoip_conf.insert(
                        CompactString::from(check_key),
                        target_map[check_key].clone(),
                    );
                }
            }
            *GEOIP_DB_YAML.write().unwrap() = Some(static_geoip_conf);
        };
        let output_path = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.output.as_ref(),
            Some(Action::JsonTimeline(opt)) => opt.output.as_ref(),
            Some(Action::Metrics(opt)) => opt.output.as_ref(),
            Some(Action::PivotKeywordsList(opt)) => opt.output.as_ref(),
            Some(Action::LogonSummary(opt)) => opt.output.as_ref(),
            _ => None,
        };
        let general_ch_abbr = create_output_filter_config(
            utils::check_setting_path(config_path, "generic_abbreviations.txt", false)
                .unwrap_or_else(|| {
                    utils::check_setting_path(
                        &CURRENT_EXE_PATH.to_path_buf(),
                        "rules/config/generic_abbreviations.txt",
                        true,
                    )
                    .unwrap()
                })
                .to_str()
                .unwrap(),
            false,
        );
        let multiline_flag = match &input_config.as_ref().unwrap().action {
            Some(Action::CsvTimeline(opt)) => opt.multiline,
            _ => false,
        };
        let mut ret = StoredStatic {
            config: input_config.as_ref().unwrap().to_owned(),
            config_path: config_path.to_path_buf(),
            ch_config: create_output_filter_config(
                utils::check_setting_path(config_path, "channel_abbreviations.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/channel_abbreviations.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
                true,
            ),
            disp_abbr_generic: AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .build(general_ch_abbr.keys().map(|x| x.as_str())),
            disp_abbr_general_values: general_ch_abbr.values().map(|x| x.to_owned()).collect_vec(),
            provider_abbr_config: create_output_filter_config(
                utils::check_setting_path(config_path, "provider_abbreviations.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/provider_abbreviations.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
                false,
            ),
            default_details: Self::get_default_details(
                utils::check_setting_path(config_path, "default_details.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/default_details.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
            ),
            eventkey_alias: load_eventkey_alias(
                utils::check_setting_path(config_path, "eventkey_alias.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/eventkey_alias.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
            ),
            logon_summary_flag: action_id == 2,
            metrics_flag: action_id == 3,
            output_option: extract_output_options(input_config.as_ref().unwrap()),
            pivot_keyword_list_flag: action_id == 4,
            quiet_errors_flag,
            verbose_flag,
            html_report_flag: htmlreport::check_html_flag(input_config.as_ref().unwrap()),
            profiles: None,
            thread_number: check_thread_number(input_config.as_ref().unwrap()),
            event_timeline_config: load_eventcode_info(
                utils::check_setting_path(config_path, "channel_eid_info.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/channel_eid_info.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
            ),
            target_eventids: load_target_ids(
                utils::check_setting_path(config_path, "target_event_IDs.txt", false)
                    .unwrap_or_else(|| {
                        utils::check_setting_path(
                            &CURRENT_EXE_PATH.to_path_buf(),
                            "rules/config/target_event_IDs.txt",
                            true,
                        )
                        .unwrap()
                    })
                    .to_str()
                    .unwrap(),
            ),
            json_input_flag,
            output_path: output_path.cloned(),
            common_options,
            multiline_flag,
        };
        ret.profiles = load_profile(
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
            Some(&ret),
        );
        ret
    }
    /// detailsのdefault値をファイルから読み取る関数
    pub fn get_default_details(filepath: &str) -> HashMap<CompactString, CompactString> {
        let read_result = utils::read_csv(filepath);
        match read_result {
            Err(_e) => {
                AlertMessage::alert(&_e).ok();
                HashMap::new()
            }
            Ok(lines) => {
                let mut ret: HashMap<CompactString, CompactString> = HashMap::new();
                lines
                    .iter()
                    .try_for_each(|line| -> Result<(), String> {
                        let provider = match line.get(0) {
                            Some(_provider) => _provider.trim(),
                            _ => {
                                return Result::Err(
                                    "Failed to read provider in default_details.txt.".to_string(),
                                )
                            }
                        };
                        let eid = match line.get(1) {
                            Some(eid_str) => match eid_str.trim().parse::<i64>() {
                                Ok(_eid) => _eid,
                                _ => {
                                    return Result::Err(
                                        "EventID parsing error in default_details.txt.".to_string(),
                                    )
                                }
                            },
                            _ => {
                                return Result::Err(
                                    "Failed to read EventID in default_details.txt.".to_string(),
                                )
                            }
                        };
                        let details = match line.get(2) {
                            Some(detail) => detail.trim(),
                            _ => {
                                return Result::Err(
                                    "Failed to read details in default_details.txt.".to_string(),
                                )
                            }
                        };
                        ret.insert(
                            CompactString::from(format!("{provider}_{eid}")),
                            CompactString::from(details),
                        );
                        Ok(())
                    })
                    .ok();
                ret
            }
        }
    }
}

/// config情報からthred_numberの情報を抽出する関数
fn check_thread_number(config: &Config) -> Option<usize> {
    match config.action.as_ref()? {
        Action::CsvTimeline(opt) => opt.output_options.detect_common_options.thread_number,
        Action::JsonTimeline(opt) => opt.output_options.detect_common_options.thread_number,
        Action::LogonSummary(opt) => opt.detect_common_options.thread_number,
        Action::Metrics(opt) => opt.detect_common_options.thread_number,
        Action::PivotKeywordsList(opt) => opt.detect_common_options.thread_number,
        _ => None,
    }
}

// コマンド生成用のClapの定義
#[derive(Subcommand, Clone, Debug)]
pub enum Action {
    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe csv-timeline <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Alert the timeline to Elastic.
    AlertElastic(AlertElasticOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.0-dev\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe csv-timeline <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Save the timeline in CSV format.
    CsvTimeline(CsvOutputOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe json-timeline <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Save the timeline in JSON/JSONL format.
    JsonTimeline(JSONOutputOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe logon-summary <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Print a summary of successful and failed logons
    LogonSummary(LogonSummaryOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe metrics <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Print event ID metrics
    Metrics(MetricsOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe pivot-keywords-list <INPUT> [OPTIONS]\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Create a list of pivot keywords
    PivotKeywordsList(PivotKeywordOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  {usage}\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Update to the latest rules in the hayabusa-rules github repository
    UpdateRules(UpdateOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  {usage}\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Tune alert levels (default: ./rules/config/level_tuning.txt)
    LevelTuning(LevelTuningOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.3\n{author-with-newline}\n{usage-heading}\n  {usage}\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Set default output profile
    SetDefaultProfile(DefaultProfileOption),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.0-dev\n{author-with-newline}\n{usage-heading}\n  {usage}\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// Print the list of contributors
    ListContributors(CommonOptions),

    #[clap(
        author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
        help_template = "\nHayabusa v2.3.0-dev\n{author-with-newline}\n{usage-heading}\n  {usage}\n\n{all-args}",
        term_width = 400,
        disable_help_flag = true
    )]
    /// List the output profiles
    ListProfiles(CommonOptions),
}

impl Action {
    pub fn to_usize(action: Option<&Action>) -> usize {
        if let Some(a) = action {
            match a {
                Action::CsvTimeline(_) => 0,
                Action::JsonTimeline(_) => 1,
                Action::LogonSummary(_) => 2,
                Action::Metrics(_) => 3,
                Action::PivotKeywordsList(_) => 4,
                Action::UpdateRules(_) => 5,
                Action::LevelTuning(_) => 6,
                Action::SetDefaultProfile(_) => 7,
                Action::ListContributors(_) => 8,
                Action::ListProfiles(_) => 9,
                Action::AlertElastic(_) => 10,
            }
        } else {
            100
        }
    }
    pub fn get_action_name(action: Option<&Action>) -> &str {
        if let Some(a) = action {
            match a {
                Action::CsvTimeline(_) => "csv-timeline",
                Action::JsonTimeline(_) => "json-timeline",
                Action::LogonSummary(_) => "logon-summary",
                Action::Metrics(_) => "metrics",
                Action::PivotKeywordsList(_) => "pivot-keywords-list",
                Action::UpdateRules(_) => "update-rules",
                Action::LevelTuning(_) => "level-tuning",
                Action::SetDefaultProfile(_) => "set-default-profile",
                Action::ListContributors(_) => "list-contributors",
                Action::ListProfiles(_) => "list-profiles",
                Action::AlertElastic(_) => "alert-elastic",
            }
        } else {
            ""
        }
    }
}

#[derive(Args, Clone, Debug)]
pub struct DetectCommonOption {
    /// Scan JSON formatted logs instead of .evtx (.json or .jsonl)
    #[arg(help_heading = Some("Input"), short = 'J', long = "JSON-input", display_order = 999)]
    // JSON-inputのみ通常の並び順とは別にInputの後ろに配置したいためdisplay_orderの値は大きい値(999)で調整した
    pub json_input: bool,

    /// Specify additional file extensions (ex: evtx_data) (ex: evtx1,evtx2)
    #[arg(help_heading = Some("General Options"), long = "target-file-ext", use_value_delimiter = true, value_delimiter = ',', display_order = 460)]
    pub evtx_file_ext: Option<Vec<String>>,

    /// Number of threads (default: optimal number for performance)
    #[arg(
        help_heading = Some("General Options"),
        short = 't',
        long = "threads",
        value_name = "NUMBER",
        display_order = 461
    )]
    pub thread_number: Option<usize>,

    /// Quiet errors mode: do not save error logs
    #[arg(help_heading = Some("General Options"), short = 'Q', long = "quiet-errors", display_order = 430)]
    pub quiet_errors: bool,

    /// Specify custom rule config directory (default: ./rules/config)
    #[arg(
        help_heading = Some("General Options"),
        short = 'c',
        long = "rules-config",
        default_value = "./rules/config",
        hide_default_value = true,
        value_name = "DIR",
        display_order = 441
    )]
    pub config: PathBuf,

    /// Output verbose information
    #[arg(help_heading = Some("Display Settings"), short = 'v', long, display_order = 480)]
    pub verbose: bool,
}

#[derive(Args, Clone, Debug)]
pub struct DefaultProfileOption {
    #[clap(flatten)]
    pub common_options: CommonOptions,
    /// Specify output profile

    #[arg(help_heading = Some("General Options"), short = 'p', long = "profile", display_order = 420)]
    pub profile: Option<String>,
}

#[derive(Args, Clone, Debug)]
pub struct UpdateOption {
    #[clap(flatten)]
    pub common_options: CommonOptions,

    /// Specify a custom rule directory or file (default: ./rules)
    #[arg(
        help_heading = Some("General Options"),
        short = 'r',
        long,
        default_value = "./rules",
        hide_default_value = true,
        value_name = "DIR/FILE",
        display_order = 440
    )]
    pub rules: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub struct LevelTuningOption {
    #[clap(flatten)]
    pub common_options: CommonOptions,

    /// Tune alert levels (default: ./rules/config/level_tuning.txt)
    #[arg(
            help_heading = Some("General Options"),
            short = 'f',
            long = "file",
            default_value = "./rules/config/level_tuning.txt",
            hide_default_value = true,
            value_name = "FILE",
            display_order = 320
        )]
    pub level_tuning: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub struct MetricsOption {
    #[clap(flatten)]
    pub input_args: InputOption,

    /// Save the Metrics in CSV format (ex: metrics.csv)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILE", display_order = 410)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    pub common_options: CommonOptions,

    #[clap(flatten)]
    pub detect_common_options: DetectCommonOption,

    /// Output timestamp in European time format (ex: 22-02-2022 22:00:00.123 +02:00)
    #[arg(help_heading = Some("Time Format"), long = "European-time", display_order = 50)]
    pub european_time: bool,

    /// Output timestamp in ISO-8601 format (ex: 2022-02-22T10:10:10.1234567Z) (Always UTC)
    #[arg(help_heading = Some("Time Format"), long = "ISO-8601", display_order = 90)]
    pub iso_8601: bool,

    /// Output timestamp in RFC 2822 format (ex: Fri, 22 Feb 2022 22:00:00 -0600)
    #[arg(help_heading = Some("Time Format"), long = "RFC-2822", display_order = 180)]
    pub rfc_2822: bool,

    /// Output timestamp in RFC 3339 format (ex: 2022-02-22 22:00:00.123456-06:00)
    #[arg(help_heading = Some("Time Format"), long = "RFC-3339", display_order = 180)]
    pub rfc_3339: bool,

    /// Output timestamp in US military time format (ex: 02-22-2022 22:00:00.123 -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-military-time", display_order = 210)]
    pub us_military_time: bool,

    /// Output timestamp in US time format (ex: 02-22-2022 10:00:00.123 PM -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-time", display_order = 210)]
    pub us_time: bool,

    /// Output time in UTC format (default: local time)
    #[arg(help_heading = Some("Time Format"), short = 'U', long = "UTC", display_order = 210)]
    pub utc: bool,
}

#[derive(Args, Clone, Debug)]
#[clap(group(ArgGroup::new("level_rule_filtering").args(["min_level", "exact_level"]).multiple(false)))]
pub struct PivotKeywordOption {
    #[clap(flatten)]
    pub input_args: InputOption,

    /// Save pivot words to separate files (ex: PivotKeywords)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILENAMES-BASE", display_order = 410)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    pub common_options: CommonOptions,

    /// Enable rules with status of deprecated
    #[arg(help_heading = Some("Filtering"), short = 'D', long = "enable-deprecated-rules", display_order = 310)]
    pub enable_deprecated_rules: bool,

    /// Enable rules with status of unsupported
    #[arg(help_heading = Some("Filtering"), short = 'u', long = "enable-unsupported-rules", display_order = 312)]
    pub enable_unsupported_rules: bool,

    /// Ignore rules according to status (ex: experimental) (ex: stable,test)
    #[arg(help_heading = Some("Filtering"), long = "exclude-status", value_name = "STATUS", use_value_delimiter = true, value_delimiter = ',', display_order = 314)]
    pub exclude_status: Option<Vec<String>>,

    /// Minimum level for rules (default: informational)
    #[arg(
        help_heading = Some("Filtering"),
        short = 'm',
        long = "min-level",
        default_value = "informational",
        hide_default_value = true,
        value_name = "LEVEL",
        display_order = 390
    )]
    pub min_level: String,

    /// Scan for only specific levels (informational, low, medium, high, critical)
    #[arg(
        help_heading = Some("Filtering"),
        short = 'e',
        long = "exact-level",
        value_name = "LEVEL",
        display_order = 313
    )]
    pub exact_level: Option<String>,

    /// Enable rules set to noisy (./rules/config/noisy_rules.txt)
    #[arg(help_heading = Some("Filtering"), short = 'n', long = "enable-noisy-rules", display_order = 311)]
    pub enable_noisy_rules: bool,

    /// End time of the event logs to load (ex: "2022-02-22 23:59:59 +09:00")
    #[arg(help_heading = Some("Filtering"), long = "timeline-end", value_name = "DATE", display_order = 460)]
    pub end_timeline: Option<String>,

    /// Start time of the event logs to load (ex: "2020-02-22 00:00:00 +09:00")
    #[arg(help_heading = Some("Filtering"), long = "timeline-start", value_name = "DATE", display_order = 460)]
    pub start_timeline: Option<String>,

    /// Scan only common EIDs for faster speed (./rules/config/target_event_IDs.txt)
    #[arg(help_heading = Some("Filtering"), short = 'E', long = "EID-filter", display_order = 50)]
    pub eid_filter: bool,

    #[clap(flatten)]
    pub detect_common_options: DetectCommonOption,
}

#[derive(Args, Clone, Debug)]
pub struct LogonSummaryOption {
    #[clap(flatten)]
    pub input_args: InputOption,

    /// Save the Logon summary in CSV format (ex: logon-summary.csv)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILE", display_order = 410)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    pub common_options: CommonOptions,

    #[clap(flatten)]
    pub detect_common_options: DetectCommonOption,

    /// Output timestamp in European time format (ex: 22-02-2022 22:00:00.123 +02:00)
    #[arg(help_heading = Some("Time Format"), long = "European-time", display_order = 50)]
    pub european_time: bool,

    /// Output timestamp in ISO-8601 format (ex: 2022-02-22T10:10:10.1234567Z) (Always UTC)
    #[arg(help_heading = Some("Time Format"), long = "ISO-8601", display_order = 90)]
    pub iso_8601: bool,

    /// Output timestamp in RFC 2822 format (ex: Fri, 22 Feb 2022 22:00:00 -0600)
    #[arg(help_heading = Some("Time Format"), long = "RFC-2822", display_order = 180)]
    pub rfc_2822: bool,

    /// Output timestamp in RFC 3339 format (ex: 2022-02-22 22:00:00.123456-06:00)
    #[arg(help_heading = Some("Time Format"), long = "RFC-3339", display_order = 180)]
    pub rfc_3339: bool,

    /// Output timestamp in US military time format (ex: 02-22-2022 22:00:00.123 -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-military-time", display_order = 210)]
    pub us_military_time: bool,

    /// Output timestamp in US time format (ex: 02-22-2022 10:00:00.123 PM -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-time", display_order = 210)]
    pub us_time: bool,

    /// Output time in UTC format (default: local time)
    #[arg(help_heading = Some("Time Format"), short = 'U', long = "UTC", display_order = 210)]
    pub utc: bool,
}

/// Options can be set when outputting
#[derive(Args, Clone, Debug)]
#[clap(group(ArgGroup::new("level_rule_filtering").args(["min_level", "exact_level"]).multiple(false)))]
pub struct OutputOption {
    #[clap(flatten)]
    pub input_args: InputOption,

    /// Specify output profile
    #[arg(help_heading = Some("Output"), short = 'p', long = "profile", display_order = 420)]
    pub profile: Option<String>,

    #[clap(flatten)]
    pub common_options: CommonOptions,

    /// Enable rules with status of deprecated
    #[arg(help_heading = Some("Filtering"), short = 'D', long = "enable-deprecated-rules", display_order = 310)]
    pub enable_deprecated_rules: bool,

    /// Enable rules with status of unsupported
    #[arg(help_heading = Some("Filtering"), short = 'u', long = "enable-unsupported-rules", display_order = 312)]
    pub enable_unsupported_rules: bool,

    /// Ignore rules according to status (ex: experimental) (ex: stable,test)
    #[arg(help_heading = Some("Filtering"), long = "exclude-status", value_name = "STATUS", use_value_delimiter = true, value_delimiter = ',', display_order = 314)]
    pub exclude_status: Option<Vec<String>>,

    /// Minimum level for rules (default: informational)
    #[arg(
        help_heading = Some("Filtering"),
        short = 'm',
        long = "min-level",
        default_value = "informational",
        hide_default_value = true,
        value_name = "LEVEL",
        display_order = 390,
    )]
    pub min_level: String,

    /// Scan for only specific levels (informational, low, medium, high, critical)
    #[arg(
        help_heading = Some("Filtering"),
        short = 'e',
        long = "exact-level",
        value_name = "LEVEL",
        display_order = 313
    )]
    pub exact_level: Option<String>,

    /// Enable rules set to noisy (./rules/config/noisy_rules.txt)
    #[arg(help_heading = Some("Filtering"), short = 'n', long = "enable-noisy-rules", display_order = 311)]
    pub enable_noisy_rules: bool,

    /// End time of the event logs to load (ex: "2022-02-22 23:59:59 +09:00")
    #[arg(help_heading = Some("Filtering"), long = "timeline-end", value_name = "DATE", display_order = 460)]
    pub end_timeline: Option<String>,

    /// Start time of the event logs to load (ex: "2020-02-22 00:00:00 +09:00")
    #[arg(help_heading = Some("Filtering"), long = "timeline-start", value_name = "DATE", display_order = 460)]
    pub start_timeline: Option<String>,

    /// Scan only common EIDs for faster speed (./rules/config/target_event_IDs.txt)
    #[arg(help_heading = Some("Filtering"), short = 'E', long = "EID-filter", display_order = 50)]
    pub eid_filter: bool,

    #[clap(flatten)]
    pub detect_common_options: DetectCommonOption,

    /// Output timestamp in European time format (ex: 22-02-2022 22:00:00.123 +02:00)
    #[arg(help_heading = Some("Time Format"), long = "European-time", display_order = 50)]
    pub european_time: bool,

    /// Output timestamp in ISO-8601 format (ex: 2022-02-22T10:10:10.1234567Z) (Always UTC)
    #[arg(help_heading = Some("Time Format"), long = "ISO-8601", display_order = 90)]
    pub iso_8601: bool,

    /// Output timestamp in RFC 2822 format (ex: Fri, 22 Feb 2022 22:00:00 -0600)
    #[arg(help_heading = Some("Time Format"), long = "RFC-2822", display_order = 180)]
    pub rfc_2822: bool,

    /// Output timestamp in RFC 3339 format (ex: 2022-02-22 22:00:00.123456-06:00)
    #[arg(help_heading = Some("Time Format"), long = "RFC-3339", display_order = 180)]
    pub rfc_3339: bool,

    /// Output timestamp in US military time format (ex: 02-22-2022 22:00:00.123 -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-military-time", display_order = 210)]
    pub us_military_time: bool,

    /// Output timestamp in US time format (ex: 02-22-2022 10:00:00.123 PM -06:00)
    #[arg(help_heading = Some("Time Format"), long = "US-time", display_order = 210)]
    pub us_time: bool,

    /// Output time in UTC format (default: local time)
    #[arg(help_heading = Some("Time Format"), short = 'U', long = "UTC", display_order = 210)]
    pub utc: bool,

    /// Output event frequency timeline (terminal needs to support unicode)
    #[arg(help_heading = Some("Display Settings"), short = 'T', long = "visualize-timeline", display_order = 481)]
    pub visualize_timeline: bool,

    /// Specify a custom rule directory or file (default: ./rules)
    #[arg(
        help_heading = Some("General Options"),
        short = 'r',
        long,
        default_value = "./rules",
        hide_default_value = true,
        value_name = "DIR/FILE",
        display_order = 440
    )]
    pub rules: PathBuf,

    /// Save Results Summary details to an HTML report (ex: results.html)
    #[arg(help_heading = Some("Output"), short = 'H', long="HTML-report", value_name = "FILE", display_order = 80)]
    pub html_report: Option<PathBuf>,

    /// Do not display Results Summary (slightly faster speed)
    #[arg(help_heading = Some("Display Settings"), long = "no-summary", display_order = 400)]
    pub no_summary: bool,
}

#[derive(Copy, Args, Clone, Debug)]
pub struct CommonOptions {
    /// Disable color output
    #[arg(help_heading = Some("Display Settings"), long = "no-color", global = true, display_order = 400)]
    pub no_color: bool,

    /// Quiet mode: do not display the launch banner
    #[arg(help_heading = Some("Display Settings"), short, long, global = true, display_order = 430)]
    pub quiet: bool,
}

#[derive(Args, Clone, Debug)]
pub struct InputOption {
    /// Directory of multiple .evtx files
    #[arg(help_heading = Some("Input"), short = 'd', long, value_name = "DIR", display_order = 300)]
    pub directory: Option<PathBuf>,

    /// File path to one .evtx file
    #[arg(help_heading = Some("Input"), short = 'f', long = "file", value_name = "FILE", display_order = 320)]
    pub filepath: Option<PathBuf>,

    /// Analyze the local C:\Windows\System32\winevt\Logs folder
    #[arg(help_heading = Some("Input"), short = 'l', long = "live-analysis", display_order = 380)]
    pub live_analysis: bool,
}

#[derive(Args, Clone, Debug)]
pub struct CsvOutputOption {
    #[clap(flatten)]
    pub output_options: OutputOption,

    /// Output event field information in multiple rows
    #[arg(help_heading = Some("Output"), short = 'M', long="multiline", display_order = 390)]
    pub multiline: bool,

    // display_order value is defined acronym of long option (A=10,B=20,...,Z=260,a=270, b=280...,z=520)
    /// Add GeoIP (ASN, city, country) info to IP addresses
    #[arg(
        help_heading = Some("Output"),
        short = 'G',
        long = "GeoIP",
        value_name = "MAXMIND-DB-DIR",
        display_order = 70
    )]
    pub geo_ip: Option<PathBuf>,

    /// Save the timeline in CSV format (ex: results.csv)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILE", display_order = 410)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct JSONOutputOption {
    #[clap(flatten)]
    pub output_options: OutputOption,

    /// Save the timeline in JSON format (ex: results.json)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILE", display_order = 410)]
    pub output: Option<PathBuf>,

    /// Save the timeline in JSONL format (ex: -L -o results.jsonl)
    #[arg(help_heading = Some("Output"), short = 'L', long = "JSONL-output", requires = "output", display_order = 100)]
    pub jsonl_timeline: bool,

    /// Add GeoIP (ASN, city, country) info to IP addresses
    #[arg(
        help_heading = Some("Output"),
        short = 'G',
        long = "GeoIP",
        value_name = "MAXMIND-DB-DIR",
        display_order = 70
    )]
    pub geo_ip: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct ElasticSettingOption {
    ///strict mode: do not only warn, but abort if an error occurs
    #[arg(help_heading = Some("Elastic Settings"),long , display_order = 450)]
    pub strict: bool,

    ///name of the elasticsearch index
    #[arg(help_heading = Some("Elastic Settings"),short, long , value_name="INDEX_NAME", display_order = 350)]
    pub index: String,

    ///server name or IP address of elasticsearch server
    #[arg(help_heading = Some("Elastic Settings"),short='H', long , value_name="HOST", display_order = 340)]
    pub host: String,

    /// API port number of elasticsearch server [default: 9200]
    #[arg(help_heading = Some("Elastic Settings"),short='P', long , value_name="PORT", default_value="9200", display_order = 420)]
    pub port: u16,

    ///protocol to be used to connect to elasticsearch [default: https] [possible values: http, https]
    #[arg(help_heading = Some("Elastic Settings"),long , value_name="PROTOCOL",default_value="https", display_order = 350)]
    pub proto: String,

    /// omit certificate validation
    #[arg(help_heading = Some("Elastic Settings"), short='k', long, value_name="PROTOCOL", display_order = 420)]
    pub insecure: bool,

    ///username for elasticsearch server [default: elastic]
    #[arg(help_heading = Some("Elastic Settings"),short='U', long , value_name="USERNAME", default_value="elastic", display_order = 470)]
    pub username: String,

    ///password for authenticating at elasticsearch
    #[arg(help_heading = Some("Elastic Settings"),short='W', long , value_name="PASSWORD", display_order = 420)]
    pub password: String,
}

#[derive(Args, Clone, Debug)]
pub struct AlertElasticOption {
    #[clap(flatten)]
    pub input_args: InputOption,

    /// Specify output profile
    #[arg(help_heading = Some("Output"), short = 'p', long = "profile", display_order = 420)]
    pub profile: Option<String>,

    #[clap(flatten)]
    pub common_options: CommonOptions,

    /// Save the timeline in JSON format (ex: results.json)
    #[arg(help_heading = Some("Output"), short = 'o', long, value_name = "FILE", display_order = 410)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    pub detect_common_options: DetectCommonOption,

    #[clap(flatten)]
    pub elastic_options: ElasticSettingOption,

}

#[derive(Parser, Clone, Debug)]
#[clap(
    author = "Yamato Security (https://github.com/Yamato-Security/hayabusa) @SecurityYamato)",
    help_template = "\nHayabusa 2.3.3\n{author-with-newline}\n{usage-heading}\n  hayabusa.exe <COMMAND> [OPTIONS]\n  hayabusa.exe help <COMMAND>\n\n{all-args}{options}",
    term_width = 400,
    disable_help_flag = true
)]
pub struct Config {
    #[command(subcommand)]
    pub action: Option<Action>,

    /// Print debug information (memory usage, etc...)
    #[clap(long = "debug", global = true, hide = true)]
    pub debug: bool,
}

impl ConfigReader {
    pub fn new() -> Self {
        let parse = Config::parse();
        let help_term_width = if let Some((Width(w), _)) = terminal_size() {
            w as usize
        } else {
            400
        };
        let build_cmd = Config::command()
            .disable_version_flag(true)
            .color(ColorChoice::Auto)
            .term_width(help_term_width);
        ConfigReader {
            app: build_cmd,
            config: Some(parse),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TargetEventIds {
    ids: HashSet<String>,
}

impl Default for TargetEventIds {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetEventIds {
    pub fn new() -> TargetEventIds {
        TargetEventIds {
            ids: HashSet::new(),
        }
    }

    pub fn is_target(&self, id: &str) -> bool {
        // 中身が空の場合は全EventIdを対象とする。
        if self.ids.is_empty() {
            return true;
        }
        self.ids.contains(id)
    }
}

fn load_target_ids(path: &str) -> TargetEventIds {
    let mut ret = TargetEventIds::default();
    let lines = utils::read_txt(path); // ファイルが存在しなければエラーとする
    if lines.is_err() {
        AlertMessage::alert(lines.as_ref().unwrap_err()).ok();
        return ret;
    }

    for line in lines.unwrap_or_else(|_| Nested::<String>::new()).iter() {
        if line.is_empty() {
            continue;
        }
        ret.ids.insert(line.to_string());
    }

    ret
}

#[derive(Debug, Clone)]
pub struct TargetEventTime {
    parse_success_flag: bool,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
}

impl TargetEventTime {
    pub fn new(stored_static: &StoredStatic) -> Self {
        let mut parse_success_flag = true;
        let mut get_time = |input_time: Option<&String>, error_contents: &str| {
            if let Some(time) = input_time {
                match DateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S %z") // 2014-11-28 21:00:09 +09:00
                    .or_else(|_| DateTime::parse_from_str(time, "%Y/%m/%d %H:%M:%S %z")) // 2014/11/28 21:00:09 +09:00
                {
                    Ok(dt) => Some(dt.with_timezone(&Utc)),
                    Err(_) => {
                        AlertMessage::alert(error_contents)
                        .ok();
                        parse_success_flag = false;
                        None
                    }
                }
            } else {
                None
            }
        };
        match &stored_static.config.action.as_ref().unwrap() {
            Action::CsvTimeline(option) => {
                let start_time = get_time(
                    option.output_options.start_timeline.as_ref(),
                    "start-timeline field: the timestamp format is not correct.",
                );
                let end_time = get_time(
                    option.output_options.end_timeline.as_ref(),
                    "end-timeline field: the timestamp format is not correct.",
                );
                Self::set(parse_success_flag, start_time, end_time)
            }
            Action::JsonTimeline(option) => {
                let start_time = get_time(
                    option.output_options.start_timeline.as_ref(),
                    "start-timeline field: the timestamp format is not correct.",
                );
                let end_time = get_time(
                    option.output_options.start_timeline.as_ref(),
                    "end-timeline field: the timestamp format is not correct.",
                );
                Self::set(parse_success_flag, start_time, end_time)
            }
            _ => Self::set(parse_success_flag, None, None),
        }
    }

    pub fn set(
        input_parse_success_flag: bool,
        input_start_time: Option<chrono::DateTime<chrono::Utc>>,
        input_end_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        Self {
            parse_success_flag: input_parse_success_flag,
            start_time: input_start_time,
            end_time: input_end_time,
        }
    }

    pub fn is_parse_success(&self) -> bool {
        self.parse_success_flag
    }

    pub fn is_target(&self, eventtime: &Option<DateTime<Utc>>) -> bool {
        if eventtime.is_none() {
            return true;
        }
        if let Some(starttime) = self.start_time {
            if eventtime.unwrap() < starttime {
                return false;
            }
        }
        if let Some(endtime) = self.end_time {
            if eventtime.unwrap() > endtime {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct EventKeyAliasConfig {
    key_to_eventkey: HashMap<String, String>,
    key_to_split_eventkey: HashMap<String, Vec<usize>>,
}

impl EventKeyAliasConfig {
    pub fn new() -> EventKeyAliasConfig {
        EventKeyAliasConfig {
            key_to_eventkey: HashMap::new(),
            key_to_split_eventkey: HashMap::new(),
        }
    }

    pub fn get_event_key(&self, alias: &str) -> Option<&String> {
        self.key_to_eventkey.get(alias)
    }

    pub fn get_event_key_split(&self, alias: &str) -> Option<&Vec<usize>> {
        self.key_to_split_eventkey.get(alias)
    }
}

impl Default for EventKeyAliasConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub fn load_eventkey_alias(path: &str) -> EventKeyAliasConfig {
    let mut config = EventKeyAliasConfig::new();

    // eventkey_aliasが読み込めなかったらエラーで終了とする。
    let read_result = utils::read_csv(path);
    if read_result.is_err() {
        AlertMessage::alert(read_result.as_ref().unwrap_err()).ok();
        return config;
    }

    read_result.unwrap().iter().for_each(|line| {
        if line.len() != 2 {
            return;
        }

        let empty = &"".to_string();
        let alias = line.get(0).unwrap_or(empty);
        let event_key = line.get(1).unwrap_or(empty);
        if alias.is_empty() || event_key.is_empty() {
            return;
        }

        config
            .key_to_eventkey
            .insert(alias.to_owned(), event_key.to_owned());
        config.key_to_split_eventkey.insert(
            alias.to_owned(),
            event_key.split('.').map(|s| s.len()).collect(),
        );
    });
    config.key_to_eventkey.shrink_to_fit();
    config
}

///設定ファイルを読み込み、keyとfieldsのマップをPIVOT_KEYWORD大域変数にロードする。
pub fn load_pivot_keywords(path: &str) {
    let read_result = utils::read_txt(path);
    if read_result.is_err() {
        AlertMessage::alert(read_result.as_ref().unwrap_err()).ok();
    }

    read_result.unwrap().iter().for_each(|line| {
        let mut map = line.split('.').take(2);
        if let Some(size) = map.size_hint().1 {
            if size < 2 {
                return;
            }
        } else {
            return;
        }
        let key = map.next().unwrap();
        let value = map.next().unwrap();

        //存在しなければ、keyを作成
        PIVOT_KEYWORD
            .write()
            .unwrap()
            .entry(key.to_string())
            .or_insert_with(PivotKeyword::new);

        PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut(key)
            .unwrap()
            .fields
            .insert(value.to_string());
    });
}

/// --target-file-extで追加された拡張子から、調査対象ファイルの拡張子セットを返す関数。--json-inputがtrueの場合はjsonのみを対象とする
pub fn get_target_extensions(arg: Option<&Vec<String>>, json_input_flag: bool) -> HashSet<String> {
    let mut target_file_extensions: HashSet<String> = convert_option_vecs_to_hs(arg);
    if json_input_flag {
        target_file_extensions.insert(String::from("json"));
    } else {
        target_file_extensions.insert(String::from("evtx"));
    }
    target_file_extensions
}

/// Option<Vec<String>>の内容をHashSetに変換する関数
pub fn convert_option_vecs_to_hs(arg: Option<&Vec<String>>) -> HashSet<String> {
    let ret: HashSet<String> = arg.unwrap_or(&Vec::new()).iter().cloned().collect();
    ret
}

/// configから出力に関連したオプションの値を格納した構造体を抽出する関数
fn extract_output_options(config: &Config) -> Option<OutputOption> {
    match &config.action.as_ref()? {
        Action::CsvTimeline(option) => Some(option.output_options.clone()),
        Action::JsonTimeline(option) => Some(option.output_options.clone()),
        Action::PivotKeywordsList(option) => Some(OutputOption {
            input_args: option.input_args.clone(),
            enable_deprecated_rules: option.enable_deprecated_rules,
            enable_noisy_rules: option.enable_noisy_rules,
            profile: None,
            exclude_status: option.exclude_status.clone(),
            min_level: option.min_level.clone(),
            exact_level: option.exact_level.clone(),
            end_timeline: option.end_timeline.clone(),
            start_timeline: option.start_timeline.clone(),
            eid_filter: option.eid_filter,
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
            common_options: option.common_options,
            detect_common_options: option.detect_common_options.clone(),
            enable_unsupported_rules: option.enable_unsupported_rules,
        }),
        Action::Metrics(option) => Some(OutputOption {
            input_args: option.input_args.clone(),
            enable_deprecated_rules: false,
            enable_noisy_rules: false,
            profile: None,
            exclude_status: None,
            min_level: String::default(),
            exact_level: None,
            end_timeline: None,
            start_timeline: None,
            eid_filter: false,
            european_time: option.european_time,
            iso_8601: option.iso_8601,
            rfc_2822: option.rfc_2822,
            rfc_3339: option.rfc_3339,
            us_military_time: option.us_military_time,
            us_time: option.us_time,
            utc: option.utc,
            visualize_timeline: false,
            rules: Path::new("./rules").to_path_buf(),
            html_report: None,
            no_summary: false,
            common_options: option.common_options,
            detect_common_options: option.detect_common_options.clone(),
            enable_unsupported_rules: false,
        }),
        Action::LogonSummary(option) => Some(OutputOption {
            input_args: option.input_args.clone(),
            enable_deprecated_rules: false,
            enable_noisy_rules: false,
            profile: None,
            exclude_status: None,
            min_level: String::default(),
            exact_level: None,
            end_timeline: None,
            start_timeline: None,
            eid_filter: false,
            european_time: option.european_time,
            iso_8601: option.iso_8601,
            rfc_2822: option.rfc_2822,
            rfc_3339: option.rfc_3339,
            us_military_time: option.us_military_time,
            us_time: option.us_time,
            utc: option.utc,
            visualize_timeline: false,
            rules: Path::new("./rules").to_path_buf(),
            html_report: None,
            no_summary: false,
            common_options: option.common_options,
            detect_common_options: option.detect_common_options.clone(),
            enable_unsupported_rules: false,
        }),
        Action::SetDefaultProfile(option) => Some(OutputOption {
            input_args: InputOption {
                directory: None,
                filepath: None,
                live_analysis: false,
            },
            enable_deprecated_rules: false,
            enable_noisy_rules: false,
            profile: None,
            exclude_status: None,
            min_level: String::default(),
            exact_level: None,
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
            common_options: option.common_options,
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: false,
            },
            enable_unsupported_rules: false,
        }),
        Action::UpdateRules(option) => Some(OutputOption {
            input_args: InputOption {
                directory: None,
                filepath: None,
                live_analysis: false,
            },
            enable_deprecated_rules: true,
            enable_noisy_rules: true,
            profile: None,
            exclude_status: None,
            min_level: String::default(),
            exact_level: None,
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
            common_options: option.common_options,
            detect_common_options: DetectCommonOption {
                evtx_file_ext: None,
                thread_number: None,
                quiet_errors: false,
                config: Path::new("./rules/config").to_path_buf(),
                verbose: false,
                json_input: false,
            },
            enable_unsupported_rules: true,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct EventInfo {
    pub evttitle: String,
}

impl Default for EventInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl EventInfo {
    pub fn new() -> EventInfo {
        let evttitle = "Unknown".to_string();
        EventInfo { evttitle }
    }
}
#[derive(Debug, Clone)]
pub struct EventInfoConfig {
    eventinfo: HashMap<(String, String), EventInfo>,
}

impl Default for EventInfoConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl EventInfoConfig {
    pub fn new() -> EventInfoConfig {
        EventInfoConfig {
            eventinfo: HashMap::new(),
        }
    }
    pub fn get_event_id(&self, channel: &str, eventid: &str) -> Option<&EventInfo> {
        self.eventinfo
            .get(&(channel.to_string(), eventid.to_string()))
    }
}

fn load_eventcode_info(path: &str) -> EventInfoConfig {
    let mut infodata = EventInfo::new();
    let mut config = EventInfoConfig::new();
    let read_result = utils::read_csv(path);
    if read_result.is_err() {
        AlertMessage::alert(read_result.as_ref().unwrap_err()).ok();
        return config;
    }

    // channel_eid_info.txtが読み込めなかったらエラーで終了とする。
    read_result.unwrap().iter().for_each(|line| {
        if line.len() != 3 {
            return;
        }

        let empty = &"".to_string();
        let channel = line.get(0).unwrap_or(empty);
        let eventcode = line.get(1).unwrap_or(empty);
        let event_title = line.get(2).unwrap_or(empty);
        infodata = EventInfo {
            evttitle: event_title.to_string(),
        };
        config.eventinfo.insert(
            (channel.to_lowercase(), eventcode.to_owned()),
            infodata.to_owned(),
        );
    });
    config
}

#[cfg(test)]
mod tests {
    use crate::detections::configs;
    use chrono::{DateTime, Utc};
    use hashbrown::HashSet;

    //     #[test]
    //     #[ignore]
    //     fn singleton_read_and_write() {
    //         let message =
    //             "EventKeyAliasConfig { key_to_eventkey: {\"EventID\": \"Event.System.EventID\"} }";
    //         configs::EVENT_KEY_ALIAS_CONFIG =
    //             configs::load_eventkey_alias("test_files/config/eventkey_alias.txt");
    //         let display = format!(
    //             "{}",
    //             format_args!(
    //                 "{:?}",
    //                 configs::CONFIG.write().unwrap().event_key_alias_config
    //             )
    //         );
    //         assert_eq!(message, display);
    //     }
    // }

    #[test]
    fn target_event_time_filter() {
        let start_time = Some("2018-02-20T12:00:09Z".parse::<DateTime<Utc>>().unwrap());
        let end_time = Some("2020-03-30T12:00:09Z".parse::<DateTime<Utc>>().unwrap());
        let time_filter = configs::TargetEventTime::set(true, start_time, end_time);

        let out_of_range1 = Some("1999-01-01T12:00:09Z".parse::<DateTime<Utc>>().unwrap());
        let within_range = Some("2019-02-27T01:05:01Z".parse::<DateTime<Utc>>().unwrap());
        let out_of_range2 = Some("2021-02-27T01:05:01Z".parse::<DateTime<Utc>>().unwrap());

        assert!(!time_filter.is_target(&out_of_range1));
        assert!(time_filter.is_target(&within_range));
        assert!(!time_filter.is_target(&out_of_range2));
    }

    #[test]
    fn target_event_time_filter_containes_on_time() {
        let start_time = Some("2018-02-20T12:00:09Z".parse::<DateTime<Utc>>().unwrap());
        let end_time = Some("2020-03-30T12:00:09Z".parse::<DateTime<Utc>>().unwrap());
        let time_filter = configs::TargetEventTime::set(true, start_time, end_time);

        assert!(time_filter.is_target(&start_time));
        assert!(time_filter.is_target(&end_time));
    }

    #[test]
    fn test_get_target_extensions() {
        let data = vec!["evtx_data".to_string(), "evtx_stars".to_string()];
        let arg = Some(&data);
        let ret = configs::get_target_extensions(arg, false);
        let expect: HashSet<&str> = HashSet::from(["evtx", "evtx_data", "evtx_stars"]);
        assert_eq!(ret.len(), expect.len());
        for contents in expect.iter() {
            assert!(ret.contains(&contents.to_string()));
        }
    }

    #[test]
    fn no_target_extensions() {
        let ret = configs::get_target_extensions(None, false);
        let expect: HashSet<&str> = HashSet::from(["evtx"]);
        assert_eq!(ret.len(), expect.len());
        for contents in expect.iter() {
            assert!(ret.contains(&contents.to_string()));
        }
    }
}
