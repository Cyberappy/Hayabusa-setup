# Changes

## v1.3.0 [2022/xx/xx]

**New Features:**

- Added Event Frequency Timeline feature to visualize the number of events. (Note: There needs to be more than 5 events.) (#533)(@hitenkoku)
- Display all the `tags` defined in a rule to the `MitreAttack` column when saving to CSV file with the `--all-tags` option. (#525) (@hitenkoku)
- Added the `-R / --display-record-id` option: Display the event record ID (`<Event><System><EventRecordID>`). (#548) (@hitenkoku)
- Display date with most detections by level. (#550) (@hitenkoku)

**Enhancements:**

- In the `details` line in a rule, when a placeholder points to a field that does not exist or there is an incorrect alias mapping, it will be outputted as `n/a` (not available). (#528) (@hitenkoku)
- Display total event and data reduction count. (How many and what percent of events were ignored.) (#538) (@hitenkoku)
- New logo. (#536) (@YamatoSecurity)
- Display total evtx file size. (#540) (@hitenkoku)
- Changed logo color. (#537) (@hitenkoku)
- Display the original `Channel` name when not specified in `channel_abbrevations.txt`. (#553) (@hitenkoku)
- Display separately `Ignored rules` to `Exclude rules`, `Noisy rules`, and `Deprecated rules`. (#556) (@hitenkoku)

**Bug Fixes:**

- Fixed the `--start-timeline` and `--end-timeline` options as they were not working. (#546) @hitenkoku)


## v1.2.2 [2022/05/20]

**New Features:**

- Added a logon summary feature. (`-L` / `--logon-summary`) (@garigariganzy)

**Enhancements:**

- Colored output is now on by default and supports Command and Powershell prompts. (@hitenkoku)

**Bug Fixes:**

- Fixed a bug in the update feature when the rules repository does not exist but the rules folder exists. (#516) (@hitenkoku)
- Fixed a rule parsing error bug when there were .yml files in a .git folder. (#524) (@hitenkoku)
- Fixed wrong version number in the 1.2.1 binary.

## v1.2.1 [2022/04/20] Black Hat Asia Arsenal 2022 RC2

**New Features:**

- Added a `Channel` column to the output based on the `./config/channel_abbreviations.txt` config file. (@hitenkoku)
- Rule and rule config files are now forcefully updated. (@hitenkoku)

**Bug Fixes:**

- Rules marked as noisy or excluded would not have their `level` changed with `--level-tuning` but now all rules will be checked. (@hitenkoku)

## v1.2.0 [2022/04/15] Black Hat Asia Arsenal 2022 RC1

**New Features:**

- Specify config directory (`-C / --config`): When specifying a different rules directory, the rules config directory will still be the default `rules/config`, so this option is useful when you want to test rules and their config files in a different directory. (@hitenkoku) 
- `|equalsfield` aggregator: In order to write rules that compare if two fields are equal or not. (@hach1yon)
- Pivot keyword list generator feature (`-p / --pivot-keywords-list`): Will generate a list of keywords to grep for to quickly identify compromised machines, suspicious usernames, files, etc... (@kazuminn)
- `-F / --full-data` option: Will output all field information in addition to the fields defined in the rule’s `details`. (@hach1yon)
- `--level-tuning` option: You can tune the risk `level` in hayabusa and sigma rules to your environment. (@itib and @hitenkoku)

**Enhancements:**

- Updated detection rules and documentation. (@YamatoSecurity)
- Mac and Linux binaries now statically compile the OpenSSL libraries. (@YamatoSecurity)
- Performance and accuracy improvement for fields with tabs, etc... in them. (@hach1yon and @hitenkoku)
- Fields that are not defined in eventkey_alias.txt will automatically be searched in Event.EventData. (@kazuminn and @hitenkoku)
- When updating rules, the names of new rules as well as the count will be displayed. (@hitenkoku)
- Removed all Clippy warnings from the source code. (@hitenkoku and @hach1yon)
- Updated the event ID and title config file (`timeline_event_info.txt`) and changed the name to `statistics_event_info.txt`. (@YamatoSecurity and @garigariganzy)
- 32-bit Hayabusa Windows binaries are now prevented from running on 64-bit Windows as it would cause unexpected results. (@hitenkoku)
- MITRE ATT&CK tag output can be customized in `output_tag.txt`. (@hitenkoku)
- Added Channel column output. (@hitenkoku)

**Bug Fixes:**

- `.yml` files in the `.git` folder would cause parse errors so they are now ignored. (@hitenkoku)
- Removed unnecessary newline due to loading test file rules. (@hitenkoku)
- Fixed output stopping in Windows Terminal due a bug in Terminal itself. (@hitenkoku)

## v1.1.0 [2022/03/03]

**New Features:**

- Can specify a single rule with the `-r / --rules` option. (Great for testing rules!) (@kazuminn)
- Rule update option (`-u / --update-rules`): Update to the latest rules in the [hayabusa-rules](https://github.com/Yamato-Security/hayabusa-rules) repository. (@hitenkoku)
- Live analysis option (`-l / --live-analysis`): Can easily perform live analysis on Windows machines without specifying the Windows event log directory. (@hitenkoku) 

**Enhancements:**

- Updated documentation. (@kazuminn , @hitenkoku , @YamatoSecurity)
- Updated rules. (20+ Hayabusa rules, 200+ Sigma rules) (@YamatoSecurity)
- Windows binaries are now statically compiled so installing Visual C++ Redistributable is not required. (@hitenkoku)
- Color output (`-c / --color`) for terminals that support True Color (Windows Terminal, iTerm2, etc...). (@hitenkoku)
- MITRE ATT&CK tactics are included in the saved CSV output. (@hitenkoku)
- Performance improvement. (@hitenkoku)
- Comments added to exclusion and noisy config files. (@kazuminn)
- Using faster memory allocators (rpmalloc for Windows, jemalloc for macOS and Linux.) (@kazuminn)
- Updated cargo crates. (@YamatoSecurity)

**Bug Fixes:**

- Made the clap library version static to make `cargo update` more stable. (@hitenkoku)
- Some rules were not alerting if there were tabs or carriage returns in the fields. (@hitenkoku)

## v1.0.0-Release 2 [2022/01/27]

- Removed Excel result sample files as they were being flagged by anti-virus. (@YamatoSecurity)
- Updated the Rust evtx library to 0.7.2 (@YamatoSecurity)

## v1.0.0 [2021/12/25]

- Initial release.
