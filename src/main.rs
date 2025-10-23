use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use deepl_api::{DeepL, TranslatableTextList, TranslationOptions};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use soft_canonicalize::soft_canonicalize;

const APP_DIR_PATH: &str = "./locale-translate";
const MANIFEST_PATH: &str = "./locale-translate/manifest.toml";
const SOURCE_LOCALE_HISTORY_PATH: &str = "./locale-translate/source-history.json";

type LocaleJsonData = JsonMap<String, JsonValue>;

#[derive(Serialize, Deserialize)]
struct LocaleManifest {
    source_locale_path: PathBuf,
    locale_paths: HashMap<String, PathBuf>,
}

impl LocaleManifest {
    fn enabled_languages(&self, available_languages: &[Language]) -> Vec<Language> {
        available_languages
            .iter()
            .filter_map(|l| self.locale_paths.contains_key(&l.code).then_some(l.clone()))
            .collect()
    }

    fn unenabled_languages(&self, available_languages: &[Language]) -> Vec<Language> {
        available_languages
            .iter()
            .filter_map(|l| (!self.locale_paths.contains_key(&l.code)).then_some(l.clone()))
            .collect()
    }
}

struct DeepLContext {
    api_connection: DeepL,
    translation_options: TranslationOptions,
}

#[derive(Clone)]
struct Language {
    code: String,
    name: String,
}

struct JsonMapDiff {
    changed_or_added: LocaleJsonData,
    removed: LocaleJsonData,
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

fn main() {
    if let Some(mut manifest_data) = get_existing_manifest() {
        let deepl = connect_deepl();
        let source_locale_history = parse_locale(&PathBuf::from(SOURCE_LOCALE_HISTORY_PATH));
        let source_locale_current = parse_locale(&PathBuf::from(&manifest_data.source_locale_path));

        let Some(diff) = diff_locales(&source_locale_history, &source_locale_current) else {
            return;
        };

        println!("EDITED: {:#?}", diff.changed_or_added);
        println!("REMOVED: {:#?}", diff.removed);

        todo!();

        // Translate all changed values
        let available_target_langs = get_available_target_langs(&deepl);
        let translated_data_all = translate_locale_all(
            &deepl,
            &source_locale_current,
            manifest_data.enabled_languages(&available_target_langs),
        );

        // Get all locale data
        // Remove all deleted keys
        // Overwrite all updated values
        // Write all locale files
    } else {
        let mut manifest_data = set_up_project();
        let deepl = connect_deepl();
        let target_languages = select_target_languages(&deepl);
        select_output_locale_all(&target_languages)
            .into_iter()
            .for_each(|(lang, path)| {
                let _ = manifest_data.locale_paths.insert(lang, path);
            });

        // Check with user before continuing to avoid wasting API credit
        if !confirm_prompt("Are you sure you want to translate these file(s)?") {
            exit("Translation canceled.");
        }

        let source_locale_data = parse_locale(&manifest_data.source_locale_path);

        eprintln!("Translation in progress. Please wait...");
        let translated_data_all =
            translate_locale_all(&deepl, &source_locale_data, target_languages);
        eprintln!("Translation complete! Writing output data to file...");
        write_locale_file_all(&manifest_data, &source_locale_data, translated_data_all);
        write_appdata(manifest_data, source_locale_data);
    }
}

fn get_existing_manifest() -> Option<LocaleManifest> {
    let data = std::fs::read_to_string(MANIFEST_PATH).ok()?;
    let Ok(manifest) = toml::from_str::<LocaleManifest>(&data) else {
        exit("Failed to parse manifest file.");
    };

    Some(manifest)
}

fn set_up_project() -> LocaleManifest {
    if !confirm_prompt(
        "It looks like you're using locale-translate for the first time. Would you like to set up a new project in the current directory?",
    ) {
        exit("Setup canceled.");
    }

    if !confirm_prompt("Do you have an English locale file ready to be translated?") {
        eprintln!("You will need an English locale file in order to set up locale-translate.");
        exit("Setup canceled.");
    }

    let english_locale_path = loop {
        let english_locale_path: PathBuf =
            input_prompt("What is the name of the English locale file?").into();
        if !file_exists(&english_locale_path) {
            eprintln!("The file you specified does not exist. Please try again.");
            continue;
        }

        break english_locale_path;
    };

    LocaleManifest {
        source_locale_path: english_locale_path,
        locale_paths: HashMap::new(),
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

    DeepLContext {
        api_connection,
        translation_options,
    }
}

fn select_output_locale_all(target_languages: &[Language]) -> HashMap<String, PathBuf> {
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

fn parse_locale(locale_path: &Path) -> JsonMap<String, JsonValue> {
    let Ok(locale_data) = std::fs::read_to_string(locale_path) else {
        exit("Failed to open and read locale file.");
    };

    let Ok(locale_obj) = serde_json::from_str::<JsonMap<String, JsonValue>>(&locale_data) else {
        exit("Failed to parse locale file.");
    };

    locale_obj
}

fn select_target_languages(deepl_context: &DeepLContext) -> Vec<Language> {
    let available_target_langs = get_available_target_langs(deepl_context);
    let Ok(selected_lang_indices) = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("What languages do you want to translate to?")
        .items(&available_target_langs)
        .interact()
    else {
        exit("Unknown error occurred with language selector.");
    };

    available_target_langs
        .into_iter()
        .enumerate()
        .filter_map(|(i, l)| selected_lang_indices.contains(&i).then_some(l))
        .collect()
}

fn write_locale_file_all(
    manifest_data: &LocaleManifest,
    source_locale_data: &JsonMap<String, JsonValue>,
    translated_data_all: HashMap<String, Vec<String>>,
) {
    for (lang_code, path) in manifest_data.locale_paths.iter() {
        let Some(translated_data) = translated_data_all.get(lang_code) else {
            exit(&format!(
                "Missing translation data for language '{}'. This is likely a logic bug.",
                lang_code
            ));
        };

        write_locale_file(
            path,
            create_locale_json(source_locale_data, translated_data),
        );
    }
}

fn create_locale_json(
    source_locale_data: &JsonMap<String, JsonValue>,
    translated_data: &[String],
) -> JsonMap<String, JsonValue> {
    let mut new_locale_json = JsonMap::new();
    for (i, key) in source_locale_data.keys().enumerate() {
        let translated_value = translated_data[i].clone();
        new_locale_json.insert(key.clone(), serde_json::Value::String(translated_value));
    }

    new_locale_json
}

fn write_locale_file(locale_path: &Path, locale_data: JsonMap<String, JsonValue>) {
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

fn translate_locale_all(
    deepl_context: &DeepLContext,
    source_locale_data: &JsonMap<String, JsonValue>,
    target_languages: Vec<Language>,
) -> HashMap<String, Vec<String>> {
    let source_locale_text = source_locale_data
        .values()
        .map(|t| {
            let Some(t) = t.as_str() else {
                exit("Encountered non-string value in source locale data.");
            };

            t.to_owned()
        })
        .collect::<Vec<String>>();

    target_languages
        .into_iter()
        .map(|l| {
            (
                l.code.clone(),
                translate_locale(deepl_context, &source_locale_text, l),
            )
        })
        .collect()
}

fn translate_locale(
    deepl_context: &DeepLContext,
    source_locale_text: &[String],
    target_language: Language,
) -> Vec<String> {
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

    translated_values
        .into_iter()
        .map(|t| t.text)
        .collect::<Vec<String>>()
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

fn get_available_target_langs(deepl_context: &DeepLContext) -> Vec<Language> {
    let Ok(languages) = deepl_context.api_connection.target_languages() else {
        exit(
            "Failed to fetch available target languages. This may be because of a connection issue with DeepL.",
        );
    };

    languages
        .into_iter()
        .map(|l| Language {
            code: l.language,
            name: l.name,
        })
        .collect()
}

fn file_exists(path: &Path) -> bool {
    let Ok(path) = soft_canonicalize(path) else {
        exit("Provided path was malformed.");
    };

    path.exists()
}

fn write_appdata(manifest_data: LocaleManifest, locale_data: JsonMap<String, JsonValue>) {
    let Ok(formatted_data) = toml::to_string_pretty(&manifest_data) else {
        exit("Unknown error occured when serializing manifest data.");
    };

    create_app_directory_if_not_exists();

    write_locale_file(&PathBuf::from(SOURCE_LOCALE_HISTORY_PATH), locale_data);

    let Ok(mut manifest_file) = File::create(MANIFEST_PATH) else {
        exit(&format!(
            "Failed to open manifest file. Ensure that the file permissions are set correctly. Please manually copy the data below into locale-translate/manifest.toml, then report this as a bug.\n{}",
            formatted_data
        ));
    };

    let Ok(_) = manifest_file.write_all(formatted_data.as_bytes()) else {
        exit(&format!(
            "Failed to write data to manifest file. Ensure that the file permissions are set correctly. Please manually copy the data below into locale-translate/manifest.toml, then report this as a bug.\n{}",
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
            "Failed to create or write to locale-translate directory. Ensure that the file permissions are set correctly.",
        );
    }
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
