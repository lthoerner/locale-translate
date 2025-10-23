use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use deepl_api::{DeepL, TranslatableTextList, TranslationOptions};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use soft_canonicalize::soft_canonicalize;

const APP_DIR_PATH: &str = "./locale-translate";
const MANIFEST_PATH: &str = "./locale-translate/manifest.toml";

#[derive(Serialize, Deserialize)]
struct LocaleManifest {
    source_locale_path: String,
    locale_paths: HashMap<String, String>,
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

fn main() {
    let locale_manifest_data = match std::fs::read_to_string(MANIFEST_PATH) {
        // If the manifest file already exists, read and parse it
        Ok(data) => {
            let Ok(locale_manifest) = toml::from_str::<LocaleManifest>(&data) else {
                exit("Faield to parse manifest file.");
            };

            locale_manifest
        }
        // If the manifest file does not exist, set up a new project
        Err(_) => set_up_project(),
    };

    let deepl = connect_deepl();
    let input_locale_data = parse_input_locale(&select_input_locale());
    let target_language = select_target_language(&deepl);
    let output_locale_path = select_output_locale();

    // Check with user before continuing to avoid wasting API credit
    if !confirm_prompt("Are you sure you want to translate this file?") {
        exit("Translation canceled.");
    }

    let translated_locale_data = translate_locale(&deepl, &input_locale_data, target_language);

    eprintln!("Translation complete! Writing output data to file...");
    write_locale_file(
        &output_locale_path,
        &input_locale_data,
        &translated_locale_data,
    );

    write_manifest(locale_manifest_data);
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
        source_locale_path: english_locale_path.to_string_lossy().to_string(),
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

fn select_input_locale() -> PathBuf {
    loop {
        let input_locale_path: PathBuf =
            input_prompt("What is the name of the locale file you want to translate?").into();
        if !file_exists(&input_locale_path) {
            eprintln!("The file you specified does not exist. Please try again.");
            continue;
        }

        return input_locale_path;
    }
}

fn select_output_locale() -> PathBuf {
    loop {
        let output_locale_path: PathBuf =
            input_prompt("What should the output file be called?").into();
        if output_locale_path.exists() {
            eprintln!("The file you specified already exists. Please give it a different name.");
            continue;
        }

        return output_locale_path;
    }
}

fn parse_input_locale(input_path: &Path) -> JsonMap<String, JsonValue> {
    let Ok(input_locale_data) = std::fs::read_to_string(input_path) else {
        exit("Failed to open and read provided input file.");
    };

    let Ok(input_locale_obj) =
        serde_json::from_str::<JsonMap<String, JsonValue>>(&input_locale_data)
    else {
        exit("Failed to parse input file.");
    };

    input_locale_obj
}

fn select_target_language(deepl_context: &DeepLContext) -> Language {
    let available_target_langs = get_available_target_langs(deepl_context);
    let Ok(Some(selected_lang_index)) = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("What language do you want to translate to?")
        .items(&available_target_langs)
        .interact_opt()
    else {
        exit("Unknown error occurred with language selector.");
    };

    if selected_lang_index >= available_target_langs.len() {
        exit("Selected language index is out of bounds. This is a logic error, please report it.");
    }

    available_target_langs[selected_lang_index].clone()
}

fn write_locale_file(
    output_path: &Path,
    input_locale_data: &JsonMap<String, JsonValue>,
    translated_values: &[String],
) {
    // Create a new JSON object with the translated text
    let mut output_locale_json = JsonMap::new();
    for (i, key) in input_locale_data.keys().enumerate() {
        let translated_value = translated_values[i].clone();
        output_locale_json.insert(key.clone(), serde_json::Value::String(translated_value));
    }

    let output_locale_json = serde_json::Value::Object(output_locale_json);

    // Save the JSON object to an output file
    let Ok(mut output_locale_file) = File::create(&output_path) else {
        exit("Failed to create output file.");
    };

    let Ok(output_locale_json) = serde_json::to_string_pretty(&output_locale_json) else {
        exit("Failed to format output data.");
    };

    let Ok(_) = output_locale_file.write_all(output_locale_json.as_bytes()) else {
        exit("Failed to write data to output file.");
    };

    eprintln!("Output saved to {}.", output_path.to_string_lossy());
}

fn translate_locale(
    deepl_context: &DeepLContext,
    input_locale_data: &JsonMap<String, JsonValue>,
    target_language: Language,
) -> Vec<String> {
    let text_to_translate = TranslatableTextList {
        source_language: Some("EN".to_string()),
        target_language: target_language.code,
        texts: input_locale_data
            .values()
            .map(|v| {
                let Some(val_str) = v.as_str() else {
                    exit("Encountered non-string value in input JSON data.");
                };
                val_str.to_owned()
            })
            .collect(),
    };

    let Ok(translated_values) = deepl_context.api_connection.translate(
        Some(deepl_context.translation_options.clone()),
        text_to_translate,
    ) else {
        exit("Failed to translate values. This may be because of a connection issue with DeepL.");
    };

    if translated_values.len() != input_locale_data.keys().len() {
        exit("The number of translated values does not match the number of input values.");
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

fn write_appdata(manifest_data: LocaleManifest, locale_data: &JsonMap<String, JsonValue>) {
    let Ok(formatted_data) = toml::to_string_pretty(&manifest_data) else {
        exit("Unknown error occured when serializing manifest data.");
    };

    create_app_directory();

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

fn create_app_directory() {
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
