use crate::detections::print::AlertMessage;
use crate::detections::rule::AggResult;
use crate::detections::rule::AggregationParseInfo;
use crate::detections::rule::Message;
use crate::detections::rule::RuleNode;
use chrono::{DateTime, TimeZone, Utc};
use hashbrown::HashMap;
use serde_json::Value;
use std::num::ParseIntError;
use std::path::Path;

use crate::detections::rule::aggregation_parser::AggregationConditionToken;

use crate::detections::utils;

/// 検知された際にカウント情報を投入する関数
pub fn count(rule: &mut RuleNode, record: &Value) {
    let key = create_count_key(&rule, record);
    let field_name: String = match rule.get_agg_condition() {
        None => String::default(),
        Some(aggcondition) => aggcondition
            ._field_name
            .as_ref()
            .unwrap_or(&String::default())
            .to_owned(),
    };
    let field_value =
        get_alias_value_in_record(rule, &field_name, record, false).unwrap_or(String::default());
    let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
    countup(
        rule,
        key,
        field_value,
        Message::get_event_time(record).unwrap_or(default_time),
    );
}

///count byの条件に合致する検知済みレコードの数を増やすための関数
pub fn countup(
    rule: &mut RuleNode,
    key: String,
    field_value: String,
    record_time_value: DateTime<Utc>,
) {
    let value_map = rule.countdata.entry(key).or_insert(Vec::new());
    value_map.push(AggRecordTimeInfo {
        field_record_value: field_value,
        record_time: record_time_value,
    });
}

/// 与えられたエイリアスから対象レコード内の値を取得してダブルクオーテーションを外す関数。
///  ダブルクオーテーションを外す理由は結果表示の際に余計なダブルクオーテーションが入るのを防ぐため
/// is_by_aliasはこの関数を呼び出す際はcountのbyの値もしくはfieldの値のどちらかであるためboolとした
fn get_alias_value_in_record(
    rule: &RuleNode,
    alias: &String,
    record: &Value,
    is_by_alias: bool,
) -> Option<String> {
    if alias == "" {
        return None;
    }
    match utils::get_event_value(alias, record) {
        Some(value) => {
            return Some(value.to_string().replace("\"", ""));
        }
        None => {
            AlertMessage::alert(
                &mut std::io::stderr().lock(),
                match is_by_alias {
                    true => format!(
            "count by clause alias value not found in count process. rule file:{} EventID:{}",
            Path::new(&rule.rulepath)
              .file_name()
              .unwrap()
              .to_str()
              .unwrap(),
            utils::get_event_value(&utils::get_event_id_key(), record).unwrap()
          ),
                    false => format!(
            "count field clause alias value not found in count process. rule file:{} EventID:{}",
            Path::new(&rule.rulepath)
              .file_name()
              .unwrap()
              .to_str()
              .unwrap(),
            utils::get_event_value(&utils::get_event_id_key(), record).unwrap()
          ),
                },
            )
            .ok();
            return None;
        }
    };
}

/// countでgroupbyなどの情報を区分するためのハッシュマップのキーを作成する関数。
/// 以下の場合は空文字を返却
/// groupbyの指定がない、groubpbyで指定したエイリアスがレコードに存在しない場合は_のみとする。空文字ではキーを指定してデータを取得することができなかった
pub fn create_count_key(rule: &RuleNode, record: &Value) -> String {
    let agg_condition = rule.get_agg_condition().unwrap();
    if agg_condition._by_field_name.is_some() {
        let by_field_key = agg_condition._by_field_name.as_ref().unwrap();
        return get_alias_value_in_record(rule, by_field_key, record, true)
            .unwrap_or("_".to_string());
    } else {
        return "_".to_string();
    }
}

///現状のレコードの状態から条件式に一致しているかを判定する関数
pub fn aggregation_condition_select(rule: &RuleNode) -> Vec<AggResult> {
    // recordでaliasが登録されている前提とする
    let value_map = &rule.countdata;
    let mut ret = Vec::new();
    for (key, value) in value_map {
        ret.append(&mut judge_timeframe(&rule, &value, &key.to_string()));
    }
    return ret;
}

/// aggregation condition内での条件式を文字として返す関数
pub fn get_str_agg_eq(rule: &RuleNode) -> String {
    //この関数はaggregation ruleのパースが正常終了した後に呼ばれる想定のためOptionの判定は行わない
    let agg_condition = rule.detection.aggregation_condition.as_ref().unwrap();
    let mut ret: String = "".to_owned();
    match agg_condition._cmp_op {
        AggregationConditionToken::EQ => {
            ret.push_str("== ");
        }
        AggregationConditionToken::GE => {
            ret.push_str(">= ");
        }
        AggregationConditionToken::LE => {
            ret.push_str("<= ");
        }
        AggregationConditionToken::GT => {
            ret.push_str("> ");
        }
        AggregationConditionToken::LT => {
            ret.push_str("< ");
        }
        _ => {
            //想定しない演算子のため、空白文字で対応するものがない
            return "".to_string();
        }
    }
    ret.push_str(&agg_condition._cmp_num.to_string());
    return ret;
}

#[derive(Clone, Debug)]
/// countの括弧内の情報とレコードの情報を所持する構造体
pub struct AggRecordTimeInfo {
    pub field_record_value: String,
    pub record_time: DateTime<Utc>,
}

#[derive(Debug)]
/// timeframeに設定された情報。SIGMAルール上timeframeで複数の単位(日、時、分、秒)が複合で記載されているルールがなかったためタイプと数値のみを格納する構造体
pub struct TimeFrameInfo {
    pub timetype: String,
    pub timenum: Result<i64, ParseIntError>,
}

impl TimeFrameInfo {
    /// timeframeの文字列をパースし、構造体を返す関数
    pub fn parse_tframe(value: String) -> TimeFrameInfo {
        let mut ttype: String = "".to_string();
        let mut tnum = value.clone();
        if value.contains("s") {
            ttype = "s".to_owned();
            tnum.retain(|c| c != 's');
        } else if value.contains("m") {
            ttype = "m".to_owned();
            tnum.retain(|c| c != 'm');
        } else if value.contains("h") {
            ttype = "h".to_owned();
            tnum.retain(|c| c != 'h');
        } else if value.contains("d") {
            ttype = "d".to_owned();
            tnum.retain(|c| c != 'd');
        } else {
            AlertMessage::alert(
                &mut std::io::stderr().lock(),
                format!("Timeframe is invalid. Input value:{}", value),
            )
            .ok();
        }
        return TimeFrameInfo {
            timetype: ttype,
            timenum: tnum.parse::<i64>(),
        };
    }
}

/// TimeFrameInfoで格納されたtimeframeの値を秒数に変換した結果を返す関数
pub fn get_sec_timeframe(timeframe: &Option<TimeFrameInfo>) -> Option<i64> {
    if timeframe.is_none() {
        return Option::None;
    }
    let tfi = timeframe.as_ref().unwrap();
    match &tfi.timenum {
        Ok(n) => {
            if tfi.timetype == "d" {
                return Some(n * 86400);
            } else if tfi.timetype == "h" {
                return Some(n * 3600);
            } else if tfi.timetype == "m" {
                return Some(n * 60);
            } else {
                return Some(*n);
            }
        }
        Err(err) => {
            AlertMessage::alert(
                &mut std::io::stderr().lock(),
                format!("Timeframe number is invalid. timeframe.{}", err),
            )
            .ok();
            return Option::None;
        }
    }
}
/// conditionのパイプ以降の処理をAggregationParseInfoを参照し、conditionの条件を満たすか判定するための関数
pub fn select_aggcon(cnt: i32, aggcondition: &AggregationParseInfo) -> bool {
    match aggcondition._cmp_op {
        AggregationConditionToken::EQ => {
            if cnt == aggcondition._cmp_num {
                return true;
            } else {
                return false;
            }
        }
        AggregationConditionToken::GE => {
            if cnt >= aggcondition._cmp_num {
                return true;
            } else {
                return false;
            }
        }
        AggregationConditionToken::GT => {
            if cnt > aggcondition._cmp_num {
                return true;
            } else {
                return false;
            }
        }
        AggregationConditionToken::LE => {
            if cnt <= aggcondition._cmp_num {
                return true;
            } else {
                return false;
            }
        }
        AggregationConditionToken::LT => {
            if cnt < aggcondition._cmp_num {
                return true;
            } else {
                return false;
            }
        }
        _ => {
            return false;
        }
    }
}

/// condtionの分岐によって同じ型を返すif-letのジェネリクス
fn _if_condition_fn_caller<T: FnMut() -> S, S, U: FnMut() -> S>(
    condition: bool,
    mut process_true: T,
    mut process_false: U,
) -> S {
    if condition {
        process_true()
    } else {
        process_false()
    }
}

/// count済みデータ内でタイムフレーム内に存在するselectの条件を満たすレコードが、timeframe単位でcountの条件を満たしているAggResultを配列として返却する関数
pub fn judge_timeframe(
    rule: &RuleNode,
    time_datas: &Vec<AggRecordTimeInfo>,
    key: &String,
) -> Vec<AggResult> {
    let mut ret: Vec<AggResult> = Vec::new();
    let mut time_data = time_datas.clone();
    // 番兵
    let stop_time = Utc.ymd(9999, 12, 31).and_hms(23, 59, 59);
    let aggcondition = rule.detection.aggregation_condition.as_ref().unwrap();
    let exist_field = aggcondition._field_name.is_some();

    let mut start_point = 0;
    // timeframeで指定された基準の値を秒数として保持
    let judge_sec_frame = get_sec_timeframe(&rule.detection.timeframe);
    let mut loaded_field_value: HashMap<String, u128> = HashMap::new();

    let mut stop_time_datas: Vec<AggRecordTimeInfo> = (1..=aggcondition._cmp_num)
        .map(|_a| AggRecordTimeInfo {
            record_time: stop_time,
            field_record_value: "".to_string(),
        })
        .collect();

    time_data.append(&mut stop_time_datas);
    time_data.sort_by(|a, b| a.record_time.cmp(&b.record_time));

    // 次のチェックポイントのindexを取得する関数
    let get_next_checkpoint = |cal_point| {
        if cal_point + aggcondition._cmp_num - 1 > (time_data.len() - 1) as i32 {
            (time_data.len() - 1) as i32
        } else {
            cal_point + aggcondition._cmp_num - 1
        }
    };

    // 最初はcountの条件として記載されている分のレコードを取得するためのindex指定
    let mut check_point = get_next_checkpoint(start_point);

    *loaded_field_value
        .entry(time_data[0].field_record_value.to_string())
        .or_insert(0) += 1;

    while time_data[start_point as usize].record_time != stop_time
        && check_point < time_data.len() as i32
    {
        // 基準となるレコードと時刻比較を行う対象のレコード時刻情報を取得する
        let check_point_date = &time_data[check_point as usize];
        let diff = check_point_date.record_time.timestamp()
            - time_data[start_point as usize].record_time.timestamp();
        // timeframeで指定した情報と比較して時刻差がtimeframeの枠を超えていた場合
        if judge_sec_frame.is_some() && diff > judge_sec_frame.unwrap() {
            // 検査対象データが1個しかない状態でaggregation conditionの条件が1であるときにデータ個数が0になってしまう問題への対応
            let count_set_cnt = check_point - start_point;
            // timeframe内に入っている場合があるため判定を行う
            let result_set_cnt: i32 = _if_condition_fn_caller(
                exist_field,
                || {
                    time_data[(start_point as usize + 1)..(check_point as usize)]
                        .iter()
                        .for_each(|timedata| {
                            *loaded_field_value
                                .entry(timedata.field_record_value.to_string())
                                .or_insert(0) += 1;
                        });
                    loaded_field_value.len() as i32
                },
                || count_set_cnt as i32,
            );
            // timeframe内の対象のレコード数がcountの条件を満たさなかった場合、基準となるレコードを1つずらし、countの判定基準分のindexを設定して、次のレコードから始まるtimeframeの判定を行う
            if !select_aggcon(result_set_cnt, &aggcondition) {
                _if_condition_fn_caller(
                    exist_field && time_data[start_point as usize].record_time != stop_time,
                    || {
                        let counter = loaded_field_value
                            .entry(
                                time_data[start_point as usize]
                                    .field_record_value
                                    .to_string(),
                            )
                            .or_insert(1);
                        *counter -= 1;
                        if *counter == 0 as u128 {
                            loaded_field_value
                                .remove(&time_data[start_point as usize].field_record_value);
                        }
                    },
                    || {},
                );
                start_point += 1;
                check_point = get_next_checkpoint(start_point);
                continue;
            }
            let field_values: Vec<String> = loaded_field_value
                .keys()
                .filter(|key| **key != "")
                .map(|key| key.to_string())
                .collect();
            //timeframe内の対象のレコード数がcountの条件を満たした場合は返却用の変数に結果を投入する
            ret.push(AggResult::new(
                result_set_cnt,
                key.to_string(),
                field_values,
                time_data[start_point as usize].record_time,
                get_str_agg_eq(rule),
            ));
            // timeframe投入内の対象レコード数がcountの条件を満たした場合は、すでに判定済みのtimeframe内では同様に検知を行うことになり、過検知となってしまうため、今回timeframe内と判定された最後のレコードの次のレコードを次の基準として参照するようにindexを設定する
            start_point = check_point;
            check_point = get_next_checkpoint(start_point);
            loaded_field_value = HashMap::new();
            *loaded_field_value
                .entry(time_data[0].field_record_value.to_string())
                .or_insert(0) += 1;
        } else {
            // 条件の基準が1の時に最初の要素を2回読み込む事を防止するため
            _if_condition_fn_caller(
                check_point_date.record_time != stop_time && check_point != 0,
                || {
                    *loaded_field_value
                        .entry(check_point_date.field_record_value.to_string())
                        .or_insert(0) += 1;
                    ()
                },
                || {},
            );
            // timeframeで指定した情報と比較して、時刻差がtimeframeの枠を超えていない場合は次のレコード時刻情報を参照して、timeframe内であるかを判定するため
            check_point += 1;
        }
    }

    // timeframeがないルールの場合の判定(フィールドの読み込みはwhile内で実施済み)

    if judge_sec_frame.is_none() {
        if exist_field && select_aggcon(loaded_field_value.keys().len() as i32, &aggcondition) {
            let field_values: Vec<String> = loaded_field_value
                .keys()
                .filter(|key| **key != "")
                .map(|key| key.to_string())
                .collect();
            //timeframe内の対象のレコード数がcountの条件を満たした場合は返却用の変数に結果を投入する
            ret.push(AggResult::new(
                loaded_field_value.values().map(|value| *value as i32).sum(),
                key.to_string(),
                field_values,
                time_data[start_point as usize].record_time,
                get_str_agg_eq(rule),
            ));
        } else {
            if select_aggcon(
                *loaded_field_value.get("").unwrap_or(&0) as i32,
                &aggcondition,
            ) {
                //timeframe内の対象のレコード数がcountの条件を満たした場合は返却用の変数に結果を投入する
                ret.push(AggResult::new(
                    *loaded_field_value.get("").unwrap_or(&0) as i32,
                    key.to_string(),
                    vec![],
                    time_data[start_point as usize].record_time,
                    get_str_agg_eq(rule),
                ));
            }
        }
    }
    return ret;
}

#[cfg(test)]
mod tests {
    use crate::detections;
    use crate::detections::rule::create_rule;
    use crate::detections::rule::AggResult;
    use crate::detections::utils;
    use hashbrown::HashMap;

    use chrono::{TimeZone, Utc};
    use yaml_rust::YamlLoader;

    const SIMPLE_RECORD_STR: &str = r#"
    {
      "Event": {
        "System": {
          "EventID": 7040,
          "Channel": "System"
        },
        "EventData": {
          "param1": "Windows Event Log",
          "param2": "auto start"
        }
      },
      "Event_attributes": {
        "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
      }
    }"#;

    #[test]
    /// countのカッコ内の記載及びcount byの記載がない場合(timeframeなし)にruleで検知ができることのテスト
    fn test_count_no_field_and_by() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 7040,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1996-02-27T01:05:01Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'System'
            selection2:
                EventID: 7040
            selection3:
                param1: 'Windows Event Log'
            condition: selection1 and selection2 and selection3 | count() >= 1
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 2);
        let expected_agg_result: Vec<AggResult> = vec![AggResult::new(
            2,
            "_".to_string(),
            vec![],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 1".to_string(),
        )];
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    #[test]
    /// countのカッコ内の記載及びcount byの記載がない場合(timeframeあり)にruleで検知ができることのテスト
    fn test_count_no_field_and_by_with_timeframe() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 7040,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1996-02-27T01:05:01Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'System'
            selection2:
                EventID: 7040
            selection3:
                param1: 'Windows Event Log'
            condition: selection1 and selection2 and selection3 | count() >= 1
            timeframe: 15m
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 2);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            1,
            "_".to_string(),
            vec![],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            1,
            "_".to_string(),
            vec![],
            Utc.ymd(1996, 2, 27).and_hms(1, 5, 1),
            ">= 1".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    #[test]
    /// countでカッコ内の記載がある場合にruleでcountの検知ができることを確認する
    fn test_count_exist_field() {
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'System'
            selection2:
                EventID: 7040
            selection3:
                param1: 'Windows Event Log'
            condition: selection1 and selection2 and selection3 | count(Channel) >= 1
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 1);
        let expected_agg_result = AggResult::new(
            1,
            "_".to_string(),
            vec!["System".to_owned()],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 1".to_string(),
        );
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR],
            expected_count,
            vec![expected_agg_result],
        );
    }

    #[test]
    /// countでカッコ内の記載、byの記載両方がある場合にruleでcountの検知ができることを確認する
    fn test_count_exist_field_and_by() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 9999,
              "Channel": "Test",
              "TimeCreated_attributes": {
                "SystemTime": "1996-02-27T01:05:01Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                param1: 'Windows Event Log'
            condition: selection1 | count(EventID) by Channel >= 1
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;

        let mut expected_count = HashMap::new();
        expected_count.insert("System".to_owned(), 1);
        expected_count.insert("Test".to_owned(), 1);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            1,
            "System".to_owned(),
            vec!["7040".to_owned()],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            1,
            "Test".to_owned(),
            vec!["9999".to_owned()],
            Utc.ymd(1996, 2, 27).and_hms(1, 5, 1),
            ">= 1".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    #[test]
    /// countでカッコ内の記載、byの記載両方がある場合(複数レコードでカッコ内の指定する値が異なる場合)に値の組み合わせごとに分けてcountが実行していることを確認する
    fn test_count_exist_field_and_by_with_othervalue_in_timeframe() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 9999,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-01T00:05:00Z"
              }
            },
            "EventData": {
              "param1": "Test",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'System'
            condition: selection1 | count(EventID) by param1 >= 1
            timeframe: 1h
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;
        let mut expected_count = HashMap::new();
        expected_count.insert("Windows Event Log".to_owned(), 1);
        expected_count.insert("Test".to_owned(), 1);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            1,
            "Windows Event Log".to_owned(),
            vec!["7040".to_owned()],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            1,
            "Test".to_owned(),
            vec!["9999".to_owned()],
            Utc.ymd(1977, 1, 1).and_hms(0, 5, 0),
            ">= 1".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    #[test]
    /// countでtimeframeの条件によってruleのcountの条件を満たさない場合に空の配列を返すことを確認する
    fn test_count_not_satisfy_in_timeframe() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 7040,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-01T01:05:00Z"
              }
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                Channel: 'System'
            condition: selection1 | count(EventID) >= 2
            timeframe: 1h
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;
        let mut rule_yaml = YamlLoader::load_from_str(rule_str).unwrap().into_iter();
        let test = rule_yaml.next().unwrap();
        let mut rule_node = create_rule("testpath".to_string(), test);
        let init_result = rule_node.init();
        assert!(init_result.is_ok());
        let target = vec![SIMPLE_RECORD_STR, record_str];
        for record in target {
            match serde_json::from_str(record) {
                Ok(rec) => {
                    let keys = detections::rule::get_detection_keys(&rule_node);
                    let recinfo = utils::create_rec_info(rec, "testpath".to_owned(), &keys);
                    let _result = rule_node.select(&recinfo);
                }
                Err(_rec) => {
                    assert!(false, "failed to parse json record.");
                }
            }
        }
        //countupの関数が機能しているかを確認
        assert_eq!(
            *&rule_node.countdata.get(&"_".to_owned()).unwrap().len() as i32,
            2
        );
        let judge_result = rule_node.judge_satisfy_aggcondition();
        assert_eq!(judge_result.len(), 0);
    }
    #[test]
    /// countでカッコ内の記載、byの記載両方がありtimeframe内に存在する場合にruleでcountの検知ができることを確認する
    fn test_count_exist_field_and_by_with_timeframe() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 9999,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-01T00:05:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                param1: 'Windows Event Log'
            condition: selection1 | count(EventID) by Channel >= 2
            timeframe: 30m
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;

        let mut expected_count = HashMap::new();
        expected_count.insert("System".to_owned(), 2);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            2,
            "System".to_owned(),
            vec!["7040".to_owned(), "9999".to_owned()],
            Utc.ymd(1977, 1, 1).and_hms(0, 0, 0),
            ">= 2".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    #[test]
    /// countで括弧内の記載、byの記載両方がありtimeframe内に存在する場合にruleでcountの検知ができることを確認する(countの括弧内の項目が異なる場合)
    fn test_count_exist_field_and_by_with_timeframe_other_field_value() {
        let record_str: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 9999,
              "Channel": "System",
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-01T00:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log",
              "param2": "auto start"
            }
          },
          "Event_attributes": {
            "xmlns": "http://schemas.microsoft.com/win/2004/08/events/event"
          }
        }"#;
        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                param1: 'Windows Event Log'
            condition: selection1 | count(EventID) by Channel >= 1
            timeframe: 1h
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;

        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("System".to_owned(), 2);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            2,
            "System".to_owned(),
            vec!["7040".to_owned(), "9999".to_owned()],
            default_time,
            ">= 1".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
            expected_count,
            expected_agg_result,
        );
    }

    // timeframeの検査
    // timeframe=2hで、パイプ以降はcount(EventID) >= 3とする。
    //
    // このとき先頭の3行だと検知しないが、2行目から4行目は検知するはず
    // このように先頭行ではなく、途中から数えて検知するパターンをチェックする。
    // 0:30 EventID=1
    // 1:30 EventID=1
    // 2:30 EventID=2
    // 3:30 EventID=3
    #[test]
    fn test_count_timeframe() {
        let record_str1: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T00:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str2: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T01:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str3: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 2,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T02:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str4: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 3,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T03:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                param1: 'Windows Event Log'
            condition: selection1 | count(EventID) >= 3
            timeframe: 2h
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;

        let default_time = Utc.ymd(1977, 1, 9).and_hms(1, 30, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 4);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            3,
            "_".to_owned(),
            vec!["1".to_owned(), "2".to_owned(), "3".to_owned()],
            default_time,
            ">= 3".to_string(),
        ));
        check_count(
            rule_str,
            vec![record_str1, record_str2, record_str3, record_str4],
            expected_count,
            expected_agg_result,
        );
    }

    // timeframeの検査
    // timeframe=2hで、パイプ以降はcount(EventID) >= 3とする。
    //
    // このパターンをチェック
    // 0:30 EventID=1
    // 1:30 EventID=1
    // 2:30 EventID=2
    // 3:30 EventID=2
    // 4:30 EventID=3
    // 5:30 EventID=4
    // 19:00 EventID=1
    // 20:00 EventID=1
    // 21:00 EventID=3
    // 22:00 EventID=4
    #[test]
    fn test_count_timeframe2() {
        let record_str1: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T00:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str2: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T01:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str3: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 2,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T02:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str4: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 2,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T03:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str5: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 3,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T04:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str6: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 4,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T05:30:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str7: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T19:00:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str8: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 1,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T20:00:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str9: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 3,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T21:00:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let record_str10: &str = r#"
        {
          "Event": {
            "System": {
              "EventID": 4,
              "TimeCreated_attributes": {
                "SystemTime": "1977-01-09T22:00:00Z"
              }
            },
            "EventData": {
              "param1": "Windows Event Log"
            }
          }
        }"#;

        let rule_str = r#"
        enabled: true
        detection:
            selection1:
                param1: 'Windows Event Log'
            condition: selection1 | count(EventID) >= 3
            timeframe: 2h
        output: 'Service name : %param1%¥nMessage : Event Log Service Stopped¥nResults: Selective event log manipulation may follow this event.'
        "#;

        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 10);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            3,
            "_".to_owned(),
            vec!["2".to_owned(), "3".to_owned(), "4".to_owned()],
            Utc.ymd(1977, 1, 9).and_hms(3, 30, 0),
            ">= 3".to_string(),
        ));

        expected_agg_result.push(AggResult::new(
            3,
            "_".to_owned(),
            vec!["1".to_owned(), "3".to_owned(), "4".to_owned()],
            Utc.ymd(1977, 1, 9).and_hms(20, 00, 0),
            ">= 3".to_string(),
        ));
        check_count(
            rule_str,
            vec![
                record_str1,
                record_str2,
                record_str3,
                record_str4,
                record_str5,
                record_str6,
                record_str7,
                record_str8,
                record_str9,
                record_str10,
            ],
            expected_count,
            expected_agg_result,
        );
    }

    /// countで対象の数値確認を行うためのテスト用関数
    fn check_count(
        rule_str: &str,
        records_str: Vec<&str>,
        expected_counts: HashMap<String, i32>,
        expect_agg_results: Vec<AggResult>,
    ) {
        let mut rule_yaml = YamlLoader::load_from_str(rule_str).unwrap().into_iter();
        let test = rule_yaml.next().unwrap();
        let mut rule_node = create_rule("testpath".to_string(), test);
        let error_checker = rule_node.init();
        if error_checker.is_err() {
            assert!(false, "Failed to init rulenode");
        }
        for record_str in records_str {
            match serde_json::from_str(record_str) {
                Ok(record) => {
                    let keys = detections::rule::get_detection_keys(&rule_node);
                    let recinfo = utils::create_rec_info(record, "testpath".to_owned(), &keys);
                    let result = &rule_node.select(&recinfo);
                    assert_eq!(result, &true);
                }
                Err(_rec) => {
                    assert!(false, "Failed to parse json record.");
                }
            }
        }
        let agg_results = &rule_node.judge_satisfy_aggcondition();
        assert_eq!(agg_results.len(), expect_agg_results.len());

        let mut expect_data = vec![];
        let mut expect_key = vec![];
        let mut expect_field_values = vec![];
        let mut expect_start_timedate = vec![];
        let mut expect_condition_op_num = vec![];
        for expect_agg in expect_agg_results {
            let expect_count = expected_counts.get(&expect_agg.key).unwrap_or(&-1);
            //countupの関数が機能しているかを確認
            assert_eq!(
                *&rule_node.countdata.get(&expect_agg.key).unwrap().len() as i32,
                *expect_count
            );
            expect_data.push(expect_agg.data);
            expect_key.push(expect_agg.key);
            expect_field_values.push(expect_agg.field_values);
            expect_start_timedate.push(expect_agg.start_timedate);
            expect_condition_op_num.push(expect_agg.condition_op_num);
        }
        for agg_result in agg_results {
            //ここですでにstart_timedateの格納を確認済み
            let index = expect_start_timedate
                .binary_search(&agg_result.start_timedate)
                .unwrap();
            assert_eq!(agg_result.data, expect_data[index]);
            assert_eq!(agg_result.key, expect_key[index]);
            assert!(agg_result.field_values.len() == expect_field_values[index].len());
            for expect_field_value in &expect_field_values[index] {
                // テストによってはtimeframeの値と各fieldの値で配列の順番が想定したものと変化してしまう可能性があるため配列の長さを確認したうえで期待した各要素が存在するかを確認する。
                // field`要素の順番については以降の処理で関連しない
                assert!(agg_result.field_values.contains(&expect_field_value));
            }
            assert_eq!(agg_result.condition_op_num, expect_condition_op_num[index]);
        }
    }
}
