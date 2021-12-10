use crate::detections::print::AlertMessage;
use crate::detections::rule::AggResult;
use crate::detections::rule::AggregationParseInfo;
use crate::detections::rule::Message;
use crate::detections::rule::RuleNode;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::num::ParseIntError;

use crate::detections::rule::aggregation_parser::AggregationConditionToken;

use crate::detections::utils;

/// 検知された際にカウント情報を投入する関数
pub fn count(rule: &mut RuleNode, filepath: &String, record: &Value) {
    let key = create_count_key(&rule, record);
    let field_name: String;
    match get_agg_condition(&rule) {
        None => {
            field_name = "".to_owned();
        }
        Some(aggcondition) => {
            field_name = aggcondition
                ._field_name
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned();
        }
    }
    let field_value = get_alias_value_in_record(&field_name, record).unwrap_or("".to_owned());
    let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
    countup(
        rule,
        filepath,
        &key,
        &field_value,
        Message::get_event_time(record).unwrap_or(default_time),
    );
}

///count byの条件に合致する検知済みレコードの数を増やすための関数
pub fn countup(
    rule: &mut RuleNode,
    filepath: &String,
    key: &str,
    field_value: &str,
    record_time_value: DateTime<Utc>,
) {
    rule.countdata
        .entry(filepath.to_string())
        .or_insert(HashMap::new());
    let value_map = rule.countdata.get_mut(filepath).unwrap();
    value_map.entry(key.to_string()).or_insert(Vec::new());
    let mut prev_value = value_map[key].clone();
    prev_value.push(AggRecordTimeInfo {
        field_record_value: field_value.to_string(),
        record_time: record_time_value,
    });
    value_map.insert(key.to_string(), prev_value.to_vec());
}

/// 与えられたエイリアスから対象レコード内の値を取得してダブルクオーテーションを外す関数
fn get_alias_value_in_record(alias: &String, record: &Value) -> Option<String> {
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
                format!("alias not found.value:{}", alias),
            )
            .ok();
            return None;
        }
    };
}

/// ルール内のAggregationParseInfo(Aggregation Condition)を取得する関数
fn get_agg_condition(rule: &RuleNode) -> Option<&AggregationParseInfo> {
    match rule
        .detection
        .as_ref()
        .unwrap()
        .aggregation_condition
        .as_ref()
    {
        None => {
            return None;
        }
        Some(agg_parse_info) => {
            return Some(agg_parse_info);
        }
    }
}

/// countでgroupbyなどの情報を区分するためのハッシュマップのキーを作成する関数
pub fn create_count_key(rule: &RuleNode, record: &Value) -> String {
    let aggref = get_agg_condition(rule);
    let mut key = "_".to_string();
    if aggref.is_none() {
        return key;
    }
    let aggcondition = aggref.unwrap();
    // recordでaliasが登録されている前提とする
    if aggcondition._by_field_name.is_some() {
        let by_field_value = aggcondition._by_field_name.as_ref().unwrap();
        key = get_alias_value_in_record(by_field_value, record).unwrap_or("_".to_owned());
    }
    return key;
}

///現状のレコードの状態から条件式に一致しているかを判定する関数
pub fn aggregation_condition_select(rule: &RuleNode, filepath: &String) -> Vec<AggResult> {
    // recordでaliasが登録されている前提とする
    let value_map = rule.countdata.get(filepath).unwrap();
    let mut ret = Vec::new();
    for (key, value) in value_map {
        ret.append(&mut judge_timeframe(
            &rule,
            &filepath.to_string(),
            value,
            &key.to_string(),
        ));
    }
    return ret;
}

/// aggregation condition内での条件式を文字として返す関数
pub fn get_str_agg_eq(rule: &RuleNode) -> String {
    //この関数はaggregation ruleのパースが正常終了した後に呼ばれる想定のためOptionの判定は行わない
    let agg_condition = rule
        .detection
        .as_ref()
        .unwrap()
        .aggregation_condition
        .as_ref()
        .unwrap();
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

/// count済みデータ内でタイムフレーム内に存在するselectの条件を満たすレコードが、timeframe単位でcountの条件を満たしているAggResultを配列として返却する関数
pub fn judge_timeframe(
    rule: &RuleNode,
    filepath: &String,
    time_datas: &Vec<AggRecordTimeInfo>,
    key: &String,
) -> Vec<AggResult> {
    let mut ret: Vec<AggResult> = Vec::new();
    let mut time_data = time_datas.clone();
    time_data.sort_by(|a, b| b.record_time.cmp(&a.record_time));
    let aggcondition = get_agg_condition(rule).unwrap();
    let mut start_point = 0;
    // 最初はcountの条件として記載されている分のレコードを取得するためのindex指定
    let mut check_point = start_point + aggcondition._cmp_num - 1;
    // timeframeで指定された基準の値を秒数として保持
    let judge_sec_frame = get_sec_timeframe(&rule.detection.as_ref().unwrap().timeframe);
    let exist_field = aggcondition._field_name.is_some();
    let mut loaded_field_value: Vec<String> = Vec::new();
    loop {
        // 基準となるレコードもしくはcountを最低限満たす対象のレコードのindexが配列の領域を超えていた場合
        if start_point as usize >= time_data.len() || check_point as usize >= time_data.len() {
            // 最終のレコードを対象として時刻を確認する
            let check_point_date = time_data[time_data.len() - 1].clone();
            let diff = check_point_date.record_time.timestamp()
                - time_data[start_point as usize].record_time.timestamp();
            // 対象のレコード数を基準となるindexから計算
            let mut count_set_cnt = time_data.len() - (start_point as usize);
            // countのfieldがある場合種類での判別をする必要があるため
            if exist_field && !loaded_field_value.contains(&check_point_date.field_record_value) {
                loaded_field_value.push(check_point_date.field_record_value);
            }
            if judge_sec_frame.is_some() && diff > judge_sec_frame.unwrap() {
                //すでにcountを満たしている状態で1つずつdiffを確認している場合は適正な個数指定となり、もともとcountの条件が残りデータ個数より多い場合は-1したことによってcountの判定でもfalseになるため
                count_set_cnt -= count_set_cnt - 1;
            }

            // timeframe内に入っている場合があるため判定を行う
            let judge;
            let result_set_cnt;
            if exist_field {
                result_set_cnt = loaded_field_value.len() as i32;
                judge = select_aggcon(result_set_cnt, &aggcondition);
            } else {
                result_set_cnt = count_set_cnt as i32;
                judge = select_aggcon(result_set_cnt, &aggcondition);
            }
            if judge {
                ret.push(AggResult::new(
                    filepath.to_string(),
                    result_set_cnt,
                    key.to_string(),
                    time_data[start_point as usize].record_time,
                    get_str_agg_eq(rule),
                ));
            }
            // この段階ですべてのレコードのチェックが完了するため
            loaded_field_value.clear();
            break;
        }
        // 基準となるレコードと時刻比較を行う対象のレコード時刻情報を取得する
        let check_point_date = time_data[check_point as usize].clone();
        let diff = check_point_date.record_time.timestamp()
            - time_data[start_point as usize].record_time.timestamp();
        // timeframeで指定した情報と比較して時刻差がtimeframeの枠を超えていた場合(timeframeの属性を記載していない場合はこの処理を行わない)
        if judge_sec_frame.is_some() && diff > judge_sec_frame.unwrap() {
            let count_set_cnt = check_point - start_point;
            // timeframe内に入っている場合があるため判定を行う
            let judge;
            let result_set_cnt;
            if exist_field {
                result_set_cnt = loaded_field_value.len() as i32;
                judge = select_aggcon(result_set_cnt, &aggcondition);
            } else {
                result_set_cnt = count_set_cnt as i32;
                judge = select_aggcon(result_set_cnt, &aggcondition);
            }
            // timeframe内の対象のレコード数がcountの条件を満たさなかった場合、基準となるレコードを1つずらし、countの判定基準分のindexを設定して、次のレコードから始まるtimeframeの判定を行う
            if !judge {
                start_point += 1;
                check_point = start_point + aggcondition._cmp_num - 1;
                loaded_field_value.clear();
                continue;
            }
            //timeframe内の対象のレコード数がcountの条件を満たした場合は返却用の変数に結果を投入する
            ret.push(AggResult::new(
                filepath.to_string(),
                result_set_cnt,
                key.to_string(),
                time_data[start_point as usize].record_time,
                get_str_agg_eq(rule),
            ));
            // timeframe投入内の対象レコード数がcountの条件を満たした場合は、すでに判定済みのtimeframe内では同様に検知を行うことになり、過検知となってしまうため、今回timeframe内と判定された最後のレコードの次のレコードを次の基準として参照するようにindexを設定する
            start_point = check_point;
            check_point = start_point + aggcondition._cmp_num - 1;
            loaded_field_value.clear();
        } else {
            // timeframeで指定した情報と比較して、時刻差がtimeframeの枠を超えていない場合は次のレコード時刻情報を参照して、timeframe内であるかを判定するため
            check_point += 1;
            // countのfieldの値がある場合、fieldの値の種類を確認してストックされていなければ投入する
            if exist_field && !loaded_field_value.contains(&check_point_date.field_record_value) {
                loaded_field_value.push(check_point_date.field_record_value);
            }
        }
    }
    return ret;
}

#[cfg(test)]
mod tests {
    use crate::detections::detection::EvtxRecordInfo;
    use crate::detections::rule::create_rule;
    use crate::detections::rule::AggResult;
    use std::collections::HashMap;

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
        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 2);
        let expected_agg_result: Vec<AggResult> = vec![AggResult::new(
            "testpath".to_string(),
            2,
            "_".to_string(),
            default_time,
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
        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let record_time = Utc.ymd(1996, 2, 27).and_hms(1, 5, 1);
        let mut expected_count = HashMap::new();
        expected_count.insert("_".to_owned(), 2);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "_".to_string(),
            default_time,
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "_".to_string(),
            record_time,
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
        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("System_".to_owned(), 1);
        let expected_agg_result = AggResult::new(
            "testpath".to_string(),
            1,
            "System_".to_string(),
            default_time,
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

        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let record_time = Utc.ymd(1996, 2, 27).and_hms(1, 5, 1);
        let mut expected_count = HashMap::new();
        expected_count.insert("7040_System".to_owned(), 1);
        expected_count.insert("9999_Test".to_owned(), 1);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "7040_System".to_owned(),
            default_time,
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "9999_Test".to_owned(),
            record_time,
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
        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let record_time = Utc.ymd(1977, 1, 1).and_hms(0, 5, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("7040_Windows Event Log".to_owned(), 1);
        expected_count.insert("9999_Test".to_owned(), 1);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "7040_Windows Event Log".to_owned(),
            default_time,
            ">= 1".to_string(),
        ));
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            1,
            "9999_Test".to_owned(),
            record_time,
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
        assert_eq!(init_result.is_ok(), true);
        let target = vec![SIMPLE_RECORD_STR, record_str];
        for record in target {
            match serde_json::from_str(record) {
                Ok(rec) => {
                    let recinfo = EvtxRecordInfo {
                        evtx_filepath: "testpath".to_owned(),
                        record: rec,
                        data_string: String::default(),
                    };
                    let _result = rule_node.select(&"testpath".to_string(), &recinfo);
                }
                Err(_rec) => {
                    assert!(false, "failed to parse json record.");
                }
            }
        }
        //countupの関数が機能しているかを確認
        assert_eq!(
            *&rule_node
                .countdata
                .get("testpath")
                .unwrap()
                .get(&"7040_".to_owned())
                .unwrap()
                .len() as i32,
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
              "EventID": 7040,
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

        let default_time = Utc.ymd(1977, 1, 1).and_hms(0, 0, 0);
        let mut expected_count = HashMap::new();
        expected_count.insert("7040_System".to_owned(), 2);
        let mut expected_agg_result: Vec<AggResult> = Vec::new();
        expected_agg_result.push(AggResult::new(
            "testpath".to_string(),
            2,
            "7040_System".to_owned(),
            default_time,
            ">= 2".to_string(),
        ));
        check_count(
            rule_str,
            vec![SIMPLE_RECORD_STR, record_str],
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
                    let recinfo = EvtxRecordInfo {
                        evtx_filepath: "testpath".to_owned(),
                        record: record,
                        data_string: String::default(),
                    };
                    let result = &rule_node.select(&"testpath".to_owned(), &recinfo);
                    assert_eq!(result, &true);
                }
                Err(_rec) => {
                    assert!(false, "Failed to parse json record.");
                }
            }
        }
        let agg_results = &rule_node.judge_satisfy_aggcondition();
        let mut expect_filepath = vec![];
        let mut expect_data = vec![];
        let mut expect_key = vec![];
        let mut expect_start_timedate = vec![];
        let mut expect_condition_op_num = vec![];
        for expect_agg in expect_agg_results {
            let expect_count = expected_counts.get(&expect_agg.key).unwrap_or(&-1);
            //countupの関数が機能しているかを確認
            assert_eq!(
                *&rule_node
                    .countdata
                    .get("testpath")
                    .unwrap()
                    .get(&expect_agg.key)
                    .unwrap()
                    .len() as i32,
                *expect_count
            );
            expect_filepath.push(expect_agg.filepath);
            expect_data.push(expect_agg.data);
            expect_key.push(expect_agg.key);
            expect_start_timedate.push(expect_agg.start_timedate);
            expect_condition_op_num.push(expect_agg.condition_op_num);
        }
        for agg_result in agg_results {
            //ここですでにstart_timedateの格納を確認済み
            let index = expect_start_timedate
                .binary_search(&agg_result.start_timedate)
                .unwrap();
            assert_eq!(agg_result.filepath, expect_filepath[index]);
            assert_eq!(agg_result.data, expect_data[index]);
            assert_eq!(agg_result.key, expect_key[index]);
            assert_eq!(agg_result.condition_op_num, expect_condition_op_num[index]);
        }
    }
}
