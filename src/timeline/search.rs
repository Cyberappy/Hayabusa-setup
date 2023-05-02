use crate::detections::{
    configs::{Action, EventInfoConfig, EventKeyAliasConfig, StoredStatic},
    detection::EvtxRecordInfo,
    message::AlertMessage,
    utils::{self, write_color_buffer},
};
use compact_str::CompactString;
use csv::WriterBuilder;
use downcast_rs::__std::process;
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use nested::Nested;
use regex::Regex;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use termcolor::{BufferWriter, Color, ColorChoice};

#[derive(Debug, Clone)]
pub struct EventSearch {
    pub filepath: CompactString,
    pub search_result: HashSet<(
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
    )>,
}

impl EventSearch {
    pub fn new(
        filepath: CompactString,
        search_result: HashSet<(
            CompactString,
            CompactString,
            CompactString,
            CompactString,
            CompactString,
            CompactString,
            CompactString,
        )>,
    ) -> EventSearch {
        EventSearch {
            filepath,
            search_result,
        }
    }

    /// 検索処理を呼び出す関数。keywordsが空の場合は検索処理を行わない
    pub fn search_start(
        &mut self,
        records: &[EvtxRecordInfo],
        keywords: &[String],
        regex: &Option<String>,
        filters: &[String],
        eventkey_alias: &EventKeyAliasConfig,
        stored_static: &StoredStatic,
    ) {
        if !keywords.is_empty() {
            // 大文字小文字を区別しないかどうかのフラグを設定
            let case_insensitive_flag = match &stored_static.config.action {
                Some(Action::Search(opt)) => opt.ignore_case,
                _ => false,
            };
            self.search_keyword(
                records,
                keywords,
                filters,
                eventkey_alias,
                case_insensitive_flag,
            );
        }
        if let Some(re) = regex {
            self.search_regex(records, re, filters, eventkey_alias);
        }
    }

    /// イベントレコード内の情報からfilterに設定した情報が存在するかを返す関数
    fn filter_record(
        &mut self,
        record: &EvtxRecordInfo,
        filter_rule: &HashMap<String, Nested<String>>,
        eventkey_alias: &EventKeyAliasConfig,
    ) -> bool {
        filter_rule.iter().all(|(k, v)| {
            let alias_target_val = utils::get_serde_number_to_string(
                utils::get_event_value(k, &record.record, eventkey_alias)
                    .unwrap_or(&serde_json::Value::Null),
                true,
            )
            .unwrap_or_else(|| "n/a".into())
            .replace(['"', '\''], "");

            // aliasでマッチした場合はaliasに登録されていないフィールドを検索する必要がないためtrueを返す
            if v.iter()
                .all(|search_target| utils::contains_str(&alias_target_val, search_target))
            {
                return true;
            }

            // aliasに登録されていないフィールドも検索対象とするため
            let allfieldinfo = match utils::get_serde_number_to_string(
                &record.record["Event"]["EventData"][k],
                true,
            ) {
                Some(eventdata) => eventdata,
                _ => CompactString::new("-"),
            };
            v.iter()
                .all(|search_target| utils::contains_str(&allfieldinfo, search_target))
        })
    }

    /// イベントレコード内の情報からkeywordに設定した文字列を検索して、構造体に結果を保持する関数
    fn search_keyword(
        &mut self,
        records: &[EvtxRecordInfo],
        keywords: &[String],
        filters: &[String],
        eventkey_alias: &EventKeyAliasConfig,
        case_insensitive_flag: bool, // 検索時に大文字小文字を区別するかどうか
    ) {
        if records.is_empty() {
            return;
        }

        let filter_rule = create_filter_rule(filters);

        for record in records.iter() {
            // フィルタリングを通過しなければ検索は行わず次のレコードを読み込む
            if !self.filter_record(record, &filter_rule, eventkey_alias) {
                continue;
            }
            let search_target = if case_insensitive_flag {
                record.data_string.to_lowercase()
            } else {
                record.data_string.clone()
            };
            self.filepath = CompactString::from(record.evtx_filepath.as_str());
            if keywords.iter().any(|key| {
                let converted_key = if case_insensitive_flag {
                    key.to_lowercase()
                } else {
                    key.clone()
                };
                utils::contains_str(&search_target, &converted_key)
            }) {
                let (timestamp, hostname, channel, eventid, recordid, allfieldinfo) =
                    extract_search_event_info(record, eventkey_alias);

                self.search_result.insert((
                    timestamp,
                    hostname,
                    channel,
                    eventid,
                    recordid,
                    allfieldinfo,
                    self.filepath.clone(),
                ));
            }
        }
    }

    /// イベントレコード内の情報からregexに設定した正規表現を検索して、構造体に結果を保持する関数
    fn search_regex(
        &mut self,
        records: &[EvtxRecordInfo],
        regex: &str,
        filters: &[String],
        eventkey_alias: &EventKeyAliasConfig,
    ) {
        let re = Regex::new(regex).unwrap_or_else(|err| {
            AlertMessage::alert(&format!("Failed to create regex pattern. \n{err}")).ok();
            process::exit(1);
        });
        if records.is_empty() {
            return;
        }

        let filter_rule = create_filter_rule(filters);

        for record in records.iter() {
            // フィルタリングを通過しなければ検索は行わず次のレコードを読み込む
            if !self.filter_record(record, &filter_rule, eventkey_alias) {
                continue;
            }
            self.filepath = CompactString::from(record.evtx_filepath.as_str());
            if re.is_match(&record.data_string) {
                let (timestamp, hostname, channel, eventid, recordid, allfieldinfo) =
                    extract_search_event_info(record, eventkey_alias);
                self.search_result.insert((
                    timestamp,
                    hostname,
                    channel,
                    eventid,
                    recordid,
                    allfieldinfo,
                    self.filepath.clone(),
                ));
            }
        }
    }
}

/// filters からフィルタリング条件を作成する関数
fn create_filter_rule(filters: &[String]) -> HashMap<String, Nested<String>> {
    filters
        .iter()
        .fold(HashMap::new(), |mut acc, filter_condition| {
            let prefix_trim_condition = filter_condition
                .strip_prefix('"')
                .unwrap_or(filter_condition);
            let trimed_condition = prefix_trim_condition
                .strip_suffix('"')
                .unwrap_or(prefix_trim_condition);
            let condition = trimed_condition.split(':').map(|x| x.trim()).collect_vec();
            if condition.len() != 1 {
                let acc_val = acc
                    .entry(condition[0].to_string())
                    .or_insert(Nested::<String>::new());
                acc_val.push(condition[1..].join(":"));
            }
            acc
        })
}

/// 検索条件に合致したイベントレコードから出力する情報を抽出する関数
fn extract_search_event_info(
    record: &EvtxRecordInfo,
    eventkey_alias: &EventKeyAliasConfig,
) -> (
    CompactString,
    CompactString,
    CompactString,
    CompactString,
    CompactString,
    CompactString,
) {
    let timestamp = utils::get_event_value(
        "Event.System.TimeCreated_attributes.SystemTime",
        &record.record,
        eventkey_alias,
    )
    .map(|evt_value| {
        evt_value
            .as_str()
            .unwrap_or_default()
            .replace("\\\"", "")
            .replace('"', "")
    })
    .unwrap_or_else(|| "n/a".into())
    .replace(['"', '\''], "");

    let hostname = CompactString::from(
        utils::get_serde_number_to_string(
            utils::get_event_value("Computer", &record.record, eventkey_alias)
                .unwrap_or(&serde_json::Value::Null),
            true,
        )
        .unwrap_or_else(|| "n/a".into())
        .replace(['"', '\''], ""),
    );

    let channel =
        utils::get_serde_number_to_string(&record.record["Event"]["System"]["Channel"], false)
            .unwrap_or_default();
    let mut eventid = String::new();
    match utils::get_event_value("EventID", &record.record, eventkey_alias) {
        Some(evtid) if evtid.is_u64() => {
            eventid.push_str(evtid.to_string().as_str());
        }
        _ => {
            eventid.push('-');
        }
    }

    let recordid = match utils::get_serde_number_to_string(
        &record.record["Event"]["System"]["EventRecordID"],
        true,
    ) {
        Some(recid) => recid,
        _ => CompactString::new("-"),
    };

    let datainfo = utils::create_recordinfos(&record.record);
    let allfieldinfo = if !datainfo.is_empty() {
        datainfo.into()
    } else {
        CompactString::new("-")
    };

    (
        timestamp.into(),
        hostname,
        channel,
        eventid.into(),
        recordid,
        allfieldinfo,
    )
}

/// 検索結果を標準出力もしくはcsvファイルに出力する関数
pub fn search_result_dsp_msg(
    result_list: &HashSet<(
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
        CompactString,
    )>,
    event_timeline_config: &EventInfoConfig,
    output: &Option<PathBuf>,
    stored_static: &StoredStatic,
) {
    let header = vec![
        "Timestamp",
        "Hostname",
        "Channel",
        "Event ID",
        "Record ID",
        "EventTitle",
        "AllFieldInfo",
        "EvtxFile",
    ];
    let mut disp_wtr = None;
    let mut file_wtr = None;
    if let Some(path) = output {
        match File::create(path) {
            Ok(file) => {
                file_wtr = Some(
                    WriterBuilder::new()
                        .delimiter(b',')
                        .from_writer(BufWriter::new(file)),
                )
            }
            Err(err) => {
                AlertMessage::alert(&format!("Failed to open file. {err}")).ok();
                process::exit(1)
            }
        }
    };
    if file_wtr.is_none() {
        disp_wtr = Some(BufferWriter::stdout(ColorChoice::Always));
    }

    // Write header
    if output.is_some() {
        file_wtr.as_mut().unwrap().write_record(&header).ok();
    } else if output.is_none() && !result_list.is_empty() {
        write_color_buffer(disp_wtr.as_mut().unwrap(), None, &header.join(" ‖ "), true).ok();
    }

    // Write contents
    for (timestamp, hostname, channel, event_id, record_id, all_field_info, evtx_file) in
        result_list.iter()
    {
        let event_title = if let Some(event_info) =
            event_timeline_config.get_event_id(&channel.to_ascii_lowercase(), event_id)
        {
            event_info.evttitle.as_str()
        } else {
            "-"
        };
        let abbr_channel = stored_static.disp_abbr_generic.replace_all(
            stored_static
                .ch_config
                .get(&CompactString::from(channel.to_ascii_lowercase()))
                .unwrap_or(channel)
                .as_str(),
            &stored_static.disp_abbr_general_values,
        );

        let fmted_all_field_info = all_field_info.split_whitespace().join(" ");
        let all_field_info = if output.is_some() && stored_static.multiline_flag {
            fmted_all_field_info.replace(" ¦ ", "\r\n")
        } else {
            fmted_all_field_info
        };
        let record_data = vec![
            timestamp.as_str(),
            hostname.as_str(),
            abbr_channel.as_str(),
            event_id.as_str(),
            record_id.as_str(),
            event_title,
            all_field_info.as_str(),
            evtx_file.as_str(),
        ];
        if output.is_some() {
            file_wtr.as_mut().unwrap().write_record(&record_data).ok();
        } else {
            for (record_field_idx, record_field_data) in record_data.iter().enumerate() {
                let newline_flag = record_field_idx == record_data.len() - 1;
                if record_field_idx == 6 {
                    //AllFieldInfoの列の出力
                    let all_field_sep_info = all_field_info.split('¦').collect::<Vec<&str>>();
                    for (field_idx, fields) in all_field_sep_info.iter().enumerate() {
                        let mut separated_fields_data =
                            fields.split(':').map(|x| x.split_whitespace().join(" "));
                        write_color_buffer(
                            disp_wtr.as_mut().unwrap(),
                            Some(Color::Rgb(255, 158, 61)),
                            &format!("{}: ", separated_fields_data.next().unwrap()),
                            newline_flag,
                        )
                        .ok();
                        write_color_buffer(
                            disp_wtr.as_mut().unwrap(),
                            Some(Color::Rgb(0, 255, 255)),
                            separated_fields_data.join(":").trim(),
                            newline_flag,
                        )
                        .ok();
                        if field_idx != all_field_sep_info.len() - 1 {
                            write_color_buffer(
                                disp_wtr.as_mut().unwrap(),
                                None,
                                " ¦ ",
                                newline_flag,
                            )
                            .ok();
                        }
                    }
                } else if record_field_idx == 0 || record_field_idx == 5 {
                    //タイムスタンプとイベントタイトルは同じ色で表示
                    write_color_buffer(
                        disp_wtr.as_mut().unwrap(),
                        Some(Color::Rgb(0, 255, 0)),
                        record_field_data,
                        newline_flag,
                    )
                    .ok();
                } else {
                    write_color_buffer(
                        disp_wtr.as_mut().unwrap(),
                        None,
                        record_field_data,
                        newline_flag,
                    )
                    .ok();
                }

                if !newline_flag {
                    write_color_buffer(
                        disp_wtr.as_mut().unwrap(),
                        Some(Color::Rgb(238, 102, 97)),
                        " ‖ ",
                        false,
                    )
                    .ok();
                }
            }
        }
    }
}
