use std::collections::BTreeMap;
use std::path::PathBuf;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Select};

use crate::helper_functions::{exit, file_exists};
use crate::types::{DeepLContext, Language};

pub enum ProjectSetting {
    EditSourcePath,
    EditLangugages,
}

impl ToString for ProjectSetting {
    fn to_string(&self) -> String {
        match self {
            ProjectSetting::EditSourcePath => "source locale path".to_owned(),
            ProjectSetting::EditLangugages => "enabled languages".to_owned(),
        }
    }
}

pub fn select_project_setting() -> ProjectSetting {
    let Ok(setting_index) = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What setting would you like to change?")
        .items([
            ProjectSetting::EditSourcePath,
            ProjectSetting::EditLangugages,
        ])
        .interact()
    else {
        exit("Unknown error occurred with the settings selector.");
    };

    match setting_index {
        0 => ProjectSetting::EditSourcePath,
        1 => ProjectSetting::EditLangugages,
        _ => exit("Unknown error occurred with the settings selector."),
    }
}

pub fn select_target_language(deepl_context: &DeepLContext) -> Language {
    let Ok(lang_index) = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("What language do you want to translate to?")
        .items(&deepl_context.available_target_langs)
        .interact()
    else {
        exit("Unknown error occurred with language selector.")
    };

    deepl_context.available_target_langs[lang_index].clone()
}

pub fn select_target_languages(
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

pub fn select_source_locale() -> PathBuf {
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

pub fn select_output_locale_all(target_languages: &[Language]) -> BTreeMap<String, PathBuf> {
    target_languages
        .iter()
        .map(|l| (l.code.clone(), select_output_locale(l)))
        .collect()
}

pub fn select_output_locale(target_language: &Language) -> PathBuf {
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

pub fn confirm_prompt(prompt_text: &str) -> bool {
    let Ok(response) = Confirm::new().with_prompt(prompt_text).interact() else {
        exit("Unknown error occurred with the confirmation prompt.");
    };

    response
}

pub fn input_prompt(prompt_text: &str) -> String {
    let Ok(response) = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt_text)
        .interact_text()
    else {
        exit("Unknown error occurred with the input prompt.");
    };

    response
}
