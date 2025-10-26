# ltranslate
A utility translating locale files using DeepL. **ltranslate** supports one-off translations, but
its main feature, "project mode", allows you to fully automate the translaton process to keep your
locale files up-to-date at all times.

## Installation
You'll need to [install Cargo](https://rust-lang.org/tools/install/), then run
`cargo install ltranslate`. You may have to manually add the Cargo binary folder to your PATH, but
if all goes well the installer will do so automatically.

## Usage
Start by [setting up a DeepL API account](https://www.deepl.com/en/signup) and [generating
an API key](https://www.deepl.com/en/your-account/keys). Copy the key and use it to set the
`DEEPL_API_KEY` environment variable.

### Basic ("translate") Mode
```sh
ltranslate translate <input file> <output file> [--language <language code>]
```
This will parse and translate the input file, writing it to the output file path. If `--language` is
not specified, or the provided language code is invalid, you will be prompted with a language
selector dialog.

The input file must meet two basic requirements:
1. It must be the English locale file. Other source languages are currently not supported.
2. It must be a JSON file with a single object that *only* contains simple key-value pairs, and
*all* values must be strings.

### Project Mode
> **WARNING:** *DO NOT EDIT ANYTHING IN THE `ltranslate/` DIRECTORY, AND DO NOT EDIT THE FOREIGN
> LOCALE FILES.* If you edit any of these files directly, you *will* corrupt your project, and you
> will have to revert to a previous Git version to fix it. This can cost API credit, so be careful.

Firstly, get your English locale file ready, and run this command to set up the project:
```sh
ltranslate project setup
```
You will be prompted to select one or more target languages, and to provide a file path for each of
them.

Now, when you make edits to your English locale file, run this command to retranslate and update all
the foreign locales:
```sh
ltranslate project update
```
This can take a few seconds to complete, depending on the size of your files, as DeepL's API does
not respond instantly.

If you need to change your project's settings, such as adding or removing languages, run this
command:
```sh
ltranslate project manage
```
You will be prompted with a selector to choose which setting you want to change.

It is generally recommended that you set up `ltranslate project update` to run on a regular basis,
using an auto-runner tool such as editor on-save actions or Git hooks. You can also set up a GitHub
Action or similar test to check commits for whether the source locale file has been edited since the
`update` command was last run by diffing it with the `ltranslate/source-history.json` file.

> **NOTE:** *Make sure to monitor your DeepL API usage to avoid running out of credit.*
