extern crate serde_derive;
extern crate yaml_rust;

use crate::detections::configs::{self, StoredStatic};
use crate::detections::message::AlertMessage;
use crate::detections::message::ERROR_LOG_STACK;
use crate::filter::RuleExclude;
use hashbrown::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use yaml_rust::Yaml;
use yaml_rust::YamlLoader;

pub struct ParseYaml {
    pub files: Vec<(String, yaml_rust::Yaml)>,
    pub rulecounter: HashMap<String, u128>,
    pub rule_load_cnt: HashMap<String, u128>,
    pub rule_status_cnt: HashMap<String, u128>,
    pub errorrule_count: u128,
    pub exclude_status: HashSet<String>,
    pub level_map: HashMap<String, u128>,
}

impl ParseYaml {
    pub fn new(stored_static: &StoredStatic) -> ParseYaml {
        let exclude_status_vec = if let Some(output_option) = stored_static.output_option.as_ref() {
            output_option.exclude_status.clone()
        } else {
            None
        };
        ParseYaml {
            files: Vec::new(),
            rulecounter: HashMap::new(),
            rule_load_cnt: HashMap::from([
                ("excluded".to_string(), 0_u128),
                ("noisy".to_string(), 0_u128),
            ]),
            rule_status_cnt: HashMap::from([("deprecated".to_string(), 0_u128)]),
            errorrule_count: 0,
            exclude_status: configs::convert_option_vecs_to_hs(exclude_status_vec.as_ref()),
            level_map: HashMap::from([
                ("INFORMATIONAL".to_owned(), 1),
                ("LOW".to_owned(), 2),
                ("MEDIUM".to_owned(), 3),
                ("HIGH".to_owned(), 4),
                ("CRITICAL".to_owned(), 5),
            ]),
        }
    }

    pub fn read_file(&self, path: PathBuf) -> Result<String, String> {
        let mut file_content = String::new();

        let mut fr = fs::File::open(path)
            .map(BufReader::new)
            .map_err(|e| e.to_string())?;

        fr.read_to_string(&mut file_content)
            .map_err(|e| e.to_string())?;

        Ok(file_content)
    }

    pub fn read_dir<P: AsRef<Path>>(
        &mut self,
        path: P,
        level: &str,
        exclude_ids: &RuleExclude,
        stored_static: &StoredStatic,
    ) -> io::Result<String> {
        let metadata = fs::metadata(path.as_ref());
        if metadata.is_err() {
            let errmsg = format!(
                "fail to read metadata of file: {}",
                path.as_ref().to_path_buf().display(),
            );
            if stored_static.config.verbose {
                AlertMessage::alert(&errmsg)?;
            }
            if !stored_static.quiet_errors_flag {
                ERROR_LOG_STACK
                    .lock()
                    .unwrap()
                    .push(format!("[ERROR] {}", errmsg));
            }
            return io::Result::Ok(String::default());
        }
        let mut yaml_docs = vec![];
        if metadata.unwrap().file_type().is_file() {
            // 拡張子がymlでないファイルは無視
            if path
                .as_ref()
                .to_path_buf()
                .extension()
                .unwrap_or_else(|| OsStr::new(""))
                != "yml"
            {
                return io::Result::Ok(String::default());
            }

            // 個別のファイルの読み込みは即終了としない。
            let read_content = self.read_file(path.as_ref().to_path_buf());
            if read_content.is_err() {
                let errmsg = format!(
                    "fail to read file: {}\n{} ",
                    path.as_ref().to_path_buf().display(),
                    read_content.unwrap_err()
                );
                if stored_static.config.verbose {
                    AlertMessage::warn(&errmsg)?;
                }
                if !stored_static.quiet_errors_flag {
                    ERROR_LOG_STACK
                        .lock()
                        .unwrap()
                        .push(format!("[WARN] {}", errmsg));
                }
                self.errorrule_count += 1;
                return io::Result::Ok(String::default());
            }

            // ここも個別のファイルの読み込みは即終了としない。
            let yaml_contents = YamlLoader::load_from_str(&read_content.unwrap());
            if yaml_contents.is_err() {
                let errmsg = format!(
                    "Failed to parse yml: {}\n{} ",
                    path.as_ref().to_path_buf().display(),
                    yaml_contents.unwrap_err()
                );
                if stored_static.config.verbose {
                    AlertMessage::warn(&errmsg)?;
                }
                if !stored_static.quiet_errors_flag {
                    ERROR_LOG_STACK
                        .lock()
                        .unwrap()
                        .push(format!("[WARN] {}", errmsg));
                }
                self.errorrule_count += 1;
                return io::Result::Ok(String::default());
            }

            yaml_docs.extend(yaml_contents.unwrap().into_iter().map(|yaml_content| {
                let filepath = format!("{}", path.as_ref().to_path_buf().display());
                (filepath, yaml_content)
            }));
        } else {
            let mut entries = fs::read_dir(path)?;
            yaml_docs = entries.try_fold(vec![], |mut ret, entry| {
                let entry = entry?;
                // フォルダは再帰的に呼び出す。
                if entry.file_type()?.is_dir() {
                    self.read_dir(entry.path(), level, exclude_ids, stored_static)?;
                    return io::Result::Ok(ret);
                }
                // ファイル以外は無視
                if !entry.file_type()?.is_file() {
                    return io::Result::Ok(ret);
                }

                // 拡張子がymlでないファイルは無視
                let path = entry.path();
                if path.extension().unwrap_or_else(|| OsStr::new("")) != "yml" {
                    return io::Result::Ok(ret);
                }

                // ignore if yml file in .git folder.
                if path.to_str().unwrap().contains("/.git/")
                    || path.to_str().unwrap().contains("\\.git\\")
                {
                    return io::Result::Ok(ret);
                }

                // ignore if tool test yml file in hayabusa-rules.
                if path
                    .to_str()
                    .unwrap()
                    .contains("rules/tools/sigmac/test_files")
                    || path
                        .to_str()
                        .unwrap()
                        .contains("rules\\tools\\sigmac\\test_files")
                {
                    return io::Result::Ok(ret);
                }

                // 個別のファイルの読み込みは即終了としない。
                let read_content = self.read_file(path);
                if read_content.is_err() {
                    let errmsg = format!(
                        "fail to read file: {}\n{} ",
                        entry.path().display(),
                        read_content.unwrap_err()
                    );
                    if stored_static.config.verbose {
                        AlertMessage::warn(&errmsg)?;
                    }
                    if !stored_static.quiet_errors_flag {
                        ERROR_LOG_STACK
                            .lock()
                            .unwrap()
                            .push(format!("[WARN] {}", errmsg));
                    }
                    self.errorrule_count += 1;
                    return io::Result::Ok(ret);
                }

                // ここも個別のファイルの読み込みは即終了としない。
                let yaml_contents = YamlLoader::load_from_str(&read_content.unwrap());
                if yaml_contents.is_err() {
                    let errmsg = format!(
                        "Failed to parse yml: {}\n{} ",
                        entry.path().display(),
                        yaml_contents.unwrap_err()
                    );
                    if stored_static.config.verbose {
                        AlertMessage::warn(&errmsg)?;
                    }
                    if !stored_static.quiet_errors_flag {
                        ERROR_LOG_STACK
                            .lock()
                            .unwrap()
                            .push(format!("[WARN] {}", errmsg));
                    }
                    self.errorrule_count += 1;
                    return io::Result::Ok(ret);
                }

                let yaml_contents = yaml_contents.unwrap().into_iter().map(|yaml_content| {
                    let filepath = format!("{}", entry.path().display());
                    (filepath, yaml_content)
                });
                ret.extend(yaml_contents);
                io::Result::Ok(ret)
            })?;
        }

        let files: Vec<(String, Yaml)> = yaml_docs
            .into_iter()
            .filter_map(|(filepath, yaml_doc)| {
                //除外されたルールは無視する
                let rule_id = &yaml_doc["id"].as_str();
                if rule_id.is_some() {
                    if let Some(v) = exclude_ids
                        .no_use_rule
                        .get(&rule_id.unwrap_or(&String::default()).to_string())
                    {
                        let entry_key = if v.contains("exclude_rule") {
                            "excluded"
                        } else {
                            "noisy"
                        };
                        // テスト用のルール(ID:000...0)の場合はexcluded ruleのカウントから除外するようにする
                        if v != "00000000-0000-0000-0000-000000000000" {
                            let entry =
                                self.rule_load_cnt.entry(entry_key.to_string()).or_insert(0);
                            *entry += 1;
                        }
                        let enable_noisy_rules =
                            if let Some(o) = stored_static.output_option.as_ref() {
                                o.enable_noisy_rules
                            } else {
                                false
                            };

                        if entry_key == "excluded" || (entry_key == "noisy" && !enable_noisy_rules)
                        {
                            return Option::None;
                        }
                    }
                }

                let status = &yaml_doc["status"].as_str();
                if let Some(s) = status {
                    if self.exclude_status.contains(&s.to_string()) {
                        let entry = self
                            .rule_load_cnt
                            .entry("excluded".to_string())
                            .or_insert(0);
                        *entry += 1;
                        return Option::None;
                    }
                }

                self.rulecounter.insert(
                    yaml_doc["ruletype"].as_str().unwrap_or("Other").to_string(),
                    self.rulecounter
                        .get(&yaml_doc["ruletype"].as_str().unwrap_or("Other").to_string())
                        .unwrap_or(&0)
                        + 1,
                );

                let status_cnt = self
                    .rule_status_cnt
                    .entry(
                        yaml_doc["status"]
                            .as_str()
                            .unwrap_or("undefined")
                            .to_string(),
                    )
                    .or_insert(0);
                *status_cnt += 1;

                if stored_static.config.verbose {
                    println!("Loaded yml file path: {}", filepath);
                }

                // 指定されたレベルより低いルールは無視する
                let doc_level = &yaml_doc["level"]
                    .as_str()
                    .unwrap_or("informational")
                    .to_string()
                    .to_uppercase();
                let doc_level_num = self.level_map.get(doc_level).unwrap_or(&1);
                let args_level_num = self.level_map.get(level).unwrap_or(&1);
                if doc_level_num < args_level_num {
                    return Option::None;
                }
                Option::Some((filepath, yaml_doc))
            })
            .collect();
        self.files.extend(files);
        io::Result::Ok(String::default())
    }
}

#[cfg(test)]
mod tests {

    use crate::detections::configs::Action;
    use crate::detections::configs::Config;
    use crate::detections::configs::StoredStatic;
    use crate::detections::configs::UpdateOption;
    use crate::filter;
    use crate::yaml;
    use crate::yaml::RuleExclude;
    use hashbrown::HashMap;
    use std::path::Path;
    use yaml_rust::YamlLoader;

    fn create_dummy_stored_static() -> StoredStatic {
        StoredStatic::create_static_data(&Config {
            config: Path::new("./rules/config").to_path_buf(),
            action: Action::UpdateRules(UpdateOption {
                rules: Path::new("./rules").to_path_buf(),
            }),
            thread_number: None,
            no_color: false,
            quiet: false,
            quiet_errors: false,
            debug: false,
            list_profile: false,
            verbose: false,
        })
    }

    #[test]
    fn test_read_file_yaml() {
        let exclude_ids = RuleExclude::default();
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        let _ = &yaml.read_dir(
            "test_files/rules/yaml/1.yml",
            &String::default(),
            &exclude_ids,
            &dummy_stored_static,
        );
        assert_eq!(yaml.files.len(), 1);
    }

    #[test]
    fn test_read_dir_yaml() {
        let exclude_ids = RuleExclude {
            no_use_rule: HashMap::new(),
        };
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        let _ = &yaml.read_dir(
            "test_files/rules/yaml/",
            &String::default(),
            &exclude_ids,
            &dummy_stored_static,
        );
        assert_ne!(yaml.files.len(), 0);
    }

    #[test]
    fn test_read_yaml() {
        let yaml = yaml::ParseYaml::new(&create_dummy_stored_static());
        let path = Path::new("test_files/rules/yaml/1.yml");
        let ret = yaml.read_file(path.to_path_buf()).unwrap();
        let rule = YamlLoader::load_from_str(&ret).unwrap();
        for i in rule {
            if i["title"].as_str().unwrap() == "Sysmon Check command lines" {
                assert_eq!(
                    "*",
                    i["detection"]["selection"]["CommandLine"].as_str().unwrap()
                );
                assert_eq!(1, i["detection"]["selection"]["EventID"].as_i64().unwrap());
            }
        }
    }

    #[test]
    fn test_failed_read_yaml() {
        let yaml = yaml::ParseYaml::new(&create_dummy_stored_static());
        let path = Path::new("test_files/rules/yaml/error.yml");
        let ret = yaml.read_file(path.to_path_buf()).unwrap();
        let rule = YamlLoader::load_from_str(&ret);
        assert!(rule.is_err());
    }

    #[test]
    /// no specifed "level" arguments value is adapted default level(informational)
    fn test_default_level_read_yaml() {
        let path = Path::new("test_files/rules/level_yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 5);
    }

    #[test]
    fn test_info_level_read_yaml() {
        let dummy_stored_static = create_dummy_stored_static();
        let path = Path::new("test_files/rules/level_yaml");
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "informational",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 5);
    }
    #[test]
    fn test_low_level_read_yaml() {
        let path = Path::new("test_files/rules/level_yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "LOW",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 4);
    }
    #[test]
    fn test_medium_level_read_yaml() {
        let path = Path::new("test_files/rules/level_yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "MEDIUM",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 3);
    }
    #[test]
    fn test_high_level_read_yaml() {
        let path = Path::new("test_files/rules/level_yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "HIGH",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 2);
    }
    #[test]
    fn test_critical_level_read_yaml() {
        let path = Path::new("test_files/rules/level_yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "CRITICAL",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.files.len(), 1);
    }
    #[test]
    fn test_all_exclude_rules_file() {
        let path = Path::new("test_files/rules/yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.rule_load_cnt.get("excluded").unwrap().to_owned(), 5);
    }
    #[test]
    fn test_all_noisy_rules_file() {
        let path = Path::new("test_files/rules/yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        yaml.read_dir(
            path,
            "",
            &filter::exclude_ids(&dummy_stored_static),
            &dummy_stored_static,
        )
        .unwrap();
        assert_eq!(yaml.rule_load_cnt.get("noisy").unwrap().to_owned(), 5);
    }
    #[test]
    fn test_none_exclude_rules_file() {
        let path = Path::new("test_files/rules/yaml");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        let exclude_ids = RuleExclude::default();
        yaml.read_dir(path, "", &exclude_ids, &dummy_stored_static)
            .unwrap();
        assert_eq!(yaml.rule_load_cnt.get("excluded").unwrap().to_owned(), 0);
    }
    #[test]
    fn test_exclude_deprecated_rules_file() {
        let path = Path::new("test_files/rules/deprecated");
        let dummy_stored_static = create_dummy_stored_static();
        let mut yaml = yaml::ParseYaml::new(&dummy_stored_static);
        let exclude_ids = RuleExclude::default();
        yaml.read_dir(path, "", &exclude_ids, &dummy_stored_static)
            .unwrap();
        assert_eq!(
            yaml.rule_status_cnt.get("deprecated").unwrap().to_owned(),
            1
        );
    }
}
