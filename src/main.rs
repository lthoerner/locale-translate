mod helper_functions;
mod interact;
mod types;

use std::path::PathBuf;

use clap::{Arg, Command};
use dialoguer::Select;
use dialoguer::theme::ColorfulTheme;

use helper_functions::{exit, file_exists};
use types::{
    DeepLContext, Language, LanguageDiff, LocaleData, LocaleDataDiff, LocaleDocument,
    LocaleDocuments, LocaleManifest,
};

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
                "setup" => {
                    let mut manifest_data = LocaleManifest::from_user_setup();
                    let target_languages = interact::select_target_languages(&deepl, None);
                    interact::select_output_locale_all(&target_languages)
                        .into_iter()
                        .for_each(|(lang, path)| {
                            let _ = manifest_data.locale_paths.insert(lang, path);
                        });

                    if !interact::confirm_prompt(
                        "Are you sure you want to translate these file(s)?",
                    ) {
                        exit("Translation canceled.");
                    }

                    let Some(source_locale_data) = LocaleDocument::source(&manifest_data) else {
                        exit(
                            "Missing source locale data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
                        );
                    };

                    let source_locale_text = source_locale_data.get_raw_text_data();

                    eprintln!("Translation in progress. Please wait...");
                    full_translate_all(
                        &deepl,
                        &manifest_data,
                        &source_locale_data,
                        &source_locale_text,
                    );

                    eprintln!("Translation complete!");

                    write_appdata(manifest_data, Some(source_locale_data));
                }
                "manage" => {
                    let Some(mut manifest_data) = LocaleManifest::get_existing() else {
                        exit(
                            "Missing project data. Ensure you are in the correct working directory and run 'ltranslate project setup' to install ltranslate into your project if necessary.",
                        );
                    };

                    let deepl = DeepLContext::connect();

                    let target_setting = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("What setting would you like to change?")
                        .items(&["source locale path", "enabled languages"])
                        .interact();

                    match target_setting {
                        Ok(0) => {
                            manifest_data.source_locale_path = interact::select_source_locale();
                            write_appdata(manifest_data, None);
                        }
                        Ok(1) => {
                            let source_locale_data =
                                parse_locale(&manifest_data.source_locale_path);
                            let source_locale_text = get_locale_values(&source_locale_data);

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

                            write_appdata(manifest_data.clone(), None);
                            full_translate_new(
                                &deepl,
                                &manifest_data,
                                &source_locale_data,
                                &source_locale_text,
                            );
                        }
                        _ => exit("Unknown error occurred with the setting selector."),
                    }
                }
                "update" => {
                    // TODO: Full translate all new files and exclude them from partial translation step

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

                    let current_locale_data_all = get_existing_locale_documents(&manifest_data);
                    let mut new_locale_data_all =
                        remove_dead_keys_all(&diff.removed, &current_locale_data_all);

                    if !diff.changed_or_added.is_empty() {
                        let changed_added_locale_data = &diff.changed_or_added;
                        let changed_added_locale_text = get_locale_values(&diff.changed_or_added);

                        let updated_translation_locale_data_all = translate_locale_all(
                            &deepl,
                            changed_added_locale_data,
                            &changed_added_locale_text,
                            enabled_langs,
                        );

                        update_changed_or_added_keys_all(
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
            simple_translate_interactive(&deepl, input_file, output_file, target_language);
        }
        _ => exit("Unknown subcommand. This is likely a logic bug."),
    }
}

/// Translate all locales in the manifest, including ones that may already exist.
///
/// "Full" refers to the entire source file being retranslated, rather than only the values that
/// have changed.
fn full_translate_all(
    deepl_context: &DeepLContext,
    manifest_data: &LocaleManifest,
    source_locale_data: &LocaleData,
    source_locale_text: &[String],
) {
    manifest_data
        .enabled_languages(&deepl_context.available_target_langs)
        .into_iter()
        .for_each(|l| {
            let translated_data = translate_locale(
                deepl_context,
                source_locale_data,
                source_locale_text,
                l.clone(),
            );

            let Some(locale_path) = manifest_data.locale_paths.get(&l.code) else {
                exit(&format!(
                    "Could not locate path for locale '{}'. This is likely a logic bug.",
                    l.code
                ));
            };

            write_locale_file(locale_path, translated_data);
        });
}

/// Translate all locales in the manifest which do not already exist as files. Note that this will
/// not target any locale which has a file, even if the file is incomplete, out-of-date, or
/// incorrectly-formatted.
///
/// "Full" refers to the entire source file being retranslated, rather than only the values that
/// have changed.
fn full_translate_new(
    deepl_context: &DeepLContext,
    manifest_data: &LocaleManifest,
    source_locale_data: &LocaleData,
    source_locale_text: &[String],
) {
    manifest_data
        .enabled_languages(&deepl_context.available_target_langs)
        .into_iter()
        .for_each(|l| {
            let Some(locale_path) = manifest_data.locale_paths.get(&l.code) else {
                exit(&format!(
                    "Could not locate path for locale '{}'. This is likely a logic bug.",
                    l.code
                ));
            };

            if !file_exists(locale_path) {
                let translated_data = translate_locale(
                    deepl_context,
                    source_locale_data,
                    source_locale_text,
                    l.clone(),
                );

                let Some(locale_path) = manifest_data.locale_paths.get(&l.code) else {
                    exit(&format!(
                        "Could not locate path for locale '{}'. This is likely a logic bug.",
                        l.code
                    ));
                };

                write_locale_file(locale_path, translated_data);
            }
        });
}

/// Fully translate all locales in the manifest which do not already exist as files, then partially
/// translate all previously-existing locales.
///
/// "Full" refers to the entire source file being retranslated, rather than only the values that
/// have changed. "Partial" refers to retranslating only the values that have changed.
fn update_all_locales(deepl_context: &DeepLContext, manifest_data: &LocaleManifest) {
    let source_locale_data = parse_locale(&manifest_data.source_locale_path);
    let source_locale_text = get_locale_values(&source_locale_data);

    full_translate_new(
        deepl_context,
        manifest_data,
        &source_locale_data,
        &source_locale_text,
    );
}

/// Partially translate a given locale.
///
/// "Partial" refers to retranslating only the values that have changed.
fn partial_translate_all(
    deepl_context: &DeepLContext,
    manifest_data: &LocaleManifest,
    documents: &LocaleDocuments,
    diff: &LocaleDataDiff,
) {
    // get deleted diff
    // get changed/added diff

    // For each document
    //  remove deleted values
    //  translate changed/added lines
    //  merge changed/added lines back into working document
    //  write document
}

/// Translate a single specified locale and write the translation to an output file.
///
/// This function can be provided with a `target_language` value to avoid opening the language
/// selector prompt.
fn simple_translate_interactive(
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

    simple_translate_noninteractive(deepl_context, input_file, output_file, target_language);
    eprintln!("Translation complete. Output has been written to file.");
}

/// Translate a single specified locale and write the translation to an output file.
///
/// This function is noninteractive, so it does not prompt the user for any information. As such,
/// all relevant information must be passed in.
fn simple_translate_noninteractive(
    deepl_context: &DeepLContext,
    input_file: PathBuf,
    output_file: PathBuf,
    target_language: Language,
) {
    let input_locale = parse_locale(&input_file);
    let translated_data = translate_locale(
        &deepl_context,
        &input_locale,
        &get_locale_values(&input_locale),
        target_language,
    );

    write_locale_file(&output_file, translated_data);
}
