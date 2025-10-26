mod helper_functions;
mod interact;
mod types;

use std::path::PathBuf;

use clap::{Arg, Command};

use helper_functions::exit;
use types::{DeepLContext, LanguageDiff, LocaleDataDiff, LocaleDocument, LocaleManifest};

use crate::{interact::ProjectSetting, types::AppData};

const APP_DIR_PATH: &str = "./ltranslate";
const MANIFEST_PATH: &str = "./ltranslate/manifest.toml";
const SOURCE_LOCALE_HISTORY_PATH: &str = "./ltranslate/source-history.json";

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

    let deepl = DeepLContext::connect();

    let Some((subcommand_name, subcommand_args)) = args.subcommand() else {
        exit("Missing subcommand. This is likely a logic bug.");
    };

    match subcommand_name {
        "project" => {
            let Some((project_sub, _project_args)) = subcommand_args.subcommand() else {
                exit("Missing subcommand. This is likely a logic bug.");
            };

            match project_sub {
                "setup" => set_up_project(&deepl),
                "manage" => manage_project(&deepl),
                "update" => update_project(&deepl),
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
            translate_interactive(&deepl, input_file, output_file, target_language);
        }
        _ => exit("Unknown subcommand. This is likely a logic bug."),
    }
}

/// Prompt the user to set up the project, run initial translations, and write the app data to its
/// directory.
fn set_up_project(deepl_context: &DeepLContext) {
    let mut manifest_data = LocaleManifest::from_user_setup();
    let target_languages = interact::select_target_languages(deepl_context, None);

    interact::select_output_locale_all(&target_languages)
        .into_iter()
        .for_each(|(lang, path)| {
            let _ = manifest_data.locale_paths.insert(lang, path);
        });
    target_languages
        .iter()
        .for_each(|l| manifest_data.languages.push(l.clone()));

    if !interact::confirm_prompt("Are you sure you want to translate these file(s)?") {
        exit("Translation canceled.");
    }

    let Some(source_document) = LocaleDocument::source(&manifest_data) else {
        exit(
            "Missing source locale data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
        );
    };

    eprintln!("Translation in progress. Please wait...");
    let source_text = LocaleDocument::get_raw_text_data(&source_document);
    for lang in target_languages {
        manifest_data.languages.push(lang.clone());
        LocaleDocument::translate_full(
            deepl_context,
            &manifest_data,
            &source_document,
            &source_text,
            lang.clone(),
        )
        .write_out(None);

        eprintln!("Successfully translated locale '{}'.", lang.code,);
    }

    eprintln!("All translations complete! Writing app data...");
    AppData::new(manifest_data, source_document).write_out();
    eprintln!("App data written successfully.");
    eprintln!(
        "WARNING: DO NOT EDIT THE MANIFEST OR FOREING LOCALES DIRECTLY! If you edit anything other than the English locale file directly, you will corrupt your project. Use 'ltranslate project manage' for changing settings."
    );
}

/// Allow the user to change a project setting.
fn manage_project(deepl_context: &DeepLContext) {
    let Some(mut manifest_data) = LocaleManifest::get_existing() else {
        exit(
            "Missing project data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
        );
    };

    let target_setting = interact::select_project_setting();
    match target_setting {
        ProjectSetting::EditSourcePath => {
            manifest_data.source_locale_path = interact::select_source_locale();
            manifest_data.write_out();
        }
        ProjectSetting::EditLangugages => {
            let (Some(source_document_history), Some(source_document_current)) = (
                LocaleDocument::source_history(),
                LocaleDocument::source(&manifest_data),
            ) else {
                exit("Missing source locale or source locale history file.");
            };

            if LocaleDataDiff::diff(&source_document_history.data, &source_document_current.data)
                .is_some()
            {
                exit(
                    "Language list cannot be edited after changes have been made to the source locale file. Please update all translations using 'ltranslate project update' and try again.",
                );
            }

            let enabled_languages = &manifest_data.languages;
            let selected_languages =
                interact::select_target_languages(deepl_context, Some(enabled_languages));

            let diff = LanguageDiff::diff(enabled_languages, &selected_languages);
            if let Some(diff) = diff {
                manifest_data.remove_languages(&diff.removed);
                if !diff.removed.is_empty() {
                    eprintln!(
                        "It looks like you've removed one or more languages. Note that the files are not deleted automatically, so if you wish to delete them, remember to do so."
                    );
                }

                let source_text = LocaleDocument::get_raw_text_data(&source_document_current);
                for added_lang in diff.added {
                    manifest_data.locale_paths.insert(
                        added_lang.code.clone(),
                        interact::select_output_locale(&added_lang),
                    );
                    manifest_data.languages.push(added_lang.clone());

                    LocaleDocument::translate_full(
                        deepl_context,
                        &manifest_data,
                        &source_document_current,
                        &source_text,
                        added_lang,
                    )
                    .write_out(None);
                }
            }

            manifest_data.write_out();
        }
    }
}

/// Update all foreign locale files based on any edits made to the source file.
fn update_project(deepl_context: &DeepLContext) {
    let Some(manifest_data) = LocaleManifest::get_existing() else {
        exit(
            "Missing project data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
        );
    };

    let (Some(source_document_history), Some(source_document_current)) = (
        LocaleDocument::source_history(),
        LocaleDocument::source(&manifest_data),
    ) else {
        exit("Missing source locale or source locale history file.");
    };

    let Some(diff) =
        LocaleDataDiff::diff(&source_document_history.data, &source_document_current.data)
    else {
        return;
    };

    let enabled_languages = &manifest_data.languages;
    for lang in enabled_languages {
        let Some(mut locale_document) = LocaleDocument::from_language(&manifest_data, lang.clone())
        else {
            exit(&format!(
                "Missing locale file for language '{}'.",
                lang.code
            ));
        };

        locale_document.update_translations(deepl_context, &diff);
        locale_document.write_out(None);
    }

    AppData::new(manifest_data, source_document_current).write_out();
}

/// Translate a single specified locale and write the translation to an output file.
///
/// This function can be provided with a `target_language` value to avoid opening the language
/// selector prompt.
fn translate_interactive(
    deepl_context: &DeepLContext,
    input_file: PathBuf,
    output_file: PathBuf,
    target_language: Option<String>,
) {
    let target_language = match target_language {
        Some(language_code) => deepl_context
            .available_target_langs
            .iter()
            .find(|l| l.code == language_code)
            .cloned()
            .unwrap_or_else(|| interact::select_target_language(deepl_context)),
        None => interact::select_target_language(deepl_context),
    };

    if !interact::confirm_prompt("Are you sure you want to translate this file?") {
        exit("Translation canceled.");
    }

    let Some(source_data) = LocaleDocument::parse_data_from_file(&input_file) else {
        exit("Missing input file. This is likely a logic bug.");
    };

    LocaleDocument::translate_full_direct(
        deepl_context,
        &source_data,
        target_language,
        output_file,
    )
    .write_out(None);

    eprintln!("Translation complete. Output has been written to file.");
}
