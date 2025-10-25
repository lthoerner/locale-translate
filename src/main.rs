use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Arg, Command};
use deepl_api::{DeepL, TranslatableTextList, TranslationOptions};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Select};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use soft_canonicalize::soft_canonicalize;

const APP_DIR_PATH: &str = "./ltranslate";
const MANIFEST_PATH: &str = "./ltranslate/manifest.toml";
const SOURCE_LOCALE_HISTORY_PATH: &str = "./ltranslate/source-history.json";

type LocaleJsonData = JsonMap<String, JsonValue>;
type LocaleJsonDataAll = BTreeMap<String, LocaleJsonData>;

#[derive(Serialize, Deserialize)]
struct LocaleManifest {
    source_locale_path: PathBuf,
    locale_paths: BTreeMap<String, PathBuf>,
}

impl LocaleManifest {
    fn enabled_languages(&self, available_languages: &[Language]) -> Vec<Language> {
        available_languages
            .iter()
            .filter_map(|l| self.locale_paths.contains_key(&l.code).then_some(l.clone()))
            .collect()
    }
}

struct DeepLContext {
    api_connection: DeepL,
    translation_options: TranslationOptions,
    available_target_langs: Vec<Language>,
}

impl DeepLContext {
    fn get_target_language_if_available(&self, language_code: &str) -> Option<Language> {
        self.available_target_langs
            .iter()
            .find(|l| l.code == language_code)
            .cloned()
    }
}

#[derive(Clone, PartialEq)]
struct Language {
    code: String,
    name: String,
}

struct JsonMapDiff {
    changed_or_added: LocaleJsonData,
    removed: LocaleJsonData,
}

struct LanguageDiff {
    added: Vec<Language>,
    removed: Vec<Language>,
}

fn main() {
    let args = Command::new("ltranslate")
        .author("Lowell Thoerner, contact@lthoerner.com")
        .version(env!("CARGO_PKG_VERSION"))
        .about("A basic utility for parsing locale files and translating them to a given target language using DeepL.")
        .subcommand(
            Command::new("project")
                .about("Use project mode to automatically translate locales for you")
                .subcommand(Command::new("setup").about("Set up a new project and point it at your existing English locale file"))
                .subcommand(Command::new("manage").about("Alter project settings such as enabled languages"))
                .subcommand(Command::new("update").about("Check the English locale file for changes and update all other locales accordingly"))
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("translate")
                .about("Translate a single locale file in its entirety without engaging project mode")
                .arg(Arg::new("input_file").required(true).index(1))
                .arg(Arg::new("output_file").required(true).index(2))
                .arg(Arg::new("language").short('l').long("language").help(Some("Specify the traget language instead of picking it from a list (useful for scripts)")))
                .arg_required_else_help(true)
        )
        .arg_required_else_help(true)
        .get_matches();

    let deepl = connect_deepl();

    let Some((subcommand_name, subcommand_args)) = args.subcommand() else {
        exit("Missing subcommand. This is likely a logic bug.");
    };

    match subcommand_name {
        "project" => {
            let Some((project_sub, _project_args)) = subcommand_args.subcommand() else {
                exit("Missing subcommand. This is likely a logic bug.");
            };

            match project_sub {
                "setup" => {
                    let mut manifest_data = set_up_project();
                    let target_languages = select_target_languages(&deepl, None);
                    select_output_locale_all(&target_languages)
                        .into_iter()
                        .for_each(|(lang, path)| {
                            let _ = manifest_data.locale_paths.insert(lang, path);
                        });

                    if !confirm_prompt("Are you sure you want to translate these file(s)?") {
                        exit("Translation canceled.");
                    }

                    let source_locale_data = parse_locale(&manifest_data.source_locale_path);

                    eprintln!("Translation in progress. Please wait...");
                    let new_locale_data_all =
                        translate_locale_all(&deepl, &source_locale_data, target_languages);
                    eprintln!("Translation complete! Writing output data to file...");
                    write_locale_file_all(&manifest_data, new_locale_data_all);
                    write_appdata(manifest_data, Some(source_locale_data));
                }
                "manage" => {
                    let Some(mut manifest_data) = get_existing_manifest() else {
                        exit(
                            "Missing project data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
                        );
                    };

                    let deepl = connect_deepl();

                    let target_setting = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("What setting would you like to change?")
                        .items(&["source locale path", "enabled languages"])
                        .interact();

                    match target_setting {
                        Ok(0) => {
                            manifest_data.source_locale_path = select_source_locale();
                            write_appdata(manifest_data, None);
                        }
                        Ok(1) => {
                            let enabled_languages =
                                manifest_data.enabled_languages(&deepl.available_target_langs);
                            let new_selected_languages =
                                select_target_languages(&deepl, Some(&enabled_languages));

                            let diff = diff_languages(&enabled_languages, &new_selected_languages);
                            if let Some(diff) = diff {
                                for removed_lang in diff.removed {
                                    manifest_data.locale_paths.remove(&removed_lang.code);
                                }

                                for added_lang in diff.added {
                                    manifest_data.locale_paths.insert(
                                        added_lang.code.clone(),
                                        select_output_locale(&added_lang),
                                    );
                                }
                            }

                            write_appdata(manifest_data, None);
                        }
                        _ => exit("Unknown error occurred with the setting selector."),
                    }
                }
                "update" => {
                    let Some(manifest_data) = get_existing_manifest() else {
                        exit(
                            "Missing project data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
                        );
                    };

                    let source_locale_history =
                        parse_locale(&PathBuf::from(SOURCE_LOCALE_HISTORY_PATH));
                    let source_locale_current =
                        parse_locale(&PathBuf::from(&manifest_data.source_locale_path));

                    let Some(diff) = diff_locales(&source_locale_history, &source_locale_current)
                    else {
                        return;
                    };

                    let enabled_langs =
                        manifest_data.enabled_languages(&deepl.available_target_langs);

                    let current_locale_data_all = get_existing_locale_data_all(&manifest_data);
                    let mut new_locale_data_all =
                        remove_dead_keys(&diff.removed, &current_locale_data_all);

                    if !diff.changed_or_added.is_empty() {
                        let updated_translation_locale_data_all =
                            translate_locale_all(&deepl, &diff.changed_or_added, enabled_langs);
                        update_changed_or_added_keys(
                            updated_translation_locale_data_all,
                            &mut new_locale_data_all,
                        );
                    }

                    write_locale_file_all(&manifest_data, new_locale_data_all);
                    write_appdata(manifest_data, Some(source_locale_current));
                }
                _ => exit("Unknown subcommand. This is likely a logic bug."),
            }
        }
        "translate" => {
            let Some(input_file) = subcommand_args
                .get_one::<String>("input_file")
                .map(PathBuf::from)
            else {
                exit("Missing input file. This is likely a logic bug.");
            };

            let Some(output_file) = subcommand_args
                .get_one::<String>("output_file")
                .map(PathBuf::from)
            else {
                exit("Missing output file. This is likely a logic bug.");
            };

            let target_language = subcommand_args.get_one::<String>("language").cloned();

            full_translate_interactive(&deepl, input_file, output_file, target_language);
        }
        _ => exit("Unknown subcommand. This is likely a logic bug."),
    }
}

fn full_translate_interactive(
    deepl_context: &DeepLContext,
    input_file: PathBuf,
    output_file: PathBuf,
    target_language: Option<String>,
) {
    let target_language = match target_language {
        Some(language_code) => deepl_context
            .get_target_language_if_available(&language_code)
            .unwrap_or(select_target_language(deepl_context)),
        None => select_target_language(deepl_context),
    };

    if !confirm_prompt("Are you sure you want to translate this file?") {
        exit("Translation canceled.");
    }

    full_translate_noninteractive(deepl_context, input_file, output_file, target_language);
    eprintln!("Translation complete. Output has been written to file.");
}

fn full_translate_noninteractive(
    deepl_context: &DeepLContext,
    input_file: PathBuf,
    output_file: PathBuf,
    target_language: Language,
) {
    let input_locale = parse_locale(&input_file);
    let translated_data = translate_locale(
        &deepl_context,
        &input_locale,
        &get_translated_text(&input_locale),
        target_language,
    );

    write_locale_file(&output_file, translated_data);
}

fn get_existing_manifest() -> Option<LocaleManifest> {
    let data = std::fs::read_to_string(MANIFEST_PATH).ok()?;
    let Ok(manifest) = toml::from_str::<LocaleManifest>(&data) else {
        exit("Failed to parse manifest file.");
    };

    Some(manifest)
}

fn get_existing_locale_data_all(manifest_data: &LocaleManifest) -> LocaleJsonDataAll {
    let mut locale_data_all = LocaleJsonDataAll::new();
    for (lang_code, path) in manifest_data.locale_paths.iter() {
        locale_data_all.insert(lang_code.clone(), parse_locale(path));
    }

    locale_data_all
}

fn set_up_project() -> LocaleManifest {
    if !confirm_prompt(
        "It looks like you're using ltranslate for the first time. Would you like to set up a new project in the current directory?",
    ) {
        exit("Setup canceled.");
    }

    if !confirm_prompt("Do you have an English locale file ready to be translated?") {
        eprintln!("You will need an English locale file in order to set up ltranslate.");
        exit("Setup canceled.");
    }

    let english_locale_path = select_source_locale();

    LocaleManifest {
        source_locale_path: english_locale_path,
        locale_paths: BTreeMap::new(),
    }
}

fn connect_deepl() -> DeepLContext {
    let Ok(deepl_api_key) = std::env::var("DEEPL_API_KEY") else {
        exit("DeepL API key was not found. Set it using the DEEPL_API_KEY environment variable.");
    };

    let api_connection = DeepL::new(deepl_api_key);
    if !valid_deepl_api_key(&api_connection) {
        exit("Provided DeepL API key is invalid.");
    }

    let translation_options = TranslationOptions {
        split_sentences: None,
        preserve_formatting: Some(true),
        formality: None,
        glossary_id: None,
    };

    let Ok(available_target_langs) = api_connection.target_languages() else {
        exit(
            "Failed to fetch available target languages. This may be because of a connection issue with DeepL.",
        );
    };

    let available_target_langs = available_target_langs
        .into_iter()
        .map(|l| Language {
            code: l.language,
            name: l.name,
        })
        .collect();

    DeepLContext {
        api_connection,
        translation_options,
        available_target_langs,
    }
}

fn select_source_locale() -> PathBuf {
    loop {
        let english_locale_path: PathBuf =
            input_prompt("What is the name of the English locale file?").into();
        if !file_exists(&english_locale_path) {
            eprintln!("The file you specified does not exist. Please try again.");
            continue;
        }

        return english_locale_path;
    }
}

fn select_output_locale_all(target_languages: &[Language]) -> BTreeMap<String, PathBuf> {
    target_languages
        .iter()
        .map(|l| (l.code.clone(), select_output_locale(l)))
        .collect()
}

fn select_output_locale(target_language: &Language) -> PathBuf {
    loop {
        let output_locale_path: PathBuf = input_prompt(&format!(
            "[{}] What should the output file be called? Include the relative file path.",
            target_language.to_string()
        ))
        .into();

        if output_locale_path.exists() {
            eprintln!("The file you specified already exists. Please give it a different name.");
            continue;
        }

        return output_locale_path;
    }
}

fn parse_locale(locale_path: &Path) -> LocaleJsonData {
    let Ok(locale_data) = std::fs::read_to_string(locale_path) else {
        exit("Failed to open and read locale file.");
    };

    let Ok(locale_obj) = serde_json::from_str::<LocaleJsonData>(&locale_data) else {
        exit("Failed to parse locale file.");
    };

    locale_obj
}

fn select_target_language(deepl_context: &DeepLContext) -> Language {
    let Ok(lang_index) = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("What language do you want to translate to?")
        .items(&deepl_context.available_target_langs)
        .interact()
    else {
        exit("Unknown error occurred with language selector.")
    };

    deepl_context.available_target_langs[lang_index].clone()
}

fn select_target_languages(
    deepl_context: &DeepLContext,
    enabled_languages: Option<&[Language]>,
) -> Vec<Language> {
    let preselected_langs = match enabled_languages {
        Some(enabled_langs) => deepl_context
            .available_target_langs
            .iter()
            .map(|l| enabled_langs.contains(l))
            .collect(),
        None => Vec::new(),
    };

    let Ok(selected_lang_indices) = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("What languages do you want to translate to?")
        .items(&deepl_context.available_target_langs)
        .defaults(&preselected_langs)
        .interact()
    else {
        exit("Unknown error occurred with language selector.");
    };

    deepl_context
        .available_target_langs
        .iter()
        .enumerate()
        .filter_map(|(i, l)| selected_lang_indices.contains(&i).then_some(l.clone()))
        .collect()
}

fn create_locale_json(
    source_locale_data: &LocaleJsonData,
    translated_data: &[String],
) -> LocaleJsonData {
    let mut new_locale_json = JsonMap::new();
    for (i, key) in source_locale_data.keys().enumerate() {
        let translated_value = translated_data[i].clone();
        new_locale_json.insert(key.clone(), serde_json::Value::String(translated_value));
    }

    new_locale_json
}

fn write_locale_file_all(manifest_data: &LocaleManifest, mut locale_data_all: LocaleJsonDataAll) {
    for (lang_code, path) in manifest_data.locale_paths.iter() {
        let Some(locale_data) = locale_data_all.remove(lang_code) else {
            exit(&format!(
                "Missing translation data for language '{}'. This is likely a logic bug.",
                lang_code
            ));
        };

        write_locale_file(path, locale_data);
    }
}

fn write_locale_file(locale_path: &Path, locale_data: LocaleJsonData) {
    let Ok(mut locale_file) = File::create(&locale_path) else {
        exit("Failed to create output file.");
    };

    let Ok(locale_data) = serde_json::to_string_pretty(&locale_data) else {
        exit("Failed to format output data.");
    };

    let Ok(_) = locale_file.write_all(locale_data.as_bytes()) else {
        exit("Failed to write data to output file.");
    };
}

fn get_translated_text(locale_data: &LocaleJsonData) -> Vec<String> {
    locale_data
        .values()
        .map(|t| {
            let Some(t) = t.as_str() else {
                exit("Encountered non-string value in source locale data.");
            };

            t.to_owned()
        })
        .collect()
}

fn translate_locale_all(
    deepl_context: &DeepLContext,
    source_locale_data: &LocaleJsonData,
    target_languages: Vec<Language>,
) -> LocaleJsonDataAll {
    let source_locale_text = get_translated_text(&source_locale_data);

    target_languages
        .into_iter()
        .map(|l| {
            (
                l.code.clone(),
                translate_locale(deepl_context, &source_locale_data, &source_locale_text, l),
            )
        })
        .collect()
}

fn translate_locale(
    deepl_context: &DeepLContext,
    source_locale_data: &LocaleJsonData,
    source_locale_text: &[String],
    target_language: Language,
) -> LocaleJsonData {
    let text_to_translate = TranslatableTextList {
        source_language: Some("EN".to_string()),
        target_language: target_language.code,
        texts: source_locale_text.to_owned(),
    };

    let Ok(translated_values) = deepl_context.api_connection.translate(
        Some(deepl_context.translation_options.clone()),
        text_to_translate,
    ) else {
        exit("Failed to translate values. This may be because of a connection issue with DeepL.");
    };

    if translated_values.len() != source_locale_text.len() {
        exit("The number of translated values does not match the number of source values.");
    }

    let translated_text = translated_values
        .into_iter()
        .map(|t| t.text)
        .collect::<Vec<String>>();

    create_locale_json(source_locale_data, &translated_text)
}

fn confirm_prompt(prompt_text: &str) -> bool {
    let Ok(response) = Confirm::new().with_prompt(prompt_text).interact() else {
        exit("Unknown error occurred with the confirmation prompt.");
    };

    response
}

fn input_prompt(prompt_text: &str) -> String {
    let Ok(response) = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt_text)
        .interact_text()
    else {
        exit("Unknown error occurred with the input prompt.");
    };

    response
}

fn valid_deepl_api_key(deepl_api_connection: &DeepL) -> bool {
    deepl_api_connection.usage_information().is_ok()
}

fn diff_locales(original: &LocaleJsonData, current: &LocaleJsonData) -> Option<JsonMapDiff> {
    let changed_or_added = current
        .iter()
        .filter(|(k, v)| original.get(*k).map_or(true, |old_v| old_v != *v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<LocaleJsonData>();

    let removed = original
        .iter()
        .filter(|(k, _)| !current.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<LocaleJsonData>();

    if changed_or_added.is_empty() && removed.is_empty() {
        return None;
    }

    Some(JsonMapDiff {
        changed_or_added,
        removed,
    })
}

fn diff_languages(original: &[Language], current: &[Language]) -> Option<LanguageDiff> {
    let added = current
        .iter()
        .filter(|curr| !original.iter().any(|orig| orig.code == curr.code))
        .cloned()
        .collect::<Vec<_>>();

    let removed = original
        .iter()
        .filter(|orig| !current.iter().any(|curr| curr.code == orig.code))
        .cloned()
        .collect::<Vec<_>>();

    if added.is_empty() && removed.is_empty() {
        return None;
    }

    Some(LanguageDiff { added, removed })
}

fn remove_dead_keys(
    entries_to_remove: &LocaleJsonData,
    current_locale_data_all: &LocaleJsonDataAll,
) -> LocaleJsonDataAll {
    let mut new_locale_data_all = current_locale_data_all.clone();
    entries_to_remove.keys().for_each(|k| {
        for (lang, data) in new_locale_data_all.iter_mut() {
            let Some(_) = data.remove(k) else {
                exit(&format!(
                    "Failed to remove key '{}' from locale '{}'",
                    k, lang
                ));
            };
        }
    });

    new_locale_data_all
}

fn update_changed_or_added_keys(
    updated_locale_data_all: LocaleJsonDataAll,
    working_locale_data_all: &mut LocaleJsonDataAll,
) {
    for (lang, updated_data) in updated_locale_data_all.into_iter() {
        let Some(new_data) = working_locale_data_all.get_mut(&lang) else {
            exit(&format!(
                "Could not find locale data for language '{}'. This is likely a logic bug.",
                lang
            ));
        };

        updated_data.into_iter().for_each(|(k, v)| {
            let _ = new_data.insert(k, v);
        });
    }
}

fn write_appdata(manifest_data: LocaleManifest, locale_data: Option<LocaleJsonData>) {
    let Ok(formatted_data) = toml::to_string_pretty(&manifest_data) else {
        exit("Unknown error occured when serializing manifest data.");
    };

    create_app_directory_if_not_exists();

    if let Some(locale_data) = locale_data {
        write_locale_file(&PathBuf::from(SOURCE_LOCALE_HISTORY_PATH), locale_data);
    }

    let Ok(mut manifest_file) = File::create(MANIFEST_PATH) else {
        exit(&format!(
            "Failed to open manifest file. Ensure that the file permissions are set correctly. Please manually copy the data below into ltranslate/manifest.toml, then report this as a bug.\n{}",
            formatted_data
        ));
    };

    let Ok(_) = manifest_file.write_all(formatted_data.as_bytes()) else {
        exit(&format!(
            "Failed to write data to manifest file. Ensure that the file permissions are set correctly. Please manually copy the data below into ltranslate/manifest.toml, then report this as a bug.\n{}",
            formatted_data
        ));
    };
}

fn create_app_directory_if_not_exists() {
    if PathBuf::from(APP_DIR_PATH).exists() {
        return;
    }

    if std::fs::create_dir(APP_DIR_PATH).is_err() {
        exit(
            "Failed to create or write to ltranslate directory. Ensure that the file permissions are set correctly.",
        );
    }
}

fn file_exists(path: &Path) -> bool {
    let Ok(path) = soft_canonicalize(path) else {
        exit("Provided path was malformed.");
    };

    path.exists()
}

fn exit(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

impl ToString for &Language {
    fn to_string(&self) -> String {
        format!("{} ({})", self.code, self.name)
    }
}
