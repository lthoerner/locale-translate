use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use deepl_api::{DeepL, TranslatableTextList, TranslationOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::helper_functions::{self, exit};
use crate::interact;
use crate::{MANIFEST_PATH, SOURCE_LOCALE_HISTORY_PATH};

pub type LocaleData = JsonMap<String, JsonValue>;
// pub type LocaleJsonDataAll = BTreeMap<String, LocaleData>;

pub struct DeepLContext {
    pub api_connection: DeepL,
    pub translation_options: TranslationOptions,
    pub available_target_langs: Vec<Language>,
}

pub struct AppData {
    manifest: LocaleManifest,
    source_locale: LocaleDocument,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocaleManifestExternal {
    source_locale_path: PathBuf,
    locale_paths: BTreeMap<String, PathBuf>,
    language_names: BTreeMap<String, String>,
}

pub struct LocaleManifest {
    pub source_locale_path: PathBuf,
    pub locale_paths: BTreeMap<String, PathBuf>,
    pub languages: Vec<Language>,
}

pub struct LocaleDocuments {
    pub documents: Vec<LocaleDocument>,
}

pub struct LocaleDocument {
    pub data: LocaleData,
    pub language: Language,
    pub path: PathBuf,
}

#[derive(Clone, PartialEq)]
pub struct Language {
    pub code: String,
    pub name: String,
}

pub struct LocaleDataDiff {
    pub changed_or_added: LocaleData,
    pub removed: LocaleData,
}

pub struct LanguageDiff {
    pub added: Vec<Language>,
    pub removed: Vec<Language>,
}

impl AppData {
    pub fn new(manifest: LocaleManifest, source_locale: LocaleDocument) -> Self {
        AppData {
            manifest,
            source_locale,
        }
    }

    /// Write [`Self::manifest`] and [`Self::source_locale`] to their respective files.
    pub fn write_out(self) {
        helper_functions::create_app_directory_if_not_exists();
        self.manifest.write_out();
        self.source_locale
            .write_out(Some(PathBuf::from(SOURCE_LOCALE_HISTORY_PATH)));
    }
}

impl DeepLContext {
    pub fn connect() -> Self {
        let Ok(deepl_api_key) = std::env::var("DEEPL_API_KEY") else {
            exit(
                "DeepL API key was not found. Set it using the DEEPL_API_KEY environment variable.",
            );
        };

        let api_connection = DeepL::new(deepl_api_key);
        if !Self::valid_key(&api_connection) {
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

    pub fn get_target_language_if_available(&self, language_code: &str) -> Option<Language> {
        self.available_target_langs
            .iter()
            .find(|l| l.code == language_code)
            .cloned()
    }

    fn valid_key(api_connection: &DeepL) -> bool {
        api_connection.usage_information().is_ok()
    }
}

impl LocaleManifest {
    /// Get the current manifest data, if it exists.
    pub fn get_existing() -> Option<Self> {
        let data = std::fs::read_to_string(MANIFEST_PATH).ok()?;
        let Ok(manifest) = toml::from_str::<LocaleManifestExternal>(&data) else {
            exit("Failed to parse manifest file.");
        };

        Some(manifest.into())
    }

    /// Set up a new project by prompting the user, and return the manifest data.
    pub fn from_user_setup() -> Self {
        if LocaleManifest::get_existing().is_some() {
            exit(
                "Project has already been set up. To fully reset the project, remove the 'ltranslate' directory.",
            );
        }

        if !interact::confirm_prompt(
            "It looks like you're using ltranslate for the first time. Would you like to set up a new project in the current directory?",
        ) {
            exit("Setup canceled.");
        }

        if !interact::confirm_prompt("Do you have an English locale file ready to be translated?") {
            eprintln!("You will need an English locale file in order to set up ltranslate.");
            exit("Setup canceled.");
        }

        let english_locale_path = interact::select_source_locale();

        LocaleManifest {
            source_locale_path: english_locale_path,
            locale_paths: BTreeMap::new(),
            languages: Vec::new(),
        }
    }

    /// Write the manifest data into its file.
    pub fn write_out(self) {
        let manifest = LocaleManifestExternal::from(self);
        let Ok(formatted_data) = toml::to_string_pretty(&manifest) else {
            exit("Unknown error occured when serializing manifest data.");
        };

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
}

impl LocaleDocuments {
    /// Get all the existing locale file data.
    ///
    /// This will skip any files which appear in the manifest but that have not been created yet.
    /// Thus, the caller must ensure all the necessary files exist before calling this function.
    pub fn get_existing(manifest_data: &LocaleManifest) -> Self {
        let documents = manifest_data
            .languages
            .iter()
            .filter_map(|l| LocaleDocument::from_language(manifest_data, l.clone()))
            .collect();

        LocaleDocuments { documents }
    }

    /// Write all [`LocaleDocument`]s from [`Self::documents`] to their respective files.
    pub fn write_out(self) {
        self.documents.into_iter().for_each(|d| d.write_out(None));
    }
}

impl LocaleDocument {
    /// Get a [`LocaleDocument`] from the source locale history file, as specified by
    /// [`SOURCE_LOCALE_HISTORY_PATH`].
    pub fn source_history() -> Option<Self> {
        let history_path = PathBuf::from(SOURCE_LOCALE_HISTORY_PATH);
        Some(LocaleDocument {
            data: Self::parse_data_from_file(&history_path)?,
            language: Language::english(),
            path: history_path,
        })
    }

    /// Get a [`LocaleDocument`] from the source locale file, as specified by
    /// [`LocaleManifest::source_locale_path`].
    pub fn source(manifest_data: &LocaleManifest) -> Option<Self> {
        Some(LocaleDocument {
            data: Self::parse_data_from_file(&manifest_data.source_locale_path)?,
            language: Language::english(),
            path: manifest_data.source_locale_path.clone(),
        })
    }

    /// Get a [`LocaleDocument`] from an existing locale file, using
    /// [`LocaleManifest::locale_paths`] to identify the path.
    pub fn from_language(manifest_data: &LocaleManifest, language: Language) -> Option<Self> {
        let Some(path) = manifest_data.locale_paths.get(&language.code).cloned() else {
            exit(&format!(
                "Missing locale path for language '{}' in manifest.",
                language.code
            ));
        };

        Some(LocaleDocument {
            data: Self::parse_data_from_file(&path)?,
            language,
            path,
        })
    }

    /// Translate a [`LocaleDocument`] into a given language.
    ///
    /// Before calling this function, the language must be enabled, and the path must be present in
    /// [`LocaleManifest::locale_paths`],
    pub fn translate_full(
        deepl_context: &DeepLContext,
        manifest_data: &LocaleManifest,
        source_document: &LocaleDocument,
        source_text: &[String],
        language: Language,
    ) -> Self {
        let Some(path) = manifest_data.locale_paths.get(&language.code).cloned() else {
            exit(&format!(
                "Could not find path for locale '{}' in the manifest.",
                language.code
            ));
        };

        let translated_data = LocaleDocument::translate_data(
            deepl_context,
            &source_document.data,
            source_text,
            &language,
        );

        LocaleDocument {
            data: translated_data,
            language,
            path,
        }
    }

    /// Retranslate a [`LocaleDocument`] into its given language, only translating values that have
    /// been created, updated, or deleted in the source locale file.
    ///
    /// The source locale history file must exist for this function to work.
    // TODO: Probably DI source data
    fn update_translations(
        &mut self,
        deepl_context: &DeepLContext,
        manifest_data: &LocaleManifest,
    ) {
        let (Some(source_document_history), Some(source_document_current)) = (
            LocaleDocument::source_history(),
            LocaleDocument::source(manifest_data),
        ) else {
            exit("Missing source locale or source locale history file.");
        };

        let Some(diff) =
            LocaleDataDiff::diff(&source_document_history.data, &source_document_current.data)
        else {
            return;
        };

        let changed_or_added_text = LocaleDocument::get_raw_text_data(&diff.changed_or_added);
        let translated_data = LocaleDocument::translate_data(
            deepl_context,
            &diff.changed_or_added,
            &changed_or_added_text,
            &self.language,
        );

        self.remove_dead_entries(diff.removed);
        self.update_entries(translated_data);
    }

    /// Translate a [`LocaleData`] map into a given language.
    ///
    /// This uses [`LocaleData`] in order to accommodate usage in both full and partial
    /// translations, without being too internally complex. It is up to the caller to determine what
    /// values should be translated, and to merge translated data into a [`LocaleDocument`] as
    /// needed.
    fn translate_data(
        deepl_context: &DeepLContext,
        source_data: &LocaleData,
        source_text: &[String],
        language: &Language,
    ) -> LocaleData {
        if source_data.len() != source_text.len() {
            exit(
                "The number of locale data entries does not match the number of raw text entries.",
            );
        }

        let text_to_translate = TranslatableTextList {
            source_language: Some("EN".to_string()),
            target_language: language.code.clone(),
            texts: source_text.to_owned(),
        };

        let Ok(translated_data) = deepl_context.api_connection.translate(
            Some(deepl_context.translation_options.clone()),
            text_to_translate,
        ) else {
            exit(
                "Failed to translate values. This may be because of a connection issue with DeepL.",
            );
        };

        if translated_data.len() != source_text.len() {
            exit("The number of translated values does not match the number of source values.");
        }

        source_data
            .keys()
            .enumerate()
            .map(|(i, k)| {
                (
                    k.clone(),
                    JsonValue::String(translated_data[i].text.clone()),
                )
            })
            .collect()
    }

    /// Parse the [`LocaleJsonData`] from the file at the given path.
    ///
    /// If the file is missing, returns [`None`]. This usually happens because a language has been
    /// added but a locale file has not yet been generated.
    fn parse_data_from_file(path: &Path) -> Option<LocaleData> {
        let locale_data = std::fs::read_to_string(path).ok()?;
        let Ok(locale_data) = serde_json::from_str::<LocaleData>(&locale_data) else {
            exit("Failed to parse locale file.");
        };

        Some(locale_data)
    }

    /// Remove a given list of entries from the [`LocaleDocument::data`].
    fn remove_dead_entries(&mut self, to_remove: LocaleData) {
        to_remove.keys().for_each(|k| {
            let Some(_) = self.data.remove(k) else {
                exit(&format!(
                    "Failed to remove key '{}' from locale '{}'.",
                    k, self.language.code
                ));
            };
        });
    }

    /// Update a given list of entries in the [`LocaleDocument::data`].
    fn update_entries(&mut self, to_update: LocaleData) {
        to_update.into_iter().for_each(|(k, v)| {
            self.data.insert(k, v);
        });
    }

    /// Get a [`Vec<String>`] representing all values from [`Self::data`].
    ///
    /// This is used to prevent repeated cloning when having to translate one document multiple
    /// times.
    pub fn get_raw_text_data<'a>(data: impl Into<&'a LocaleData>) -> Vec<String> {
        data.into()
            .values()
            .clone()
            .map(|v| {
                v.as_str()
                    .unwrap_or_else(|| exit("Encountered non-string value in source locale data."))
                    .to_owned()
            })
            .collect()
    }

    /// Write the [`LocaleDocument`] to a file using its given path, or a different path if
    /// specified.
    pub fn write_out(self, override_path: Option<PathBuf>) {
        let path = override_path.unwrap_or(self.path);

        let Ok(mut locale_file) = File::create(&path) else {
            exit("Failed to create output file.");
        };

        let Ok(locale_data) = serde_json::to_string_pretty(&self.data) else {
            exit("Failed to format output data.");
        };

        let Ok(_) = locale_file.write_all(locale_data.as_bytes()) else {
            exit("Failed to write data to output file.");
        };
    }
}

impl Language {
    fn new(code: &str, name: &str) -> Self {
        Language {
            code: code.to_owned(),
            name: name.to_owned(),
        }
    }

    fn english() -> Self {
        Language {
            code: "EN".to_owned(),
            name: "English".to_owned(),
        }
    }
}

impl LocaleDataDiff {
    pub fn diff(original: &LocaleData, current: &LocaleData) -> Option<Self> {
        if original == current {
            return None;
        }

        let changed_or_added = current
            .iter()
            .filter(|(k, v)| original.get(*k).map_or(true, |old_v| old_v != *v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<LocaleData>();

        let removed = original
            .iter()
            .filter(|(k, _)| !current.contains_key(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<LocaleData>();

        if changed_or_added.is_empty() && removed.is_empty() {
            return None;
        }

        Some(LocaleDataDiff {
            changed_or_added,
            removed,
        })
    }
}

impl LanguageDiff {
    pub fn diff(original: &[Language], current: &[Language]) -> Option<Self> {
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
}

impl ToString for &Language {
    fn to_string(&self) -> String {
        format!("{} ({})", self.code, self.name)
    }
}

impl From<LocaleManifestExternal> for LocaleManifest {
    fn from(value: LocaleManifestExternal) -> Self {
        let LocaleManifestExternal {
            source_locale_path,
            locale_paths,
            language_names,
        } = value;

        LocaleManifest {
            source_locale_path,
            locale_paths,
            languages: language_names
                .iter()
                .map(|(c, n)| Language::new(c, n))
                .collect(),
        }
    }
}

impl From<LocaleManifest> for LocaleManifestExternal {
    fn from(value: LocaleManifest) -> Self {
        let LocaleManifest {
            source_locale_path,
            locale_paths,
            languages,
        } = value;

        LocaleManifestExternal {
            source_locale_path,
            locale_paths,
            language_names: languages.into_iter().map(|l| (l.code, l.name)).collect(),
        }
    }
}

impl<'a> From<&'a LocaleDocument> for &'a LocaleData {
    fn from(value: &'a LocaleDocument) -> Self {
        &value.data
    }
}
