use hashbrown::{HashMap, HashSet};
use lazy_static::lazy_static;
use serde_json::Value;
use std::sync::RwLock;

use crate::detections::utils::get_serde_number_to_string;

use super::configs::EventKeyAliasConfig;

#[derive(Debug)]
pub struct PivotKeyword {
    pub keywords: HashSet<String>,
    pub fields: HashSet<String>,
}

lazy_static! {
    pub static ref PIVOT_KEYWORD: RwLock<HashMap<String, PivotKeyword>> =
        RwLock::new(HashMap::new());
}

impl Default for PivotKeyword {
    fn default() -> Self {
        Self::new()
    }
}

impl PivotKeyword {
    pub fn new() -> PivotKeyword {
        PivotKeyword {
            keywords: HashSet::new(),
            fields: HashSet::new(),
        }
    }
}

///levelがlowより大きいレコードの場合、keywordがrecord内にみつかれば、
///それをPIVOT_KEYWORD.keywordsに入れる。
pub fn insert_pivot_keyword(event_record: &Value, eventkey_alias: &EventKeyAliasConfig) {
    //levelがlow以上なら続ける
    let mut is_exist_event_key = false;
    let mut tmp_event_record: &Value = event_record;
    for s in ["Event", "System", "Level"] {
        if let Some(record) = tmp_event_record.get(s) {
            is_exist_event_key = true;
            tmp_event_record = record;
        }
    }
    if is_exist_event_key {
        let hash_value = get_serde_number_to_string(tmp_event_record);

        if hash_value.is_some() && hash_value.as_ref().unwrap() == "infomational"
            || hash_value.as_ref().unwrap() == "undefined"
            || hash_value.as_ref().unwrap() == "-"
        {
            return;
        }
    } else {
        return;
    }
    let mut pivots = PIVOT_KEYWORD.write().unwrap();
    pivots.iter_mut().into_iter().for_each(|(_, pivot)| {
        for field in &pivot.fields {
            if let Some(array_str) = eventkey_alias.get_event_key(&String::from(field)) {
                let split: Vec<&str> = array_str.split('.').collect();
                let mut is_exist_event_key = false;
                let mut tmp_event_record: &Value = event_record;
                for s in split {
                    if let Some(record) = tmp_event_record.get(s) {
                        is_exist_event_key = true;
                        tmp_event_record = record;
                    }
                }
                if is_exist_event_key {
                    let hash_value = get_serde_number_to_string(tmp_event_record);

                    if let Some(value) = hash_value {
                        if value == "-" || value == "127.0.0.1" || value == "::1" {
                            continue;
                        }
                        pivot.keywords.insert(value);
                    };
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::detections::configs::load_eventkey_alias;
    use crate::detections::configs::load_pivot_keywords;
    use crate::detections::configs::CURRENT_EXE_PATH;
    use crate::detections::pivot::insert_pivot_keyword;
    use crate::detections::pivot::PIVOT_KEYWORD;
    use crate::detections::utils;
    use serde_json;

    #[test]
    fn insert_pivot_keyword_local_ip4() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "high"
                },
                "EventData": {
                    "IpAddress": "127.0.0.1"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(!PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("127.0.0.1"));
    }

    #[test]
    fn insert_pivot_keyword_ip4() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "high"
                },
                "EventData": {
                    "IpAddress": "10.0.0.1"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("10.0.0.1"));
    }

    #[test]
    fn insert_pivot_keyword_ip_empty() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "high"
                },
                "EventData": {
                    "IpAddress": "-"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(!PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("-"));
    }

    #[test]
    fn insert_pivot_keyword_local_ip6() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "high"
                },
                "EventData": {
                    "IpAddress": "::1"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(!PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("::1"));
    }

    #[test]
    fn insert_pivot_keyword_level_infomational() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "infomational"
                },
                "EventData": {
                    "IpAddress": "10.0.0.2"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(!PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("10.0.0.2"));
    }

    #[test]
    fn insert_pivot_keyword_level_low() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "low"
                },
                "EventData": {
                    "IpAddress": "10.0.0.1"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("10.0.0.1"));
    }

    #[test]
    fn insert_pivot_keyword_level_none() {
        PIVOT_KEYWORD.write().unwrap().clear();
        load_pivot_keywords("test_files/config/pivot_keywords.txt");
        let record_json_str = r#"
        {
            "Event": { 
                "System": {
                    "Level": "-"
                },
                "EventData": {
                    "IpAddress": "10.0.0.3"
                }
            }
        }"#;
        insert_pivot_keyword(
            &serde_json::from_str(record_json_str).unwrap(),
            &load_eventkey_alias(
                utils::check_setting_path(
                    &CURRENT_EXE_PATH.to_path_buf(),
                    "rules/config/eventkey_alias.txt",
                    true,
                )
                .unwrap()
                .to_str()
                .unwrap(),
            ),
        );

        assert!(!PIVOT_KEYWORD
            .write()
            .unwrap()
            .get_mut("Ip Addresses")
            .unwrap()
            .keywords
            .contains("10.0.0.3"));
    }
}
