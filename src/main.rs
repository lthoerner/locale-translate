use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use deepl_api::{DeepL, TranslatableTextList, TranslationOptions};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input};
use serde::{Deserialize, Serialize};
use soft_canonicalize::soft_canonicalize;

#[derive(Serialize, Deserialize)]
struct LocaleManifest(LocaleFileLocations);

#[derive(Serialize, Deserialize, Default)]
#[serde(rename = "locale_files")]
struct LocaleFileLocations {
    ar: Option<String>,
    bg: Option<String>,
    cs: Option<String>,
    da: Option<String>,
    de: Option<String>,
    el: Option<String>,
    en_gb: Option<String>,
    en_us: Option<String>,
    es: Option<String>,
    es_419: Option<String>,
    et: Option<String>,
    fi: Option<String>,
    fr: Option<String>,
    hu: Option<String>,
    id: Option<String>,
    it: Option<String>,
    ja: Option<String>,
    ko: Option<String>,
    lt: Option<String>,
    lv: Option<String>,
    nb: Option<String>,
    nl: Option<String>,
    pl: Option<String>,
    pt_br: Option<String>,
    pt_pt: Option<String>,
    ro: Option<String>,
    ru: Option<String>,
    sk: Option<String>,
    sl: Option<String>,
    sv: Option<String>,
    tr: Option<String>,
    uk: Option<String>,
    zh: Option<String>,
    zh_hans: Option<String>,
    zh_hant: Option<String>,
}

fn main() {
    let locale_manifest_data = match std::fs::read_to_string("./locale_manifest.toml") {
        // If the manifest file already exists, read and parse it
        Ok(data) => {
            let Ok(locale_manifest) = toml::from_str::<LocaleManifest>(&data) else {
                exit("Faield to parse manifest file.");
            };

            locale_manifest
        }
        // If the manifest file does not exist, set up a new project
        Err(_) => {
            if !confirm_prompt(
                "It looks like you're using locale-translate for the first time. Would you like to set up a new project in the current directory?",
            ) {
                exit("Setup canceled.");
            }

            if !confirm_prompt("Do you have an English locale file ready to be translated?") {
                eprintln!(
                    "You will need an English locale file in order to set up locale-translate."
                );
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

            LocaleManifest(LocaleFileLocations {
                en_us: Some(english_locale_path.to_string_lossy().to_string()),
                ..Default::default()
            })
        }
    };

    // Set up DeepL API connection
    let Ok(deepl_api_key) = std::env::var("DEEPL_API_KEY") else {
        exit("DeepL API key was not found. Set it using the DEEPL_API_KEY environment variable.");
    };

    let deepl_api_connection = DeepL::new(deepl_api_key);
    if !valid_deepl_api_key(&deepl_api_connection) {
        exit("Provided DeepL API key is invalid.");
    }

    let translation_settings = TranslationOptions {
        split_sentences: None,
        preserve_formatting: Some(true),
        formality: None,
        glossary_id: None,
    };

    // Select the target language
    let available_target_langs = get_available_target_langs(&deepl_api_connection);
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

    let target_language = available_target_langs[selected_lang_index].code.clone();

    // Select the locale file being used
    let input_locale_path = loop {
        let input_locale_path: PathBuf =
            input_prompt("What is the name of the locale file you want to translate?").into();
        if !file_exists(&input_locale_path) {
            eprintln!("The file you specified does not exist. Please try again.");
            continue;
        }

        break input_locale_path;
    };

    // Select the output file
    let output_locale_path = loop {
        let output_locale_path: PathBuf =
            input_prompt("What should the output file be called?").into();
        if output_locale_path.exists() {
            eprintln!("The file you specified already exists. Please give it a different name.");
            continue;
        }

        break output_locale_path;
    };

    // Read and parse the provided locale file
    let Ok(input_locale_data) = std::fs::read_to_string(input_locale_path) else {
        exit("Failed to open and read provided input file.");
    };

    let Ok(input_locale_json) = serde_json::from_str::<serde_json::Value>(&input_locale_data)
    else {
        exit("Failed to parse input file.");
    };

    let Some(input_locale_json) = input_locale_json.as_object() else {
        exit("Failed to parse input file JSON as object.");
    };

    let mut output_locale_json = serde_json::Map::new();

    // Prepare the text (JSON string values) for translation
    let values_to_translate = TranslatableTextList {
        source_language: Some("EN".to_string()),
        target_language,
        texts: input_locale_json
            .values()
            .map(|v| {
                let Some(val_str) = v.as_str() else {
                    exit("Encountered non-string value in input JSON data.");
                };
                val_str.to_owned()
            })
            .collect(),
    };

    // Check with user before continuing to avoid wasting API credit
    if !confirm_prompt("Are you sure you want to translate this file?") {
        exit("Translation canceled.");
    }

    // Translate the text
    let Ok(translated_values) =
        deepl_api_connection.translate(Some(translation_settings), values_to_translate)
    else {
        exit("Failed to translate values. This may be because of a connection issue with DeepL.");
    };

    if translated_values.len() != input_locale_json.keys().len() {
        exit("The number of translated values does not match the number of input values.");
    }

    eprintln!("Translation complete! Writing output data to file...");

    // Create a new JSON object with the translated text
    for (i, key) in input_locale_json.keys().enumerate() {
        let translated_value = translated_values[i].text.clone();
        output_locale_json.insert(key.clone(), serde_json::Value::String(translated_value));
    }

    let output_locale_json = serde_json::Value::Object(output_locale_json);

    // Save the JSON object to an output file
    let Ok(mut output_locale_file) = File::create(&output_locale_path) else {
        exit("Failed to create output file.");
    };

    let Ok(output_locale_json) = serde_json::to_string_pretty(&output_locale_json) else {
        exit("Failed to format output data.");
    };

    let Ok(_) = output_locale_file.write_all(output_locale_json.as_bytes()) else {
        exit("Failed to write data to output file.");
    };

    eprintln!("Output saved to {}.", output_locale_path.to_string_lossy());
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

fn get_available_target_langs(deepl_api_connection: &DeepL) -> Vec<Language> {
    let Ok(languages) = deepl_api_connection.target_languages() else {
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

fn write_manifest(data: LocaleManifest) {
    let Ok(mut manifest_file) = File::create("./locale_manifest.toml") else {
        todo!()
    };

    let Ok(formatted_data) = toml::to_string_pretty(&data) else {
        todo!()
    };

    let Ok(_) = manifest_file.write_all(formatted_data.as_bytes()) else {
        todo!()
    };
}

fn exit(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

struct Language {
    code: String,
    name: String,
}

impl ToString for &Language {
    fn to_string(&self) -> String {
        format!("{} ({})", self.code, self.name)
    }
}
